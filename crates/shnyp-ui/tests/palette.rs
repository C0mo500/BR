//! The command search and shnyp's keys, driven headlessly: every command of shnyp, FilmCraft and
//! LightCraft is in the list and comes first when its id is typed; German and CapCut words find
//! the command meant; Strg+K opens the search, a typed word and Enter run the command; FilmCraft's
//! keys start as CapCut's (⌘B splits, ⌫ deletes and closes the gap), without two commands on one
//! key; Strg+Alt+1…6 open the pages. Video tests make clips with `ffmpeg` (skipped without it,
//! unless SHNYP_REQUIRE_FFMPEG).

use std::time::Duration;

use egui::{Key, Modifiers};
use serde_json::json;
use shnyp_ui::headless::{Driver, png};
use shnyp_ui::palette::{Source, index, search};
use shnyp_ui::{Page, ShnypApp};

const SETTLE: Duration = Duration::from_secs(90);

fn window() -> (ShnypApp, Driver) {
    lightcraft_engine::gpu::set_enabled(false);
    let mut app = ShnypApp::new(shnyp_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new([1600.0, 1000.0], 1.0);
    assert!(d.settle(&mut app, SETTLE));
    (app, d)
}

fn snapshot(d: &Driver, name: &str) {
    if let Some(dir) = std::env::var_os("SHNYP_SNAPSHOT_DIR") {
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png(&d.paint())).unwrap();
    }
}

fn have_ffmpeg() -> bool {
    let found = std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("SHNYP_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (SHNYP_REQUIRE_FFMPEG is set)");
    found
}

fn clip(dir: &std::path::Path, name: &str) -> String {
    let p = dir.join(name);
    let o = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=360x640:rate=30:duration=4"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&p)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    p.to_string_lossy().into_owned()
}

fn mains(app: &ShnypApp) -> Vec<(u64, f64, f64)> {
    let q = app.fc.session.active_sequence().unwrap();
    shnyp_video::main_track::main_clips(q).iter().map(|it| (it.id.0, it.start.seconds(), it.end().seconds())).collect()
}

#[test]
fn every_command_is_in_the_list_and_first_by_its_id() {
    let (app, _d) = window();
    let entries = index(&app);
    let ids = |s: Source| entries.iter().filter(move |e| e.source == s).map(|e| e.id.as_str()).collect::<std::collections::HashSet<_>>();
    // every command of the three
    let film = ids(Source::Film);
    for c in filmcraft_engine::command_specs() {
        assert!(film.contains(c.id), "FilmCraft's {} is missing", c.id);
    }
    let photo = ids(Source::Photo);
    for c in lightcraft_engine::command_specs() {
        assert!(photo.contains(c.id), "LightCraft's {} is missing", c.id);
    }
    let shnyp = ids(Source::Shnyp);
    for c in shnyp_video::COMMANDS {
        assert!(shnyp.contains(c.id), "{} is missing", c.id);
    }
    for (id, _, _) in shnyp_photo::commands::COMMANDS {
        assert!(shnyp.contains(id), "{id} is missing");
    }
    let film_ui = ids(Source::FilmUi);
    for c in filmcraft_ui_egui::menus::external_commands() {
        assert!(film_ui.contains(c.id.as_str()) || film.contains(c.id.as_str()), "FilmCraft's window command {} is missing", c.id);
    }
    // typed as it is, each id is found before anything else
    for e in &entries {
        let found = search(&entries, &e.id, &[], true);
        let same = found.iter().take_while(|x| x.id == e.id).count();
        assert!(
            same >= 1 && found[..same].iter().any(|x| x.source == e.source),
            "{} ({:?}) is not first: {:?}",
            e.id,
            e.source,
            found.iter().take(3).map(|x| &x.id).collect::<Vec<_>>()
        );
    }
    // every entry has a name
    assert!(entries.iter().all(|e| !e.title.trim().is_empty()), "{:?}", entries.iter().find(|e| e.title.trim().is_empty()).map(|e| &e.id));
}

