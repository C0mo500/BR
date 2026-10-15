//! The Program monitor, CapCut's way, driven headlessly: a clip selected in the timeline shows
//! its transform box at once; a single click on the picture selects the clip under the pointer
//! (an overlay over the main clip, the main clip through its background, nothing on an empty
//! spot); the Format popover fills the background of every main clip. Clips are made with
//! `ffmpeg` (skipped without it, unless BR_REQUIRE_FFMPEG).

use std::time::Duration;

use br_ui::BrApp;
use br_ui::headless::{Driver, png};
use egui::pos2;
use filmcraft_project::ClipId;
use serde_json::json;

const SETTLE: Duration = Duration::from_secs(90);

fn have_ffmpeg() -> bool {
    let found = std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("BR_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (BR_REQUIRE_FFMPEG is set)");
    found
}

fn clip(dir: &std::path::Path, name: &str, size: &str, seconds: u32) -> String {
    let path = dir.join(name);
    let o = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size={size}:rate=30:duration={seconds}"))
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    path.to_string_lossy().into_owned()
}

fn snapshot(d: &Driver, name: &str) {
    if let Some(dir) = std::env::var_os("BR_SNAPSHOT_DIR") {
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png(&d.paint())).unwrap();
    }
}

struct Video {
    app: BrApp,
    d: Driver,
    main: u64,
    overlay: u64,
    dir: std::path::PathBuf,
}

/// A 9:16 video: a landscape main clip (fitted: black above and below) and a small overlay near
/// the top.
fn video(name: &str) -> Video {
    lightcraft_engine::gpu::set_enabled(false);
    let dir = std::env::temp_dir().join(format!("br-monitor-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, o) = (clip(&dir, "a.mp4", "640x360", 4), clip(&dir, "o.mp4", "360x640", 4));
    let mut app = BrApp::new(br_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1600.0, 1000.0], 1.0);
    assert!(d.settle(&mut app, SETTLE));
    let run =
        |app: &mut BrApp, id: &str, p: serde_json::Value| br_video::execute(&mut app.fc.session, id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut app, "br.video.new", json!({"format": "9:16"}));
    run(&mut app, "br.video.add", json!({"paths": [a]}));
    let r = run(&mut app, "br.video.place", json!({"paths": [o], "target": "overlay", "seconds": 0.0, "position": [0.5, 0.15]}));
    let q = app.fc.session.active_sequence().unwrap();
    let main = br_video::main_track::main_clips(q)[0].id.0;
    let overlay = r["clips"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c.as_u64())
        .find(|c| q.video_tracks.iter().any(|t| t.item(ClipId(*c)).is_some()))
        .unwrap();
    run(&mut app, "br.video.transform", json!({"clips": [overlay], "scale": 60}));
    run(&mut app, "timeline.select", json!({"clips": []}));
    app.fc.session.set_playhead(filmcraft_time::Tick::from_seconds_f64(1.0));
    assert!(d.settle(&mut app, SETTLE));
    Video { app, d, main, overlay, dir }
}

fn picture(app: &BrApp) -> egui::Rect {
    let e = app.fc.auto.find("program.picture").expect("the Program picture");
    egui::Rect::from_min_size(pos2(e.rect[0], e.rect[1]), egui::vec2(e.rect[2], e.rect[3]))
}

fn selected(app: &BrApp) -> Vec<u64> {
    app.fc.session.state.selection.iter().map(|c| c.0).collect()
}

#[test]
fn a_clip_selected_in_the_timeline_shows_its_box_at_once() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("box");
    let main = v.main;
    let r = v.app.widgets[&format!("tl.clip.{main}")];
    v.d.click_at(&mut v.app, r.center());
    assert!(v.d.settle(&mut v.app, SETTLE));
    let sel = v.app.fc.session.state.selected_effect.clone().expect("an effect is selected");
    assert_eq!(sel.clip, ClipId(main), "Motion of the clip: its transform box");
    assert!(v.app.fc.auto.elements.iter().any(|e| e.id.starts_with("program.transform")), "the box's handles are drawn");
    snapshot(&v.d, "monitor-box");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn a_click_on_the_picture_selects_what_is_under_the_pointer() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("click");
    let pic = picture(&v.app);
    // the overlay near the top (15 % down, in the middle)
    v.d.click_at(&mut v.app, pos2(pic.center().x, pic.min.y + pic.height() * 0.15));
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert_eq!(selected(&v.app), vec![v.overlay], "the overlay");
    // the main clip in the middle
    v.d.click_at(&mut v.app, pos2(pic.center().x, pic.center().y));
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(selected(&v.app).contains(&v.main), "the main clip: {:?}", selected(&v.app));
    // the black band below the fitted clip: nothing
    v.d.click_at(&mut v.app, pos2(pic.center().x, pic.max.y - pic.height() * 0.05));
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(selected(&v.app).is_empty(), "{:?}", selected(&v.app));
    // with a filled background the band is the main clip's background: the main clip
    v.d.click(&mut v.app, "video.format").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    v.d.click(&mut v.app, "vformat.bg.blur").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    let q = v.app.fc.session.active_sequence().unwrap();
    assert!(br_video::frame::background_of(q, ClipId(v.main)).is_some(), "the Format popover filled the background");
    v.d.key(&mut v.app, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let pic = picture(&v.app);
    v.d.click_at(&mut v.app, pos2(pic.center().x, pic.max.y - pic.height() * 0.05));
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(selected(&v.app).contains(&v.main), "through its background: {:?}", selected(&v.app));
    snapshot(&v.d, "monitor-background");
    let _ = std::fs::remove_dir_all(&v.dir);
}
