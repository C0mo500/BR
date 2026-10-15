//! Speed, beats and templates in the window, driven headlessly: the Tempo popover bakes a curve,
//! freezes a frame and plays a clip backwards; "Beats" in the timeline marks the music's beats; a
//! template card on the start screen makes a video of placeholders, and the asset bar offers to
//! fill them. Clips are made with `ffmpeg` (skipped without it, unless BR_REQUIRE_FFMPEG).

use std::time::Duration;

use br_ui::BrApp;
use br_ui::headless::{Driver, png};
use serde_json::json;

const SETTLE: Duration = Duration::from_secs(90);

fn have_ffmpeg() -> bool {
    let found = std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("BR_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (BR_REQUIRE_FFMPEG is set)");
    found
}

fn ffmpeg(args: &[&str], out: &std::path::Path) {
    let o = std::process::Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(out).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

fn snapshot(d: &Driver, name: &str) {
    if let Some(dir) = std::env::var_os("BR_SNAPSHOT_DIR") {
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png(&d.paint())).unwrap();
    }
}

fn window() -> (BrApp, Driver) {
    lightcraft_engine::gpu::set_enabled(false);
    let mut app = BrApp::new(br_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1600.0, 1000.0], 1.0);
    assert!(d.settle(&mut app, SETTLE));
    (app, d)
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("br-speed-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn mains(app: &BrApp) -> Vec<(u64, f64, f64)> {
    let q = app.fc.session.active_sequence().unwrap();
    br_video::main_track::main_clips(q).iter().map(|it| (it.id.0, it.start.seconds(), it.end().seconds())).collect()
}

#[test]
fn the_tempo_popover_bakes_curves_freezes_and_reverses() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("tempo");
    let a = d.join("a.mp4");
    ffmpeg(&["-f", "lavfi", "-i", "testsrc2=size=360x640:rate=30:duration=4", "-c:v", "libx264", "-pix_fmt", "yuv420p"], &a);
    let (mut app, mut drv) = window();
    br_video::execute(&mut app.fc.session, "br.video.new", &json!({"format": "9:16"})).unwrap();
    br_video::execute(&mut app.fc.session, "br.video.add", &json!({"paths": [a.to_string_lossy()]})).unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    let clip = mains(&app)[0].0;
    let at = app.widgets[&format!("tl.clip.{clip}")].center();
    drv.click_at(&mut app, at);
    assert!(drv.settle(&mut app, SETTLE));
    drv.click(&mut app, "cut.speed").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    snapshot(&drv, "speed-popover");
    drv.click(&mut app, "speed.curve.montage").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    assert!(mains(&app).len() > 3, "baked into pieces: {:?}", mains(&app));
    drv.click(&mut app, "speed.curve.none").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    assert_eq!(mains(&app).len(), 1, "back together");
    drv.click(&mut app, "speed.reverse").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    let q = app.fc.session.active_sequence().unwrap();
    assert!(br_video::main_track::main_clips(q)[0].reverse, "backwards");
    app.fc.session.set_playhead(filmcraft_time::Tick::from_seconds_f64(1.0));
    drv.click(&mut app, "speed.freeze").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    let m = mains(&app);
    assert_eq!(m.len(), 3, "a still in the middle: {m:?}");
    assert!((m[2].2 - 7.0).abs() < 0.1, "3 s longer: {m:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn beats_in_the_timeline_and_a_template_from_the_start_screen() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("beats");
    let (mut app, mut drv) = window();
    drv.click(&mut app, "page.schnitt").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    // the start screen offers templates
    assert!(app.widgets.contains_key("start.template.story"), "{:?}", app.widgets.keys().filter(|k| k.starts_with("start")).collect::<Vec<_>>());
    snapshot(&drv, "start-templates");
    drv.click(&mut app, "start.template.story").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    assert_eq!(br_video::template::empty_placeholders(&app.fc.session).len(), 6);
    assert_eq!(app.assets.tab, br_ui::assets::Tab::Templates, "the asset bar shows what comes next");
    assert!(app.widgets.contains_key("assets.template.fill"), "Medien einsetzen");
    snapshot(&drv, "template-placeholders");
    // music with a steady beat; Beats marks it
    let clicks = d.join("klick.wav");
    ffmpeg(&["-f", "lavfi", "-i", "aevalsrc='if(lt(mod(t\\,0.5)\\,0.03)\\,sin(2*PI*1200*t)\\,0)':s=48000:d=12"], &clicks);
    br_video::execute(&mut app.fc.session, "br.video.music", &json!({"path": clicks.to_string_lossy(), "duck": false})).unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    drv.click(&mut app, "tl.beats").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    let marks = br_video::beats::markers(app.fc.session.active_sequence().unwrap());
    assert!(marks.len() >= 12, "beats marked: {marks:?}");
    drv.click(&mut app, "tl.beats").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    assert!(br_video::beats::markers(app.fc.session.active_sequence().unwrap()).is_empty(), "and away again");
    let _ = std::fs::remove_dir_all(&d);
}
