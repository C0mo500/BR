//! BR's control channel: a loopback JSON-lines server through which an agent works in the window
//! the user sees (`br --control`, `br mcp --connect`): one `{"id","method","params"}` line in, one
//! `{"id","ok","result"|"error"}` line out — LightCraft's and FilmCraft's control protocol.
//!
//! Every connection starts with `{"method":"auth","params":{"token":…}}`: the token is made at
//! each start and written, readable only by the user, next to the window's settings
//! ([`write_info`]), so other users of the computer (and web pages, which cannot send it) are
//! refused. Then each request goes where it belongs:
//!
//! - BR's own commands (`br.*` photo commands, `br.video.*`, FilmCraft's commands as
//!   `film.<id>` through `engine.execute`, a video frame through `ui.render`, `br.page`,
//!   `br.inspect`) run on BR's side of the window ([`handle`]);
//! - `film:<method>` goes to FilmCraft's own control handler (its `ui.inspect`, `ui.screenshot`,
//!   `ui.clickWidget` …, without the prefix);
//! - everything else to LightCraft's (its `engine.execute`, `ui.render`, `ui.click`, `ui.key`,
//!   `ui.screenshot` of the whole window …).
//!
//! The line handling is LightCraft's (`apps/lightcraft/src/control_server.rs` in
//! storytold/lightcraft, MIT OR Apache-2.0): bounded lines and connections, junk closes the
//! connection, a request for a window that draws no frames is given up instead of run late.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use lightcraft_ui_egui::ControlRequest;
use serde_json::{Value, json};

use crate::{BrApp, Page};

/// Longest accepted request line (bytes, without the newline).
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// Connections served at once; further ones get an error line and are closed.
pub const MAX_CONNECTIONS: usize = 16;
/// How long a request may take once the window runs it (a video export waits for its file).
const LONG_REPLY: Duration = Duration::from_secs(3 * 60 * 60);
/// How long LightCraft's and FilmCraft's own requests may take.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);
/// A request the window has not picked up while it drew no frame for this long is given up.
const NO_FRAMES_AFTER: Duration = Duration::from_secs(5);

/// The reply for a request given up because the window drew no frames.
pub const HIDDEN_WINDOW_ERROR: &str = "the BR window drew no frame for 5 s, so the request was not run: it is probably minimized or \
on a hidden workspace (such a window is not redrawn). Bring it on screen and send the request again";

/// `film.<id>` and `ui.render` of a video frame: see `br_mcp::VIDEO_FRAME`.
pub const VIDEO_FRAME: u64 = 1 << 52;

/// The three receivers a window drains: LightCraft's, FilmCraft's, BR's.
pub struct Channels {
    pub photo: Receiver<ControlRequest>,
    pub video: Receiver<filmcraft_ui_egui::control::ControlRequest>,
    pub br: Receiver<ControlRequest>,
}

struct Senders {
    photo: Sender<ControlRequest>,
    video: Sender<filmcraft_ui_egui::control::ControlRequest>,
    br: Sender<ControlRequest>,
}

/// Where a request goes.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    Br,
    /// FilmCraft's handler, with the method it gets.
    Video(String),
    Photo,
}

/// Where `method` with `params` goes.
pub fn route(method: &str, params: &Value) -> Target {
    if let Some(m) = method.strip_prefix("film:") {
        return Target::Video(m.to_string());
    }
    if method.starts_with("br.") {
        return Target::Br;
    }
    if matches!(method, "engine.execute" | "ui.menu.invoke" | "command")
        && let Some(id) = params.get("command").or(params.get("id")).and_then(Value::as_str)
        && (id.starts_with("br.") || id.starts_with("film."))
    {
        return Target::Br;
    }
    if method == "ui.render" && params.get("id").and_then(Value::as_u64).is_some_and(|i| i >= VIDEO_FRAME) {
        return Target::Br;
    }
    Target::Photo
}

