//! shnyp's timeline on the Schnitt page, driven headlessly: dragging a main clip reorders the video,
//! dragging an edge trims, a click selects, the ruler moves the playhead, "Profi" shows
//! FilmCraft's own timeline in its place, and an edit made with FilmCraft's own tools is followed
//! in the same frame. Clips are made with `ffmpeg` (skipped without it, unless SHNYP_REQUIRE_FFMPEG).

use std::time::Duration;

use egui::pos2;
use serde_json::json;
use shnyp_ui::ShnypApp;
use shnyp_ui::headless::{Driver, png};

const SETTLE: Duration = Duration::from_secs(90);

fn have_ffmpeg() -> bool {
    let found = std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("SHNYP_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (SHNYP_REQUIRE_FFMPEG is set)");
    found
}

fn clip(dir: &std::path::Path, name: &str, seconds: u32) -> String {
    let path = dir.join(name);
    let o = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size=360x640:rate=30:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={seconds}"))
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    path.to_string_lossy().into_owned()
}

fn snapshot(d: &Driver, name: &str) {
    if let Some(dir) = std::env::var_os("SHNYP_SNAPSHOT_DIR") {
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png(&d.paint())).unwrap();
    }
}

struct Video {
    app: ShnypApp,
    d: Driver,
    a: u64,
    b: u64,
    c: u64,
    title: u64,
    dir: std::path::PathBuf,
}

