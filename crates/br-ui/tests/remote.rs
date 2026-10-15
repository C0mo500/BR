//! BR used from the browser page: files the page uploads after they were dropped on the BR
//! picture are handed to the window over the control channel (`br.drop`, as br-dateien does) and
//! land where they were let go: on the start screen as a new video, on the timeline at the cut
//! there. The command search answers over the channel too. Clips are made with `ffmpeg` (skipped
//! without it, unless BR_REQUIRE_FFMPEG).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use br_ui::BrApp;
use br_ui::headless::Driver;
use serde_json::{Value, json};

fn have_ffmpeg() -> bool {
    let found = std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("BR_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (BR_REQUIRE_FFMPEG is set)");
    found
}

fn clip(dir: &std::path::Path, name: &str) -> String {
    let p = dir.join(name);
    let o = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=360x640:rate=30:duration=3"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&p)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    p.to_string_lossy().into_owned()
}

/// One request over the channel (signed in), answered while the window draws frames.
fn request(app: &mut BrApp, d: &mut Driver, port: u16, token: &str, method: &str, params: Value) -> Value {
    let token = token.to_string();
    let (method, params) = (method.to_string(), params);
    let h = std::thread::spawn(move || {
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut w = s;
        let mut line = String::new();
        writeln!(w, "{}", json!({"id": 1, "method": "auth", "params": {"token": token}})).unwrap();
        r.read_line(&mut line).unwrap();
        line.clear();
        writeln!(w, "{}", json!({"id": 2, "method": method, "params": params})).unwrap();
        r.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    });
    let t0 = Instant::now();
    while !h.is_finished() {
        d.step(app, Vec::new());
        assert!(t0.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(2));
    }
    h.join().unwrap()
}

#[test]
fn files_dropped_on_the_page_land_where_they_were_let_go() {
    if !have_ffmpeg() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("br-remote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (clip(&dir, "a.mp4"), clip(&dir, "b.mp4"));
    lightcraft_engine::gpu::set_enabled(false);
    let mut d = Driver::new([1600.0, 1000.0], 1.0);
    let token = br_ui::control::new_token();
    let (port, ch) = br_ui::control::start(0, token.clone(), d.view.ctx.clone()).unwrap();
    let mut app = BrApp::new(br_photo::library::demo_session(), None).with_control(ch);
    app.lc.ui.settings.gpu = false;
    assert!(d.settle(&mut app, Duration::from_secs(90)));
    // the start screen: a new video from the dropped clip
    let r = request(&mut app, &mut d, port, &token, "br.drop", json!({"paths": [a], "at": [800.0, 500.0]}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(d.settle(&mut app, Duration::from_secs(90)));
    let q = app.fc.session.active_sequence().expect("a video from the drop");
    assert_eq!(br_video::main_track::main_clips(q).len(), 1);
    // onto the first clip in the timeline: after it on the main track (the cut nearest to it)
    let first = br_video::main_track::main_clips(q)[0].id.0;
    let at = app.widgets[&format!("tl.clip.{first}")].right_center() - egui::vec2(4.0, 0.0);
    let r = request(&mut app, &mut d, port, &token, "br.drop", json!({"paths": [b], "at": [at.x, at.y]}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(d.settle(&mut app, Duration::from_secs(90)));
    let names: Vec<String> = br_video::main_track::main_clips(app.fc.session.active_sequence().unwrap()).iter().map(|it| it.name.clone()).collect();
    assert_eq!(names, ["a.mp4", "b.mp4"], "placed on the main track");
    // what cannot be dropped
    let r = request(&mut app, &mut d, port, &token, "br.drop", json!({"paths": [dir.join("fehlt.mp4")]}));
    assert_eq!(r["ok"], false, "{r}");
    // the command search over the channel
    let r = request(&mut app, &mut d, port, &token, "br.palette.search", json!({"query": "teilen", "limit": 3}));
    assert_eq!(r["result"][0]["id"], "br.video.split", "{r}");
    let r = request(&mut app, &mut d, port, &token, "br.palette.run", json!({"id": "br.ui.page.foto"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.page, br_ui::Page::Foto);
    let _ = std::fs::remove_dir_all(&dir);
}
