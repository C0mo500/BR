//! shnyp's inspector on the Schnitt page, driven headlessly: a clip clicked in the timeline shows its
//! Basis rows, a slider drag is one undo step, the ◆ sets a keyframe, tabs switch areas, Mehr
//! lists every effect and adds one from FilmCraft's list, Voll draws FilmCraft's own panel in the
//! column, typing a title's text is one step. Clips are made with `ffmpeg` (skipped without it,
//! unless SHNYP_REQUIRE_FFMPEG).

use std::time::Duration;

use egui::pos2;
use filmcraft_project::ClipId;
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
    title: u64,
    dir: std::path::PathBuf,
}

/// The window with a 9:16 video of two clips and a title on the second.
fn video(name: &str) -> Video {
    lightcraft_engine::gpu::set_enabled(false);
    let dir = std::env::temp_dir().join(format!("shnyp-insp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (clip(&dir, "a.mp4", 4), clip(&dir, "b.mp4", 3));
    let mut app = ShnypApp::new(shnyp_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1500.0, 950.0], 1.0);
    assert!(d.settle(&mut app, SETTLE));
    let run = |app: &mut ShnypApp, id: &str, p: serde_json::Value| {
        shnyp_video::execute(&mut app.fc.session, id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(&mut app, "shnyp.video.new", json!({"format": "9:16"}));
    run(&mut app, "shnyp.video.add", json!({"paths": [a, b]}));
    let title = run(&mut app, "shnyp.video.text", json!({"text": "Hallo", "seconds": 1.5, "at": 5.0}))["clip"].as_u64().unwrap();
    run(&mut app, "timeline.select", json!({"clips": []}));
    let q = app.fc.session.active_sequence().unwrap();
    let a = shnyp_video::main_track::main_clips(q)[0].id.0;
    assert!(d.settle(&mut app, SETTLE));
    Video { app, d, a, title, dir }
}

fn select(v: &mut Video, clip: u64) {
    let r = v.app.widgets[&format!("tl.clip.{clip}")];
    v.d.click_at(&mut v.app, r.center());
    assert!(v.d.settle(&mut v.app, SETTLE));
}

fn inspect(v: &Video, p: serde_json::Value) -> serde_json::Value {
    shnyp_video::inspect::inspect(&v.app.fc.session, &p).unwrap()
}

fn row(v: &Video, clip: u64, key: &str) -> serde_json::Value {
    let i = inspect(v, json!({"clip": clip}));
    i["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap().iter())
        .find(|r| r["key"] == key)
        .cloned()
        .unwrap_or_else(|| panic!("{key}: {i}"))
}

#[test]
fn a_selected_clip_shows_its_rows_and_a_slider_drag_is_one_undo_step() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("drag");
    assert!(v.app.widgets.contains_key("insp.level.basis"), "the inspector is there");
    assert!(!v.app.widgets.contains_key("insp.row.transform.scale"), "nothing selected yet");
    let a = v.a;
    select(&mut v, a);
    let slider = *v
        .app
        .widgets
        .get("insp.row.transform.scale")
        .unwrap_or_else(|| panic!("{:?}", v.app.widgets.keys().filter(|k| k.starts_with("insp")).collect::<Vec<_>>()));
    snapshot(&v.d, "inspector-basis");
    let before = row(&v, v.a, "transform.scale")["value"].as_f64().unwrap();
    let undo = v.app.fc.session.history.undo.len();
    let y = slider.center().y;
    v.d.drag(&mut v.app, pos2(slider.min.x + 30.0, y), pos2(slider.min.x + 90.0, y), 8);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let after = row(&v, v.a, "transform.scale")["value"].as_f64().unwrap();
    assert!((after - before).abs() > 1.0, "the scale changed: {before} → {after}");
    assert_eq!(v.app.fc.session.history.undo.len(), undo + 1, "one undo step for the drag");
    // the ◆ animates it
    v.d.click(&mut v.app, "insp.row.transform.scale.key").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert_eq!(row(&v, v.a, "transform.scale")["animated"], true);
    // another area: Anpassen puts the grade on the clip
    v.d.click(&mut v.app, "insp.tab.adjust").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    let exposure = v.app.widgets["insp.row.adjust.exposure"];
    v.d.drag(&mut v.app, pos2(exposure.min.x + 40.0, exposure.center().y), pos2(exposure.min.x + 70.0, exposure.center().y), 5);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let it = v.app.fc.session.active_sequence().unwrap().find_item(ClipId(v.a)).unwrap().1.clone();
    assert_eq!(it.effects[0].effect, "lumetri", "the grade went on the clip");
    assert!(v.app.fc.ui_error.is_none(), "{:?}", v.app.fc.ui_error);
    snapshot(&v.d, "inspector-anpassen");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn mehr_lists_every_effect_and_adds_one_from_filmcraft_s_list() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("mehr");
    let a = v.a;
    select(&mut v, a);
    v.d.click(&mut v.app, "insp.level.mehr").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    let it = v.app.fc.session.active_sequence().unwrap().find_item(ClipId(v.a)).unwrap().1.clone();
    let motion = it.effects.iter().position(|e| e.effect == "motion").unwrap();
    assert!(
        v.app.widgets.contains_key(&format!("insp.fx.{}.{motion}", v.a)),
        "{:?}",
        v.app.widgets.keys().filter(|k| k.starts_with("insp.fx")).collect::<Vec<_>>()
    );
    v.d.click(&mut v.app, "insp.add").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    snapshot(&v.d, "inspector-mehr");
    v.d.click(&mut v.app, "insp.add.gaussian_blur").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    let it = v.app.fc.session.active_sequence().unwrap().find_item(ClipId(v.a)).unwrap().1.clone();
    let blur = it.effects.iter().position(|e| e.effect == "gaussian_blur").expect("the blur was added");
    // it folds open with its parameters
    v.d.click(&mut v.app, &format!("insp.fx.{}.{blur}", v.a)).unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(
        v.app.widgets.contains_key(&format!("insp.fx.{}.{blur}.blurriness", v.a)),
        "{:?}",
        v.app.widgets.keys().filter(|k| k.starts_with("insp.fx")).collect::<Vec<_>>()
    );
    v.d.click(&mut v.app, &format!("insp.fx.{}.{blur}.enable", v.a)).unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    let it = v.app.fc.session.active_sequence().unwrap().find_item(ClipId(v.a)).unwrap().1.clone();
    assert!(!it.effects[blur].enabled, "switched off");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn voll_draws_filmcraft_s_panel_in_the_column() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("voll");
    let mut panels = Vec::new();
    v.app.fc.ui.dock.panels(&mut panels);
    use filmcraft_ui_egui::dock::PanelKind;
    for p in [PanelKind::Properties, PanelKind::EffectControls, PanelKind::LumetriColor, PanelKind::EssentialSound] {
        assert!(!panels.contains(&p), "{p:?} is shnyp's inspector's now: {panels:?}");
    }
    let a = v.a;
    select(&mut v, a);
    v.d.click(&mut v.app, "insp.level.voll").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(v.app.widgets.contains_key("insp.voll.effects"));
    assert!(v.app.fc.ui_error.is_none(), "{:?}", v.app.fc.ui_error);
    let texts = v.d.texts();
    assert!(texts.iter().any(|t| t.contains("Motion") || t.contains("Bewegung")), "Effect Controls shows Motion: {texts:?}");
    snapshot(&v.d, "inspector-voll");
    for key in ["properties", "lumetri", "sound", "captions"] {
        v.d.click(&mut v.app, &format!("insp.voll.{key}")).unwrap();
        assert!(v.d.settle(&mut v.app, SETTLE));
        assert!(v.app.fc.ui_error.is_none(), "{key}: {:?}", v.app.fc.ui_error);
    }
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn typing_a_title_s_text_is_one_undo_step() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("text");
    let title = v.title;
    select(&mut v, title);
    let field = v.app.widgets["insp.row.text.text"];
    v.d.click_at(&mut v.app, field.center());
    v.d.key(&mut v.app, egui::Key::A, egui::Modifiers::COMMAND);
    let undo = v.app.fc.session.history.undo.len();
    for ch in ["W", "e", "l", "t"] {
        v.d.step(&mut v.app, vec![egui::Event::Text(ch.into())]);
    }
    // the keys typed go to the field, not to FilmCraft's shortcuts
    assert_eq!(row(&v, v.title, "text.text")["value"], "Welt");
    assert_eq!(v.app.fc.session.history.undo.len(), undo + 1, "one step for the typing");
    snapshot(&v.d, "inspector-text");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn the_level_is_switched_from_outside() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("control");
    let r = shnyp_ui::inspector::control(&mut v.app, &json!({"panel": "lumetri"})).unwrap();
    assert_eq!(r, json!({"level": "voll", "panel": "lumetri"}));
    assert!(shnyp_ui::inspector::control(&mut v.app, &json!({"level": "alles"})).is_err());
    shnyp_ui::inspector::control(&mut v.app, &json!({"level": "basis"})).unwrap();
    assert_eq!(v.app.inspector.level, shnyp_ui::inspector::Level::Basis);
    let _ = std::fs::remove_dir_all(&v.dir);
}
