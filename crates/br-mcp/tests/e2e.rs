//! `br mcp` end to end: JSON-RPC lines in, replies out, on LightCraft's demo photos.

use br_mcp::{BrBackend, BrServer};
use lightcraft_mcp::Headless;
use serde_json::{Value, json};

fn server(full: bool) -> BrServer {
    lightcraft_engine::gpu::set_enabled(false);
    let mut s = BrServer::new(BrBackend::new(Headless::demo(), "demo photos"), full);
    let init = s.handle(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}));
    assert_eq!(init.unwrap()["result"]["serverInfo"]["name"], "br");
    assert!(s.handle(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
    s
}

fn call(s: &mut BrServer, name: &str, args: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string();
    let reply: Value = serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap();
    assert_eq!(reply["id"], 7);
    reply
}

fn ok(s: &mut BrServer, name: &str, args: Value) -> Value {
    let r = call(s, name, args);
    assert_eq!(r["result"]["isError"], false, "{name}: {r}");
    let text = r["result"]["content"][0]["text"].as_str().unwrap_or("null");
    serde_json::from_str(text).unwrap_or(Value::Null)
}

#[test]
fn br_tools_come_first_and_lightcrafts_follow() {
    let mut s = server(false);
    let r = s.handle(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
    let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names[0], "br_state");
    for n in ["br_look", "br_adjust", "br_crop", "br_export", "br_preview", "import", "set_develop", "command_run"] {
        assert!(names.contains(&n), "{n} missing from {names:?}");
    }
    assert!(!names.iter().any(|n| n.starts_with("cmd_")), "compact by default");
    let full = server(true).handle(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
    let full: Vec<&str> = full["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(full.contains(&"cmd_br_look") && full.contains(&"cmd_develop_set"), "full mode lists every command");
}

#[test]
fn an_agent_edits_and_exports_a_photo() {
    let mut s = server(false);
    let looks = ok(&mut s, "br_looks", json!({}));
    assert!(looks.as_array().unwrap().len() >= 18);
    ok(&mut s, "br_look", json!({"look": "lc.teal-orange", "amount": 80}));
    let r = ok(&mut s, "br_adjust", json!({"light": 25, "warmth": 15}));
    assert_eq!(r["sliders"]["light"], 25.0);
    let r = ok(&mut s, "br_crop", json!({"format": "9:16"}));
    assert_eq!(r["format"], "9:16");
    let st = ok(&mut s, "br_state", json!({}));
    assert_eq!(st["format"], "9:16");
    assert_eq!(st["sliders"]["warmth"], 15.0);
    assert_eq!(st["undo"], "Format 9:16");

    // a preview comes back as an image
    let r = call(&mut s, "br_preview", json!({"max_side": 256}));
    let content = r["result"]["content"].as_array().unwrap();
    assert!(content.iter().any(|c| c["type"] == "image" && c["mimeType"] == "image/jpeg"), "{r}");

    let dir = std::env::temp_dir().join(format!("br-mcp-e2e-{}", std::process::id()));
    let r = ok(&mut s, "br_export", json!({"preset": "story", "dir": dir.to_string_lossy()}));
    let f = &r["files"][0];
    assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(1080), Some(1920)), "{r}");
    assert!(std::path::Path::new(f["path"].as_str().unwrap()).exists());
    let _ = std::fs::remove_dir_all(&dir);

    // undo through BR, and the same command through LightCraft's generic tool
    ok(&mut s, "br_undo", json!({}));
    assert_eq!(ok(&mut s, "br_state", json!({}))["undo"], "Anpassen");
    let r = ok(&mut s, "command_run", json!({"id": "br.state", "params": {}}));
    assert_eq!(r["sliders"]["light"], 25.0);
}

#[test]
fn mistakes_are_reported_clearly() {
    let mut s = server(false);
    // a wrong argument name is a protocol error
    let r = call(&mut s, "br_adjust", json!({"brightness": 10}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = call(&mut s, "br_crop", json!({}));
    assert!(r["error"]["message"].as_str().unwrap().contains("format"));
    // a failing command is a tool error, and the server keeps going
    let r = call(&mut s, "br_look", json!({"look": "nope"}));
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("unknown look"));
    let r = call(&mut s, "br_nope", json!({}));
    assert!(r["error"]["message"].as_str().unwrap().contains("unknown tool"));
    ok(&mut s, "br_state", json!({}));
}

#[test]
fn requests_lightcraft_would_refuse_are_refused_for_br_tools_too() {
    let mut s = server(false);
    let before = ok(&mut s, "br_state", json!({}));
    // an id that is not a string or number: nothing runs
    let r = s.handle(json!({"jsonrpc": "2.0", "id": null, "method": "tools/call", "params": {"name": "br_adjust", "arguments": {"light": 40}}}));
    assert_eq!(r.unwrap()["error"]["code"], -32600);
    assert_eq!(ok(&mut s, "br_state", json!({}))["sliders"], before["sliders"]);
    // a photo id that is not a whole number
    let r = call(&mut s, "br_adjust", json!({"id": "14", "light": 40}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    // newer protocol revisions get LightCraft's result fields on br_* results too
    let r = s
        .handle(json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
            "params": {"name": "br_state", "arguments": {}, "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}}))
        .unwrap();
    assert_eq!(r["result"]["resultType"], "complete", "{r}");
}

/// `Write` into a shared buffer, to read what `serve` wrote.
#[derive(Clone, Default)]
struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn serve_keeps_lightcrafts_progress_notifications() {
    lightcraft_engine::gpu::set_enabled(false);
    let dir = std::env::temp_dir().join(format!("br-mcp-progress-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let input = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "br_look", "arguments": {"look": "lc.moody"}}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "export",
            "arguments": {"ids": [1, 2, 3], "dir": dir.to_string_lossy(), "longEdge": 64, "format": "png"}, "_meta": {"progressToken": "tok"}}}),
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "br_adjust", "arguments": {"nope": 1}}}),
    ]
    .iter()
    .map(|m| m.to_string() + "\n")
    .collect::<String>();
    let sink = Sink::default();
    br_mcp::serve(BrBackend::new(Headless::demo(), "demo photos"), false, std::io::Cursor::new(input), sink.clone()).unwrap();
    let out = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<Value> = out.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let by_id = |id: i64| lines.iter().find(|l| l["id"] == id).unwrap_or_else(|| panic!("no reply {id} in {out}"));
    assert_eq!(by_id(1)["result"]["serverInfo"]["name"], "br");
    assert_eq!(by_id(2)["result"]["tools"][0]["name"], "br_state");
    assert_eq!(by_id(3)["result"]["isError"], false, "{}", by_id(3));
    assert_eq!(by_id(4)["result"]["isError"], false, "{}", by_id(4));
    assert_eq!(by_id(5)["error"]["code"], -32602);
    let progress = lines.iter().filter(|l| l["method"] == "notifications/progress" && l["params"]["progressToken"] == "tok").count();
    assert!(progress >= 1, "progress notifications: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn null_means_not_given_and_batches_stay_one_array() {
    let mut s = server(false);
    let r = ok(&mut s, "br_look", json!({"look": "lc.moody", "amount": null, "id": null}));
    assert_eq!(r["amount"], 100.0);
    let r = call(&mut s, "br_look", json!({"look": null}));
    assert!(r["error"]["message"].as_str().unwrap().contains("needs `look`"), "{r}");
    let r = call(&mut s, "br_preview", json!({"id": null, "max_side": null, "format": null}));
    assert_eq!(r["result"]["isError"], false, "{r}");

    // over stdio: a batch with a rejected br_* call and a ping comes back as one array
    let input = [
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}).to_string(),
        json!([{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "br_adjust", "arguments": {"nope": 1}}}, {"jsonrpc": "2.0", "id": 2, "method": "ping"}]).to_string(),
        json!([{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "br_adjust", "arguments": {"nope": 1}}}]).to_string(),
    ]
    .join("\n");
    let sink = Sink::default();
    br_mcp::serve(BrBackend::new(Headless::demo(), "demo"), false, std::io::Cursor::new(input), sink.clone()).unwrap();
    let out = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<Value> = out.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let arrays: Vec<&Vec<Value>> = lines.iter().filter_map(Value::as_array).collect();
    assert_eq!(arrays.len(), 2, "{out}");
    let ids = |a: &Vec<Value>| -> Vec<i64> {
        let mut v: Vec<i64> = a.iter().filter_map(|r| r["id"].as_i64()).collect();
        v.sort();
        v
    };
    assert!(arrays.iter().any(|a| ids(a) == vec![1, 2]), "{out}");
    assert!(arrays.iter().any(|a| ids(a) == vec![3]), "{out}");
    assert!(lines.iter().filter(|l| l.is_object()).all(|l| l["id"] == 0), "only the initialize reply is a bare object: {out}");
}

