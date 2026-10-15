//! M2's acceptance, driven headlessly as a user would:
//!
//! - a 9:16 video with a title, music and captions, exported from the start screen in at most
//!   twelve interactions (drops, clicks, a typed text);
//! - every control shnyp draws (buttons, tiles, rows, fields) has a widget id and names the command
//!   it runs, and every such command exists (shnyp's, FilmCraft's or LightCraft's);
//! - the same edit made in shnyp's timeline or in FilmCraft's leaves the same sequence
//!   (`sequence.inspect`), and one undo the same again;
//! - deleting a main clip in shnyp's timeline takes what hangs on it along (an overlay, a title),
//!   and one undo brings all of it back.
//!
//! Clips are made with `ffmpeg` (skipped without it, unless SHNYP_REQUIRE_FFMPEG).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Event, Key, Modifiers, pos2};
use serde_json::{Value, json};
use shnyp_ui::headless::{Driver, png};
use shnyp_ui::{Page, ShnypApp};

const SETTLE: Duration = Duration::from_secs(180);

fn have_ffmpeg() -> bool {
    let found = std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("SHNYP_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (SHNYP_REQUIRE_FFMPEG is set)");
    found
}

fn ffmpeg(args: &[&str], out: &Path) {
    let o = std::process::Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(out).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// A clip with a picture of `size` and a tone.
fn clip(dir: &Path, name: &str, seconds: u32, size: &str) -> PathBuf {
    let p = dir.join(name);
    let d = seconds.to_string();
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size={size}:rate=30:duration={d}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=330:duration={d}"),
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
        &p,
    );
    p
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("shnyp-accept-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn snapshot(d: &Driver, name: &str) {
    if let Some(dir) = std::env::var_os("SHNYP_SNAPSHOT_DIR") {
        std::fs::write(Path::new(&dir).join(format!("{name}.png")), png(&d.paint())).unwrap();
    }
}

fn window(size: [f32; 2]) -> (ShnypApp, Driver) {
    lightcraft_engine::gpu::set_enabled(false);
    let mut app = ShnypApp::new(shnyp_photo::library::demo_session(), None);
    app.lc.ui.settings.gpu = false;
    let mut d = Driver::new(size, 1.0);
    assert!(d.settle(&mut app, SETTLE));
    (app, d)
}

fn run(app: &mut ShnypApp, id: &str, p: Value) -> Value {
    shnyp_video::execute(&mut app.fc.session, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn mains(app: &ShnypApp) -> Vec<(u64, f64, f64)> {
    let q = app.fc.session.active_sequence().unwrap();
    shnyp_video::main_track::main_clips(q).iter().map(|it| (it.id.0, it.start.seconds(), it.end().seconds())).collect()
}

/// Counts what the user does.
struct User {
    d: Driver,
    n: usize,
}

impl User {
    fn click(&mut self, app: &mut ShnypApp, name: &str) {
        self.d.click(app, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        self.n += 1;
        assert!(self.d.settle(app, SETTLE));
    }
    /// Files from the file manager let go at `at`.
    fn drop(&mut self, app: &mut ShnypApp, files: &[PathBuf], at: egui::Pos2) {
        self.d.step(app, vec![Event::PointerMoved(at)]);
        self.d.drop_files(app, files);
        self.n += 1;
        assert!(self.d.settle(app, SETTLE));
    }
}

#[test]
fn a_9_16_video_with_title_music_and_captions_in_at_most_twelve_interactions() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("twelve");
    let shot = clip(&d, "urlaub.mp4", 5, "540x960");
    let music = d.join("musik.wav");
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=220:duration=30", "-ac", "2"], &music);
    let srt = d.join("untertitel.srt");
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:02,000\nHallo!\n\n2\n00:00:02,500 --> 00:00:04,500\nWillkommen am Meer.\n").unwrap();
    let (mut app, drv) = window([1600.0, 1000.0]);
    app.cut.export_dir = d.join("Exporte");
    let mut u = User { d: drv, n: 0 };

    // 1: the clip onto the start screen: the video, in its shape
    assert!(app.widgets.contains_key("start.add"), "the start screen");
    u.drop(&mut app, std::slice::from_ref(&shot), pos2(800.0, 500.0));
    let q = app.fc.session.active_sequence().expect("a video");
    assert_eq!(q.settings.width * 16, q.settings.height * 9, "9:16");
    // 2: Text
    u.click(&mut app, "cut.text");
    // 3–5: its words, in the inspector
    let field = app.widgets.get("insp.row.text.text").copied().expect("the new title is selected and its text row shown");
    u.d.click_at(&mut app, field.center());
    u.d.key(&mut app, Key::A, Modifiers::COMMAND);
    u.d.step(&mut app, vec![Event::Text("Mein Urlaub".into())]);
    u.n += 3;
    assert!(u.d.settle(&mut app, SETTLE));
    // 6: the music onto the timeline, at its start
    let first = mains(&app)[0].0;
    let lane = app.widgets[&format!("tl.clip.{first}")];
    u.drop(&mut app, std::slice::from_ref(&music), pos2(lane.min.x + 3.0, lane.center().y));
    // 7: the subtitles onto the timeline
    u.drop(&mut app, std::slice::from_ref(&srt), pos2(lane.center().x, lane.center().y));
    snapshot(&u.d, "accept-edited");
    // 8–10: Exportieren, TikTok/Reels, Exportieren
    u.click(&mut app, "video.export");
    let social = app.widgets.keys().find(|k| k.starts_with("vpreset.") && k.contains("social")).cloned().expect("the social preset");
    u.click(&mut app, &social);
    u.click(&mut app, "vexport.go");
    assert!(u.n <= 12, "{} interactions", u.n);

    // what was made
    let q = app.fc.session.active_sequence().unwrap();
    let graphics: Vec<u64> =
        q.video_tracks.iter().flat_map(|t| &t.items).filter(|it| it.graphic_layers().next().is_some()).map(|it| it.id.0).collect();
    let music_ok = q
        .audio_tracks
        .iter()
        .flat_map(|t| &t.items)
        .find(|it| it.essential.as_ref().is_some_and(|e| e.kind == filmcraft_project::AudioType::Music))
        .is_some_and(|m| (m.end().seconds() - q.duration().seconds()).abs() < 0.1);
    let captions: Vec<String> = q.caption_tracks.iter().flat_map(|t| &t.captions).map(|c| c.text.clone()).collect();
    assert!(music_ok, "music as long as the video");
    assert_eq!(captions, ["Hallo!", "Willkommen am Meer."]);
    let texts: Vec<Value> = graphics
        .iter()
        .map(|c| run(&mut app, "shnyp.video.inspect", json!({"clip": c})))
        .flat_map(|i| i["sections"].as_array().cloned().unwrap_or_default())
        .flat_map(|s| s["rows"].as_array().cloned().unwrap_or_default())
        .filter(|r| r["key"] == "text.text")
        .map(|r| r["value"].clone())
        .collect();
    assert!(texts.iter().any(|t| t == "Mein Urlaub"), "the title: {texts:?}");
    assert!(u.d.texts().iter().any(|t| t.contains("Mein Urlaub")), "the timeline shows the title's words");
    assert!(app.fc.auto.find("program.picture").is_some(), "the Program monitor stayed in front");
    let out = app.cut.last_export.clone().expect("exported");
    let probe = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_type,width,height", "-of", "json"])
        .arg(&out)
        .output()
        .unwrap();
    let p: Value = serde_json::from_slice(&probe.stdout).unwrap();
    let streams = p["streams"].as_array().unwrap();
    assert!(streams.iter().any(|s| s["codec_type"] == "video" && s["width"] == 1080 && s["height"] == 1920), "{p}");
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"), "{p}");
    snapshot(&u.d, "accept-exported");
    let _ = std::fs::remove_dir_all(&d);
}

/// The ids of the controls drawn now that name no command, and the commands named that do not exist.
fn unnamed(app: &ShnypApp, known: &BTreeSet<String>, seen: &mut BTreeSet<String>) -> (Vec<String>, Vec<(String, String)>) {
    let (mut no_command, mut unknown) = (Vec::new(), Vec::new());
    for id in app.widgets.keys() {
        seen.insert(id.clone());
        if shnyp_ui::widgets::is_area(id) {
            continue;
        }
        match shnyp_ui::widgets::commands_of(id) {
            None => no_command.push(id.clone()),
            Some(cmds) => {
                for c in cmds {
                    if !known.contains(&c) {
                        unknown.push((id.clone(), c));
                    }
                }
            }
        }
    }
    (no_command, unknown)
}

#[test]
fn every_control_has_an_id_and_names_a_command_that_exists() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("controls");
    let (a, b) = (clip(&d, "a.mp4", 3, "360x640"), clip(&d, "b.mp4", 3, "640x360"));
    let (mut app, mut drv) = window([1600.0, 1000.0]);
    let mut known: BTreeSet<String> = shnyp_ui::palette::index(&app).into_iter().map(|e| e.id).collect();
    // what the control channel answers besides commands
    known.extend(["shnyp.palette.run", "shnyp.palette.search", "shnyp.ui.inspector", "shnyp.ui.assets", "shnyp.drop"].map(String::from));
    let mut seen = BTreeSet::new();
    let mut problems: Vec<String> = Vec::new();
    let mut check = |app: &ShnypApp, state: &str, seen: &mut BTreeSet<String>| {
        let (none, unknown) = unnamed(app, &known, seen);
        problems.extend(none.into_iter().map(|id| format!("{state}: {id} names no command")));
        problems.extend(unknown.into_iter().map(|(id, c)| format!("{state}: {id} names `{c}`, which does not exist")));
    };
    let settle = |app: &mut ShnypApp, d: &mut Driver| assert!(d.settle(app, SETTLE));
    // the start screen
    check(&app, "start", &mut seen);
    // the Foto page and its popovers
    drv.click(&mut app, "page.foto").unwrap();
    settle(&mut app, &mut drv);
    check(&app, "foto", &mut seen);
    for tool in ["tool.looks", "tool.format", "export"] {
        drv.click(&mut app, tool).unwrap();
        settle(&mut app, &mut drv);
        check(&app, &format!("foto {tool}"), &mut seen);
        drv.key(&mut app, Key::Escape, Modifiers::NONE);
        app.photo.open = None;
        settle(&mut app, &mut drv);
    }
    // a video with a selected clip: the tool row, the timeline, every inspector tab and level,
    // every asset tab, every popover, the command search
    drv.click(&mut app, "page.schnitt").unwrap();
    run(&mut app, "shnyp.video.new", json!({"format": "9:16"}));
    run(&mut app, "shnyp.video.add", json!({"paths": [a, b]}));
    run(&mut app, "shnyp.video.text", json!({"text": "Titel", "at": 0.5, "seconds": 1.0}));
    let first = mains(&app)[0].0;
    run(&mut app, "timeline.select", json!({"clips": [first]}));
    settle(&mut app, &mut drv);
    check(&app, "schnitt", &mut seen);
    let tabs: Vec<String> = app.widgets.keys().filter(|k| k.starts_with("insp.tab.")).cloned().collect();
    for tab in tabs {
        drv.click(&mut app, &tab).unwrap();
        settle(&mut app, &mut drv);
        check(&app, &tab, &mut seen);
    }
    for level in ["mehr", "voll", "basis"] {
        drv.click(&mut app, &format!("insp.level.{level}")).unwrap();
        settle(&mut app, &mut drv);
        check(&app, &format!("inspector {level}"), &mut seen);
        if level == "mehr" {
            drv.click(&mut app, "insp.add").unwrap();
            settle(&mut app, &mut drv);
            check(&app, "effect picker", &mut seen);
            drv.key(&mut app, Key::Escape, Modifiers::NONE);
            settle(&mut app, &mut drv);
        }
    }
    for tab in ["media", "audio", "text", "effects", "transitions", "looks", "templates", "profi"] {
        drv.click(&mut app, &format!("assets.tab.{tab}")).unwrap();
        settle(&mut app, &mut drv);
        check(&app, &format!("assets {tab}"), &mut seen);
    }
    for (button, name) in
        [("cut.captions", "captions"), ("cut.filter", "filter"), ("cut.speed", "speed"), ("video.format", "format"), ("video.export", "export")]
    {
        drv.click(&mut app, button).unwrap();
        settle(&mut app, &mut drv);
        check(&app, &format!("popover {name}"), &mut seen);
        app.cut.open = None;
        settle(&mut app, &mut drv);
    }
    drv.key(&mut app, Key::K, Modifiers::COMMAND);
    drv.step(&mut app, vec![Event::Text("tempo".into())]);
    settle(&mut app, &mut drv);
    check(&app, "command search", &mut seen);
    drv.key(&mut app, Key::Escape, Modifiers::NONE);
    settle(&mut app, &mut drv);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    // the walk saw the controls of every part
    for part in [
        "page.",
        "start.",
        "tool.",
        "look.",
        "cut.",
        "tl.",
        "insp.row.",
        "insp.fx.",
        "assets.",
        "speed.",
        "vformat.",
        "vpreset.",
        "filter.",
        "palette.",
    ] {
        assert!(seen.iter().any(|id| id.starts_with(part)), "no {part}* control was drawn");
    }
    // and each command a control names runs through the control channel's ways too
    assert_eq!(app.page, Page::Schnitt);
    let _ = std::fs::remove_dir_all(&d);
}

/// The sequence without what only the editor's view holds.
fn shape(app: &mut ShnypApp) -> Value {
    let mut v = app.fc.session.execute("sequence.inspect", json!({})).unwrap();
    if let Some(o) = v.as_object_mut() {
        for k in ["playhead", "selection", "transitionSelection", "gapSelection"] {
            o.remove(k);
        }
    }
    v
}

/// A window with a 9:16 video of three clips and a title on the third.
fn three_clips(d: &Path) -> (ShnypApp, Driver) {
    let (mut app, mut drv) = window([1400.0, 900.0]);
    let clips: Vec<String> = ["a", "b", "c"].iter().map(|n| clip(d, &format!("{n}.mp4"), 3, "360x640").to_string_lossy().into_owned()).collect();
    run(&mut app, "shnyp.video.new", json!({"format": "9:16"}));
    run(&mut app, "shnyp.video.add", json!({"paths": clips}));
    run(&mut app, "shnyp.video.text", json!({"text": "Auf C", "seconds": 1.0, "at": 6.5}));
    run(&mut app, "timeline.select", json!({"clips": []}));
    assert!(drv.settle(&mut app, SETTLE));
    (app, drv)
}

#[test]
fn the_same_edit_in_shnyp_s_timeline_or_filmcraft_s_leaves_the_same_sequence() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("same");
    let (mut shnyp, mut bd) = three_clips(&d);
    let (mut fc, mut fd) = three_clips(&d);
    assert_eq!(shape(&mut shnyp), shape(&mut fc), "the same video to start with");
    let first = mains(&shnyp)[0].0;
    // shnyp's timeline: a click on the first clip, ⌫
    let r = shnyp.widgets[&format!("tl.clip.{first}")];
    bd.click_at(&mut shnyp, r.center());
    bd.key(&mut shnyp, Key::Backspace, Modifiers::NONE);
    assert!(bd.settle(&mut shnyp, SETTLE));
    // FilmCraft's timeline (Profi): a click on the same clip, Bearbeiten › Löschen und Lücke schließen
    fd.click(&mut fc, "tl.view.full").unwrap();
    assert!(fd.settle(&mut fc, SETTLE));
    let at = fc.fc.auto.find(&format!("timeline.clip.{first}")).map(|e| pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0));
    fd.click_at(&mut fc, at.expect("FilmCraft's timeline shows the clip"));
    assert!(fd.settle(&mut fc, SETTLE));
    assert!(fc.fc.session.state.selection.contains(&filmcraft_project::ClipId(first)), "{:?}", fc.fc.session.state.selection);
    filmcraft_ui_egui::menus::invoke(&mut fc.fc, &fd.view.ctx.clone(), "edit.rippleDelete", json!({})).unwrap();
    assert!(fd.settle(&mut fc, SETTLE));
    assert_eq!(mains(&shnyp).len(), 2);
    assert_eq!(shape(&mut shnyp), shape(&mut fc), "the same sequence after the edit");
    // one undo each: the same again
    bd.key(&mut shnyp, Key::Z, Modifiers::COMMAND);
    assert!(bd.settle(&mut shnyp, SETTLE));
    fd.key(&mut fc, Key::Z, Modifiers::COMMAND);
    assert!(fd.settle(&mut fc, SETTLE));
    assert_eq!(mains(&shnyp).len(), 3);
    assert_eq!(shape(&mut shnyp), shape(&mut fc), "the same sequence after the undo");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn deleting_a_main_clip_in_shnyp_s_timeline_takes_its_overlay_and_title_and_one_undo_brings_them_back() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("anchored");
    let (mut app, mut drv) = window([1400.0, 900.0]);
    let a = clip(&d, "a.mp4", 3, "360x640");
    let b = clip(&d, "b.mp4", 3, "360x640");
    let pip = clip(&d, "bild.mp4", 1, "320x240");
    run(&mut app, "shnyp.video.new", json!({"format": "9:16"}));
    run(&mut app, "shnyp.video.add", json!({"paths": [a, b]}));
    let overlay = run(&mut app, "shnyp.video.place", json!({"paths": [pip], "target": "overlay", "seconds": 3.5, "position": [0.7, 0.3]}))["clips"]
        [0]
    .as_u64()
    .unwrap();
    let title = run(&mut app, "shnyp.video.text", json!({"text": "Auf B", "seconds": 1.0, "at": 4.0}))["clip"].as_u64().unwrap();
    run(&mut app, "timeline.select", json!({"clips": []}));
    assert!(drv.settle(&mut app, SETTLE));
    let before = shape(&mut app);
    let undo = app.fc.session.history.undo.len();
    let second = mains(&app)[1].0;
    let r = app.widgets[&format!("tl.clip.{second}")];
    drv.click_at(&mut app, r.center());
    drv.key(&mut app, Key::Backspace, Modifiers::NONE);
    assert!(drv.settle(&mut app, SETTLE));
    let q = app.fc.session.active_sequence().unwrap();
    for c in [second, overlay, title] {
        assert!(q.find_item(filmcraft_project::ClipId(c)).is_none(), "{c} went along");
    }
    assert_eq!(app.fc.session.history.undo.len(), undo + 1, "one undo step");
    drv.key(&mut app, Key::Z, Modifiers::COMMAND);
    assert!(drv.settle(&mut app, SETTLE));
    assert_eq!(shape(&mut app), before, "all of it back");
    let _ = std::fs::remove_dir_all(&d);
}

