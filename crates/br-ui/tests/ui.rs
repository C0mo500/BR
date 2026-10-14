//! BR's window driven headlessly on LightCraft's demo photos: clicks, drags and keys in, the
//! session's state and painted pixels out. Set `BR_SNAPSHOT_DIR` to also write screenshots.

use std::time::Duration;

use br_photo::crop::{self, Format};
use br_photo::looks;
use br_ui::headless::{Driver, png};
use br_ui::{BrApp, Mode, Tab};
use lightcraft_ui_egui::state::{RightPanel, ViewMode};

const SETTLE: Duration = Duration::from_secs(90);

fn app() -> (BrApp, Driver) {
    lightcraft_engine::gpu::set_enabled(false);
    let mut app = BrApp::new(br_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1280.0, 800.0], 1.0);
    assert!(d.settle(&mut app, SETTLE), "the first frames settle");
    (app, d)
}

fn snapshot(d: &Driver, name: &str) {
    if let Some(dir) = std::env::var_os("BR_SNAPSHOT_DIR") {
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        std::fs::write(&path, png(&d.paint())).unwrap();
    }
}

/// How different the pixels of an image are: 0 for a single colour.
fn variety(img: &egui::ColorImage) -> usize {
    let mut seen = std::collections::HashSet::new();
    for p in img.pixels.iter().step_by(97) {
        seen.insert(p.to_array());
    }
    seen.len()
}

#[test]
fn simple_mode_shows_the_photo_the_tabs_and_the_filmstrip() {
    let (mut app, mut d) = app();
    assert_eq!(app.mode, Mode::Simple);
    let texts = d.texts();
    for t in ["Looks", "Anpassen", "Format", "Export", "Einfach", "Pro", "Exportieren"] {
        assert!(texts.iter().any(|x| x.contains(t)), "`{t}` not drawn: {texts:?}");
    }
    let id = app.lc.session.active().unwrap();
    assert!(app.lc.renderer.textures.get(&lightcraft_ui_egui::render::Slot::Main).is_some_and(|t| t.photo == id), "the preview rendered");
    assert!(app.widgets.contains_key(&format!("photo.{}", id.0)));
    let img = d.paint();
    assert_eq!(img.size, [1280, 800]);
    assert!(variety(&img) > 200, "a photo, not a blank window");
    snapshot(&d, "1-looks");

    for tab in [Tab::Adjust, Tab::Format, Tab::Export] {
        d.click(&mut app, &format!("tab.{}", tab.key())).unwrap();
        assert!(d.settle(&mut app, SETTLE));
        assert_eq!(app.simple.tab, tab);
        snapshot(&d, &format!("{}-{}", tab as usize + 1, tab.key()));
    }
}

#[test]
fn a_look_is_one_tap_and_its_amount_one_undo_step() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    let undo0 = app.lc.session.undo.len();
    d.click(&mut app, "look.lc.warm-glow").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let l = looks::current(&app.simple.br.look, &app.lc.session, id).expect("look applied");
    assert_eq!((l.preset_id.as_str(), l.amount), ("lc.warm-glow", 100.0));
    assert_eq!(app.lc.session.undo.len(), undo0 + 1);
    snapshot(&d, "look-applied");

    // drag the Amount slider to the left
    let r = app.widgets["look.amount"];
    d.drag(&mut app, r.center(), egui::pos2(r.left() + r.width() * 0.2, r.center().y), 6);
    assert!(d.settle(&mut app, SETTLE));
    let l = looks::current(&app.simple.br.look, &app.lc.session, id).expect("still the current look");
    assert!(l.amount < 60.0, "amount {}", l.amount);
    assert_eq!(app.lc.session.undo.len(), undo0 + 1, "the drag replaced the look's step");
    assert!(app.lc.session.interaction.is_none());

    // "Original" takes it off again
    d.click(&mut app, "look.original").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(looks::current(&app.simple.br.look, &app.lc.session, id).map(|l| l.amount), Some(0.0));
}