/// The window with a 9:16 video of three clips (4, 3 and 2 s) and a title on the third.
fn video(name: &str) -> Video {
    lightcraft_engine::gpu::set_enabled(false);
    let dir = std::env::temp_dir().join(format!("shnyp-tl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b, c) = (clip(&dir, "a.mp4", 4), clip(&dir, "b.mp4", 3), clip(&dir, "c.mp4", 2));
    let mut app = ShnypApp::new(shnyp_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1400.0, 900.0], 1.0);
    assert!(d.settle(&mut app, SETTLE));
    let run = |app: &mut ShnypApp, id: &str, p: serde_json::Value| {
        shnyp_video::execute(&mut app.fc.session, id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(&mut app, "shnyp.video.new", json!({"format": "9:16"}));
    run(&mut app, "shnyp.video.add", json!({"paths": [a, b, c]}));
    let title = run(&mut app, "shnyp.video.text", json!({"text": "Auf C", "seconds": 1.0, "at": 7.5}))["clip"].as_u64().unwrap();
    let mains = mains(&app);
    assert_eq!(mains.len(), 3);
    assert!(d.settle(&mut app, SETTLE));
    Video { a: mains[0].0, b: mains[1].0, c: mains[2].0, title, app, d, dir }
}

/// The main track: (clip, start, end) in seconds.
fn mains(app: &ShnypApp) -> Vec<(u64, f64, f64)> {
    let q = app.fc.session.active_sequence().unwrap();
    shnyp_video::main_track::main_clips(q).iter().map(|it| (it.id.0, it.start.seconds(), it.end().seconds())).collect()
}

fn start_of(app: &ShnypApp, clip: u64) -> f64 {
    app.fc.session.active_sequence().unwrap().find_item(filmcraft_project::ClipId(clip)).map(|(_, it)| it.start.seconds()).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.08
}

#[test]
fn the_timeline_shows_the_video_and_filmcraft_s_is_not_docked() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("show");
    for c in [v.a, v.b, v.c, v.title] {
        assert!(
            v.app.widgets.contains_key(&format!("tl.clip.{c}")),
            "clip {c} is drawn: {:?}",
            v.app.widgets.keys().filter(|k| k.starts_with("tl.")).collect::<Vec<_>>()
        );
    }
    let mut panels = Vec::new();
    v.app.fc.ui.dock.panels(&mut panels);
    assert!(!panels.contains(&filmcraft_ui_egui::dock::PanelKind::Timeline), "FilmCraft's timeline is not docked: {panels:?}");
    // opening another sequence does not bring it back
    shnyp_video::execute(&mut v.app.fc.session, "shnyp.video.new", &json!({"format": "16:9"})).unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    let mut panels = Vec::new();
    v.app.fc.ui.dock.panels(&mut panels);
    assert!(!panels.contains(&filmcraft_ui_egui::dock::PanelKind::Timeline), "{panels:?}");
    snapshot(&v.d, "timeline");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn dragging_a_main_clip_to_the_front_reorders_the_video_and_its_title_goes_along() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("reorder");
    let c = v.app.widgets[&format!("tl.clip.{}", v.c)];
    let a = v.app.widgets[&format!("tl.clip.{}", v.a)];
    let undo = v.app.fc.session.history.undo.len();
    v.d.drag(&mut v.app, c.center(), pos2(a.min.x + 2.0, a.center().y), 12);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let m = mains(&v.app);
    assert_eq!(m.iter().map(|x| x.0).collect::<Vec<_>>(), vec![v.c, v.a, v.b], "{m:?}");
    assert_eq!(v.app.fc.session.history.undo.len(), undo + 1, "one undo step");
    assert!(close(start_of(&v.app, v.title), 0.5), "the title went with c: {}", start_of(&v.app, v.title));
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn dragging_an_edge_trims_and_the_rest_moves_up() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("trim");
    let a = v.app.widgets[&format!("tl.clip.{}", v.a)];
    let pps = a.width() as f64 / 4.0;
    // a's end one second earlier
    let from = pos2(a.max.x - 2.0, a.center().y);
    v.d.drag(&mut v.app, from, pos2(from.x - pps as f32, from.y), 10);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let m = mains(&v.app);
    assert!(close(m[0].2, 3.0) && close(m[1].1, 3.0), "a ends at 3 s and b follows: {m:?}");
    assert!(close(start_of(&v.app, v.title), 6.5), "the title on c moved up: {}", start_of(&v.app, v.title));
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn a_click_selects_and_the_ruler_moves_the_playhead() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("click");
    let b = v.app.widgets[&format!("tl.clip.{}", v.b)];
    v.d.click_at(&mut v.app, b.center());
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(v.app.fc.session.state.selection.contains(&filmcraft_project::ClipId(v.b)), "{:?}", v.app.fc.session.state.selection);
    let ruler = v.app.widgets["tl.ruler"];
    let a = v.app.widgets[&format!("tl.clip.{}", v.a)];
    v.d.click_at(&mut v.app, pos2(a.max.x, ruler.center().y));
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(close(v.app.fc.session.playhead().seconds(), 4.0), "{}", v.app.fc.session.playhead().seconds());
    // a click into empty space selects nothing
    let lane_end = pos2(ruler.max.x - 4.0, b.center().y);
    v.d.click_at(&mut v.app, lane_end);
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(v.app.fc.session.state.selection.is_empty());
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn profi_shows_filmcraft_s_timeline_in_its_place() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("profi");
    v.d.click(&mut v.app, "tl.view.full").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(v.app.timeline.full);
    assert!(v.app.fc.tl.layout.is_some(), "FilmCraft's timeline panel was drawn");
    assert!(!v.app.widgets.contains_key(&format!("tl.clip.{}", v.a)), "shnyp's clips are not");
    assert!(v.app.fc.ui_error.is_none(), "{:?}", v.app.fc.ui_error);
    snapshot(&v.d, "timeline-profi");
    v.d.click(&mut v.app, "tl.view.full").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(v.app.widgets.contains_key(&format!("tl.clip.{}", v.a)));
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn an_edit_made_with_filmcraft_s_own_tools_is_followed_in_its_step() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("native");
    let undo = v.app.fc.session.history.undo.len();
    // as FilmCraft's timeline does it: one execute, then the frame
    v.app.fc.session.execute("edit.rippleDelete", json!({"clips": [v.a]})).unwrap();
    v.d.step(&mut v.app, Vec::new());
    assert!(close(start_of(&v.app, v.title), 3.5), "the title on c moved up with it: {}", start_of(&v.app, v.title));
    assert_eq!(v.app.fc.session.history.undo.len(), undo + 1, "in FilmCraft's step");
    // and one undo brings both back
    v.app.fc.session.execute("edit.undo", json!({})).unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(close(start_of(&v.app, v.title), 7.5));
    assert_eq!(mains(&v.app).len(), 3);
    let _ = std::fs::remove_dir_all(&v.dir);
}