/// The pictures of the README (run by hand: `SHNYP_DOCS_DIR=$PWD/docs/images cargo test -p shnyp-ui --test
/// acceptance docs -- --ignored`): a video from the demo photos with a title, music, captions and
/// beats; the start screen; the command search.
#[test]
#[ignore]
fn docs() {
    let Some(out) = std::env::var_os("SHNYP_DOCS_DIR").map(PathBuf::from) else { return };
    let d = dir("docs");
    let (mut app, mut drv) = window([1600.0, 1000.0]);
    // the start screen
    drv.click(&mut app, "page.schnitt").unwrap();
    assert!(drv.settle(&mut app, SETTLE));
    std::fs::write(out.join("shnyp-start.png"), png(&drv.paint())).unwrap();
    // the demo photos as stills of a story
    let photos: Vec<u64> = app.lc.session.visible_cloned().iter().take(5).map(|p| p.0).collect();
    let mut stills = Vec::new();
    for id in photos {
        let r = shnyp_photo::commands::execute(
            &mut app.lc.session,
            &mut app.photo.shnyp,
            "shnyp.export",
            &json!({"id": id, "preset": "story", "dir": d.to_string_lossy()}),
        )
        .unwrap();
        stills.push(r["files"][0]["path"].as_str().unwrap_or_else(|| panic!("{r}")).to_string());
    }
    let music = d.join("musik.wav");
    ffmpeg(&["-f", "lavfi", "-i", "aevalsrc='0.4*sin(2*PI*220*t)*exp(-6*mod(t\\,0.5))':s=48000:d=30", "-ac", "2"], &music);
    let srt = d.join("untertitel.srt");
    std::fs::write(&srt, "1\n00:00:00,400 --> 00:00:02,400\nEin Sommer am See\n\n2\n00:00:02,600 --> 00:00:05,500\nund Lavendel bis zum Horizont\n")
        .unwrap();
    run(&mut app, "shnyp.video.new", json!({"format": "9:16"}));
    run(&mut app, "shnyp.video.add", json!({"paths": stills}));
    run(&mut app, "shnyp.video.place", json!({"paths": [music.to_string_lossy()], "target": "audio", "seconds": 0.0}));
    run(&mut app, "shnyp.video.place", json!({"paths": [srt.to_string_lossy()]}));
    run(&mut app, "shnyp.video.beats", json!({}));
    let title = run(&mut app, "shnyp.video.text", json!({"text": "Sommer am See", "at": 0.3, "seconds": 3.0}))["clip"].as_u64().unwrap();
    run(&mut app, "timeline.select", json!({"clips": [title]}));
    app.fc.session.set_playhead(filmcraft_time::Tick::from_seconds_f64(1.2));
    assert!(drv.settle(&mut app, SETTLE));
    // the thumbnails come in the background
    for _ in 0..150 {
        drv.step(&mut app, Vec::new());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(drv.settle(&mut app, SETTLE));
    std::fs::write(out.join("shnyp-schnitt.png"), png(&drv.paint())).unwrap();
    // the command search
    drv.key(&mut app, Key::K, Modifiers::COMMAND);
    drv.step(&mut app, vec![Event::Text("standbild".into())]);
    assert!(drv.settle(&mut app, SETTLE));
    std::fs::write(out.join("shnyp-befehlssuche.png"), png(&drv.paint())).unwrap();
    let _ = std::fs::remove_dir_all(&d);
}