#[test]
fn sliders_drag_as_one_undo_step_and_undo_from_the_keyboard() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    d.click(&mut app, "tab.adjust").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let undo0 = app.lc.session.undo.len();
    let before = app.lc.session.develop_of(id).unwrap();
    let r = app.widgets["slider.light"];
    d.drag(&mut app, r.center(), egui::pos2(r.left() + r.width() * 0.8, r.center().y), 8);
    assert!(d.settle(&mut app, SETTLE));
    let light = br_photo::adjust::values(&app.lc.session, id)[0].1;
    assert!(light > 30.0, "Licht {light}");
    assert_eq!(app.lc.session.undo.len(), undo0 + 1);
    assert_eq!(app.lc.session.undo.last().unwrap().label, "Licht");
    snapshot(&d, "adjust-dragged");

    // Ctrl+Z is BR's undo, not LightCraft's too (one step, not two)
    d.key(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.undo.len(), undo0);
    assert_eq!(app.lc.session.develop_of(id).unwrap(), before);

    // LightCraft's own shortcuts stay quiet in Simple mode: 3 would rate, Delete would ask to delete
    let rating = app.lc.session.catalog.photo(id).unwrap().rating;
    d.key(&mut app, egui::Key::Num3, egui::Modifiers::NONE);
    d.key(&mut app, egui::Key::Delete, egui::Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.catalog.photo(id).unwrap().rating, rating);
    assert!(app.lc.ui.dialog.is_none());

    // Auto
    d.click(&mut app, "adjust.auto").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.undo.last().unwrap().label, "Auto");
}

#[test]
fn formats_crop_exactly_and_the_filmstrip_switches_photos() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    d.click(&mut app, "tab.format").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    d.click(&mut app, "format.9:16").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(crop::current(&app.lc.session, id), Some(Format::Story));
    assert!(d.texts().iter().any(|t| t.contains("· 9:16")), "the caption names the format");
    snapshot(&d, "format-story");

    let other = app.lc.session.visible_cloned().into_iter().find(|p| *p != id).unwrap();
    d.click(&mut app, &format!("photo.{}", other.0)).unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.active(), Some(other));
    d.key(&mut app, egui::Key::ArrowLeft, egui::Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert_ne!(app.lc.session.active(), Some(other), "arrow keys step through the photos");
}

#[test]
fn export_writes_an_exact_instagram_file() {
    let (mut app, mut d) = app();
    let dir = std::env::temp_dir().join(format!("br-ui-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.simple.export_dir = dir.clone();
    d.click(&mut app, "export").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.simple.tab, Tab::Export, "the top bar's Export opens the tab");
    d.click(&mut app, "preset.instagram").unwrap();
    d.click(&mut app, "export.go").unwrap();
    assert!(d.settle(&mut app, Duration::from_secs(180)), "export finished");
    let files = app.simple.last_export.clone().expect("exported");
    assert_eq!((files[0]["width"].as_u64(), files[0]["height"].as_u64()), (Some(1080), Some(1350)));
    assert!(std::path::Path::new(files[0]["path"].as_str().unwrap()).exists());
    snapshot(&d, "export-done");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pro_mode_is_lightcraft_on_the_same_photo_and_back() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    // an edit in Simple mode ...
    br_photo::adjust::set(&mut app.lc.session, id, &[(br_photo::adjust::Slider::Contrast, 40.0)]).unwrap();
    d.click(&mut app, "mode.pro").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.mode, Mode::Pro);
    assert_eq!(app.lc.session.active(), Some(id));
    assert_eq!((app.lc.ui.view, app.lc.ui.right), (ViewMode::Detail, RightPanel::Edit));
    // ... shows in LightCraft's Edit panel
    snapshot(&d, "pro");
    let texts = d.texts();
    assert!(texts.iter().any(|t| t.trim_start_matches('+') == "40"), "Contrast 40 in the Pro panel: {texts:?}");
    assert!(variety(&d.paint()) > 200);
    // an edit in Pro mode shows in Simple mode
    app.lc.run("develop.set", serde_json::json!({"control": "light.exposure", "value": 1.0})).unwrap();
    d.click(&mut app, "mode.simple").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.mode, Mode::Simple);
    assert_eq!(br_photo::adjust::values(&app.lc.session, id)[0].1, 50.0);
}

#[test]
fn a_second_look_replaces_the_first() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    let before = app.lc.session.develop_of(id).unwrap();
    let undo0 = app.lc.session.undo.len();
    d.click(&mut app, "look.lc.warm-glow").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    d.click(&mut app, "look.lc.cool-morning").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let s = &app.lc.session;
    let cool = looks::find(s, "lc.cool-morning").unwrap().clone();
    assert_eq!(*s.develop_of(id).unwrap(), looks::blend_look_for(&s.source_info(id), &before, &cool, 1.0), "exactly the tile");
    assert_eq!(s.undo.len(), undo0 + 1);
    d.click(&mut app, "look.original").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.develop_of(id).unwrap(), before);
}

#[test]
fn a_quick_release_is_still_one_undo_step() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    let before = app.lc.session.develop_of(id).unwrap();
    d.click(&mut app, "tab.adjust").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let undo0 = app.lc.session.undo.len();
    let r = app.widgets["slider.contrast"];
    let (from, mid, to) = (r.center(), egui::pos2(r.left() + r.width() * 0.65, r.center().y), egui::pos2(r.left() + r.width() * 0.8, r.center().y));
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    d.step(&mut app, vec![egui::Event::PointerMoved(from)]);
    d.step(&mut app, vec![button(from, true)]);
    d.step(&mut app, vec![egui::Event::PointerMoved(mid)]);
    // the last move and the release in the same frame
    d.step(&mut app, vec![egui::Event::PointerMoved(to), button(to, false)]);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.undo.len(), undo0 + 1, "{:?}", app.lc.session.undo.iter().map(|u| &u.label).collect::<Vec<_>>());
    d.key(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.develop_of(id).unwrap(), before);
}

