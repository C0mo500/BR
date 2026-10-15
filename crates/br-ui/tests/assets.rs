//! BR's asset bar and drag and drop on the Schnitt page, driven headlessly: "+" on a media tile
//! goes in at the cut nearest the playhead, a tile dragged above the main track is an overlay
//! there, onto the main track an insert at the nearest cut, onto the Program picture an overlay
//! where it was dropped; an effect or a look dragged onto a clip goes onto that clip; a title
//! from the Text tab; files from the file manager dropped onto the timeline; "Profi" shows
//! FilmCraft's Project panel. Clips are made with `ffmpeg` (skipped without it, unless
//! BR_REQUIRE_FFMPEG).

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
    if let Some(dir) = std::env::var_os("BR_SNAPSHOT_DIR") {
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png(&d.paint())).unwrap();
    }
}

struct Video {
    app: BrApp,
    d: Driver,
    a: u64,
    b: u64,
    /// A third clip, imported into the project but not in the video.
    extra: u64,
    extra_path: String,
    dir: std::path::PathBuf,
}

fn video(name: &str) -> Video {
    lightcraft_engine::gpu::set_enabled(false);
    let dir = std::env::temp_dir().join(format!("br-assets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b, c) = (clip(&dir, "a.mp4", 4), clip(&dir, "b.mp4", 3), clip(&dir, "c.mp4", 2));
    let mut app = BrApp::new(br_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1600.0, 1000.0], 1.0);
    assert!(d.settle(&mut app, SETTLE));
    let run =
        |app: &mut BrApp, id: &str, p: serde_json::Value| br_video::execute(&mut app.fc.session, id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut app, "br.video.new", json!({"format": "9:16"}));
    run(&mut app, "br.video.add", json!({"paths": [a, b]}));
    let extra = run(&mut app, "file.import", json!({"paths": [&c]}))["items"][0].as_u64().unwrap();
    run(&mut app, "timeline.select", json!({"clips": []}));
    let m = mains(&app);
    assert!(d.settle(&mut app, SETTLE));
    Video { a: m[0].0, b: m[1].0, extra, extra_path: c, app, d, dir }
}

fn mains(app: &BrApp) -> Vec<(u64, f64, f64)> {
    let q = app.fc.session.active_sequence().unwrap();
    br_video::main_track::main_clips(q).iter().map(|it| (it.id.0, it.start.seconds(), it.end().seconds())).collect()
}

/// The clips of `item` in the video: (clip, video track index or 100 + audio track index, start).
fn placed(app: &BrApp, item: u64) -> Vec<(u64, usize, f64)> {
    let q = app.fc.session.active_sequence().unwrap();
    let mut out = Vec::new();
    for (i, t) in q.video_tracks.iter().enumerate() {
        out.extend(t.items.iter().filter(|it| it.item.0 == item).map(|it| (it.id.0, i, it.start.seconds())));
    }
    for (i, t) in q.audio_tracks.iter().enumerate() {
        out.extend(t.items.iter().filter(|it| it.item.0 == item).map(|it| (it.id.0, 100 + i, it.start.seconds())));
    }
    out
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.1
}

#[test]
fn the_plus_on_a_media_tile_goes_in_at_the_nearest_cut() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("plus");
    let mut panels = Vec::new();
    v.app.fc.ui.dock.panels(&mut panels);
    assert_eq!(panels, vec![filmcraft_ui_egui::dock::PanelKind::Program, filmcraft_ui_egui::dock::PanelKind::Source], "the dock is the picture only");
    assert!(
        v.app.widgets.contains_key(&format!("assets.media.{}", v.extra)),
        "{:?}",
        v.app.widgets.keys().filter(|k| k.starts_with("assets")).collect::<Vec<_>>()
    );
    snapshot(&v.d, "assets-media");
    v.app.fc.session.set_playhead(filmcraft_time::Tick::from_seconds_f64(3.0));
    let undo = v.app.fc.session.history.undo.len();
    let tile = v.app.widgets[&format!("assets.media.{}", v.extra)];
    v.d.step(&mut v.app, vec![egui::Event::PointerMoved(tile.center())]);
    v.d.click(&mut v.app, &format!("assets.media.{}.add", v.extra)).unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    let m = mains(&v.app);
    assert_eq!(m.len(), 3, "{m:?}");
    assert!(close(m[1].1, 4.0) && m[0].0 == v.a && m[2].0 == v.b, "in at the cut at 4 s: {m:?}");
    assert_eq!(v.app.fc.session.history.undo.len(), undo + 1);
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn a_tile_dragged_onto_the_timeline_lands_by_the_rules() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("drag");
    let tile = v.app.widgets[&format!("assets.media.{}", v.extra)];
    let a = v.app.widgets[&format!("tl.clip.{}", v.a)];
    let pps = a.width() / 4.0;
    // above the main track at 1 s: an overlay there
    let to = pos2(a.min.x + pps, a.min.y - 14.0);
    v.d.drag(&mut v.app, tile.center(), to, 10);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let p = placed(&v.app, v.extra);
    let over = p.iter().find(|x| x.1 >= 1 && x.1 < 100).unwrap_or_else(|| panic!("an overlay: {p:?}"));
    assert!(close(over.2, 1.0), "at 1 s: {p:?}");
    assert_eq!(mains(&v.app).len(), 2, "the main track is as it was");
    // onto the main track near its end: in at the cut at 4 s
    let a = v.app.widgets[&format!("tl.clip.{}", v.a)];
    v.d.drag(&mut v.app, tile.center(), pos2(a.max.x - 10.0, a.center().y), 10);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let m = mains(&v.app);
    assert_eq!(m.len(), 3, "{m:?}");
    assert!(close(m[1].1, 4.0) && m[2].0 == v.b, "{m:?}");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn a_tile_dropped_on_the_picture_is_an_overlay_where_it_was_dropped() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("monitor");
    let pic = v
        .app
        .fc
        .auto
        .find("program.picture")
        .map(|e| egui::Rect::from_min_size(pos2(e.rect[0], e.rect[1]), egui::vec2(e.rect[2], e.rect[3])))
        .expect("the Program picture");
    let tile = v.app.widgets[&format!("assets.media.{}", v.extra)];
    let to = pos2(pic.min.x + pic.width() * 0.25, pic.min.y + pic.height() * 0.25);
    v.d.drag(&mut v.app, tile.center(), to, 10);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let p = placed(&v.app, v.extra);
    let (clip, track, _) = *p.iter().find(|x| x.1 >= 1 && x.1 < 100).unwrap_or_else(|| panic!("an overlay: {p:?}"));
    assert!(track >= 1);
    let q = v.app.fc.session.active_sequence().unwrap();
    let pos = q.find_item(ClipId(clip)).unwrap().1.effect("motion").unwrap().params["position"].value.as_vec2().unwrap();
    assert!((pos.x - 270.0).abs() < 30.0 && (pos.y - 480.0).abs() < 50.0, "a quarter in: {pos:?}");
    // a second drop right after it lands too (FilmCraft's drag ghost must not swallow it)
    let tile = v.app.widgets[&format!("assets.media.{}", v.extra)];
    let to = pos2(pic.min.x + pic.width() * 0.75, pic.min.y + pic.height() * 0.75);
    v.d.drag(&mut v.app, tile.center(), to, 10);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let p = placed(&v.app, v.extra);
    assert_eq!(p.iter().filter(|x| x.1 >= 1 && x.1 < 100).count(), 2, "two overlays: {p:?}");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn effects_looks_and_titles_are_dragged_onto_clips_and_the_timeline() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("effects");
    v.d.click(&mut v.app, "assets.tab.effects").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    snapshot(&v.d, "assets-effects");
    let blur = *v.app.widgets.get("assets.effect.gaussian_blur").expect("the blur tile");
    let b = v.app.widgets[&format!("tl.clip.{}", v.b)];
    v.d.drag(&mut v.app, blur.center(), b.center(), 10);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let fx = |app: &BrApp, c: u64, id: &str| {
        app.fc.session.active_sequence().unwrap().find_item(ClipId(c)).unwrap().1.effects.iter().filter(|e| e.effect == id).count()
    };
    assert_eq!(fx(&v.app, v.b, "gaussian_blur"), 1, "onto b");
    assert_eq!(fx(&v.app, v.a, "gaussian_blur"), 0);
    // a look onto a
    v.d.click(&mut v.app, "assets.tab.looks").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    let look = *v.app.widgets.get("assets.look.Golden Dusk").expect("the look tile");
    let a = v.app.widgets[&format!("tl.clip.{}", v.a)];
    v.d.drag(&mut v.app, look.center(), a.center(), 10);
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert_eq!(fx(&v.app, v.a, "lumetri"), 1, "the look on a");
    // a title onto the timeline at 5 s
    v.d.click(&mut v.app, "assets.tab.text").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    let text = v.app.widgets["assets.text.default"];
    let b = v.app.widgets[&format!("tl.clip.{}", v.b)];
    let pps = b.width() / 3.0;
    v.d.drag(&mut v.app, text.center(), pos2(b.min.x + pps, b.min.y - 14.0), 10);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let q = v.app.fc.session.active_sequence().unwrap();
    let title = q.video_tracks.iter().flat_map(|t| &t.items).find(|it| it.graphic_layers().next().is_some()).map(|it| it.start.seconds());
    assert!(title.is_some_and(|s| close(s, 5.0)), "a title at 5 s: {title:?}");
    // FilmCraft's own panels under Profi
    v.d.click(&mut v.app, "assets.tab.profi").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(v.app.fc.ui_error.is_none(), "{:?}", v.app.fc.ui_error);
    snapshot(&v.d, "assets-profi");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn files_from_the_file_manager_are_placed_where_they_are_dropped() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("os");
    let a = v.app.widgets[&format!("tl.clip.{}", v.a)];
    let pps = a.width() / 4.0;
    // above the main track at 2 s
    let at = pos2(a.min.x + 2.0 * pps, a.min.y - 14.0);
    v.d.step(&mut v.app, vec![egui::Event::PointerMoved(at)]);
    let path = std::path::PathBuf::from(&v.extra_path);
    v.d.drop_files(&mut v.app, &[path]);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let q = v.app.fc.session.active_sequence().unwrap();
    let main = br_video::main_track::main_track_id(q);
    let over: Vec<f64> = q.video_tracks.iter().filter(|t| Some(t.id) != main).flat_map(|t| &t.items).map(|it| it.start.seconds()).collect();
    assert!(over.iter().any(|s| close(*s, 2.0)), "an overlay at 2 s: {over:?}");
    assert_eq!(mains(&v.app).len(), 2);
    let _ = std::fs::remove_dir_all(&v.dir);
}