fn have_ffmpeg() -> bool {
    let found = std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    // in CI (BR_REQUIRE_FFMPEG) the video tests must run, not skip
    assert!(found || std::env::var_os("BR_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (BR_REQUIRE_FFMPEG is set)");
    found
}

#[test]
fn an_agent_cuts_a_vertical_video_and_looks_at_it() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg missing: skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("br-mcp-video-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("IMG_0002.mp4");
    let o = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1080x1920:rate=30:duration=3"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&clip)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    let mut s = server(false);
    let r = s.handle(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
    let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in ["br_video_add", "br_video_export", "br_video_preview", "film_run", "film_commands", "br_state"] {
        assert!(names.contains(&n), "{n} missing");
    }
    assert_eq!(ok(&mut s, "br_video_state", json!({})), json!({"video": null}));
    let st = ok(&mut s, "br_video_add", json!({"paths": [clip.to_string_lossy()]}));
    assert_eq!(st["format"], "9:16", "{st}");
    ok(&mut s, "br_video_split", json!({"seconds": 1.0}));
    let seq = ok(&mut s, "film_run", json!({"command": "sequence.inspect"}));
    assert_eq!(seq["video"][0]["items"].as_array().map(Vec::len), Some(2), "{seq}");
    ok(&mut s, "br_video_text", json!({"text": "Hallo"}));
    let found = ok(&mut s, "film_commands", json!({"query": "razor"}));
    assert!(found.as_array().unwrap().iter().any(|c| c["id"] == "timeline.razor"), "{found}");
    let looks = ok(&mut s, "br_video_looks", json!({}));
    let first = looks["presets"][0]["name"].as_str().unwrap().to_string();
    ok(&mut s, "br_video_filter", json!({"preset": first}));
    let undo = ok(&mut s, "br_video_undo", json!({}));
    assert!(undo["undone"].as_str().is_some_and(|l| l.starts_with("Filter")), "the filter is undone: {undo}");
    let seq = ok(&mut s, "film_run", json!({"command": "sequence.inspect"}));
    assert!(
        seq["video"].as_array().unwrap().iter().flat_map(|t| t["items"].as_array().unwrap()).all(|c| !c["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["effect"] == "lumetri")),
        "no look left: {seq}"
    );

    // a frame as an image
    let r = call(&mut s, "br_video_preview", json!({"seconds": 0.5, "max_side": 256}));
    assert_eq!(r["result"]["isError"], false, "{r}");
    let content = r["result"]["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "image");
    let info: Value = serde_json::from_str(content[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(144), Some(256)), "{info}");

    // the file, waited for
    let out = dir.join("reel.mp4");
    let r = ok(&mut s, "br_video_export", json!({"preset": "social", "path": out.to_string_lossy()}));
    assert!(std::fs::metadata(&out).unwrap().len() > 1000, "{r}");

    // mistakes come back as tool errors, the video unchanged
    let r = call(&mut s, "br_video_format", json!({"format": "3:2"}));
    assert!(r["error"].is_object() || r["result"]["isError"] == true, "{r}");
    let r = call(&mut s, "film_run", json!({"command": "no.such"}));
    assert_eq!(r["result"]["isError"], true, "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A recogniser that stops only when cancelled.
struct Endless;

impl filmcraft_speech::Transcriber for Endless {
    fn id(&self) -> String {
        "endless".into()
    }
    fn transcribe(
        &self,
        _: &[f32],
        _: &filmcraft_speech::Options,
        progress: filmcraft_speech::ProgressFn,
    ) -> Result<filmcraft_project::Transcript, filmcraft_speech::SpeechError> {
        while progress(0.4, "") {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err(filmcraft_speech::SpeechError::Cancelled)
    }
}

#[test]
fn long_video_work_runs_in_the_background_and_can_be_stopped() {
    if !have_ffmpeg() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("br-mcp-jobs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("talk.mp4");
    let o = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=3"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&clip)
        .output()
        .unwrap();
    assert!(o.status.success());
    lightcraft_engine::gpu::set_enabled(false);
    let mut backend = BrBackend::new(Headless::demo(), "demo photos");
    backend.video.transcriber = Some(std::sync::Arc::new(Endless));
    let mut s = BrServer::new(backend, false);
    s.handle(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}));
    ok(&mut s, "br_video_add", json!({"paths": [clip.to_string_lossy()]}));

    // captions in the background: running, then stopped
    let r = ok(&mut s, "br_video_auto_captions", json!({"wait": false}));
    assert_eq!(r["started"], true, "{r}");
    let jobs = ok(&mut s, "br_video_jobs", json!({}));
    assert_eq!(jobs["captions"]["running"], true, "{jobs}");
    assert_eq!(ok(&mut s, "br_video_cancel", json!({"captions": true}))["cancelled"], true);
    let t0 = std::time::Instant::now();
    let status = loop {
        let jobs = ok(&mut s, "br_video_jobs", json!({}));
        if jobs["captions"]["done"] == true {
            break jobs["captions"].clone();
        }
        assert!(t0.elapsed().as_secs() < 60, "{jobs}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert_eq!(status["error"], "abgebrochen", "{status}");

    // an export in the background is complete in its folder by itself, nobody asking again
    let out = dir.join("later.mp4");
    let r = ok(&mut s, "br_video_export", json!({"preset": "video", "path": out.to_string_lossy(), "wait": false}));
    assert!(r["job"].is_u64(), "{r}");
    let t0 = std::time::Instant::now();
    while !out.exists() {
        assert!(t0.elapsed().as_secs() < 120, "the export never appeared");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let jobs = ok(&mut s, "br_video_jobs", json!({}));
    assert!(jobs["exports"].as_array().unwrap().iter().any(|j| j["done"] == true && j["error"].is_null()), "{jobs}");
    let _ = std::fs::remove_dir_all(&dir);
}
