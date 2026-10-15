//! Speed curves baked into pieces of constant speed, freeze frames, playing backwards and beat
//! markers: each one undo step, the rest of the video (and what hangs on it) moving up or on.
//! Clips are made with `ffmpeg` (skipped without it, unless SHNYP_REQUIRE_FFMPEG is set).

use std::path::{Path, PathBuf};
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

fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    shnyp_video::execute(s, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.05
}

struct Video {
    s: Session,
    a: u64,
    b: u64,
    title: u64,
    d: PathBuf,
}

/// A 9:16 video: a (4 s), b (3 s); a title on b at 5 s; nothing selected.
fn video(name: &str) -> Video {
    let d = std::env::temp_dir().join(format!("shnyp-speed-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (a, b) = (clip(&d, "a.mp4", 4), clip(&d, "b.mp4", 3));
    let mut s = Session::default();
    ok(&mut s, "shnyp.video.new", json!({"format": "9:16"}));
    ok(&mut s, "shnyp.video.add", json!({"paths": [a, b]}));
    let title = ok(&mut s, "shnyp.video.text", json!({"text": "Auf B", "seconds": 1.0, "at": 5.0}))["clip"].as_u64().unwrap();
    ok(&mut s, "timeline.select", json!({"clips": []}));
    let m: Vec<u64> = main_track::main_clips(s.active_sequence().unwrap()).iter().map(|it| it.id.0).collect();
    Video { a: m[0], b: m[1], title, s, d }
}

/// The main clips: (clip, start, end, speed, media in, media out) in seconds.
fn mains(s: &Session) -> Vec<(u64, f64, f64, f64, f64, f64)> {
    main_track::main_clips(s.active_sequence().unwrap())
        .iter()
        .map(|it| (it.id.0, it.start.seconds(), it.end().seconds(), it.speed, it.source_in.seconds(), it.source_out().seconds()))
        .collect()
}

fn start_of(s: &Session, c: u64) -> f64 {
    s.active_sequence().unwrap().find_item(ClipId(c)).unwrap().1.start.seconds()
}

#[test]
fn a_curve_is_baked_into_pieces_and_put_back_together() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("curve");
    let s = &mut v.s;
    let before = mains(s);
    let undo = s.history.undo.len();
    let r = ok(s, "shnyp.video.curve", json!({"clips": [v.a], "preset": "montage"}));
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let pieces = r["pieces"].as_array().unwrap().len();
    assert!(pieces >= 4, "pieces of constant speed: {r}");
    let m = mains(s);
    let a: Vec<_> = m.iter().take(pieces).collect();
    // back to back, the media continuous from 0 to 4 s, speeds different
    for w in a.windows(2) {
        assert!(close(w[0].2, w[1].1), "back to back: {m:?}");
        assert!(close(w[0].5, w[1].4), "the media continues: {m:?}");
    }
    assert!(close(a[0].4, 0.0) && close(a.last().unwrap().5, 4.0), "the whole clip: {m:?}");
    assert!(a.iter().any(|p| p.3 > 1.5) && a.iter().any(|p| p.3 < 0.7), "fast and slow parts: {m:?}");
    // b and its title follow
    let end = a.last().unwrap().2;
    let b = m.iter().find(|x| x.0 == v.b).unwrap();
    assert!(close(b.1, end), "b right after the curve: {m:?}");
    assert!(close(start_of(s, v.title), end + 1.0), "the title on b went along: {}", start_of(s, v.title));
    // the sound pieces match the picture pieces
    let q = s.active_sequence().unwrap();
    for p in &a {
        let (_, it) = q.find_item(ClipId(p.0)).unwrap();
        let sound = q.audio_tracks.iter().flat_map(|t| &t.items).find(|x| x.link == it.link).expect("its sound");
        assert!(close(sound.start.seconds(), p.1) && close(sound.end().seconds(), p.2) && (sound.speed - p.3).abs() < 1e-6, "sound in step: {p:?}");
    }
    // the inspector and the state know the curve; none puts the clip back together
    ok(s, "shnyp.video.curve", json!({"clips": [m[1].0], "preset": "none"}));
    let back = mains(s);
    assert_eq!(back.len(), before.len());
    assert!(close(back[0].2, 4.0) && close(back[0].3, 1.0) && close(back[1].1, 4.0), "{back:?}");
    assert!(close(start_of(s, v.title), 5.0), "the title came back: {}", start_of(s, v.title));
    // another curve over a curve replaces it
    ok(s, "shnyp.video.curve", json!({"clips": [v.a], "preset": "blitzRein"}));
    ok(s, "shnyp.video.curve", json!({"clips": [v.a], "preset": "sprung"}));
    let q = s.active_sequence().unwrap();
    let presets: Vec<String> = main_track::main_clips(q).iter().filter_map(|it| shnyp_video::speed::curve_of(q, it.id).map(|c| c.0)).collect();
    assert!(!presets.is_empty() && presets.iter().all(|p| p == "sprung"), "{presets:?}");
    // one undo: the earlier curve again
    s.execute("edit.undo", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    assert!(main_track::main_clips(q).iter().filter_map(|it| shnyp_video::speed::curve_of(q, it.id)).all(|c| c.0 == "blitzRein"));
    assert!(shnyp_video::execute(s, "shnyp.video.curve", &json!({"clips": [v.a], "preset": "zickzack"})).is_err());
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn a_freeze_holds_the_picture_and_the_rest_moves_on() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("freeze");
    let s = &mut v.s;
    let undo = s.history.undo.len();
    ok(s, "shnyp.video.freeze", json!({"clip": v.a, "at": 1.0, "seconds": 2.0}));
    assert_eq!(s.history.undo.len(), undo + 1);
    let m = mains(s);
    assert_eq!(m.len(), 4, "a, the still, the rest of a, b: {m:?}");
    assert!(
        close(m[0].2, 1.0) && close(m[1].1, 1.0) && close(m[1].2, 3.0) && close(m[2].1, 3.0) && close(m[2].2, 6.0) && close(m[3].1, 6.0),
        "{m:?}"
    );
    let q = s.active_sequence().unwrap();
    let still = q.find_item(ClipId(m[1].0)).unwrap().1;
    assert!(still.frame_hold.is_some_and(|h| close(h.seconds(), 1.0)), "held at 1 s: {:?}", still.frame_hold);
    assert!(q.audio_tracks.iter().flat_map(|t| &t.items).all(|it| it.link != still.link), "the sound pauses");
    assert!(close(start_of(s, v.title), 7.0), "the title on b moved on: {}", start_of(s, v.title));
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn backwards_and_forwards_again() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("reverse");
    let s = &mut v.s;
    ok(s, "shnyp.video.reverse", json!({"clips": [v.b]}));
    let q = s.active_sequence().unwrap();
    let b = q.find_item(ClipId(v.b)).unwrap().1;
    assert!(b.reverse);
    let sound = q.audio_tracks.iter().flat_map(|t| &t.items).find(|x| x.link == b.link).unwrap();
    assert!(sound.reverse, "its sound too");
    ok(s, "shnyp.video.reverse", json!({"clips": [v.b]}));
    assert!(!s.active_sequence().unwrap().find_item(ClipId(v.b)).unwrap().1.reverse);
    // with a curve
    ok(s, "shnyp.video.curve", json!({"clips": [v.a], "preset": "blitzRaus"}));
    ok(s, "shnyp.video.reverse", json!({"clips": [v.a]}));
    let q = s.active_sequence().unwrap();
    let pieces: Vec<_> = main_track::main_clips(q).into_iter().filter(|it| shnyp_video::speed::curve_of(q, it.id).is_some()).collect();
    assert!(pieces.len() > 1 && pieces.iter().all(|p| p.reverse), "the curve plays backwards");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn beats_become_markers() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("beats");
    let s = &mut v.s;
    // clicks at 120 beats per minute
    let clicks = v.d.join("klick.wav");
    ffmpeg(&["-f", "lavfi", "-i", "aevalsrc='if(lt(mod(t\\,0.5)\\,0.03)\\,sin(2*PI*1200*t)\\,0)':s=48000:d=12"], &clicks);
    ok(s, "shnyp.video.music", json!({"path": clicks.to_string_lossy(), "duck": false}));
    let r = ok(s, "shnyp.video.beats", json!({}));
    let bpm = r["bpm"].as_f64().unwrap();
    assert!((bpm - 120.0).abs() < 3.0, "120 bpm: {r}");
    let n = r["count"].as_u64().unwrap();
    assert!((12..=15).contains(&n), "a beat every 0.5 s over the 7 s of music: {r}");
    let marks = shnyp_video::beats::markers(s.active_sequence().unwrap());
    assert_eq!(marks.len() as u64, n);
    for w in marks.windows(2) {
        assert!((w[1] - w[0] - 0.5).abs() < 0.06, "half a second apart: {marks:?}");
    }
    let bars = ok(s, "shnyp.video.beats", json!({"mode": "bars"}));
    assert!(bars["count"].as_u64().unwrap() <= n / 3, "{bars}");
    ok(s, "shnyp.video.beats", json!({"mode": "none"}));
    assert!(shnyp_video::beats::markers(s.active_sequence().unwrap()).is_empty());
    let _ = std::fs::remove_dir_all(&v.d);
}
