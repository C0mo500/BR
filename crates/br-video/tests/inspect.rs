//! The inspector's commands: every Basis row of every kind of clip can be read, set and undone in
//! one step; Mehr lists every parameter of every effect; a drag is one undo step; values are held
//! to their schema; keyframes; effects added, switched off, reset and removed; the grade survives
//! a new filter; music gets louder without fighting its ducking. Clips are made with `ffmpeg`
//! (skipped without it, unless BR_REQUIRE_FFMPEG is set).

use std::path::{Path, PathBuf};
use std::process::Command;

use br_video::Session;
use filmcraft_project::{AudioType, ClipId, TrackItem};
use serde_json::{Value, json};

fn have_ffmpeg() -> bool {
    let found = Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("BR_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (BR_REQUIRE_FFMPEG is set)");
    found
}

fn ffmpeg(args: &[&str], out: &Path) {
    let o = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(out).output().unwrap();
    assert!(o.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    br_video::execute(s, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn item(s: &Session, c: u64) -> TrackItem {
    s.active_sequence().unwrap().find_item(ClipId(c)).map(|(_, it)| it.clone()).unwrap_or_else(|| panic!("clip {c}"))
}

/// A 9:16 video: a clip with sound (4 s), a picture (2 s), a title at 1 s, a sound effect at
/// 2 s, captions, and music lowered under speech.
struct Video {
    s: Session,
    clip: u64,
    sound_of_clip: u64,
    picture: u64,
    title: u64,
    sfx: u64,
    music: u64,
    caption: u64,
    d: PathBuf,
}

fn video(name: &str) -> Video {
    let d = std::env::temp_dir().join(format!("br-inspect-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (a, pic, fx, m) = (d.join("a.mp4"), d.join("bild.png"), d.join("plopp.m4a"), d.join("musik.m4a"));
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=360x640:rate=30:duration=4",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=4",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
        &a,
    );
    ffmpeg(&["-f", "lavfi", "-i", "testsrc2=size=360x640", "-frames:v", "1"], &pic);
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=880:duration=0.5", "-c:a", "aac"], &fx);
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=220:duration=12", "-c:a", "aac"], &m);
    let srt = d.join("untertitel.srt");
    std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nHallo\n").unwrap();
    let mut s = Session::default();
    ok(&mut s, "br.video.new", json!({"format": "9:16"}));
    ok(&mut s, "br.video.add", json!({"paths": [a.to_string_lossy(), pic.to_string_lossy()]}));
    let q = s.active_sequence().unwrap();
    let mains: Vec<u64> = br_video::main_track::main_clips(q).iter().map(|it| it.id.0).collect();
    assert_eq!(mains.len(), 2, "{mains:?}");
    let (clip, picture) = (mains[0], mains[1]);
    let link = item(&s, clip).link;
    let sound_of_clip =
        q.audio_tracks.iter().flat_map(|t| &t.items).find(|it| it.link == link && link.is_some()).map(|it| it.id.0).expect("the clip's sound");
    let title = ok(&mut s, "br.video.text", json!({"text": "Hallo", "seconds": 1.0, "at": 1.0}))["clip"].as_u64().unwrap();
    ok(&mut s, "br.video.add", json!({"paths": [fx.to_string_lossy()], "at": 2.0}));
    ok(&mut s, "br.video.captions", json!({"path": srt.to_string_lossy()}));
    ok(&mut s, "br.video.music", json!({"path": m.to_string_lossy(), "duck": true}));
    let q = s.active_sequence().unwrap();
    let kind = |it: &&TrackItem, k: AudioType| it.essential.as_ref().is_some_and(|e| e.kind == k);
    let sfx = q.audio_tracks.iter().flat_map(|t| &t.items).find(|it| kind(it, AudioType::Sfx)).map(|it| it.id.0).expect("the sound effect");
    let music = q.audio_tracks.iter().flat_map(|t| &t.items).find(|it| kind(it, AudioType::Music)).map(|it| it.id.0).expect("the music");
    let caption = q.caption_tracks[0].captions[0].id.0;
    Video { s, clip, sound_of_clip, picture, title, sfx, music, caption, d }
}

/// Every row of an inspect answer: (key, row).
fn rows(v: &Value) -> Vec<(String, Value)> {
    v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap().iter())
        .map(|r| (r["key"].as_str().unwrap().to_string(), r.clone()))
        .collect()
}

fn row(v: &Value, key: &str) -> Value {
    rows(v).into_iter().find(|r| r.0 == key).map(|r| r.1).unwrap_or_else(|| panic!("no row {key} in {v}"))
}

/// A value different from `row`'s, inside its range.
fn another(row: &Value) -> Option<Value> {
    let v = &row["value"];
    Some(match row["type"].as_str().unwrap() {
        // times and fades: a little later
        "float" if row["unit"] == "s" => json!(v.as_f64().unwrap_or(0.0) + 0.1),
        "float" | "angle" => {
            let x = v.as_f64().unwrap_or(0.0);
            let (lo, hi) = (row["slider"][0].as_f64().unwrap_or(-100.0), row["slider"][1].as_f64().unwrap_or(100.0));
            let hi = row["max"].as_f64().map_or(hi, |m| hi.min(m));
            let to = if x + (hi - lo) * 0.25 <= hi { x + (hi - lo) * 0.25 } else { x - (hi - lo) * 0.25 };
            json!(to)
        }
        "bool" => json!(!v.as_bool().unwrap()),
        "choice" => {
            let opts = row["options"].as_array().unwrap();
            opts.iter().find(|o| *o != v).cloned().unwrap()
        }
        "color" => json!(if v == "#ff0000" { "#00ff00" } else { "#ff0000" }),
        "text" => json!(format!("{}x", v.as_str().unwrap_or("").chars().take(8).collect::<String>())),
        "point" => json!([v[0].as_f64().unwrap_or(0.0) + 50.0, v[1].as_f64().unwrap_or(0.0) - 40.0]),
        _ => return None,
    })
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() < 0.02,
        _ => match (a.as_array(), b.as_array()) {
            (Some(x), Some(y)) if x.len() == y.len() => x.iter().zip(y).all(|(p, q)| same(p, q)),
            _ => a == b,
        },
    }
}

/// Set every settable row of `subject` to another value, check it, and undo it in one step.
fn round_trip(s: &mut Session, subject: Value, kind: &str) -> usize {
    let first = ok(s, "br.video.inspect", subject.clone());
    assert_eq!(first["kind"], kind, "{first}");
    let mut n = 0;
    for (key, r) in rows(&first) {
        let Some(to) = another(&r) else { continue };
        let undo = s.history.undo.len();
        let mut p = subject.clone();
        p["row"] = json!(key);
        p["value"] = to.clone();
        let after = br_video::execute(s, "br.video.set", &p).unwrap_or_else(|e| panic!("{kind} {key} = {to}: {e}"));
        let now = row(&ok(s, "br.video.inspect", subject.clone()), &key);
        assert!(same(&now["value"], &to), "{kind} {key}: set {to}, reads {} (answer {after})", now["value"]);
        assert_eq!(s.history.undo.len(), undo + 1, "{kind} {key}: one undo step");
        s.execute("edit.undo", json!({})).unwrap();
        let back = row(&ok(s, "br.video.inspect", subject.clone()), &key);
        assert!(same(&back["value"], &r["value"]), "{kind} {key}: undo gives {} back, reads {}", r["value"], back["value"]);
        n += 1;
    }
    n
}

#[test]
fn every_basis_row_of_every_kind_round_trips_in_one_undo_step() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("rows");
    let s = &mut v.s;
    s.set_playhead(filmcraft_time::Tick(filmcraft_time::TICKS_PER_SECOND));
    let video_rows = round_trip(s, json!({"clip": v.clip}), "video");
    assert!(video_rows >= 25, "transform, crop, opacity, Anpassen, Tempo, Ton: {video_rows}");
    round_trip(s, json!({"clip": v.picture}), "picture");
    let text_rows = round_trip(s, json!({"clip": v.title}), "text");
    assert!(text_rows >= 20, "{text_rows}");
    round_trip(s, json!({"clip": v.sfx}), "sound");
    round_trip(s, json!({"clip": v.music}), "music");
    round_trip(s, json!({"caption": v.caption}), "caption");
    // a transition at the cut between the clip and the picture
    ok(s, "br.video.transition", json!({"clips": [v.clip, v.picture]}));
    let tr = s.active_sequence().unwrap().video_tracks.iter().flat_map(|t| &t.transitions).map(|t| t.id.0).next().expect("a transition");
    round_trip(s, json!({"transition": tr}), "transition");
    // the picture has no Tempo and no sound; the clip's sound rows land on its sound
    let pic = ok(s, "br.video.inspect", json!({"clip": v.picture}));
    assert!(rows(&pic).iter().all(|r| !r.0.starts_with("speed.") && !r.0.starts_with("audio.")), "{pic}");
    let clip = ok(s, "br.video.inspect", json!({"clip": v.clip}));
    assert_eq!(row(&clip, "audio.volume")["clip"], v.sound_of_clip);
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn mehr_lists_every_parameter_of_every_effect() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("mehr");
    let s = &mut v.s;
    ok(s, "br.video.filter", json!({"preset": "Golden Dusk", "clips": [v.clip]}));
    ok(s, "br.video.effect", json!({"clip": v.clip, "op": "add", "effect": "gaussian_blur"}));
    for c in [v.clip, v.title, v.music] {
        let it = item(s, c);
        let m = ok(s, "br.video.inspect", json!({"clip": c, "level": "mehr"}));
        let fx = m["effects"].as_array().unwrap();
        let shown: usize = it.effects.iter().filter(|e| e.def().is_some()).count();
        assert_eq!(fx.len(), shown, "{c}: every effect with a schema");
        for e in fx {
            let i = e["index"].as_u64().unwrap() as usize;
            let def = it.effects[i].def().unwrap();
            let ids: Vec<&str> = e["params"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
            assert_eq!(ids, def.params.iter().map(|p| p.id).collect::<Vec<_>>(), "{}: every parameter", def.id);
        }
        // the intrinsic effects come first, as FilmCraft shows them
        let first_standard = fx.iter().position(|e| e["intrinsic"] == false).unwrap_or(fx.len());
        assert!(fx[first_standard..].iter().all(|e| e["intrinsic"] == false), "{c}");
    }
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn a_drag_is_one_undo_step_whatever_command_it_takes() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("drag");
    let s = &mut v.s;
    let fitted = row(&ok(s, "br.video.inspect", json!({"clip": v.clip})), "transform.scale")["value"].clone();
    assert_eq!(fitted, 300.0, "a 360×640 clip fitted into 1080×1920");
    let undo = s.history.undo.len();
    for x in [110.0, 120.0, 130.0] {
        ok(s, "br.video.set", json!({"clip": v.clip, "row": "transform.scale", "value": x, "gesture": "g1"}));
    }
    assert_eq!(s.history.undo.len(), undo + 1, "one step for the drag");
    for x in [140.0, 150.0] {
        ok(s, "br.video.set", json!({"clip": v.clip, "row": "transform.scale", "value": x, "gesture": "g2"}));
    }
    assert_eq!(s.history.undo.len(), undo + 2, "a new drag is a new step");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(row(&ok(s, "br.video.inspect", json!({"clip": v.clip})), "transform.scale")["value"], 130.0);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(row(&ok(s, "br.video.inspect", json!({"clip": v.clip})), "transform.scale")["value"], fitted);
    // a command FilmCraft does not merge (clip gain) folds just the same
    let undo = s.history.undo.len();
    for db in [-2.0, -4.0, -6.0] {
        ok(s, "br.video.set", json!({"clip": v.music, "row": "audio.volume", "value": db, "gesture": "g3"}));
    }
    assert_eq!(s.history.undo.len(), undo + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(item(s, v.music).gain_db, 0.0);
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn music_gets_louder_without_touching_its_ducking() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("music");
    let s = &mut v.s;
    let level = |s: &Session| item(s, v.music).effect("volume").unwrap().params["level"].clone();
    let before = level(s);
    let m = ok(s, "br.video.inspect", json!({"clip": v.music}));
    assert_eq!(m["kind"], "music");
    assert_eq!(row(&m, "audio.ducking")["value"], true, "br.video.music ducks: {m}");
    ok(s, "br.video.set", json!({"clip": v.music, "row": "audio.volume", "value": -6.0}));
    assert_eq!(item(s, v.music).gain_db, -6.0);
    assert_eq!(level(s), before, "the ducking curve is untouched");
    ok(s, "br.video.set", json!({"clip": v.music, "row": "audio.ducking", "value": false}));
    assert!(!item(s, v.music).essential.unwrap().ducking.enabled);
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn a_new_filter_replaces_the_look_and_keeps_the_grade() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("grade");
    let s = &mut v.s;
    ok(s, "br.video.filter", json!({"preset": "Golden Dusk", "clips": [v.clip]}));
    ok(s, "br.video.set", json!({"clip": v.clip, "row": "adjust.exposure", "value": 1.0}));
    let lumetris = |s: &Session| item(s, v.clip).effects.iter().filter(|e| e.effect == "lumetri").cloned().collect::<Vec<_>>();
    assert_eq!(lumetris(s).len(), 2, "the grade is its own Lumetri, the look stays");
    assert_eq!(item(s, v.clip).effects[0].effect, "lumetri", "the grade goes first");
    ok(s, "br.video.filter", json!({"preset": "Steel Blue", "clips": [v.clip]}));
    let l = lumetris(s);
    assert_eq!(l.len(), 2, "the old look gave way to the new one: {}", l.len());
    assert_eq!(l[0].params["exposure"].value, filmcraft_project::ParamValue::Float(1.0), "the grade stayed");
    assert_eq!(row(&ok(s, "br.video.inspect", json!({"clip": v.clip})), "adjust.exposure")["value"], 1.0);
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn values_are_held_to_their_schema() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("schema");
    let s = &mut v.s;
    let r = ok(s, "br.video.set", json!({"clip": v.clip, "row": "opacity.opacity", "value": 250}));
    assert_eq!(r["value"], 100.0, "held to 0…100: {r}");
    assert!(br_video::execute(s, "br.video.set", &json!({"clip": v.clip, "row": "opacity.blend", "value": 99})).is_err());
    let r = ok(s, "br.video.set", json!({"clip": v.clip, "row": "opacity.blend", "value": "multiply"}));
    assert_eq!(r["value"], "Multiply");
    assert!(br_video::execute(s, "br.video.set", &json!({"clip": v.clip, "row": "transform.scale", "value": "groß"})).is_err());
    assert!(
        br_video::execute(s, "br.video.set", &json!({"clip": v.clip, "row": "opacity.blend", "value": "Screen", "keyframe": true})).is_err(),
        "a choice does not animate"
    );
    assert!(br_video::execute(s, "br.video.set", &json!({"clip": v.clip, "row": "nope.nope", "value": 1})).is_err());
    let r = ok(s, "br.video.set", json!({"clip": v.clip, "effect": "motion", "param": "anti_flicker", "value": 5}));
    assert_eq!(r["value"], 1.0, "any parameter, by effect and id: {r}");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn keyframes_at_the_playhead_one_row_or_a_whole_section() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("keys");
    let s = &mut v.s;
    let scale = |s: &mut Session, sec: f64| row(&ok(s, "br.video.inspect", json!({"clip": v.clip, "seconds": sec})), "transform.scale");
    ok(s, "br.video.keyframe", json!({"clip": v.clip, "row": "transform.scale", "seconds": 0.5}));
    ok(s, "br.video.set", json!({"clip": v.clip, "row": "transform.scale", "value": 200.0, "seconds": 3.0, "keyframe": true}));
    let mid = scale(s, 1.75);
    assert_eq!(mid["animated"], true);
    // from the fitted 300 % at 0.5 s to 200 % at 3 s
    assert!(mid["value"].as_f64().unwrap() > 220.0 && mid["value"].as_f64().unwrap() < 280.0, "between the keyframes: {mid}");
    assert_eq!(scale(s, 3.0)["atKeyframe"], true);
    // the previous and next keyframe
    s.set_playhead(filmcraft_time::Tick(2 * filmcraft_time::TICKS_PER_SECOND));
    let p = ok(s, "br.video.keyframe", json!({"clip": v.clip, "row": "transform.scale", "op": "prev"}));
    assert!((p["seconds"].as_f64().unwrap() - 0.5).abs() < 0.05, "{p}");
    let n = ok(s, "br.video.keyframe", json!({"clip": v.clip, "row": "transform.scale", "op": "next"}));
    assert!((n["seconds"].as_f64().unwrap() - 3.0).abs() < 0.05, "{n}");
    // a whole section in one step, and off again
    let group = json!(["transform.scale", "transform.position", "transform.rotation"]);
    let undo = s.history.undo.len();
    let on = ok(s, "br.video.keyframe", json!({"clip": v.clip, "row": group, "seconds": 2.0}));
    assert_eq!(on["keyframeHere"], true);
    assert_eq!(s.history.undo.len(), undo + 1, "one step");
    let off = ok(s, "br.video.keyframe", json!({"clip": v.clip, "row": group, "seconds": 2.0}));
    assert_eq!(off["keyframeHere"], false);
    // eased, then no animation at all
    ok(s, "br.video.keyframe", json!({"clip": v.clip, "row": "transform.scale", "op": "ease", "interpolation": "easeOut", "all": true}));
    let it = item(s, v.clip);
    let keys = &it.effect("motion").unwrap().params["scale"].keyframes;
    assert!(keys.iter().all(|k| k.interp == filmcraft_project::Interpolation::EaseOut));
    ok(s, "br.video.keyframe", json!({"clip": v.clip, "row": "transform.scale", "op": "clear", "seconds": 3.0}));
    let r = scale(s, 1.0);
    assert_eq!(r["animated"], false);
    assert_eq!(r["value"], 200.0, "the value at the playhead stays");
    assert!(br_video::execute(s, "br.video.keyframe", &json!({"clip": v.clip, "row": "opacity.blend"})).is_err(), "a choice has no keyframes");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn effects_are_added_switched_off_reset_and_removed() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("effects");
    let s = &mut v.s;
    let added = ok(s, "br.video.effect", json!({"clip": v.clip, "op": "add", "effect": "Gaussian Blur"}));
    let i = added["index"].as_u64().unwrap() as usize;
    assert_eq!(item(s, v.clip).effects[i].effect, "gaussian_blur");
    let r = ok(s, "br.video.set", json!({"clip": v.clip, "effect": i, "param": "blurriness", "value": 40}));
    assert_eq!(r["value"], 40.0);
    ok(s, "br.video.effect", json!({"clip": v.clip, "op": "disable", "effect": i}));
    assert!(!item(s, v.clip).effects[i].enabled);
    ok(s, "br.video.effect", json!({"clip": v.clip, "op": "enable", "effect": "gaussian_blur"}));
    assert!(item(s, v.clip).effects[i].enabled);
    ok(s, "br.video.effect", json!({"clip": v.clip, "op": "reset", "effect": i}));
    assert_eq!(item(s, v.clip).effects[i].params["blurriness"].value, filmcraft_project::ParamValue::Float(0.0));
    ok(s, "br.video.effect", json!({"clip": v.clip, "op": "remove", "effect": i}));
    assert!(item(s, v.clip).effect("gaussian_blur").is_none());
    assert!(br_video::execute(s, "br.video.effect", &json!({"clip": v.clip, "op": "remove", "effect": "motion"})).is_err(), "Motion stays");
    // an audio effect on a picture goes on its sound
    let eq = ok(s, "br.video.effect", json!({"clip": v.clip, "op": "add", "effect": "parametric_eq"}));
    assert_eq!(eq["clip"], v.sound_of_clip, "{eq}");
    // the schema says what can be set
    let blur = ok(s, "br.video.schema", json!({"effect": "gaussian_blur"}));
    assert_eq!(blur["params"][0]["id"], "blurriness", "{blur}");
    let audio = ok(s, "br.video.schema", json!({"kind": "audio"}));
    assert!(audio.as_array().unwrap().len() > 40);
    let _ = std::fs::remove_dir_all(&v.d);
}
