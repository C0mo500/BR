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

fn have_ffmpeg() -> bool {
    let found = std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    // in CI (BR_REQUIRE_FFMPEG) the video tests must run, not skip
    assert!(found || std::env::var_os("BR_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (BR_REQUIRE_FFMPEG is set)");
    found
}

/// A short phone-style clip (with sound) written by ffmpeg.
fn clip(dir: &std::path::Path, name: &str, size: &str, seconds: u32) -> std::path::PathBuf {
    let path = dir.join(name);
    let o = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size={size}:rate=30:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={seconds}"))
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    path
}

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("br-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn seq_size(app: &BrApp) -> Option<(u32, u32)> {
    app.fc.session.active_sequence().map(|q| (q.settings.width, q.settings.height))
}

#[test]
fn an_empty_project_starts_on_capcuts_start_screen_and_a_format_tile_makes_the_video() {
    let (mut app, mut d) = video_app();
    assert!(br_ui::cut::start_screen_visible(&app));
    let texts = d.texts();
    for t in ["Neues Video", "9:16", "16:9", "1:1", "4:5", "Projekt öffnen…", "Beispielprojekt"] {
        assert!(texts.iter().any(|x| x.contains(t)), "`{t}` not on the start screen: {texts:?}");
    }
    snapshot(&d, "start");
    let undo = app.fc.session.history.undo.len();
    d.click(&mut app, "start.9:16").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(seq_size(&app), Some((1080, 1920)), "a vertical video");
    assert_eq!(app.fc.session.history.undo.len(), undo + 1);
    assert!(!br_ui::cut::start_screen_visible(&app), "the editor takes over");
    // undo brings the start back; "Ohne Video weiter" leaves it for good
    d.click(&mut app, "cut.undo").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert!(app.fc.session.active_sequence().is_none());
    assert!(br_ui::cut::start_screen_visible(&app));
    d.click(&mut app, "start.skip").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert!(!br_ui::cut::start_screen_visible(&app));
    assert!(app.fc.ui_error.is_none(), "{:?}", app.fc.ui_error);
}

#[test]
fn files_dropped_on_the_start_screen_become_the_video_and_videos_dropped_on_foto_go_to_schnitt() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg missing: skipped");
        return;
    }
    let dir = temp("drops");
    let a = clip(&dir, "a.mp4", "1080x1920", 2);
    let b = clip(&dir, "b.mov", "1080x1920", 1);
    let (mut app, mut d) = video_app();
    let photos = app.lc.session.visible_cloned().len();
    d.drop_files(&mut app, std::slice::from_ref(&a));
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(seq_size(&app), Some((1080, 1920)), "the first clip's shape");
    assert_eq!(app.lc.session.visible_cloned().len(), photos, "LightCraft did not import the drop");
    // on the Foto page a video goes to the Schnitt page, at the end of the video
    app.open(Page::Foto);
    assert!(d.settle(&mut app, SETTLE));
    d.drop_files(&mut app, std::slice::from_ref(&b));
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.page, Page::Schnitt);
    let st = br_video::state(&mut app.fc.session).unwrap();
    assert_eq!(st["videoClips"], 2, "{st}");
    assert!((st["seconds"].as_f64().unwrap() - 3.0).abs() < 0.2, "{st}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tool_row_cuts_speeds_filters_and_reshapes_the_video() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg missing: skipped");
        return;
    }
    let dir = temp("tools");
    let a = clip(&dir, "a.mp4", "1080x1920", 3);
    let b = clip(&dir, "b.mp4", "1080x1920", 2);
    let (mut app, mut d) = video_app();
    br_video::execute(&mut app.fc.session, "br.video.add", &serde_json::json!({"paths": [a, b]})).unwrap();
    assert!(d.settle(&mut app, SETTLE));
    for key in ["add", "text", "captions", "music", "filter", "transition", "split", "delete", "speed", "detach", "undo", "redo"] {
        assert!(app.widgets.contains_key(&format!("cut.{key}")), "tool `{key}` missing");
    }
    snapshot(&d, "schnitt-tools");

    // split at the playhead: one clip more, one undo step
    br_video::execute(&mut app.fc.session, "playhead.set", &serde_json::json!({"seconds": 1.0})).unwrap();
    let clips = |app: &mut BrApp| br_video::state(&mut app.fc.session).unwrap()["videoClips"].as_u64().unwrap();
    let n = clips(&mut app);
    d.click(&mut app, "cut.split").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(clips(&mut app), n + 1);

    // select the first clip: Tempo 2× halves it, Löschen closes the gap
    let first = app.fc.session.active_sequence().unwrap().video_tracks[0].items[0].id;
    app.fc.session.state.selection = vec![first];
    d.click(&mut app, "cut.speed").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.cut.popover(), Some(br_ui::cut::Popover::Speed));
    let before = br_video::state(&mut app.fc.session).unwrap()["seconds"].as_f64().unwrap();
    d.click(&mut app, "speed.2").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let after = br_video::state(&mut app.fc.session).unwrap()["seconds"].as_f64().unwrap();
    assert!((before - after - 0.5).abs() < 0.1, "1 s at 2× is 0.5 s: {before} → {after}");
    snapshot(&d, "schnitt-speed");

    // Löschen closes the gap the clip leaves
    let before = br_video::state(&mut app.fc.session).unwrap()["seconds"].as_f64().unwrap();
    let n = clips(&mut app);
    d.click(&mut app, "cut.delete").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(clips(&mut app), n - 1);
    let after = br_video::state(&mut app.fc.session).unwrap()["seconds"].as_f64().unwrap();
    assert!((before - after - 0.5).abs() < 0.1, "the 0.5 s clip is gone and the gap closed: {before} → {after}");
    let first = app.fc.session.active_sequence().unwrap().video_tracks[0].items[0].id;
    app.fc.session.state.selection = vec![first];

    // Ton trennen: picture and sound move on their own afterwards
    d.click(&mut app, "cut.detach").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let (_, it) = app.fc.session.active_sequence().unwrap().find_item(first).unwrap();
    assert!(it.link.is_none(), "detached");

    // a filter on the selection
    d.click(&mut app, "cut.filter").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    d.click(&mut app, "filter.Golden Dusk").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let fx = &app.fc.session.active_sequence().unwrap().find_item(first).unwrap().1.effects;
    assert!(fx.iter().any(|e| e.effect == "lumetri"), "{fx:?}");

    // the video's format, from the header
    d.click(&mut app, "video.format").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    d.click(&mut app, "vformat.1:1").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(seq_size(&app), Some((1080, 1080)));

    // undo through the row takes the format back
    d.click(&mut app, "cut.undo").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(seq_size(&app), Some((1080, 1920)));
    assert!(app.fc.ui_error.is_none(), "{:?}", app.fc.ui_error);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_export_popover_writes_the_video_in_the_background() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg missing: skipped");
        return;
    }
    let dir = temp("export");
    let a = clip(&dir, "a.mp4", "640x360", 2);
    let (mut app, mut d) = video_app();
    br_video::execute(&mut app.fc.session, "br.video.add", &serde_json::json!({"paths": [a]})).unwrap();
    app.cut.export_dir = dir.join("out");
    assert!(d.settle(&mut app, SETTLE));
    d.click(&mut app, "video.export").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.cut.popover(), Some(br_ui::cut::Popover::Export));
    snapshot(&d, "video-export");
    d.click(&mut app, "vexport.go").unwrap();
    // settle waits for the export (the app is busy while it runs)
    assert!(d.settle(&mut app, Duration::from_secs(300)));
    assert!(!app.cut.exporting());
    let out = app.cut.last_export.clone().expect("the export finished");
    assert_eq!(out.parent(), Some(dir.join("out").as_path()));
    assert!(std::fs::metadata(&out).unwrap().len() > 1000, "{}", out.display());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn messages_from_the_video_tools_are_shown() {
    let (mut app, mut d) = video_app();
    br_ui::cut::add_paths(&mut app, &d.view.ctx.clone(), vec!["/nowhere/gone.mp4".into()]);
    d.step(&mut app, Vec::new());
    d.step(&mut app, Vec::new());
    let texts = d.texts();
    assert!(app.widgets.contains_key("toast"), "the message is drawn");
    assert!(texts.iter().any(|t| t.contains("gone.mp4") || t.contains("nothing imported") || t.contains("import")), "{texts:?}");
}

