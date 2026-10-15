//! Video templates: placeholders from a template, media into them in order (a clip too short for
//! its slot plays slower), the template's curves, look and background, slots in beats cut on the
//! music's beats, and the video saved as a template of one's own. Clips are made with `ffmpeg`
//! (skipped without it, unless SHNYP_REQUIRE_FFMPEG is set).

use std::path::Path;
use std::process::Command;

use filmcraft_project::ClipId;
use serde_json::{Value, json};
use shnyp_video::{Session, main_track};

fn have_ffmpeg() -> bool {
    let found = Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("SHNYP_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (SHNYP_REQUIRE_FFMPEG is set)");
    found
}

fn ffmpeg(args: &[&str], out: &Path) {
    let o = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(out).output().unwrap();
    assert!(o.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn clip(d: &Path, name: &str, seconds: f64) -> String {
    let p = d.join(name);
    ffmpeg(&["-f", "lavfi", "-i", &format!("testsrc2=size=640x360:rate=30:duration={seconds}"), "-c:v", "libx264", "-pix_fmt", "yuv420p"], &p);
    p.to_string_lossy().into_owned()
}

fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    shnyp_video::execute(s, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("shnyp-template-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn mains(s: &Session) -> Vec<(u64, f64, f64, f64)> {
    main_track::main_clips(s.active_sequence().unwrap()).iter().map(|it| (it.id.0, it.start.seconds(), it.end().seconds(), it.speed)).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.06
}

#[test]
fn a_template_makes_placeholders_and_media_fill_them_in_order() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("fill");
    let list = shnyp_video::template::list(&json!({}));
    let ids: Vec<&str> = list.as_array().unwrap().iter().filter_map(|t| t["id"].as_str()).collect();
    for id in ["story", "reise", "takt", "intro"] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
    let mut s = Session::default();
    let undo = s.history.undo.len();
    let r = ok(&mut s, "shnyp.video.template.new", json!({"template": "story"}));
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    assert_eq!(r["placeholders"].as_array().unwrap().len(), 6, "{r}");
    let m = mains(&s);
    assert!(close(m.last().unwrap().2, 8.4), "1.4 + 4 × 1.2 + 2.0 s: {m:?}");
    let q = s.active_sequence().unwrap();
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
    assert!(q.video_tracks.iter().flat_map(|t| &t.items).any(|it| it.graphic_layers().next().is_some()), "the title");
    // four clips into the first four, one of them too short for its slot
    let clips: Vec<String> = (0..4).map(|i| clip(&d, &format!("c{i}.mp4"), if i == 1 { 0.6 } else { 3.0 })).collect();
    let undo = s.history.undo.len();
    let r = ok(&mut s, "shnyp.video.template.fill", json!({"paths": clips}));
    assert_eq!(s.history.undo.len(), undo + 1);
    assert_eq!(r["filled"].as_array().unwrap().len(), 4);
    assert_eq!(r["empty"].as_array().unwrap().len(), 2);
    assert_eq!(r["short"].as_array().unwrap().len(), 1, "{r}");
    let m = mains(&s);
    assert!(close(m.last().unwrap().2, 8.4), "the slots keep their lengths: {m:?}");
    assert!(m[1].3 < 0.6, "the short clip plays slower: {m:?}");
    let q = s.active_sequence().unwrap();
    let names: Vec<String> = main_track::main_clips(q).iter().map(|it| it.name.clone()).collect();
    assert_eq!(&names[..4], &["c0.mp4", "c1.mp4", "c2.mp4", "c3.mp4"], "in order");
    // the rest
    let more: Vec<String> = (4..6).map(|i| clip(&d, &format!("c{i}.mp4"), 3.0)).collect();
    let r = ok(&mut s, "shnyp.video.template.fill", json!({"paths": more}));
    assert!(r["empty"].as_array().unwrap().is_empty());
    assert!(shnyp_video::execute(&mut s, "shnyp.video.template.fill", &json!({"paths": [clips[0]]})).is_err(), "nothing empty any more");
    assert!(shnyp_video::execute(&mut s, "shnyp.video.template.new", &json!({"template": "gibtsnicht"})).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_template_s_curves_look_and_background_come_with_the_media() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("reise");
    let mut s = Session::default();
    ok(&mut s, "shnyp.video.template.new", json!({"template": "reise"}));
    let clips: Vec<String> = (0..5).map(|i| clip(&d, &format!("r{i}.mp4"), 4.0)).collect();
    ok(&mut s, "shnyp.video.template.fill", json!({"paths": clips}));
    let q = s.active_sequence().unwrap();
    let curves: Vec<String> = main_track::main_clips(q).iter().filter_map(|it| shnyp_video::speed::curve_of(q, it.id).map(|c| c.0)).collect();
    assert!(curves.contains(&"montage".to_string()) && curves.contains(&"blitzRein".to_string()), "{curves:?}");
    let looks = main_track::main_clips(q).iter().filter(|it| it.effect("lumetri").is_some()).count();
    assert!(looks >= 5, "the warm look on every clip: {looks}");
    let mut s2 = Session::default();
    ok(&mut s2, "shnyp.video.template.new", json!({"template": "intro"}));
    let clips: Vec<String> = (0..3).map(|i| clip(&d, &format!("i{i}.mp4"), 4.0)).collect();
    ok(&mut s2, "shnyp.video.template.fill", json!({"paths": clips, "fit": "fit"}));
    let q = s2.active_sequence().unwrap();
    assert!(main_track::main_clips(q).iter().all(|m| shnyp_video::frame::background_of(q, m.id).is_some()), "blurred backgrounds");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn slots_in_beats_cut_on_the_music() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("takt");
    let clicks = d.join("klick.wav");
    ffmpeg(&["-f", "lavfi", "-i", "aevalsrc='if(lt(mod(t\\,0.5)\\,0.03)\\,sin(2*PI*1200*t)\\,0)':s=48000:d=20"], &clicks);
    let mut s = Session::default();
    ok(&mut s, "shnyp.video.template.new", json!({"template": "takt", "music": clicks.to_string_lossy()}));
    let m = mains(&s);
    assert_eq!(m.len(), 8);
    // two beats at 120 bpm: a second each, the last four beats
    for x in &m[1..7] {
        assert!(close(x.2 - x.1, 1.0), "a slot of two beats: {m:?}");
    }
    assert!(close(m[7].2 - m[7].1, 2.0), "{m:?}");
    let q = s.active_sequence().unwrap();
    let music = q
        .audio_tracks
        .iter()
        .flat_map(|t| &t.items)
        .find(|it| it.essential.as_ref().is_some_and(|e| e.kind == filmcraft_project::AudioType::Music))
        .unwrap();
    assert!(close(music.end().seconds(), m[7].2), "the music as long as the video: {} vs {}", music.end().seconds(), m[7].2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_video_is_saved_as_a_template_of_one_s_own() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("save");
    let folder = d.join("Vorlagen").to_string_lossy().into_owned();
    let mut s = Session::default();
    ok(&mut s, "shnyp.video.new", json!({"format": "1:1"}));
    let clips: Vec<String> = (0..3).map(|i| clip(&d, &format!("s{i}.mp4"), 3.0)).collect();
    ok(&mut s, "shnyp.video.add", json!({"paths": clips}));
    ok(&mut s, "shnyp.video.text", json!({"text": "Mein Titel", "at": 0.5, "seconds": 2.0}));
    let first = mains(&s)[0].0;
    ok(&mut s, "shnyp.video.curve", json!({"clips": [first], "preset": "sprung"}));
    let r = ok(&mut s, "shnyp.video.template.save", json!({"name": "Mein Quadrat", "dir": folder}));
    assert!(Path::new(r["path"].as_str().unwrap()).is_file(), "{r}");
    assert_eq!(r["slots"], 3, "a curve's pieces are one slot: {r}");
    let list = shnyp_video::template::list(&json!({"dir": folder}));
    let mine = list.as_array().unwrap().iter().find(|t| t["name"] == "Mein Quadrat").cloned().expect("listed");
    assert_eq!(mine["source"], "user");
    assert_eq!(mine["format"], "1:1");
    let mut s2 = Session::default();
    ok(&mut s2, "shnyp.video.template.new", json!({"template": "Mein Quadrat", "dir": folder}));
    assert_eq!(mains(&s2).len(), 3);
    let q = s2.active_sequence().unwrap();
    assert_eq!((q.settings.width, q.settings.height), (1080, 1080));
    let fill: Vec<String> = (0..3).map(|i| clip(&d, &format!("f{i}.mp4"), 3.0)).collect();
    ok(&mut s2, "shnyp.video.template.fill", json!({"paths": fill}));
    let q = s2.active_sequence().unwrap();
    assert!(
        main_track::main_clips(q).iter().any(|it| shnyp_video::speed::curve_of(q, it.id).is_some_and(|c| c.0 == "sprung")),
        "the curve came with it"
    );
    let _ = ClipId(0);
    let _ = std::fs::remove_dir_all(&d);
}
