//! An agent working in the window the user sees: shnyp's window with its control channel, and
//! `shnyp mcp --connect`'s server signing in to it. Tool calls go through the channel and change
//! what the window shows.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use shnyp_mcp::{ShnypServer, Window};
use shnyp_ui::headless::Driver;
use shnyp_ui::{Page, ShnypApp};

fn rpc(s: &mut ShnypServer, id: u64, method: &str, params: Value) -> Value {
    s.handle(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).expect("a reply")
}

fn tool(s: &mut ShnypServer, name: &str, args: Value) -> Value {
    rpc(s, 9, "tools/call", json!({"name": name, "arguments": args}))
}

fn result(r: &Value) -> Value {
    assert_eq!(r["result"]["isError"], false, "{r}");
    serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap_or("null")).unwrap_or(Value::Null)
}

/// Run `agent` on its own thread while the window draws frames, as a real window does.
fn with_window<T: Send + 'static>(app: &mut ShnypApp, d: &mut Driver, agent: impl FnOnce() -> T + Send + 'static) -> T {
    let h = std::thread::spawn(agent);
    let t0 = Instant::now();
    while !h.is_finished() {
        d.step(app, Vec::new());
        assert!(t0.elapsed() < Duration::from_secs(240), "the agent never finished");
        std::thread::sleep(Duration::from_millis(2));
    }
    h.join().unwrap()
}

#[test]
fn an_agent_signs_in_and_edits_what_the_window_shows() {
    lightcraft_engine::gpu::set_enabled(false);
    let mut app = ShnypApp::new(shnyp_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1200.0, 800.0], 1.0);
    let token = shnyp_ui::control::new_token();
    let (port, ch) = shnyp_ui::control::start(0, token.clone(), d.view.ctx.clone()).unwrap();
    let mut app = app.with_control(ch);
    assert!(d.settle(&mut app, Duration::from_secs(90)));
    let addr = format!("127.0.0.1:{port}");

    // a wrong token: nothing runs
    let bad = addr.clone();
    let r = with_window(&mut app, &mut d, move || {
        let mut s = ShnypServer::new(Window::new(bad, "not-the-token"), false);
        rpc(&mut s, 1, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}));
        tool(&mut s, "shnyp_video_new", json!({"format": "9:16"}))
    });
    assert_eq!(r["result"]["isError"], true, "{r}");
    assert!(r.to_string().contains("refused"), "{r}");
    assert!(app.fc.session.active_sequence().is_none());

    let (a, t) = (addr.clone(), token.clone());
    let replies = with_window(&mut app, &mut d, move || {
        let mut s = ShnypServer::new(Window::new(a, t), false);
        let init =
            rpc(&mut s, 1, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}));
        let new = result(&tool(&mut s, "shnyp_video_new", json!({"format": "9:16"})));
        let text = result(&tool(&mut s, "shnyp_video_text", json!({"text": "Vom Agenten"})));
        let seq = result(&tool(&mut s, "film_run", json!({"command": "sequence.inspect"})));
        let frame = tool(&mut s, "shnyp_video_preview", json!({"max_side": 128}));
        let look = result(&tool(&mut s, "shnyp_look", json!({"look": "lc.moody", "amount": 60})));
        // one of LightCraft's own tools, answered by LightCraft in the window
        let photos = result(&tool(&mut s, "query_photos", json!({"limit": 3})));
        (init, new, text, seq, frame, look, photos)
    });
    let (init, new, _text, seq, frame, look, photos) = replies;
    assert_eq!(init["result"]["serverInfo"]["name"], "shnyp");
    assert_eq!((new["width"].as_u64(), new["height"].as_u64()), (Some(1080), Some(1920)), "{new}");
    assert!(seq["video"].as_array().is_some_and(|v| v.iter().any(|t| !t["items"].as_array().unwrap().is_empty())), "the title is in: {seq}");
    assert_eq!(frame["result"]["content"][0]["type"], "image", "{frame}");
    assert!(look["look"].is_object() || look["look"].is_string(), "{look}");
    assert!(photos.is_array() || photos.is_object(), "{photos}");

    // the window shows it
    let q = app.fc.session.active_sequence().expect("the video is in the window");
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
    assert!(app.photo.shnyp.look.is_some(), "the look is shnyp's, in the window");
    assert_eq!(app.page, Page::Schnitt);
}