#[test]
fn the_page_follows_filmcrafts_mode() {
    let (mut app, mut d) = video_app();
    assert_eq!(app.page, Page::Schnitt);
    // File ▸ Export ▸ Media… switches FilmCraft to its Export mode
    app.fc.ui.mode = filmcraft_ui_egui::state::Mode::Export;
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.page, Page::Ausgabe);
    app.fc.ui.mode = filmcraft_ui_egui::state::Mode::Edit;
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.page, Page::Schnitt);
}

#[test]
fn closing_on_foto_with_unsaved_video_shows_filmcrafts_question() {
    let (mut app, mut d) = video_app();
    br_video::execute(&mut app.fc.session, "br.video.new", &serde_json::json!({"format": "16:9"})).unwrap();
    assert!(app.fc.session.is_dirty());
    app.open(Page::Foto);
    assert!(d.settle(&mut app, SETTLE));
    d.request_close(&mut app);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.page, Page::Schnitt, "FilmCraft's save question is on its page");
    assert!(app.fc.ui.clip_dialog.as_ref().is_some_and(|q| q.command == "app.quit"));
}

#[test]
fn each_page_has_its_apps_look() {
    let (mut app, mut d) = video_app();
    let body = |d: &Driver| d.view.ctx.global_style().text_styles[&egui::TextStyle::Body].size;
    let film = body(&d);
    app.open(Page::Foto);
    assert!(d.settle(&mut app, SETTLE));
    let photo = body(&d);
    app.open(Page::Farbe);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(body(&d), film, "FilmCraft's look is back on its pages");
    assert_ne!(film, photo, "LightCraft's look on Foto (its body text is another size)");
}