#[test]
fn german_and_capcut_words_find_the_command_meant() {
    let (app, _d) = window();
    let entries = index(&app);
    let top = |q: &str, n: usize| search(&entries, q, &[], true).iter().take(n).map(|e| e.id.clone()).collect::<Vec<_>>();
    assert_eq!(top("teilen", 1), ["shnyp.video.split"], "{:?}", top("teilen", 5));
    assert_eq!(top("Rückgängig", 1), ["edit.undo"], "{:?}", top("Rückgängig", 5));
    assert!(top("Standbild", 3).iter().any(|i| i == "shnyp.video.freeze"), "{:?}", top("Standbild", 5));
    assert!(top("hintergrund unschärfe", 3).iter().any(|i| i == "shnyp.video.background"), "{:?}", top("hintergrund unschärfe", 5));
    assert!(top("beats", 3).iter().any(|i| i == "shnyp.video.beats"), "{:?}", top("beats", 5));
    assert!(top("9:16", 3).iter().any(|i| i == "shnyp.video.format"), "{:?}", top("9:16", 5));
    assert!(top("Seite Foto", 1).iter().any(|i| i == "shnyp.ui.page.foto"), "{:?}", top("Seite Foto", 5));
    // nothing for nonsense
    assert!(top("qqxxzzqq", 1).is_empty());
    // the search over the control channel
    let r = shnyp_ui::palette::search_json(&app, "exportieren", 5);
    assert!(r.as_array().unwrap().iter().any(|e| e["id"] == "shnyp.video.export" || e["id"] == "shnyp.ui.popover.export"), "{r}");
}

