//! BR's window driven headlessly on LightCraft's demo photos: clicks, drags and keys in, the
//! session's state and painted pixels out. Set `BR_SNAPSHOT_DIR` to also write screenshots.

use std::time::Duration;

use br_photo::crop::{self, Format};
use br_photo::looks;
use br_ui::headless::{Driver, png};
use br_ui::{BrApp, Page, Popover};

const SETTLE: Duration = Duration::from_secs(90);

fn app() -> (BrApp, Driver) {
    lightcraft_engine::gpu::set_enabled(false);
    let mut app = BrApp::new(br_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1400.0, 860.0], 1.0);
    assert!(d.settle(&mut app, SETTLE), "the first frames settle");
    app.open(Page::Foto);
    assert!(d.settle(&mut app, SETTLE));
    if app.lc.session.active().is_none() {
        let first = app.lc.session.visible_cloned()[0];
        app.lc.session.execute("library.select", &serde_json::json!({"ids": [first.0], "active": first.0})).unwrap();
        assert!(d.settle(&mut app, SETTLE));
    }
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

/// Open a quick tool's popover by clicking its header button.
fn open(app: &mut BrApp, d: &mut Driver, p: Popover) {
    let button = if p == Popover::Export { "export".to_string() } else { format!("tool.{}", p.key()) };
    d.click(app, &button).unwrap();
    assert!(d.settle(app, SETTLE));
    assert_eq!(app.photo.popover(), Some(p));
}

#[test]
fn the_photo_page_is_all_of_lightcraft_with_brs_quick_tools() {
    let (app, d) = app();
    assert_eq!(app.page, Page::Foto);
    let texts = d.texts();
    // BR's header ...
    for t in ["Foto", "Looks", "Format", "Exportieren"] {
        assert!(texts.iter().any(|x| x.contains(t)), "`{t}` not drawn: {texts:?}");
    }
    // ... no mode switch any more ...
    assert!(!texts.iter().any(|x| x == "Einfach" || x == "Pro"), "{texts:?}");
    // ... and LightCraft's own interface: its menus and its develop panel
    for t in ["Datei", "Bearbeiten", "Ansicht", "Foto", "Fenster"] {
        assert!(texts.iter().any(|x| x == t), "LightCraft's menu `{t}` missing: {texts:?}");
    }
    assert!(app.widgets.contains_key("page.foto"));
    assert!(variety(&d.paint()) > 200, "photos, not a blank window");
    snapshot(&d, "foto");
}

#[test]
fn lightcrafts_shortcuts_work_on_the_photo_page() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    // 3 rates the photo: LightCraft's shortcut, no longer hidden behind a simple mode
    d.key(&mut app, egui::Key::Num3, egui::Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.catalog.photo(id).unwrap().rating, 3);
    // and its undo
    d.key(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    assert_ne!(app.lc.session.catalog.photo(id).unwrap().rating, 3);
}

#[test]
fn a_look_is_one_tap_and_its_strength_one_undo_step() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    open(&mut app, &mut d, Popover::Looks);
    snapshot(&d, "looks");
    let undo0 = app.lc.session.undo.len();
    d.click(&mut app, "look.lc.warm-glow").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let l = looks::current(&app.photo.br.look, &app.lc.session, id).expect("look applied");
    assert_eq!((l.preset_id.as_str(), l.amount), ("lc.warm-glow", 100.0));
    assert_eq!(app.lc.session.undo.len(), undo0 + 1);
    assert_eq!(app.photo.popover(), Some(Popover::Looks), "a tile click keeps the popover open");

    // drag the strength slider to the left
    let r = app.widgets["look.amount"];
    d.drag(&mut app, r.center(), egui::pos2(r.left() + r.width() * 0.2, r.center().y), 6);
    assert!(d.settle(&mut app, SETTLE));
    let l = looks::current(&app.photo.br.look, &app.lc.session, id).expect("still the current look");
    assert!(l.amount < 60.0, "amount {}", l.amount);
    assert_eq!(app.lc.session.undo.len(), undo0 + 1, "the drag replaced the look's step");
    assert!(app.lc.session.interaction.is_none());

    // "Original" takes it off again
    d.click(&mut app, "look.original").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(looks::current(&app.photo.br.look, &app.lc.session, id).map(|l| l.amount), Some(0.0));
}

#[test]
fn a_second_look_replaces_the_first() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    let before = app.lc.session.develop_of(id).unwrap();
    let undo0 = app.lc.session.undo.len();
    open(&mut app, &mut d, Popover::Looks);
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
fn looks_made_in_lightcraft_show_up() {
    let (mut app, mut d) = app();
    app.lc.run("preset.create", serde_json::json!({"name": "Mein Look"})).unwrap();
    open(&mut app, &mut d, Popover::Looks);
    let id = app.lc.session.presets.iter().find(|p| p.name == "Mein Look").unwrap().id.clone();
    assert!(app.widgets.contains_key(&format!("look.{id}")), "new look listed");
}

#[test]
fn a_popover_closes_on_a_click_elsewhere_and_on_escape() {
    let (mut app, mut d) = app();
    open(&mut app, &mut d, Popover::Format);
    // a click into LightCraft's photo area closes it
    let popover = app.widgets["popover.format"];
    d.click_at(&mut app, egui::pos2(popover.left() - 300.0, popover.center().y));
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.photo.popover(), None);
    open(&mut app, &mut d, Popover::Looks);
    d.key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.photo.popover(), None);
    // its button toggles it
    open(&mut app, &mut d, Popover::Looks);
    d.click(&mut app, "tool.looks").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.photo.popover(), None);
}

