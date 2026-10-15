//! `br.video.place`, the end of every drag-and-drop path: onto the main track at the nearest cut,
//! above it as an overlay on a free track at a point of the picture, sound alone on a free sound
//! track, subtitles as captions, a part of a clip; one undo step each. Clips are made with
//! `ffmpeg` (skipped without it, unless BR_REQUIRE_FFMPEG is set).

use std::path::{Path, PathBuf};
use std::process::Command;

use br_video::{Session, main_track};
use filmcraft_project::{AudioType, ClipId};
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

fn clip(d: &Path, name: &str, seconds: u32) -> String {
    let p = d.join(name);
    let dur = seconds.to_string();
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=360x640:rate=30:duration={dur}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={dur}"),
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

fn sound(d: &Path, name: &str, seconds: f64) -> String {
    let p = d.join(name);
    ffmpeg(&["-f", "lavfi", "-i", &format!("sine=frequency=880:duration={seconds}"), "-c:a", "aac"], &p);
    p.to_string_lossy().into_owned()
}

fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    br_video::execute(s, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn sec(t: filmcraft_time::Tick) -> f64 {
    (t.0 as f64 / filmcraft_time::TICKS_PER_SECOND as f64 * 1000.0).round() / 1000.0
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.05
}

struct Video {
    s: Session,
    a: u64,
    b: u64,
    d: PathBuf,
}

/// A 9:16 video: a (4 s) and b (3 s) on the main track.
fn video(name: &str) -> Video {
    let d = std::env::temp_dir().join(format!("br-place-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (a, b) = (clip(&d, "a.mp4", 4), clip(&d, "b.mp4", 3));
    let mut s = Session::default();
    ok(&mut s, "br.video.new", json!({"format": "9:16"}));
    ok(&mut s, "br.video.add", json!({"paths": [a, b]}));
    let m = mains(&s);
    Video { a: m[0].0, b: m[1].0, s, d }
}

fn mains(s: &Session) -> Vec<(u64, f64, f64)> {
    main_track::main_clips(s.active_sequence().unwrap()).iter().map(|it| (it.id.0, sec(it.start), sec(it.end()))).collect()
}

/// (video track index, start, end) of a clip.
fn where_(s: &Session, c: u64) -> (usize, f64, f64) {
    let q = s.active_sequence().unwrap();
    let (tid, it) = q.find_item(ClipId(c)).unwrap();
    let i = q.video_tracks.iter().position(|t| t.id == tid).unwrap_or_else(|| 100 + q.audio_tracks.iter().position(|t| t.id == tid).unwrap());
    (i, sec(it.start), sec(it.end()))
}

fn picture_of(s: &Session, clips: &Value) -> u64 {
    let q = s.active_sequence().unwrap();
    clips
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_u64)
        .find(|c| q.video_tracks.iter().any(|t| t.item(ClipId(*c)).is_some()))
        .expect("a picture")
}

#[test]
fn a_picture_dropped_on_the_main_track_goes_in_at_the_nearest_cut() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("main");
    let c = clip(&v.d, "c.mp4", 2);
    let undo = v.s.history.undo.len();
    let r = ok(&mut v.s, "br.video.place", json!({"paths": [c], "target": "main", "seconds": 3.2}));
    assert_eq!(v.s.history.undo.len(), undo + 1, "import and place: one step");
    let m = mains(&v.s);
    let new = picture_of(&v.s, &r["clips"]);
    assert_eq!(m.iter().map(|x| x.0).collect::<Vec<_>>(), vec![v.a, new, v.b], "at the cut at 4 s, nothing cut in two: {m:?}");
    assert!(close(m[1].1, 4.0) && close(m[2].1, 6.0), "{m:?}");
    // a part of a clip, as the Source monitor's marks give it
    let item_a = v.s.active_sequence().unwrap().find_item(ClipId(v.a)).unwrap().1.item.0;
    let r = ok(&mut v.s, "br.video.place", json!({"items": [item_a], "target": "main", "seconds": 0.0, "sourceIn": 1.0, "duration": 1.5}));
    let part = picture_of(&v.s, &r["clips"]);
    let m = mains(&v.s);
    assert_eq!(m[0].0, part, "{m:?}");
    assert!(close(m[0].2, 1.5), "1.5 s of it: {m:?}");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn a_picture_dropped_above_is_an_overlay_on_a_free_track_where_it_was_dropped() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("overlay");
    let o = clip(&v.d, "o.mp4", 2);
    let r = ok(&mut v.s, "br.video.place", json!({"paths": [o], "target": "overlay", "seconds": 1.0, "position": [0.25, 0.75]}));
    let first = picture_of(&v.s, &r["clips"]);
    let (track, start, end) = where_(&v.s, first);
    assert!(track >= 1 && close(start, 1.0) && close(end, 3.0), "above the main track at 1 s: {:?}", where_(&v.s, first));
    let q = v.s.active_sequence().unwrap();
    let it = q.find_item(ClipId(first)).unwrap().1;
    let pos = it.effect("motion").unwrap().params["position"].value.as_vec2().unwrap();
    assert!(close(pos.x, 270.0) && close(pos.y, 1440.0), "where it was dropped in the picture: {pos:?}");
    // the main track did not move
    assert_eq!(mains(&v.s).iter().map(|m| (m.1, m.2)).collect::<Vec<_>>(), vec![(0.0, 4.0), (4.0, 7.0)]);
    // overlapping: the next free track; later on the first one again
    let r2 = ok(&mut v.s, "br.video.place", json!({"paths": [o], "target": "overlay", "seconds": 2.0}));
    let second = picture_of(&v.s, &r2["clips"]);
    assert_eq!(where_(&v.s, second).0, track + 1, "{:?}", where_(&v.s, second));
    let tracks = v.s.active_sequence().unwrap().video_tracks.len();
    let r3 = ok(&mut v.s, "br.video.place", json!({"paths": [o], "target": "overlay", "seconds": 4.5}));
    let third = picture_of(&v.s, &r3["clips"]);
    assert_eq!(where_(&v.s, third).0, track, "free there again");
    assert_eq!(v.s.active_sequence().unwrap().video_tracks.len(), tracks, "no new track for it");
    // `track` names the overlay track wanted; auto with an overlay track is an overlay
    let r4 = ok(&mut v.s, "br.video.place", json!({"paths": [o], "track": format!("V{}", track + 2), "seconds": 0.0}));
    let fourth = picture_of(&v.s, &r4["clips"]);
    assert_eq!(where_(&v.s, fourth).0, track + 1, "V{} is free at 0 s", track + 2);
    // it hangs on the main clip it starts on
    let anchors = ok(&mut v.s, "br.video.anchors", json!({}));
    let parent = anchors["anchors"].as_array().unwrap().iter().find(|x| x["clip"] == first).map(|x| x["parent"].clone());
    assert_eq!(parent, Some(json!(v.a)), "{anchors}");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn sound_goes_on_a_free_sound_track_and_long_sound_is_music() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("sound");
    let plopp = sound(&v.d, "plopp.m4a", 0.5);
    let r = ok(&mut v.s, "br.video.place", json!({"paths": [&plopp], "seconds": 2.0}));
    let fx = r["clips"][0].as_u64().unwrap();
    let (track, start, _) = where_(&v.s, fx);
    assert!(track >= 100 && close(start, 2.0), "{:?}", where_(&v.s, fx));
    let tracks = v.s.active_sequence().unwrap().audio_tracks.len();
    let r = ok(&mut v.s, "br.video.place", json!({"paths": [&plopp], "seconds": 5.0}));
    let fx2 = r["clips"][0].as_u64().unwrap();
    assert_eq!(where_(&v.s, fx2).0, track, "the same track, free there");
    assert_eq!(v.s.active_sequence().unwrap().audio_tracks.len(), tracks, "no new track");
    let kind = |s: &Session, c: u64| s.active_sequence().unwrap().find_item(ClipId(c)).unwrap().1.essential.as_ref().map(|e| e.kind);
    assert_eq!(kind(&v.s, fx), Some(AudioType::Sfx));
    let song = sound(&v.d, "lied.m4a", 25.0);
    let r = ok(&mut v.s, "br.video.place", json!({"paths": [song], "target": "audio", "seconds": 0.0}));
    let m = r["clips"][0].as_u64().unwrap();
    assert_eq!(kind(&v.s, m), Some(AudioType::Music));
    // music ends with the picture, as the Musik tool's does
    let q = v.s.active_sequence().unwrap();
    let end = q.video_tracks.iter().flat_map(|t| &t.items).map(|it| it.end().seconds()).fold(0.0, f64::max);
    assert!(close(where_(&v.s, m).2, end), "{:?} vs {end}", where_(&v.s, m));
    // a video's sound alone onto a sound track
    let c = clip(&v.d, "c.mp4", 2);
    let r = ok(&mut v.s, "br.video.place", json!({"paths": [c], "target": "audio", "seconds": 1.0}));
    let q = v.s.active_sequence().unwrap();
    assert!(r["clips"].as_array().unwrap().iter().all(|c| q.audio_tracks.iter().any(|t| t.item(ClipId(c.as_u64().unwrap())).is_some())), "{r}");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn subtitles_become_captions_and_the_first_drop_makes_the_video() {
    if !have_ffmpeg() {
        return;
    }
    let d = std::env::temp_dir().join(format!("br-place-first-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let a = clip(&d, "a.mp4", 3);
    let mut s = Session::default();
    let r = ok(&mut s, "br.video.place", json!({"paths": [a]}));
    assert!(s.active_sequence().is_some() && !r["clips"].as_array().unwrap().is_empty(), "{r}");
    let srt = d.join("u.srt");
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:01,500\nHallo\n").unwrap();
    ok(&mut s, "br.video.place", json!({"paths": [srt.to_string_lossy()]}));
    let caps: usize = s.active_sequence().unwrap().caption_tracks.iter().map(|t| t.captions.len()).sum();
    assert_eq!(caps, 1);
    assert!(br_video::execute(&mut s, "br.video.place", &json!({"paths": ["/nirgends.fcproj"]})).is_err());
    assert!(br_video::execute(&mut s, "br.video.place", &json!({"items": [1], "target": "oben"})).is_err());
    let _ = std::fs::remove_dir_all(&d);
}
