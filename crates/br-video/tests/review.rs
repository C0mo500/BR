//! What the review of M2 found, kept from coming back: sound never lands on the main track; a
//! move to `seconds` past the clip itself lands where asked; duplicating works with the magnet off
//! and keeps the transition into what follows; inserting at a cut keeps the title that ends the
//! video at its end; deleting a main clip's sound leaves its background; a locked track refuses
//! a delete instead of leaving half of it; a template's music track is free of the sync lock; a
//! curved template fills again; a constant speed replaces a curve; a curve's pieces keep its
//! speeds on a short clip. Clips are made with `ffmpeg` (skipped without it, unless
//! BR_REQUIRE_FFMPEG is set).

use std::path::{Path, PathBuf};
use std::process::Command;

use br_video::{Session, main_track};
use filmcraft_project::{ClipId, Sequence, TrackKind};
use serde_json::{Value, json};

fn have_ffmpeg() -> bool {
    let found = Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("BR_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (BR_REQUIRE_FFMPEG is set)");
    found
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("br-review-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ffmpeg(args: &[&str], out: &Path) {
    let o = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(out).output().unwrap();
    assert!(o.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

/// An upright clip with sound.
fn clip(d: &Path, name: &str, seconds: f64) -> String {
    let p = d.join(name);
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=360x640:rate=30:duration={seconds}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={seconds}"),
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
    p.to_string_lossy().into_owned()
}

fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    br_video::execute(s, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn q(s: &Session) -> &Sequence {
    s.active_sequence().expect("a video")
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.05
}

/// The main track: (clip, start, end) in seconds.
fn mains(s: &Session) -> Vec<(u64, f64, f64)> {
    main_track::main_clips(q(s)).iter().map(|it| (it.id.0, it.start.seconds(), it.end().seconds())).collect()
}

fn at(s: &Session, c: u64) -> (f64, f64) {
    q(s).find_item(ClipId(c)).map(|(_, it)| (it.start.seconds(), it.end().seconds())).unwrap_or_else(|| panic!("clip {c}"))
}

/// The sound linked with clip `c`.
fn sound_of(s: &Session, c: u64) -> u64 {
    let link = q(s).find_item(ClipId(c)).unwrap().1.link.expect("linked");
    q(s).audio_tracks.iter().flat_map(|t| &t.items).find(|it| it.link == Some(link)).expect("its sound").id.0
}

/// A 9:16 video of a (4 s), b (3 s), c (2 s).
fn video(name: &str) -> (Session, [u64; 3], PathBuf) {
    let d = dir(name);
    let paths: Vec<String> = [("a.mp4", 4.0), ("b.mp4", 3.0), ("c.mp4", 2.0)].iter().map(|(n, s)| clip(&d, n, *s)).collect();
    let mut s = Session::default();
    ok(&mut s, "br.video.new", json!({"format": "9:16"}));
    ok(&mut s, "br.video.add", json!({"paths": paths}));
    ok(&mut s, "timeline.select", json!({"clips": []}));
    let m = mains(&s);
    (s, [m[0].0, m[1].0, m[2].0], d)
}

#[test]
fn sound_does_not_land_on_the_main_track() {
    if !have_ffmpeg() {
        return;
    }
    let (mut s, [a, ..], d) = video("sound-main");
    let fx = d.join("plopp.m4a");
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=880:duration=0.5", "-c:a", "aac"], &fx);
    let r = ok(&mut s, "br.video.place", json!({"paths": [fx.to_string_lossy()], "target": "audio", "seconds": 1.0}));
    let sfx = r["clips"][0].as_u64().unwrap();
    let before = mains(&s);
    let e = br_video::execute(&mut s, "br.video.move", &json!({"clip": sfx, "seconds": 4.0, "track": "main"}));
    assert!(e.is_err(), "{e:?}");
    assert_eq!(mains(&s), before, "nothing moved");
    let (tid, _) = q(&s).find_item(ClipId(sfx)).unwrap();
    assert_eq!(q(&s).track(tid).unwrap().kind, TrackKind::Audio);
    let _ = a;
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_move_to_a_time_after_the_clip_lands_at_that_cut() {
    if !have_ffmpeg() {
        return;
    }
    let (mut s, [a, b, c], d) = video("move-right");
    // the cut between b and c (at 7 s): a goes there
    ok(&mut s, "br.video.move", json!({"clip": a, "seconds": 7.0}));
    assert_eq!(mains(&s).iter().map(|m| m.0).collect::<Vec<_>>(), [b, a, c]);
    // to its own end: nothing changes
    ok(&mut s, "br.video.move", json!({"clip": a, "seconds": 7.0}));
    assert_eq!(mains(&s).iter().map(|m| m.0).collect::<Vec<_>>(), [b, a, c]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn duplicating_works_with_the_magnet_off_and_keeps_the_transition() {
    if !have_ffmpeg() {
        return;
    }
    let (mut s, [a, b, _], d) = video("duplicate");
    ok(&mut s, "br.video.transition", json!({"clips": [a, b]}));
    let main = main_track::main_track_id(q(&s)).unwrap();
    let r = ok(&mut s, "br.video.duplicate", json!({"clips": [a]}));
    let copy = r["copies"][0].as_u64().unwrap();
    let m = mains(&s);
    assert_eq!(m.iter().map(|x| x.0).take(3).collect::<Vec<_>>(), [a, copy, b], "{m:?}");
    let tr = q(&s).track(main).unwrap();
    assert!(
        tr.transitions.iter().any(|x| x.from == Some(ClipId(copy)) && x.to == Some(ClipId(b))),
        "the transition into b went with the copy: {:?}",
        tr.transitions.iter().map(|x| (x.from, x.to)).collect::<Vec<_>>()
    );
    // the magnet off: still a copy right after it, the rest makes room
    ok(&mut s, "br.video.magnet", json!({"on": false}));
    let r = ok(&mut s, "br.video.duplicate", json!({"clips": [a]}));
    let copy2 = r["copies"][0].as_u64().unwrap();
    let m = mains(&s);
    assert_eq!(m[1].0, copy2, "{m:?}");
    for w in m.windows(2) {
        assert!(w[0].2 <= w[1].1 + 1e-6, "no overlap: {m:?}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn inserting_at_a_cut_keeps_the_title_that_ends_the_video_at_its_end() {
    if !have_ffmpeg() {
        return;
    }
    let (mut s, _, d) = video("end-card");
    let end = ok(&mut s, "br.video.text", json!({"text": "Ende", "at": 9.0, "seconds": 1.5}))["clip"].as_u64().unwrap();
    assert!(close(at(&s, end).0, 9.0));
    let more = clip(&d, "d.mp4", 2.0);
    ok(&mut s, "br.video.add", json!({"paths": [more], "at": 4.0}));
    assert!(close(at(&s, end).0, 11.0), "the end card at the new end: {:?} {:?}", at(&s, end), mains(&s));
    // added at the end too
    let more = clip(&d, "e.mp4", 1.0);
    ok(&mut s, "br.video.add", json!({"paths": [more]}));
    assert!(close(at(&s, end).0, 12.0), "{:?} {:?}", at(&s, end), mains(&s));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn deleting_a_main_clip_s_sound_leaves_its_background() {
    if !have_ffmpeg() {
        return;
    }
    let (mut s, [a, ..], d) = video("bg-sound");
    ok(&mut s, "br.video.background", json!({"mode": "blur", "clips": [a]}));
    assert!(br_video::frame::background_of(q(&s), ClipId(a)).is_some());
    let sound = sound_of(&s, a);
    ok(&mut s, "br.video.delete", json!({"clips": [sound]}));
    assert!(br_video::frame::background_of(q(&s), ClipId(a)).is_some(), "the background stayed");
    assert!(q(&s).find_item(ClipId(sound)).is_none());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_locked_sound_track_refuses_a_delete_and_changes_nothing() {
    if !have_ffmpeg() {
        return;
    }
    let (mut s, [_, b, _], d) = video("locked");
    let audio = main_track::main_audio_track(q(&s)).expect("the main sound track");
    s.edit_sequence("Sperren", |q, _, _| {
        q.track_mut(audio).unwrap().locked = true;
        Ok(())
    })
    .unwrap();
    let before = s.execute("sequence.inspect", json!({})).unwrap();
    let e = br_video::execute(&mut s, "br.video.delete", &json!({"clips": [b]}));
    assert!(e.as_ref().is_err_and(|e| e.contains("gesperrt")), "{e:?}");
    assert_eq!(s.execute("sequence.inspect", json!({})).unwrap(), before, "nothing half done");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_template_s_music_track_is_free_of_the_sync_lock() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("template-music");
    let clicks = d.join("klick.wav");
    ffmpeg(&["-f", "lavfi", "-i", "aevalsrc='if(lt(mod(t\\,0.5)\\,0.03)\\,sin(2*PI*1200*t)\\,0)':s=48000:d=20"], &clicks);
    let mut s = Session::default();
    ok(&mut s, "br.video.template.new", json!({"template": "takt", "music": clicks.to_string_lossy()}));
    let main = main_track::main_track_id(q(&s));
    let main_audio = main_track::main_audio_track(q(&s));
    let locked: Vec<String> = q(&s)
        .all_tracks()
        .filter(|t| t.sync_lock && Some(t.id) != main && Some(t.id) != main_audio && !main_track::is_background_track(t) && !t.items.is_empty())
        .map(|t| t.name.clone())
        .collect();
    assert!(locked.is_empty(), "tracks still sync-locked: {locked:?}");
    // FilmCraft's own Ripple Delete is not refused because of the music
    let first = mains(&s)[0].0;
    s.execute("edit.rippleDelete", json!({"clips": [first]})).unwrap();
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_template_with_curves_fills_again() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("refill");
    let mut s = Session::default();
    ok(&mut s, "br.video.template.new", json!({"template": "reise"}));
    let slots = br_video::template::placeholders(q(&s)).len();
    let first: Vec<String> = (0..slots).map(|i| clip(&d, &format!("r{i}.mp4"), 4.0)).collect();
    ok(&mut s, "br.video.template.fill", json!({"paths": first}));
    assert_eq!(br_video::template::placeholders(q(&s)).len(), slots, "a curve's pieces are one slot");
    let again: Vec<String> = (0..slots).map(|i| clip(&d, &format!("n{i}.mp4"), 4.0)).collect();
    let r = ok(&mut s, "br.video.template.fill", json!({"paths": again, "all": true}));
    assert_eq!(r["filled"].as_array().unwrap().len(), slots, "{r}");
    let names: Vec<String> = main_track::main_clips(q(&s)).iter().map(|it| it.name.clone()).collect();
    for i in 0..slots {
        assert!(names.iter().any(|n| *n == format!("n{i}.mp4")), "n{i} is in: {names:?}");
    }
    assert!(names.iter().all(|n| n.starts_with('n')), "nothing old left: {names:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_constant_speed_replaces_a_curve_and_backwards_turns_the_whole_curve() {
    if !have_ffmpeg() {
        return;
    }
    let (mut s, [a, b, _], d) = video("speed-curve");
    ok(&mut s, "br.video.curve", json!({"clips": [a], "preset": "montage"}));
    assert!(mains(&s).len() > 3);
    // the inspector's Rückwärts on one piece: the whole curve plays backwards
    let piece = mains(&s)[1].0;
    ok(&mut s, "br.video.set", json!({"clip": piece, "row": "speed.reverse", "value": true}));
    let pieces: Vec<bool> =
        main_track::main_clips(q(&s)).iter().filter(|it| br_video::speed::curve_of(q(&s), it.id).is_some()).map(|it| it.reverse).collect();
    assert!(pieces.len() > 1 && pieces.iter().all(|r| *r), "{pieces:?}");
    // a speed on one piece: one clip at that speed, no curve left
    let piece = mains(&s)[1].0;
    ok(&mut s, "br.video.speed", json!({"clips": [piece], "speed": 2.0}));
    let m = mains(&s);
    assert_eq!(m.len(), 3, "{m:?}");
    let first = main_track::main_clips(q(&s))[0];
    assert!(br_video::speed::curve_of(q(&s), first.id).is_none());
    assert!((first.speed - 2.0).abs() < 1e-6 && close(m[0].2, 2.0), "{m:?}");
    assert!(close(at(&s, b).0, 2.0), "b follows");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_held_curve_on_a_short_clip_keeps_its_speeds() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("held");
    let short = clip(&d, "kurz.mp4", 1.1);
    let mut s = Session::default();
    ok(&mut s, "br.video.new", json!({"format": "9:16"}));
    ok(&mut s, "br.video.add", json!({"paths": [short]}));
    let c = mains(&s)[0].0;
    ok(&mut s, "br.video.curve", json!({"clips": [c], "preset": "held"}));
    let speeds: Vec<f64> = main_track::main_clips(q(&s)).iter().map(|it| it.speed).collect();
    // "held": 2× and 0.25×, smoothly between; never slower or faster than that
    assert!(speeds.iter().all(|v| (0.2..=2.2).contains(v)), "{speeds:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_sound_dropped_with_a_video_at_the_end_stays_where_it_was_dropped() {
    if !have_ffmpeg() {
        return;
    }
    let (mut s, _, d) = video("drop-pair");
    let more = clip(&d, "d.mp4", 2.0);
    let fx = d.join("whoosh.m4a");
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=880:duration=0.5", "-c:a", "aac"], &fx);
    let r = ok(&mut s, "br.video.place", json!({"paths": [more, fx.to_string_lossy()], "seconds": 9.0}));
    let q = q(&s);
    let whoosh = r["clips"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_u64)
        .find(|c| q.find_item(ClipId(*c)).is_some_and(|(_, it)| it.link.is_none() && q.audio_tracks.iter().any(|t| t.item(ClipId(*c)).is_some())))
        .expect("the sound");
    assert!(close(at(&s, whoosh).0, 9.0), "the sound at 9 s: {:?}", at(&s, whoosh));
    let m = mains(&s);
    assert!(close(m.last().unwrap().1, 9.0) && close(m.last().unwrap().2, 11.0), "{m:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_split_placeholder_is_two_to_fill() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("split-slot");
    let mut s = Session::default();
    ok(&mut s, "br.video.template.new", json!({"template": "story"}));
    let first = br_video::template::placeholders(q(&s))[0].0;
    let (a, b) = at(&s, first.0);
    ok(&mut s, "br.video.split", json!({"clip": first.0, "seconds": (a + b) / 2.0}));
    let n = br_video::template::placeholders(q(&s)).len();
    assert_eq!(n, 7, "six slots, the first in two");
    let files: Vec<String> = (0..n).map(|i| clip(&d, &format!("s{i}.mp4"), 3.0)).collect();
    let r = ok(&mut s, "br.video.template.fill", json!({"paths": files}));
    assert!(r["empty"].as_array().unwrap().is_empty(), "no grey left: {r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_speed_on_a_sound_piece_of_a_curve_is_taken() {
    if !have_ffmpeg() {
        return;
    }
    let (mut s, [a, ..], d) = video("sound-piece");
    ok(&mut s, "br.video.curve", json!({"clips": [a], "preset": "montage"}));
    let piece = main_track::main_clips(q(&s))[1].id.0;
    let sound = sound_of(&s, piece);
    ok(&mut s, "br.video.speed", json!({"clips": [sound], "speed": 2.0}));
    let it = q(&s).find_item(ClipId(sound)).unwrap().1;
    assert!((it.speed - 2.0).abs() < 1e-6);
    let _ = std::fs::remove_dir_all(&d);
}