#[test]
fn formats_crop_exactly() {
    let (mut app, mut d) = app();
    let id = app.lc.session.active().unwrap();
    open(&mut app, &mut d, Popover::Format);
    snapshot(&d, "format");
    d.click(&mut app, "format.9:16").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(crop::current(&app.lc.session, id), Some(Format::Story));
    assert_eq!(app.lc.session.undo.last().unwrap().label, "Format 9:16");
}

#[test]
fn export_writes_an_exact_instagram_file() {
    let (mut app, mut d) = app();
    let dir = std::env::temp_dir().join(format!("br-ui-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.photo.export_dir = dir.clone();
    open(&mut app, &mut d, Popover::Export);
    d.click(&mut app, "preset.instagram").unwrap();
    d.click(&mut app, "export.go").unwrap();
    assert!(app.photo.exporting());
    assert!(!app.lc.session.activity.running_cancellable().is_empty(), "the quit prompt knows about it");
    assert!(d.settle(&mut app, Duration::from_secs(180)), "export finished");
    let files = app.photo.last_export.clone().expect("exported");
    assert_eq!((files[0]["width"].as_u64(), files[0]["height"].as_u64()), (Some(1080), Some(1350)));
    assert!(std::path::Path::new(files[0]["path"].as_str().unwrap()).exists());
    snapshot(&d, "export-done");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quit_anyway_stops_an_export_without_waiting() {
    let (mut app, mut d) = app();
    let dir = std::env::temp_dir().join(format!("br-ui-export-cancel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.photo.export_dir = dir.clone();
    app.photo.export_preset = br_photo::export::Preset::Instagram; // a second or more of rendering
    open(&mut app, &mut d, Popover::Export);
    d.click(&mut app, "export.go").unwrap();
    assert!(app.photo.exporting());
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
fn the_photo_page_says_when_nothing_is_saved_and_another_library_can_be_chosen() {
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

/// A BR on the video side: FilmCraft's empty project, the photo demo beside it.
fn video_app() -> (BrApp, Driver) {
    lightcraft_engine::gpu::set_enabled(false);
    let mut app = BrApp::new(br_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1400.0, 860.0], 1.0);
    assert!(d.settle(&mut app, SETTLE), "the first frames settle");
    (app, d)
}

#[test]
fn the_window_opens_on_the_schnitt_page_with_filmcrafts_menu_in_the_header() {
    let (app, d) = video_app();
    assert_eq!(app.page, Page::Schnitt);
    let texts = d.texts();
    for page in Page::ALL {
        assert!(app.widgets.contains_key(&format!("page.{}", page.key())), "{page:?}");
        assert!(texts.iter().any(|t| t == page.label()), "page `{}` missing: {texts:?}", page.label());
    }
    // FilmCraft's menu bar (German where FilmCraft has German) and its panels
    assert!(texts.iter().any(|t| t == "Datei" || t == "File"), "FilmCraft's menu: {texts:?}");
    assert!(app.fc.ui_error.is_none(), "{:?}", app.fc.ui_error);
    snapshot(&d, "schnitt");
}

#[test]
fn every_page_draws_and_switching_back_and_forth_is_safe() {
    let (mut app, mut d) = video_app();
    for round in 0..4 {
        for page in Page::ALL {
            d.click(&mut app, &format!("page.{}", page.key())).unwrap();
            assert!(d.settle(&mut app, SETTLE), "{page:?} settles");
            assert_eq!(app.page, page);
            assert!(app.fc.ui_error.is_none(), "round {round}, {page:?}: {:?}", app.fc.ui_error);
            if round == 0 {
                snapshot(&d, &format!("page-{}", page.key()));
            }
        }
    }
    // the fonts the two apps install for the window are complete
    let families = d.view.ctx.fonts(|f| f.families());
    for name in ["semibold", "medium"] {
        assert!(families.contains(&egui::FontFamily::Name(name.into())), "{name}: {families:?}");
    }
}

#[test]
fn shortcuts_go_to_the_app_on_the_page() {
    let (mut app, mut d) = video_app();
    let photo = app.lc.session.visible_cloned()[0];
    app.lc.session.execute("library.select", &serde_json::json!({"ids": [photo.0], "active": photo.0})).unwrap();
    let rating = app.lc.session.catalog.photo(photo).unwrap().rating;
    // on the Schnitt page, 3 is not LightCraft's rating shortcut
    d.key(&mut app, egui::Key::Num3, egui::Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.catalog.photo(photo).unwrap().rating, rating);
    // on the Foto page it is
    app.open(Page::Foto);
    assert!(d.settle(&mut app, SETTLE));
    d.key(&mut app, egui::Key::Num3, egui::Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.lc.session.catalog.photo(photo).unwrap().rating, 3);
}

#[test]
fn a_page_keeps_the_layout_it_was_left_with() {
    let (mut app, mut d) = video_app();
    let mine = br_ui::video::schnitt_layout();
    let filmcraft_ui_egui::dock::DockNode::Split { a, .. } = mine.clone() else { panic!("a split") };
    // the user changes the Schnitt page's layout (here: just its top half)
    app.fc.ui.dock = *a;
    let changed = app.fc.ui.dock.clone();
    d.click(&mut app, "page.farbe").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let panels = |n: &filmcraft_ui_egui::dock::DockNode| {
        let mut v = Vec::new();
        n.panels(&mut v);
        v
    };
    assert_ne!(panels(&app.fc.ui.dock), panels(&changed), "Farbe has its own layout");
    d.click(&mut app, "page.schnitt").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    // the same panels in the same places (FilmCraft may bring another tab of a group to the front)
    assert_eq!(panels(&app.fc.ui.dock), panels(&changed), "Schnitt got its changed layout back");
}