#[test]
fn a_narrow_window_gives_filmcrafts_menus_a_row_of_their_own() {
    lightcraft_engine::gpu::set_enabled(false);
    let mut app = BrApp::new(br_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1000.0, 700.0], 1.0);
    assert!(d.settle(&mut app, SETTLE));
    br_video::execute(&mut app.fc.session, "br.video.new", &serde_json::json!({"format": "9:16"})).unwrap();
    assert!(d.settle(&mut app, SETTLE));
    let texts = d.texts();
    for m in ["Datei", "Fenster", "Hilfe"] {
        assert!(texts.iter().any(|t| t == m), "menu `{m}` not drawn: {texts:?}");
    }
    let export = app.widgets["video.export"];
    let last_tab = app.widgets["page.foto"];
    assert!(export.left() > last_tab.right(), "the header's tools do not cover the page tabs");
    assert!(texts.iter().any(|t| t.contains("Untitled") || t.contains("Video")), "the project's name is shown: {texts:?}");
    snapshot(&d, "narrow");
}

#[test]
fn the_cut_layout_has_the_source_monitor_and_quick_export_opens() {
    let (mut app, mut d) = video_app();
    let mut panels = Vec::new();
    app.fc.ui.dock.panels(&mut panels);
    assert!(panels.contains(&filmcraft_ui_egui::dock::PanelKind::Source), "{panels:?}");
    br_video::execute(&mut app.fc.session, "br.video.new", &serde_json::json!({"format": "16:9"})).unwrap();
    // something to export
    br_video::execute(&mut app.fc.session, "br.video.text", &serde_json::json!({"text": "Hallo"})).unwrap();
    assert!(d.settle(&mut app, SETTLE));
    d.click(&mut app, "video.export").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    d.click(&mut app, "vexport.quick").unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert!(app.fc.ui.export.quick_open, "FilmCraft's Quick Export is open");
    assert!(app.fc.ui_error.is_none(), "{:?}", app.fc.ui_error);
    snapshot(&d, "quick-export");
}

#[test]
fn reset_to_saved_layout_brings_back_brs_schnitt_arrangement() {
    let (mut app, mut d) = video_app();
    assert!(app.fc.workspaces.saved.iter().any(|w| w.name == br_ui::video::SCHNITT_WORKSPACE));
    assert_eq!(app.fc.ui.workspace, br_ui::video::SCHNITT_WORKSPACE);
    let panels = |n: &filmcraft_ui_egui::dock::DockNode| {
        let mut v = Vec::new();
        n.panels(&mut v);
        v
    };
    let ours = panels(&br_ui::video::schnitt_layout());
    // the user drops most of it, then Window ▸ Workspaces ▸ Reset to Saved Layout
    app.fc.ui.dock = filmcraft_ui_egui::dock::DockNode::Tabs { panels: vec![filmcraft_ui_egui::dock::PanelKind::Timeline], active: 0 };
    filmcraft_ui_egui::menus::invoke(&mut app.fc, &d.view.ctx.clone(), "window.workspace.reset", serde_json::json!({})).unwrap();
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(panels(&app.fc.ui.dock), ours, "BR's arrangement is back");
    // Farbe is FilmCraft's Color workspace, and says so
    app.open(Page::Farbe);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.fc.ui.workspace, "Color");
}