/// A fresh secret for a window's control channel.
pub fn new_token() -> String {
    let mut bytes = [0u8; 24];
    if getrandom::fill(&mut bytes).is_err() {
        // no system randomness: the clock and the process are still unknown to a page or user
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = ((t >> ((i % 16) * 8)) as u8) ^ (std::process::id() as u8).wrapping_mul(i as u8 + 1);
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The file `br mcp --connect` reads the port and token from.
pub fn info_path(config_dir: &Path) -> PathBuf {
    config_dir.join("control.json")
}

/// Write `{port, token, pid}` for `br mcp --connect`, readable only by the user.
pub fn write_info(path: &Path, port: u16, token: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = json!({"port": port, "token": token, "pid": std::process::id()}).to_string();
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&tmp)?;
    f.write_all(body.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// Start listening on 127.0.0.1:`port` (0: a free one). Returns the port and what the window
/// drains each frame.
pub fn start(port: u16, token: String, ctx: egui::Context) -> std::io::Result<(u16, Channels)> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    let (photo_tx, photo) = channel();
    let (video_tx, video) = channel();
    let (br_tx, br) = channel();
    let senders = Senders { photo: photo_tx, video: video_tx, br: br_tx };
    log::info!("control channel on 127.0.0.1:{port}");
    std::thread::Builder::new().name("br-control".into()).spawn(move || accept_loop(listener, senders, token, ctx))?;
    Ok((port, Channels { photo, video, br }))
}

fn accept_loop(listener: TcpListener, senders: Senders, token: String, ctx: egui::Context) {
    let open = Arc::new(AtomicUsize::new(0));
    let senders = Arc::new(senders);
    let token = Arc::new(token);
    for mut stream in listener.incoming().flatten() {
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            let _ = writeln!(stream, "{}", json!({"ok": false, "error": "too many control connections"}));
            continue;
        }
        let (senders, token, ctx, open) = (senders.clone(), token.clone(), ctx.clone(), open.clone());
        let spawned = std::thread::Builder::new().name("br-control-conn".into()).spawn(move || {
            serve(stream, &senders, &token, &ctx);
            open.fetch_sub(1, Ordering::SeqCst);
        });
        if spawned.is_err() {
            log::warn!("control channel: could not start a connection thread");
        }
    }
}

/// How long a new connection may take to sign in (a connection that never does must not keep its
/// place: there are only [`MAX_CONNECTIONS`]).
const AUTH_WITHIN: Duration = Duration::from_secs(10);

fn serve(stream: TcpStream, senders: &Senders, token: &str, ctx: &egui::Context) {
    let (Ok(read), Ok(guard)) = (stream.try_clone(), stream.try_clone()) else { return };
    // the timeout is the socket's, for every handle of it
    let _ = guard.set_read_timeout(Some(AUTH_WITHIN));
    let mut authed = false;
    serve_lines(read, stream, |method, params| {
        if !authed {
            if method == "auth" && params.get("token").and_then(Value::as_str).is_some_and(|t| same(t, token)) {
                authed = true;
                // signed in: requests may be minutes apart
                let _ = guard.set_read_timeout(None);
                return Some(json!({"ok": true, "result": {"server": "br", "version": env!("CARGO_PKG_VERSION")}}));
            }
            // a wrong token closes the connection: nothing is run without it
            return None;
        }
        let frames = || ctx.cumulative_pass_nr();
        let wake = || ctx.request_repaint();
        Some(match route(&method, &params) {
            Target::Br => {
                let (req, rrx) = ControlRequest::new(method, params);
                let state = req.state.clone();
                senders.br.send(req).ok()?;
                wake();
                await_reply(&rrx, Some(&state), frames, wake, NO_FRAMES_AFTER, LONG_REPLY)
            }
            Target::Photo => {
                let (req, rrx) = ControlRequest::new(method, params);
                let state = req.state.clone();
                senders.photo.send(req).ok()?;
                wake();
                await_reply(&rrx, Some(&state), frames, wake, NO_FRAMES_AFTER, REPLY_TIMEOUT)
            }
            Target::Video(m) => {
                let (req, rrx) = filmcraft_ui_egui::control::ControlRequest::new(m, params);
                senders.video.send(req).ok()?;
                wake();
                await_reply(&rrx, None, frames, wake, NO_FRAMES_AFTER, REPLY_TIMEOUT)
            }
        })
    });
}

/// Wait for a request's reply. While the window has not picked the request up and `frames` stands
/// still for `no_frames`, the request is abandoned (it will not run late); one the window runs
/// gets the whole `timeout`. Requests without a `state` (FilmCraft's) can only time out.
fn await_reply(
    rrx: &Receiver<Value>,
    state: Option<&AtomicU8>,
    frames: impl Fn() -> u64,
    wake: impl Fn(),
    no_frames: Duration,
    timeout: Duration,
) -> Value {
    let start = std::time::Instant::now();
    let slice = (no_frames / 10).max(Duration::from_millis(1));
    let (mut last, mut since) = (frames(), std::time::Instant::now());
    loop {
        match rrx.recv_timeout(slice) {
            Ok(v) => return v,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return json!({"ok": false, "error": "the window dropped the request"}),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        let now = frames();
        if now != last {
            (last, since) = (now, std::time::Instant::now());
        }
        if let Some(state) = state {
            let taken = state.load(Ordering::SeqCst) == lightcraft_ui_egui::control::TAKEN;
            if !taken && since.elapsed() >= no_frames && ControlRequest::abandon(state) {
                return json!({"ok": false, "error": HIDDEN_WINDOW_ERROR});
            }
        } else if since.elapsed() >= no_frames.max(Duration::from_secs(30)) {
            return json!({"ok": false, "error": HIDDEN_WINDOW_ERROR});
        }
        if start.elapsed() >= timeout {
            if let Some(state) = state {
                ControlRequest::abandon(state);
            }
            return json!({"ok": false, "error": "timeout"});
        }
        wake();
    }
}

enum Line {
    Request { id: Value, method: String, params: Value },
    Blank,
    Reject(String),
}

fn parse_line(bytes: &[u8]) -> Line {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Line::Reject("not a control request (invalid UTF-8); closing the connection".into());
    };
    let text = text.trim();
    if text.is_empty() {
        return Line::Blank;
    }
    let msg = match serde_json::from_str::<Value>(text) {
        Ok(v) => v,
        Err(e) => return Line::Reject(format!("bad JSON ({e}); expected one JSON request object per line; closing the connection")),
    };
    let Some(obj) = msg.as_object() else {
        return Line::Reject("not a control request (expected a JSON object); closing the connection".into());
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Line::Reject("not a control request (missing string `method`); closing the connection".into());
    };
    Line::Request {
        id: obj.get("id").cloned().unwrap_or(Value::Null),
        method: method.to_string(),
        params: obj.get("params").cloned().unwrap_or(json!({})),
    }
}

fn read_bounded_line(reader: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> Result<bool, String> {
    buf.clear();
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let n = reader.take(limit).read_until(b'\n', buf).map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(false);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        return Ok(true);
    }
    if buf.len() > max {
        return Err(format!("request line longer than {max} bytes; closing the connection"));
    }
    Ok(true)
}

/// Serve requests from `read`, answering on `out`, until the peer closes, sends something that is
/// not a request, or `handle` returns `None` (refused, or the window is gone).
fn serve_lines<R: Read, W: Write>(read: R, mut out: W, mut handle: impl FnMut(String, Value) -> Option<Value>) {
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    loop {
        let reject = match read_bounded_line(&mut reader, &mut buf, MAX_LINE) {
            Ok(false) => return,
            Err(e) => e,
            Ok(true) => match parse_line(&buf) {
                Line::Blank => continue,
                Line::Reject(e) => e,
                Line::Request { id, method, params } => {
                    let first = method == "auth";
                    match handle(method, params) {
                        Some(mut r) => {
                            if let Some(o) = r.as_object_mut() {
                                o.insert("id".into(), id);
                            }
                            if writeln!(out, "{r}").and_then(|()| out.flush()).is_err() {
                                return;
                            }
                            continue;
                        }
                        None if first => "wrong token; closing the connection".to_string(),
                        None => "the first request must be auth with the window's token; closing the connection".to_string(),
                    }
                }
            },
        };
        let _ = writeln!(out, "{}", json!({"ok": false, "error": reject})).and_then(|()| out.flush());
        return;
    }
}

// ── BR's side, on the window's thread ───────────────────────────────────────────────────────────

/// A reply that waits for background work (an export, a caption recognition).
pub(crate) enum Pending {
    Export { job: u64, path: PathBuf, part: PathBuf, reply: Sender<Value> },
    Captions { reply: Sender<Value> },
}

fn ok(v: Value) -> Value {
    json!({"ok": true, "result": v})
}

fn err(e: impl std::fmt::Display) -> Value {
    json!({"ok": false, "error": e.to_string()})
}

/// Run BR's requests waiting in `rx` (each frame, before drawing).
pub fn drain(app: &mut BrApp, ctx: &egui::Context) {
    let Some(rx) = app.control.take() else { return };
    while let Ok(req) = rx.try_recv() {
        if !req.take() {
            continue; // its sender gave up on it
        }
        if let Some(v) = handle(app, ctx, &req) {
            let _ = req.reply.send(v);
        }
    }
    app.control = Some(rx);
    poll_pending(app);
}

/// Answer waiting replies whose work is done.
fn poll_pending(app: &mut BrApp) {
    let mut still = Vec::new();
    for p in std::mem::take(&mut app.pending) {
        match p {
            Pending::Export { job, path, part, reply } => match br_video::settle_export(&app.fc.session, job, &part, &path) {
                None => still.push(Pending::Export { job, path, part, reply }),
                Some(Ok(())) => {
                    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    let _ = reply.send(ok(json!({"path": path, "bytes": bytes})));
                }
                Some(Err(e)) => {
                    let _ = reply.send(err(e));
                }
            },
            Pending::Captions { reply } => match crate::cut::caption_result(app) {
                None => still.push(Pending::Captions { reply }),
                Some(r) => {
                    let _ = reply.send(match r {
                        Ok(v) => ok(v),
                        Err(e) => err(e),
                    });
                }
            },
        }
    }
    app.pending = still;
}

/// One of BR's requests; `None` when the reply comes later ([`Pending`]).
fn handle(app: &mut BrApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Value> {
    let p = &req.params;
    Some(match req.method.as_str() {
        "br.page" => match p.get("page").and_then(Value::as_str).and_then(|k| Page::ALL.into_iter().find(|x| x.key() == k)) {
            Some(page) => {
                app.open(page);
                ok(json!({"page": page.key()}))
            }
            None => err(format!("unknown page: {}", Page::ALL.map(Page::key).join(", "))),
        },
        "br.inspect" => {
            let widgets: serde_json::Map<String, Value> =
                app.widgets.iter().map(|(k, r)| (k.clone(), json!([r.min.x, r.min.y, r.width(), r.height()]))).collect();
            ok(json!({
                "page": app.page.key(),
                "video": br_video::state(&mut app.fc.session).unwrap_or(Value::Null),
                "photo": app.lc.session.active().map(|id| id.0),
                "popover": app.photo.popover().map(|p| p.key()).or(app.cut.popover().map(|p| p.key())),
                "inspector": crate::inspector::state(app),
                "assets": crate::assets::state(app),
                "widgets": widgets,
            }))
        }
        "br.ui.inspector" => wrap(crate::inspector::control(app, p)),
        "br.ui.assets" => wrap(crate::assets::control(app, p)),
        "ui.render" => {
            let id = p.get("id").and_then(Value::as_u64).unwrap_or(VIDEO_FRAME);
            let size = p.get("size").and_then(Value::as_u64).unwrap_or(1024).clamp(16, 4096) as u32;
            let path = p.get("path").and_then(Value::as_str).unwrap_or_default();
            let ms = id - VIDEO_FRAME;
            let seconds = (ms > 0).then(|| (ms - 1) as f64 / 1000.0);
            match br_video::frame(&app.fc.session, seconds, size).and_then(|(w, h, rgba)| {
                let img = lightcraft_codecs::EncodeImage::new(w, h, 4, lightcraft_codecs::Samples::U8(&rgba));
                let png = lightcraft_codecs::encode_png(&img, &lightcraft_codecs::EncodeMeta::default()).map_err(|e| e.to_string())?;
                std::fs::write(path, png).map_err(|e| format!("{path}: {e}"))?;
                Ok((w, h))
            }) {
                Ok((w, h)) => ok(json!({"width": w, "height": h})),
                Err(e) => err(e),
            }
        }
        "engine.execute" | "ui.menu.invoke" | "command" => {
            let id = p.get("command").or(p.get("id")).and_then(Value::as_str).unwrap_or_default().to_string();
            let params = p.get("params").cloned().unwrap_or(json!({}));
            return run_command(app, ctx, &id, params, &req.reply);
        }
        other => err(format!(
            "unknown BR request `{other}` (br.page, br.inspect, br.ui.inspector, br.ui.assets, engine.execute with br.*, br.video.* or film.<id>)"
        )),
    })
}

fn run_command(app: &mut BrApp, ctx: &egui::Context, id: &str, params: Value, reply: &Sender<Value>) -> Option<Value> {
    if let Some(film) = id.strip_prefix("film.") {
        return Some(wrap(br_video::execute_with(&mut app.fc.session, &mut app.video.watch, film, &params)));
    }
    match id {
        // long work runs in the background, as in the window; the reply waits for it
        "br.video.export" if params.get("wait").and_then(Value::as_bool).unwrap_or(true) => {
            let mut q = params.clone();
            q["wait"] = json!(false);
            match br_video::execute_with(&mut app.fc.session, &mut app.video.watch, id, &q) {
                Ok(v) => match (v.get("job").and_then(Value::as_u64), v.get("path").and_then(Value::as_str), v.get("part").and_then(Value::as_str)) {
                    (Some(job), Some(path), Some(part)) => {
                        app.pending.push(Pending::Export { job, path: PathBuf::from(path), part: PathBuf::from(part), reply: reply.clone() });
                        None
                    }
                    _ => Some(ok(v)),
                },
                Err(e) => Some(err(e)),
            }
        }
        "br.video.autoCaptions" => {
            let opts = br_video::speech::CaptionOptions {
                language: params.get("language").and_then(Value::as_str).map(str::to_string),
                model: params.get("model").and_then(Value::as_str).map(str::to_string),
            };
            let wait = params.get("wait").and_then(Value::as_bool).unwrap_or(true);
            match crate::cut::start_captions_with(app, ctx, opts) {
                Ok(()) if wait => {
                    app.pending.push(Pending::Captions { reply: reply.clone() });
                    None
                }
                Ok(()) => Some(ok(json!({"started": true, "poll": "br_video_jobs"}))),
                Err(e) => Some(err(e)),
            }
        }
        "br.ui.inspector" => Some(wrap(crate::inspector::control(app, &params))),
        "br.ui.assets" => Some(wrap(crate::assets::control(app, &params))),
        "br.video.jobs" => Some(ok(json!({"exports": br_video::jobs(&app.fc.session), "captions": crate::cut::caption_status(app)}))),
        "br.video.cancel" if params.get("captions").and_then(Value::as_bool) == Some(true) => {
            Some(ok(json!({"cancelled": crate::cut::cancel_captions(app)})))
        }
        _ if br_video::is_video(id) => Some(wrap(br_video::execute_with(&mut app.fc.session, &mut app.video.watch, id, &params))),
        _ => {
            crate::photo::finish_drags(app);
            Some(wrap(br_photo::commands::execute(&mut app.lc.session, &mut app.photo.br, id, &params).map_err(|e| e.to_string())))
        }
    }
}

fn wrap(r: Result<Value, String>) -> Value {
    match r {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_go_where_they_belong() {
        let p = |c: &str| json!({"command": c});
        assert_eq!(route("engine.execute", &p("br.video.add")), Target::Br);
        assert_eq!(route("engine.execute", &p("film.timeline.razor")), Target::Br);
        assert_eq!(route("engine.execute", &p("br.look")), Target::Br);
        assert_eq!(route("engine.execute", &p("develop.set")), Target::Photo);
        assert_eq!(route("ui.render", &json!({"id": 3})), Target::Photo);
        assert_eq!(route("ui.render", &json!({"id": VIDEO_FRAME + 5})), Target::Br);
        assert_eq!(route("film:ui.inspect", &json!({})), Target::Video("ui.inspect".into()));
        assert_eq!(route("br.page", &json!({})), Target::Br);
        assert_eq!(route("ui.click", &json!({})), Target::Photo);
    }

    fn run(input: &[u8], token: &str) -> Vec<Value> {
        let mut out = Vec::new();
        let mut authed = false;
        serve_lines(input, &mut out, |m, p| {
            if !authed {
                authed = m == "auth" && p["token"] == token;
                return authed.then(|| json!({"ok": true}));
            }
            Some(json!({"ok": true, "result": m}))
        });
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn nothing_runs_without_the_token() {
        let r = run(b"{\"id\":1,\"method\":\"auth\",\"params\":{\"token\":\"nope\"}}\n{\"id\":2,\"method\":\"app.quit\"}\n", "secret");
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(r[0]["ok"], false);
        let r = run(b"{\"id\":1,\"method\":\"app.quit\"}\n", "secret");
        assert_eq!(r.len(), 1);
        assert!(r[0]["error"].as_str().unwrap().contains("auth"), "{r:?}");
        let r = run(b"GET / HTTP/1.1\r\n", "secret");
        assert_eq!(r[0]["ok"], false);
        let r = run(b"{\"id\":1,\"method\":\"auth\",\"params\":{\"token\":\"secret\"}}\n{\"id\":2,\"method\":\"ui.inspect\"}\n", "secret");
        assert_eq!(r.len(), 2);
        assert_eq!((r[1]["id"].as_i64(), r[1]["result"].as_str()), (Some(2), Some("ui.inspect")));
    }

    #[test]
    fn tokens_are_fresh_and_compared_whole() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
        assert!(same(&a, &a.clone()));
        assert!(!same(&a, &b) && !same(&a, &a[..47]));
    }

    #[test]
    fn the_info_file_is_private() {
        let dir = std::env::temp_dir().join(format!("br-control-{}", std::process::id()));
        let path = info_path(&dir);
        write_info(&path, 4242, "t0k3n").unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((v["port"].as_u64(), v["token"].as_str()), (Some(4242), Some("t0k3n")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        write_info(&path, 4243, "again").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