#[test]
fn ctrl_k_opens_the_search_and_enter_runs_what_was_typed() {
    let (mut app, mut d) = window();
    d.key(&mut app, Key::K, Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    assert!(app.palette.open, "Strg+K opens it");
    assert!(app.widgets.contains_key("palette.search"));
    d.step(&mut app, vec![egui::Event::Text("Seite Foto".into())]);
    assert!(d.settle(&mut app, SETTLE));
    snapshot(&d, "palette");
    assert!(
        app.widgets.contains_key("palette.cmd.shnyp.ui.page.foto"),
        "{:?}",
        app.widgets.keys().filter(|k| k.starts_with("palette.")).collect::<Vec<_>>()
    );
    d.key(&mut app, Key::Enter, Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.page, Page::Foto, "Enter ran the first");
    assert!(!app.palette.open);
    // Escape closes it, on the Foto page too
    d.key(&mut app, Key::K, Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    assert!(app.palette.open);
    d.key(&mut app, Key::Escape, Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert!(!app.palette.open);
    // a command that needs values gets a form
    d.key(&mut app, Key::K, Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    d.step(&mut app, vec![egui::Event::Text("shnyp.video.text".into())]);
    assert!(d.settle(&mut app, SETTLE));
    d.key(&mut app, Key::Enter, Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert!(app.widgets.contains_key("palette.field.text"), "{:?}", app.widgets.keys().filter(|k| k.starts_with("palette.")).collect::<Vec<_>>());
    snapshot(&d, "palette-form");
    d.key(&mut app, Key::Escape, Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    // the pages: Strg+Alt+1…6
    d.key(&mut app, Key::Num2, Modifiers::COMMAND | Modifiers::ALT);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.page, Page::Schnitt);
    d.key(&mut app, Key::Num6, Modifiers::COMMAND | Modifiers::ALT);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.page, Page::Foto);
}

#[test]
fn filmcraft_starts_with_capcut_s_keys_without_two_commands_on_one_key() {
    let (app, _d) = window();
    let sc = &app.fc.session.shortcuts;
    assert_eq!(sc.preset, shnyp_ui::keys::CAPCUT);
    assert_eq!(sc.primary("shnyp.video.split").as_deref(), Some("Cmd+B"));
    assert_eq!(sc.primary("shnyp.ui.palette").as_deref(), Some("Cmd+K"));
    assert!(sc.for_command("sequence.addEdit").iter().all(|b| b.keys != "Cmd+K"), "⌘K is the search's");
    let p = filmcraft_engine::shortcuts::Platform::current();
    // what the CapCut set adds clashes with nothing (FilmCraft's own sets may have clashes of their own)
    let mine: Vec<String> =
        shnyp_ui::keys::CAPCUT_KEYS.iter().map(|(_, k)| filmcraft_engine::shortcuts::Chord::parse(k).unwrap().effective(p).canonical()).collect();
    let clashes: Vec<_> =
        sc.conflicts(p).into_iter().filter(|(ctx, key, _)| ctx == filmcraft_engine::shortcuts::APPLICATION && mine.contains(key)).collect();
    assert!(clashes.is_empty(), "{clashes:?}");
    // shnyp's commands are FilmCraft's to bind
    let cmds = sc.commands();
    for id in ["shnyp.video.split", "shnyp.video.delete", "shnyp.video.freeze", "shnyp.ui.popover.speed", "shnyp.ui.magnet"] {
        assert!(cmds.iter().any(|c| c.id == id), "{id} cannot be bound");
    }
}

#[test]
fn cmd_b_splits_and_backspace_deletes_capcut_s_way() {
    if !have_ffmpeg() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("shnyp-keys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (clip(&dir, "a.mp4"), clip(&dir, "b.mp4"));
    let (mut app, mut d) = window();
    shnyp_video::execute(&mut app.fc.session, "shnyp.video.new", &json!({"format": "9:16"})).unwrap();
    shnyp_video::execute(&mut app.fc.session, "shnyp.video.add", &json!({"paths": [a, b]})).unwrap();
    assert!(d.settle(&mut app, SETTLE));
    app.fc.session.set_playhead(filmcraft_time::Tick::from_seconds_f64(1.0));
    shnyp_video::execute(&mut app.fc.session, "timeline.select", &json!({"clips": []})).unwrap();
    d.key(&mut app, Key::B, Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    let m = mains(&app);
    assert_eq!(m.len(), 3, "⌘B split the clip at the playhead: {m:?}");
    // the first piece selected, ⌫: gone, the rest closes up
    shnyp_video::execute(&mut app.fc.session, "timeline.select", &json!({"clips": [m[0].0]})).unwrap();
    d.key(&mut app, Key::Backspace, Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    let m = mains(&app);
    assert_eq!(m.len(), 2, "{m:?}");
    assert!(m[0].1.abs() < 0.05, "the gap closed: {m:?}");
    // one undo brings it back
    d.key(&mut app, Key::Z, Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(mains(&app).len(), 3);
    // P: the magnet off and on again
    d.key(&mut app, Key::P, Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert!(!shnyp_video::main_track::settings(app.fc.session.active_sequence().unwrap()).magnet);
    d.key(&mut app, Key::P, Modifiers::NONE);
    assert!(d.settle(&mut app, SETTLE));
    assert!(shnyp_video::main_track::settings(app.fc.session.active_sequence().unwrap()).magnet);
    // ⌘R: Tempo
    let first = mains(&app)[0].0;
    shnyp_video::execute(&mut app.fc.session, "timeline.select", &json!({"clips": [first]})).unwrap();
    d.key(&mut app, Key::R, Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.cut.popover(), Some(shnyp_ui::cut::Popover::Speed));
    d.key(&mut app, Key::R, Modifiers::COMMAND);
    assert!(d.settle(&mut app, SETTLE));
    assert_eq!(app.cut.popover(), None, "again: closed");
    // the list runs a command of the video the same way
    shnyp_ui::palette::run_id(&mut app, &d.view.ctx.clone(), "shnyp.video.reverse", json!({})).unwrap();
    assert!(shnyp_video::main_track::main_clips(app.fc.session.active_sequence().unwrap())[0].reverse);
    let _ = std::fs::remove_dir_all(&dir);
}

/// FilmCraft as a window with a data folder starts it: its keys as saved there, shnyp's added.
fn film_with_data(dir: &std::path::Path) -> filmcraft_ui_egui::FilmcraftApp {
    let mut session = shnyp_video::Session::default();
    session.prefs_path = Some(dir.join("preferences.json"));
    session.shortcuts.set_dir(dir);
    let mut fc = filmcraft_ui_egui::FilmcraftApp::new(session);
    shnyp_ui::keys::register(&mut fc);
    shnyp_ui::keys::install(&mut fc);
    fc
}

#[test]
fn capcut_is_set_once_and_the_user_s_keys_stay() {
    let dir = std::env::temp_dir().join(format!("shnyp-keys-data-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // the first start: CapCut
    let mut fc = film_with_data(&dir);
    assert_eq!(fc.session.shortcuts.preset, shnyp_ui::keys::CAPCUT);
    assert_eq!(fc.session.shortcuts.primary("shnyp.video.split").as_deref(), Some("Cmd+B"));
    // the user changes a key and saves the set under its name
    fc.session.shortcuts.set("shnyp.video.split", "Cmd+Shift+X", None, false, false).unwrap();
    fc.session.shortcuts.save_preset(shnyp_ui::keys::CAPCUT).unwrap();
    drop(fc);
    let mut fc = film_with_data(&dir);
    assert_eq!(fc.session.shortcuts.primary("shnyp.video.split").as_deref(), Some("Cmd+Shift+X"), "the user's CapCut stays theirs");
    // FilmCraft's own set, chosen: it stays, also without the CapCut file
    fc.session.shortcuts.load_preset(filmcraft_engine::shortcuts::DEFAULT_PRESET).unwrap();
    let _ = fc.session.shortcuts.delete_preset(shnyp_ui::keys::CAPCUT);
    drop(fc);
    let fc = film_with_data(&dir);
    assert_eq!(fc.session.shortcuts.preset, filmcraft_engine::shortcuts::DEFAULT_PRESET, "the choice stays");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Keys saved while shnyp was called BR (commands `br.…`) work on: the CapCut set stays shnyp's
/// own (and is brought up to date as before), a set of the user's keeps its keys.
#[test]
fn keys_saved_while_shnyp_was_br_keep_working() {
    let dir = std::env::temp_dir().join(format!("shnyp-keys-br-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut fc = film_with_data(&dir);
    fc.session.shortcuts.set("shnyp.video.split", "Cmd+Shift+X", None, false, false).unwrap();
    fc.session.shortcuts.save_preset("Meine").unwrap();
    fc.session.shortcuts.load_preset(shnyp_ui::keys::CAPCUT).unwrap();
    drop(fc);
    // as BR left them
    let to_br = |text: String| text.replace("\"shnyp.", "\"br.");
    for e in std::fs::read_dir(dir.join("Keyboard Shortcuts")).unwrap() {
        let p = e.unwrap().path();
        std::fs::write(&p, to_br(std::fs::read_to_string(&p).unwrap())).unwrap();
    }
    let (old, new) = (dir.join("br-tasten-capcut.json"), dir.join("shnyp-tasten-capcut.json"));
    std::fs::write(&old, to_br(std::fs::read_to_string(&new).unwrap())).unwrap();
    std::fs::remove_file(&new).unwrap();
    shnyp_ui::keys::adopt_old_ids(&dir);
    let mut fc = film_with_data(&dir);
    let sc = &fc.session.shortcuts;
    assert_eq!(sc.preset, shnyp_ui::keys::CAPCUT);
    assert_eq!(sc.primary("shnyp.video.split").as_deref(), Some("Cmd+B"));
    assert!(!sc.bindings.iter().any(|b| b.command.starts_with("br.")));
    assert!(!old.exists());
    assert_eq!(std::fs::read_to_string(&new).unwrap(), serde_json::to_string(&sc.bindings).unwrap(), "still shnyp's own CapCut");
    fc.session.shortcuts.load_preset("Meine").unwrap();
    assert_eq!(fc.session.shortcuts.primary("shnyp.video.split").as_deref(), Some("Cmd+Shift+X"), "the user's set keeps its keys");
    let _ = std::fs::remove_dir_all(&dir);
}
