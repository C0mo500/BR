//! What the review of M2 found in the window, kept from coming back: a click on a popover lying
//! over the timeline belongs to the popover (a look clicked there goes on the selected clip, not
//! on all of them); Entf deletes project items in FilmCraft's Project panel and clips (closing the
//! gap) in the timeline; a typed number in the command search's form stays text where text is
//! asked for; BR's keys stay out of a field being typed in. Clips are made with `ffmpeg` (skipped
//! without it, unless BR_REQUIRE_FFMPEG).

use std::time::Duration;

use br_ui::BrApp;
use br_ui::headless::Driver;
use egui::{Event, Key, Modifiers};
use filmcraft_project::ClipId;
use serde_json::json;

const SETTLE: Duration = Duration::from_secs(90);

fn have_ffmpeg() -> bool {
    let found = std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("BR_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (BR_REQUIRE_FFMPEG is set)");
    found
}

fn clip(dir: &std::path::Path, name: &str) -> String {
    let p = dir.join(name);
    let o = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=360x640:rate=30:duration=3"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&p)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    p.to_string_lossy().into_owned()
}

struct Video {
    app: BrApp,
    d: Driver,
    a: u64,
    b: u64,
    dir: std::path::PathBuf,
}

/// A window of `size` with a video of two clips, nothing selected.
fn video(name: &str, size: [f32; 2]) -> Video {
    lightcraft_engine::gpu::set_enabled(false);
    let dir = std::env::temp_dir().join(format!("br-review-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (clip(&dir, "a.mp4"), clip(&dir, "b.mp4"));
    let mut app = BrApp::new(br_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new(size, 1.0);
    assert!(d.settle(&mut app, SETTLE));
    br_video::execute(&mut app.fc.session, "br.video.new", &json!({"format": "9:16"})).unwrap();
    br_video::execute(&mut app.fc.session, "br.video.add", &json!({"paths": [a, b]})).unwrap();
    br_video::execute(&mut app.fc.session, "timeline.select", &json!({"clips": []})).unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let m = mains(&app);
    Video { a: m[0], b: m[1], app, d, dir }
}

fn mains(app: &BrApp) -> Vec<u64> {
    br_video::main_track::main_clips(app.fc.session.active_sequence().unwrap()).iter().map(|it| it.id.0).collect()
}

fn has_look(app: &BrApp, c: u64) -> bool {
    app.fc.session.active_sequence().unwrap().find_item(ClipId(c)).is_some_and(|(_, it)| it.effect("lumetri").is_some())
}

#[test]
fn a_look_clicked_on_a_popover_over_the_timeline_goes_on_the_selected_clip_only() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("popover", [1400.0, 720.0]);
    let a = v.app.widgets[&format!("tl.clip.{}", v.a)];
    v.d.click_at(&mut v.app, a.center());
    assert!(v.d.settle(&mut v.app, SETTLE));
    v.d.click(&mut v.app, "cut.filter").unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    // a look lying over the timeline
    // over the lanes, not over clip a (a press there would only select a again)
    let top = v.app.widgets["tl.ruler"].max.y;
    let shown = v.app.widgets["popover.filter"].shrink(2.0);
    let look = v
        .app
        .widgets
        .iter()
        .filter(|(k, r)| {
            k.starts_with("filter.")
                && *k != "filter.none"
                && r.center().y > top + 4.0
                && r.height() > 10.0
                && shown.contains_rect(**r)
                && !a.contains(r.center())
        })
        .map(|(k, _)| k.clone())
        .next()
        .expect("the popover reaches over the timeline in a low window");
    v.d.click(&mut v.app, &look).unwrap();
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert!(has_look(&v.app, v.a), "the look went on the selected clip");
    assert!(!has_look(&v.app, v.b), "and only there");
    assert!(v.app.fc.session.state.selection.contains(&ClipId(v.a)), "the click did not reach the timeline");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn entf_deletes_project_items_in_the_project_panel_and_clips_in_the_timeline() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("entf", [1600.0, 1000.0]);
    let b = v.app.widgets[&format!("tl.clip.{}", v.b)];
    v.d.click_at(&mut v.app, b.center());
    assert!(v.d.settle(&mut v.app, SETTLE));
    // FilmCraft's Project panel has the focus: Entf is its own (no clip goes)
    v.app.fc.ui.focused = filmcraft_ui_egui::dock::PanelKind::Project;
    v.d.key(&mut v.app, Key::Delete, Modifiers::NONE);
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert_eq!(mains(&v.app), [v.a, v.b], "the Project panel's Entf left the clips alone");
    // the timeline has it: b goes, the gap closes
    v.d.click_at(&mut v.app, b.center());
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert_eq!(v.app.fc.ui.focused, filmcraft_ui_egui::dock::PanelKind::Timeline);
    v.d.key(&mut v.app, Key::Delete, Modifiers::NONE);
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert_eq!(mains(&v.app), [v.a], "Entf in the timeline: Löschen with the gap closed");
    let _ = std::fs::remove_dir_all(&v.dir);
}

#[test]
fn the_command_search_keeps_a_typed_number_as_text_and_keys_stay_in_fields() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("form", [1600.0, 1000.0]);
    v.d.key(&mut v.app, Key::K, Modifiers::COMMAND);
    v.d.step(&mut v.app, vec![Event::Text("br.video.text".into())]);
    assert!(v.d.settle(&mut v.app, SETTLE));
    v.d.key(&mut v.app, Key::Enter, Modifiers::NONE);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let field = v.app.widgets["palette.field.text"];
    v.d.click_at(&mut v.app, field.center());
    v.d.step(&mut v.app, vec![Event::Text("2025".into())]);
    // ⌘B in the field is not Teilen
    v.d.key(&mut v.app, Key::B, Modifiers::COMMAND);
    assert!(v.d.settle(&mut v.app, SETTLE));
    assert_eq!(mains(&v.app).len(), 2, "no clip was split while typing");
    v.d.key(&mut v.app, Key::Enter, Modifiers::NONE);
    assert!(v.d.settle(&mut v.app, SETTLE));
    let title = {
        let q = v.app.fc.session.active_sequence().unwrap();
        q.video_tracks.iter().flat_map(|t| &t.items).find(|it| it.graphic_layers().next().is_some()).expect("a title").id.0
    };
    let text = br_video::execute(&mut v.app.fc.session, "br.video.inspect", &json!({"clip": title})).unwrap();
    let rows: Vec<serde_json::Value> =
        text["sections"].as_array().unwrap().iter().flat_map(|s| s["rows"].as_array().unwrap().clone()).filter(|r| r["key"] == "text.text").collect();
    assert_eq!(rows[0]["value"], "2025", "{rows:?}");
    let _ = std::fs::remove_dir_all(&v.dir);
}