#[test]
fn simple_mode_says_when_nothing_is_saved() {
    let (mut app, mut d) = app();
    app.library_problem("/nowhere/BR Library", "This library is already open in br.", Vec::new());
    app.lc.library_problem.as_mut().unwrap().dismissed = true; // Continue Without Saving
    assert!(d.settle(&mut app, SETTLE));
    assert!(d.texts().iter().any(|t| t.contains("Vorübergehende Sitzung") || t.contains("Temporary session")), "{:?}", d.texts());
}

#[test]
fn an_export_is_a_task_and_finishes_in_pro_mode_too() {
    let (mut app, mut d) = app();
    let dir = std::env::temp_dir().join(format!("br-ui-export-pro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.simple.export_dir = dir.clone();
    app.simple.tab = Tab::Export;
    assert!(d.settle(&mut app, SETTLE));
    d.click(&mut app, "export.go").unwrap();
    assert!(app.simple.exporting());
    assert!(!app.lc.session.activity.running_cancellable().is_empty(), "the quit prompt knows about it");
    app.open_pro();
    assert!(d.settle(&mut app, Duration::from_secs(180)), "settles: the export is collected in Pro mode");
    assert!(!app.simple.exporting());
    assert!(app.simple.last_export.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn looks_made_in_pro_mode_show_up() {
    let (mut app, mut d) = app();
    app.lc.run("preset.create", serde_json::json!({"name": "Mein Look"})).unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let id = app.lc.session.presets.iter().find(|p| p.name == "Mein Look").unwrap().id.clone();
    assert!(app.widgets.contains_key(&format!("look.{id}")), "new look listed");
}

#[test]
fn a_pro_filter_that_hides_everything_can_be_cleared() {
    let (mut app, mut d) = app();
    app.lc.run("library.filter", serde_json::json!({"text": "zzzzzz"})).unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert!(d.texts().iter().any(|t| t.contains("Keine Fotos in dieser Ansicht")), "{:?}", d.texts());
    d.click(&mut app, "show.all").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert!(app.lc.session.active().is_some());
}

#[test]
fn quit_anyway_stops_an_export_without_waiting() {
    let (mut app, mut d) = app();
    let dir = std::env::temp_dir().join(format!("br-ui-export-cancel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.simple.export_dir = dir.clone();
    app.simple.export_preset = br_photo::export::Preset::Instagram; // a second or more of rendering
    app.simple.tab = Tab::Export;
    assert!(d.settle(&mut app, SETTLE));
    d.click(&mut app, "export.go").unwrap();
    assert!(app.simple.exporting());
    assert_eq!(app.lc.session.activity.cancel_all(), 1, "Quit Anyway cancels the export");
    let t = std::time::Instant::now();
    // what closing the window does (not LightCraft's process-wide stop: other tests run here too)
    app.close();
    assert!(t.elapsed() < Duration::from_secs(5), "on_exit waited {:?}", t.elapsed());
    // the render that was running finishes on its own; it must not be written
    std::thread::sleep(Duration::from_secs(8));
    assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).all(|e| !e.file_name().to_string_lossy().ends_with(".jpg")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn choose_another_library_works_in_simple_mode() {
    let (mut app, mut d) = app();
    let lib = std::env::temp_dir().join(format!("br-ui-other-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&lib);
    let answer = lib.to_string_lossy().into_owned();
    app.lc.services.picker = Some(Box::new(move |_req| {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(vec![answer.clone()]).unwrap();
        Ok(rx)
    }));
    // the window's services have a folder dialog (the button is off without one)
    app.lc.services.pick_folder = Some(Box::new(|| None));
    app.library_problem("/nowhere/BR Library", "This library is already open in br.", Vec::new());
    assert!(d.settle(&mut app, SETTLE));
    let r = app.lc.widgets.iter().find(|(id, _)| id == "button:libraryChoose").map(|(_, r)| *r).expect("the library problem window is shown");
    d.click_at(&mut app, r.center());
    assert!(d.settle(&mut app, SETTLE));
    assert!(app.lc.library_problem.is_none(), "the chosen library opened");
    assert_eq!(app.library.as_deref(), Some(lib.to_string_lossy().as_ref()));
    app.close();
    let _ = std::fs::remove_dir_all(&lib);
}
