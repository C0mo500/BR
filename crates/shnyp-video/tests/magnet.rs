//! CapCut's magnetic main track and attached clips on FilmCraft's sequence: deleting, inserting,
//! reordering, trimming, speeding up and splitting main clips; titles, sound effects and captions
//! follow the clip they start on; each edit is one undo step; and an edit made with FilmCraft's
//! own commands ends the same way as shnyp's. Clips are made with `ffmpeg` (skipped without it,
//! unless SHNYP_REQUIRE_FFMPEG is set).

use std::path::{Path, PathBuf};
use std::process::Command;

use filmcraft_project::{AudioType, ClipId, Sequence};
use serde_json::{Value, json};
use shnyp_video::{Session, main_track};

fn have_ffmpeg() -> bool {
    let found = Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("SHNYP_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (SHNYP_REQUIRE_FFMPEG is set)");
    found
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("shnyp-magnet-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ffmpeg(args: &[&str], out: &Path) {
    let o = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(out).output().unwrap();
    assert!(o.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

/// A small upright clip with sound, `seconds` long.
fn clip(path: &Path, seconds: u32, hz: u32) {
    let d = seconds.to_string();
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=360x640:rate=30:duration={d}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={hz}:duration={d}"),
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
        path,
    );
}

fn sound(path: &Path, seconds: f64, hz: u32) {
    ffmpeg(&["-f", "lavfi", "-i", &format!("sine=frequency={hz}:duration={seconds}"), "-c:a", "aac"], path);
}

fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    shnyp_video::execute(s, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn sec(t: filmcraft_time::Tick) -> f64 {
    (t.0 as f64 / filmcraft_time::TICKS_PER_SECOND as f64 * 1000.0).round() / 1000.0
}

fn q(s: &Session) -> &Sequence {
    s.active_sequence().expect("a video")
}

/// The main track: (clip, start, end) in seconds.
fn main_clips(s: &Session) -> Vec<(u64, f64, f64)> {
    main_track::main_clips(q(s)).iter().map(|it| (it.id.0, sec(it.start), sec(it.end()))).collect()
}

/// Where a clip is: (start, end) in seconds.
fn at(s: &Session, c: u64) -> Option<(f64, f64)> {
    q(s).find_item(ClipId(c)).map(|(_, it)| (sec(it.start), sec(it.end())))
}

/// The captions: (text, start, end).
fn captions(s: &Session) -> Vec<(String, f64, f64)> {
    q(s).caption_tracks.iter().flat_map(|t| t.captions.iter()).map(|c| (c.text.clone(), sec(c.start), sec(c.end()))).collect()
}

fn music(s: &Session) -> Vec<(f64, f64)> {
    q(s).audio_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|it| it.essential.as_ref().is_some_and(|e| e.kind == AudioType::Music))
        .map(|it| (sec(it.start), sec(it.end())))
        .collect()
}

/// The titles (graphic clips): (clip, start, end).
fn titles(s: &Session) -> Vec<(u64, f64, f64)> {
    let mut v: Vec<(u64, f64, f64)> = q(s)
        .video_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|it| it.graphic_layers().next().is_some())
        .map(|it| (it.id.0, sec(it.start), sec(it.end())))
        .collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.05
}

/// The sequence without what only the editor's view holds (playhead, selection).
fn shape(s: &mut Session) -> Value {
    let mut v = s.execute("sequence.inspect", json!({})).unwrap();
    for k in ["playhead", "selection", "transitionSelection", "gapSelection"] {
        if let Some(o) = v.as_object_mut() {
            o.remove(k);
        }
    }
    json!({"sequence": v, "captions": captions(s)})
}

/// A 9:16 video: main clips a (4 s), b (3 s), c (2 s); a title on b at 5 s and one on c at
/// 7.5 s; a sound effect on c at 8 s; captions on a, b and c; music under all of it.
struct Video {
    s: Session,
    a: u64,
    b: u64,
    c: u64,
    title_b: u64,
    title_c: u64,
    sfx: u64,
    d: PathBuf,
}

fn video(name: &str) -> Video {
    let d = dir(name);
    let (a, b, c, m, fx) = (d.join("a.mp4"), d.join("b.mp4"), d.join("c.mp4"), d.join("music.m4a"), d.join("plopp.m4a"));
    clip(&a, 4, 440);
    clip(&b, 3, 550);
    clip(&c, 2, 660);
    sound(&m, 12.0, 220);
    sound(&fx, 0.5, 880);
    let srt = d.join("untertitel.srt");
    std::fs::write(
        &srt,
        "1\n00:00:01,000 --> 00:00:02,000\nA spricht\n\n2\n00:00:04,500 --> 00:00:05,500\nB spricht\n\n3\n00:00:05,600 --> 00:00:06,400\nB weiter\n\n4\n00:00:07,200 --> 00:00:08,000\nC spricht\n",
    )
    .unwrap();
    let mut s = Session::default();
    ok(&mut s, "shnyp.video.new", json!({"format": "9:16"}));
    ok(&mut s, "shnyp.video.add", json!({"paths": [a.to_string_lossy(), b.to_string_lossy(), c.to_string_lossy()]}));
    let m3 = main_clips(&s);
    assert_eq!(m3.len(), 3, "three main clips: {m3:?}");
    let (ca, cb, cc) = (m3[0].0, m3[1].0, m3[2].0);
    assert!(close(m3[1].1, 4.0) && close(m3[2].1, 7.0) && close(m3[2].2, 9.0), "{m3:?}");
    let t1 = ok(&mut s, "shnyp.video.text", json!({"text": "Auf B", "seconds": 1.0, "at": 5.0}));
    let t2 = ok(&mut s, "shnyp.video.text", json!({"text": "Auf C", "seconds": 1.0, "at": 7.5}));
    let title_b = t1["clip"].as_u64().unwrap_or_else(|| panic!("{t1}"));
    let title_c = t2["clip"].as_u64().unwrap_or_else(|| panic!("{t2}"));
    let st = ok(&mut s, "shnyp.video.add", json!({"paths": [fx.to_string_lossy()], "at": 8.0}));
    let _ = st;
    let sfx = q(&s)
        .audio_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .find(|it| it.link.is_none() && it.essential.as_ref().is_some_and(|e| e.kind == AudioType::Sfx) && close(sec(it.start), 8.0))
        .map(|it| it.id.0)
        .unwrap_or_else(|| panic!("the sound effect at 8 s"));
    ok(&mut s, "shnyp.video.captions", json!({"path": srt.to_string_lossy()}));
    ok(&mut s, "shnyp.video.music", json!({"path": m.to_string_lossy(), "duck": false}));
    Video { s, a: ca, b: cb, c: cc, title_b, title_c, sfx, d }
}

#[test]
fn deleting_a_main_clip_takes_what_hangs_on_it_along_and_one_undo_brings_all_back() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("delete");
    let s = &mut v.s;
    let anchors = ok(s, "shnyp.video.anchors", json!({}));
    let parent_of = |c: u64| anchors["anchors"].as_array().unwrap().iter().find(|a| a["clip"] == c).map(|a| a["parent"].as_u64().unwrap());
    assert_eq!(parent_of(v.title_b), Some(v.b), "{anchors}");
    assert_eq!(parent_of(v.title_c), Some(v.c), "{anchors}");
    assert_eq!(parent_of(v.sfx), Some(v.c), "{anchors}");
    let before = shape(s);
    let undo = s.history.undo.len();

    ok(s, "shnyp.video.delete", json!({"clips": [v.b]}));
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let m = main_clips(s);
    assert_eq!(m.iter().map(|c| c.0).collect::<Vec<_>>(), vec![v.a, v.c], "{m:?}");
    assert!(close(m[1].1, 4.0) && close(m[1].2, 6.0), "the gap closed: {m:?}");
    assert_eq!(at(s, v.title_b), None, "the title on b went with it");
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 4.5)), Some(true), "the title on c moved up: {:?}", at(s, v.title_c));
    assert_eq!(at(s, v.sfx).map(|t| close(t.0, 5.0)), Some(true), "the sound effect moved up: {:?}", at(s, v.sfx));
    let caps = captions(s);
    assert_eq!(caps.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec!["A spricht", "C spricht"], "{caps:?}");
    assert!(close(caps[0].1, 1.0) && close(caps[1].1, 4.2) && close(caps[1].2, 5.0), "{caps:?}");
    let mu = music(s);
    assert_eq!(mu.len(), 1, "{mu:?}");
    assert!(close(mu[0].1, 9.0), "the music stays whole: {mu:?}");
    assert!(close(ok(s, "shnyp.video.state", json!({}))["seconds"].as_f64().unwrap(), 6.0), "the video is as long as its picture");

    ok(s, "edit.undo", json!({}));
    assert_eq!(shape(s), before, "one undo brings everything back");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn inserting_at_the_playhead_goes_to_the_nearest_cut_and_cuts_nothing_in_two() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("insert");
    let s = &mut v.s;
    // a title across the cut between a and b
    let t = ok(s, "shnyp.video.text", json!({"text": "Quer", "seconds": 2.0, "at": 3.0}));
    let across = t["clip"].as_u64().unwrap();
    let n = v.d.join("n.mp4");
    clip(&n, 2, 330);
    let undo = s.history.undo.len();
    ok(s, "playhead.set", json!({"seconds": 4.6}));
    ok(s, "shnyp.video.add", json!({"paths": [n.to_string_lossy()], "at": "playhead"}));
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let m = main_clips(s);
    assert_eq!(m.len(), 4, "{m:?}");
    assert_eq!((m[0].0, m[2].0, m[3].0), (v.a, v.b, v.c), "the new clip is second: {m:?}");
    assert!(close(m[1].1, 4.0) && close(m[1].2, 6.0) && close(m[2].1, 6.0) && close(m[3].1, 9.0), "{m:?}");
    // nothing cut: the title across the cut is one clip where it was, the music is one clip
    assert_eq!(at(s, across), Some((3.0, 5.0)), "the title on a stays");
    assert_eq!(titles(s).len(), 3, "no title cut in two: {:?}", titles(s));
    assert_eq!(music(s).len(), 1, "the music is one clip: {:?}", music(s));
    // what hangs on b and c moved on by 2 s
    assert_eq!(at(s, v.title_b).map(|t| close(t.0, 7.0)), Some(true), "{:?}", at(s, v.title_b));
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 9.5)), Some(true), "{:?}", at(s, v.title_c));
    assert_eq!(at(s, v.sfx).map(|t| close(t.0, 10.0)), Some(true), "{:?}", at(s, v.sfx));
    let caps = captions(s);
    assert!(close(caps[1].1, 6.5) && close(caps[3].1, 9.2), "captions follow their clips: {caps:?}");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn reordering_trimming_speeding_and_splitting_main_clips_keep_the_attached_clips_with_them() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("edits");
    let s = &mut v.s;

    // c to the front: c [0,2), a [2,6), b [6,9)
    let undo = s.history.undo.len();
    ok(s, "shnyp.video.move", json!({"clip": v.c, "index": 0}));
    assert_eq!(s.history.undo.len(), undo + 1);
    let m = main_clips(s);
    assert_eq!(m.iter().map(|c| c.0).collect::<Vec<_>>(), vec![v.c, v.a, v.b], "{m:?}");
    assert!(close(m[1].1, 2.0) && close(m[2].1, 6.0), "{m:?}");
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 0.5)), Some(true), "{:?}", at(s, v.title_c));
    assert_eq!(at(s, v.title_b).map(|t| close(t.0, 7.0)), Some(true), "{:?}", at(s, v.title_b));
    assert_eq!(at(s, v.sfx).map(|t| close(t.0, 1.0)), Some(true), "{:?}", at(s, v.sfx));
    ok(s, "edit.undo", json!({}));

    // b's head trimmed by 1 s (ripple): b [4,6) shows from 1 s; its speech captions follow its
    // picture (the first keeps the half that is still shown), its title keeps its place on b
    let undo = s.history.undo.len();
    ok(s, "shnyp.video.trim", json!({"clip": v.b, "edge": "in", "seconds": 1.0}));
    assert_eq!(s.history.undo.len(), undo + 1);
    let m = main_clips(s);
    assert!(close(m[1].1, 4.0) && close(m[1].2, 6.0) && close(m[2].1, 6.0), "{m:?}");
    let caps = captions(s);
    let texts: Vec<&str> = caps.iter().map(|c| c.0.as_str()).collect();
    assert_eq!(texts, vec!["A spricht", "B spricht", "B weiter", "C spricht"], "{caps:?}");
    assert!(close(caps[1].1, 4.0) && close(caps[1].2, 4.5), "the rest of the first: {caps:?}");
    assert!(close(caps[2].1, 4.6) && close(caps[3].1, 6.2), "{caps:?}");
    assert_eq!(at(s, v.title_b).map(|t| close(t.0, 5.0)), Some(true), "{:?}", at(s, v.title_b));
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 6.5)), Some(true), "{:?}", at(s, v.title_c));
    ok(s, "edit.undo", json!({}));

    // b twice as fast: 1.5 s; c and what hangs on it move up
    let undo = s.history.undo.len();
    ok(s, "shnyp.video.speed", json!({"clips": [v.b], "speed": 2.0}));
    assert_eq!(s.history.undo.len(), undo + 1);
    let m = main_clips(s);
    assert!(close(m[1].2, 5.5) && close(m[2].1, 5.5), "{m:?}");
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 6.0)), Some(true), "{:?}", at(s, v.title_c));
    assert_eq!(at(s, v.sfx).map(|t| close(t.0, 6.5)), Some(true), "{:?}", at(s, v.sfx));
    ok(s, "edit.undo", json!({}));

    // a split at 5.2 s with nothing selected cuts the main clip there only: the title over it
    // and the music stay whole
    s.state.selection.clear();
    let titles_before = titles(s).len();
    ok(s, "shnyp.video.split", json!({"seconds": 5.2}));
    assert_eq!(main_clips(s).len(), 4, "{:?}", main_clips(s));
    assert_eq!(titles(s).len(), titles_before, "no title cut: {:?}", titles(s));
    assert_eq!(music(s).len(), 1, "the music stays one clip");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn filmcraft_s_own_ripple_delete_ends_like_shnyp_s_delete() {
    if !have_ffmpeg() {
        return;
    }
    let mut v1 = video("native-shnyp");
    let mut v2 = video("native-fc");
    assert_eq!(shape(&mut v1.s), shape(&mut v2.s), "two videos made the same way are the same");
    ok(&mut v1.s, "shnyp.video.delete", json!({"clips": [v1.b]}));
    // FilmCraft's Ripple Delete of the same clip (with its sound, as its linked selection does)
    let undo = v2.s.history.undo.len();
    ok(&mut v2.s, "edit.rippleDelete", json!({"clips": [v2.b]}));
    assert_eq!(v2.s.history.undo.len(), undo + 1, "the following is part of FilmCraft's step");
    assert_eq!(shape(&mut v1.s), shape(&mut v2.s), "same video either way");
    // and one undo takes FilmCraft's edit and the following back together
    ok(&mut v2.s, "edit.undo", json!({}));
    assert_eq!(at(&v2.s, v2.title_b).map(|t| close(t.0, 5.0)), Some(true));
    assert_eq!(main_clips(&v2.s).len(), 3);
    let _ = std::fs::remove_dir_all(&v1.d);
    let _ = std::fs::remove_dir_all(&v2.d);
}

#[test]
fn with_the_magnet_off_gaps_stay_and_with_linkage_off_nothing_follows() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("switches");
    let s = &mut v.s;
    ok(s, "shnyp.video.magnet", json!({"on": false}));
    ok(s, "shnyp.video.delete", json!({"clips": [v.b]}));
    let m = main_clips(s);
    assert!(close(m[1].1, 7.0), "the gap stays: {m:?}");
    ok(s, "shnyp.video.magnet", json!({"on": true}));
    let m = main_clips(s);
    assert!(close(m[1].1, 4.0), "turning the magnet on closes the gap: {m:?}");
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 4.5)), Some(true), "{:?}", at(s, v.title_c));

    // linkage off: FilmCraft's own Ripple Delete, its title track sync-locked again (Premiere)
    ok(s, "shnyp.video.linkage", json!({"on": false}));
    let caps = captions(s);
    ok(s, "shnyp.video.delete", json!({"clips": [v.a]}));
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 0.5)), Some(true), "moved by the sync lock: {:?}", at(s, v.title_c));
    assert_eq!(captions(s), caps, "FilmCraft's ripple delete leaves captions where they are");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn a_split_made_with_filmcraft_keeps_captions_and_titles_on_the_right_half() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("razor");
    let s = &mut v.s;
    let caps = captions(s);
    let titles_before = titles(s);
    let undo = s.history.undo.len();
    // FilmCraft keeps the id on the left piece; b's captions and title are on the right
    ok(s, "timeline.razor", json!({"clip": v.b, "time": (4.2 * filmcraft_time::TICKS_PER_SECOND as f64) as i64}));
    assert_eq!(s.history.undo.len(), undo + 1, "only FilmCraft's step");
    assert_eq!(captions(s), caps, "every caption where it was");
    assert_eq!(titles(s), titles_before, "every title where it was");
    // the same with shnyp's split
    ok(s, "edit.undo", json!({}));
    s.state.selection.clear();
    ok(s, "shnyp.video.split", json!({"seconds": 4.2}));
    assert_eq!(captions(s), caps);
    assert_eq!(titles(s), titles_before);
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn an_insert_made_with_filmcraft_takes_the_right_half_s_title_along() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("native-insert");
    let s = &mut v.s;
    let n = v.d.join("n.mp4");
    clip(&n, 2, 330);
    let item = ok(s, "file.import", json!({"paths": [n.to_string_lossy()]}))["items"][0].as_u64().unwrap();
    let main = main_track::main_track_id(q(s)).unwrap();
    let undo = s.history.undo.len();
    // Premiere's insert into the middle of b (4–7 s) at 4.5 s: b is cut, its right half moves on
    ok(
        s,
        "timeline.place",
        json!({"item": item, "track": main.0, "time": (4.5 * filmcraft_time::TICKS_PER_SECOND as f64) as i64, "insert": true, "audio": false}),
    );
    assert_eq!(s.history.undo.len(), undo + 1, "only FilmCraft's step");
    // b's title (at 5 s, in its right half) stays over the same picture: now at 7 s
    assert_eq!(at(s, v.title_b).map(|t| close(t.0, 7.0)), Some(true), "{:?}", at(s, v.title_b));
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 9.5)), Some(true), "{:?}", at(s, v.title_c));
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn following_never_adds_a_step_and_redo_and_a_shared_watch_do_not_follow_twice() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("watch");
    let s = &mut v.s;
    let mut w = shnyp_video::reconcile::Watch::new(s);
    // moving a title with FilmCraft: nothing follows, nothing is added
    let undo = s.history.undo.len();
    let track = q(s).find_item(ClipId(v.title_c)).unwrap().0;
    let to = (8.0 * filmcraft_time::TICKS_PER_SECOND as f64) as i64;
    s.execute("timeline.move", json!({"moves": [{"clip": v.title_c, "track": track.0, "time": to}], "insert": false})).unwrap();
    assert!(!shnyp_video::reconcile::catch_up(s, &mut w));
    assert_eq!(s.history.undo.len(), undo + 1);
    // a ripple delete made in FilmCraft's window (not through shnyp): the window's watch follows it
    // in FilmCraft's step
    let undo = s.history.undo.len();
    s.execute("edit.rippleDelete", json!({"clips": [v.a]})).unwrap();
    assert!(shnyp_video::reconcile::catch_up(s, &mut w), "the children follow");
    assert_eq!(s.history.undo.len(), undo + 1, "no step of its own");
    let after = shape(s);
    // looking again, after shnyp's own command, after undo and redo: nothing more happens
    assert!(!shnyp_video::reconcile::catch_up(s, &mut w));
    shnyp_video::execute_with(s, &mut w, "shnyp.video.state", &json!({})).unwrap();
    s.undo();
    assert!(!shnyp_video::reconcile::catch_up(s, &mut w));
    s.redo();
    assert!(!shnyp_video::reconcile::catch_up(s, &mut w));
    assert_eq!(shape(s), after);
    assert_eq!(s.history.undo.len(), undo + 1);
    let _ = std::fs::remove_dir_all(&v.d);
}

/// A still picture.
fn still(path: &Path) {
    ffmpeg(&["-f", "lavfi", "-i", "testsrc2=size=360x640:rate=1:duration=1", "-frames:v", "1"], path);
}

/// Where the sound of each main clip starts and ends: (picture start, sound start).
fn sound_sync(s: &Session) -> Vec<(f64, f64)> {
    let q = q(s);
    main_track::main_clips(q)
        .iter()
        .filter_map(|m| {
            let p = main_track::partners(q, m.id)
                .into_iter()
                .find_map(|p| q.find_item(p).filter(|(t, _)| q.track(*t).is_some_and(|t| t.kind == filmcraft_project::TrackKind::Audio)))?;
            Some((sec(m.start), sec(p.1.start)))
        })
        .collect()
}

#[test]
fn a_clip_without_sound_never_leaves_the_later_sound_behind() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("silent");
    let (img, a, b) = (d.join("bild.png"), d.join("a.mp4"), d.join("b.mp4"));
    still(&img);
    clip(&a, 3, 440);
    clip(&b, 2, 550);
    let mut s = Session::default();
    ok(&mut s, "shnyp.video.new", json!({"format": "9:16"}));
    ok(&mut s, "shnyp.video.add", json!({"paths": [img.to_string_lossy(), a.to_string_lossy(), b.to_string_lossy()]}));
    let m = main_clips(&s);
    assert_eq!(m.len(), 3, "{m:?}");
    let in_sync = |s: &Session| sound_sync(s).iter().all(|(p, a)| close(*p, *a));
    assert!(in_sync(&s), "{:?}", sound_sync(&s));
    // the picture without sound gets shorter, faster, and goes: the sound after it moves along
    ok(&mut s, "shnyp.video.trim", json!({"clip": m[0].0, "edge": "out", "seconds": -1.0}));
    assert!(in_sync(&s), "after the trim: {:?}", sound_sync(&s));
    ok(&mut s, "shnyp.video.speed", json!({"clips": [m[0].0], "speed": 2.0}));
    assert!(in_sync(&s), "after the speed change: {:?}", sound_sync(&s));
    ok(&mut s, "shnyp.video.delete", json!({"clips": [m[0].0]}));
    assert!(in_sync(&s), "after the delete: {:?}", sound_sync(&s));
    assert!(close(main_clips(&s)[0].1, 0.0), "{:?}", main_clips(&s));
    // detached sound: the picture goes, its sound stays (that is why it was taken off)
    let first = main_clips(&s)[0].0;
    ok(&mut s, "shnyp.video.detachAudio", json!({"clips": [first]}));
    let sounds = |s: &Session| q(s).audio_tracks.iter().map(|t| t.items.len()).sum::<usize>();
    let before = sounds(&s);
    ok(&mut s, "shnyp.video.delete", json!({"clips": [first]}));
    assert_eq!(sounds(&s), before, "the taken-off sound stays");
    assert!(in_sync(&s), "the rest stays in sync: {:?}", sound_sync(&s));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn linked_selection_off_changes_nothing_in_shnyp_s_edits() {
    if !have_ffmpeg() {
        return;
    }
    let mut v1 = video("linked-on");
    let mut v2 = video("linked-off");
    ok(&mut v2.s, "sequence.linkedSelection", json!({"on": false}));
    for v in [&mut v1, &mut v2] {
        v.s.state.selection.clear();
        ok(&mut v.s, "shnyp.video.split", json!({"seconds": 5.5}));
        let m = main_clips(&v.s);
        ok(&mut v.s, "shnyp.video.delete", json!({"clips": [m[2].0]}));
        ok(&mut v.s, "shnyp.video.speed", json!({"clips": [m[0].0], "speed": 0.5}));
    }
    let strip = |s: &mut Session| {
        let mut v = shape(s);
        v["sequence"].as_object_mut().map(|o| o.remove("linkedSelection"));
        v
    };
    assert_eq!(strip(&mut v1.s), strip(&mut v2.s));
    assert!(sound_sync(&v1.s).iter().all(|(p, a)| close(*p, *a)), "{:?}", sound_sync(&v1.s));
    let _ = std::fs::remove_dir_all(&v1.d);
    let _ = std::fs::remove_dir_all(&v2.d);
}

#[test]
fn locked_tracks_keep_their_clips_and_do_not_block_the_main_track() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("locked");
    let s = &mut v.s;
    let title_track = q(s).find_item(ClipId(v.title_c)).unwrap().0;
    let caption_track = q(s).caption_tracks[0].id;
    s.edit_sequence("lock", |q, _, _| {
        q.track_mut(title_track).unwrap().locked = true;
        q.caption_track_mut(caption_track).unwrap().locked = true;
        Ok(())
    })
    .unwrap();
    let caps = captions(s);
    ok(s, "shnyp.video.delete", json!({"clips": [v.b]}));
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 7.5)), Some(true), "a locked title stays: {:?}", at(s, v.title_c));
    assert_eq!(captions(s), caps, "locked captions stay");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn an_end_card_stays_an_overlay_and_keeps_its_distance_to_the_end() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("end-card");
    let s = &mut v.s;
    let card = ok(s, "shnyp.video.text", json!({"text": "Ende", "seconds": 2.0, "at": 9.0}))["clip"].as_u64().unwrap();
    assert!(!main_track::main_clips(q(s)).iter().any(|m| m.id.0 == card), "a title never goes on the main track");
    ok(s, "shnyp.video.delete", json!({"clips": [v.a]}));
    assert_eq!(at(s, card).map(|t| close(t.0, 5.0)), Some(true), "right after the end: {:?}", at(s, card));
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn linkage_off_is_filmcraft_s_own_behaviour_again() {
    if !have_ffmpeg() {
        return;
    }
    let mut v1 = video("off-shnyp");
    let mut v2 = video("off-fc");
    let title_track = q(&v1.s).find_item(ClipId(v1.title_b)).unwrap().0;
    assert!(!q(&v1.s).track(title_track).unwrap().sync_lock, "linkage on: titles follow shnyp, not the sync lock");
    for v in [&mut v1, &mut v2] {
        ok(&mut v.s, "shnyp.video.linkage", json!({"on": false}));
    }
    assert!(q(&v1.s).track(title_track).unwrap().sync_lock, "linkage off gives the sync lock back");
    // shnyp's trim and delete now are FilmCraft's Ripple Trim and Ripple Delete (titles on the
    // sync-locked title track move by Premiere's rules)
    ok(&mut v1.s, "shnyp.video.trim", json!({"clip": v1.a, "edge": "out", "seconds": -0.5}));
    ok(&mut v1.s, "shnyp.video.delete", json!({"clips": [v1.a]}));
    let fc = |s: &mut Session, a: u64| {
        ok(s, "timeline.trim", json!({"clip": a, "edge": "out", "mode": "ripple", "delta": (-0.5 * filmcraft_time::TICKS_PER_SECOND as f64) as i64}));
        ok(s, "edit.rippleDelete", json!({"clips": [a]}));
    };
    fc(&mut v2.s, v2.a);
    assert_eq!(shape(&mut v1.s), shape(&mut v2.s));
    assert_eq!(at(&v1.s, v1.title_b).map(|t| close(t.0, 1.0)), Some(true), "moved by the sync lock: {:?}", at(&v1.s, v1.title_b));
    let _ = std::fs::remove_dir_all(&v1.d);
    let _ = std::fs::remove_dir_all(&v2.d);
}

#[test]
fn what_runs_over_many_clips_stays_when_the_clip_it_starts_on_goes() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("long");
    let s = &mut v.s;
    let logo = ok(s, "shnyp.video.text", json!({"text": "Logo", "seconds": 9.0, "at": 0.0}))["clip"].as_u64().unwrap();
    let vo = v.d.join("vo.m4a");
    sound(&vo, 8.0, 300);
    ok(s, "shnyp.video.add", json!({"paths": [vo.to_string_lossy()], "at": 0.5}));
    let voice = q(s).audio_tracks.iter().flat_map(|t| t.items.iter()).find(|it| close(sec(it.start), 0.5)).map(|it| it.id.0).unwrap();
    ok(s, "shnyp.video.delete", json!({"clips": [v.a]}));
    assert!(at(s, logo).is_some(), "the logo over the whole video stays");
    assert!(at(s, voice).is_some(), "the voice-over stays");
    assert!(at(s, v.title_b).is_some(), "b's title moved up with b");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn duplicating_a_main_clip_puts_the_copy_right_after_it() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("duplicate");
    let s = &mut v.s;
    let undo = s.history.undo.len();
    let r = ok(s, "shnyp.video.duplicate", json!({"clips": [v.b]}));
    assert_eq!(s.history.undo.len(), undo + 1);
    let copy = r["copies"][0].as_u64().unwrap();
    let m = main_clips(s);
    assert_eq!(m.iter().map(|c| c.0).collect::<Vec<_>>(), vec![v.a, v.b, copy, v.c], "{m:?}");
    assert!(close(m[2].1, 7.0) && close(m[3].1, 10.0), "{m:?}");
    assert_eq!(at(s, v.title_c).map(|t| close(t.0, 10.5)), Some(true), "{:?}", at(s, v.title_c));
    assert!(sound_sync(s).iter().all(|(p, a)| close(*p, *a)), "{:?}", sound_sync(s));
    // a title's copy goes right after it
    let r = ok(s, "shnyp.video.duplicate", json!({"clips": [v.title_b]}));
    let t = r["copies"][0].as_u64().unwrap();
    assert_eq!(at(s, t).map(|t| close(t.0, 6.0)), Some(true), "{:?}", at(s, t));
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn captions_over_a_voice_over_keep_their_place_like_titles() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("voice");
    let s = &mut v.s;
    let vo = v.d.join("vo.m4a");
    sound(&vo, 2.5, 300);
    ok(s, "shnyp.video.add", json!({"paths": [vo.to_string_lossy()], "at": 4.2}));
    // b's head trimmed: its own picture moves up 1 s, but the captions are the voice-over's
    ok(s, "shnyp.video.trim", json!({"clip": v.b, "edge": "in", "seconds": 1.0}));
    let caps = captions(s);
    assert!(caps.iter().any(|c| c.0 == "B spricht" && close(c.1, 4.5)), "{caps:?}");
    assert!(caps.iter().any(|c| c.0 == "B weiter" && close(c.1, 5.6)), "{caps:?}");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn a_sequence_shnyp_did_not_make_keeps_filmcraft_s_behaviour() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir("foreign");
    let (a, b) = (d.join("a.mp4"), d.join("b.mp4"));
    clip(&a, 3, 440);
    clip(&b, 2, 550);
    let mut s = Session::default();
    // made with FilmCraft's own commands: no shnyp marker
    let items: Vec<u64> = ok(&mut s, "file.import", json!({"paths": [a.to_string_lossy(), b.to_string_lossy()]}))["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    ok(&mut s, "file.newSequence", json!({"name": "Fremd", "fromItems": items}));
    let title = ok(
        &mut s,
        "graphics.newText",
        json!({"text": "B-Roll", "seconds": 1.0, "time": (3.5 * filmcraft_time::TICKS_PER_SECOND as f64) as i64, "track": 1}),
    )["clip"]
        .as_u64()
        .unwrap();
    assert_eq!(ok(&mut s, "shnyp.video.anchors", json!({}))["linkage"], false);
    let first = main_clips(&s)[0].0;
    // FilmCraft's Clear on V1 leaves the title above alone
    ok(&mut s, "edit.clear", json!({"clips": [first]}));
    assert!(at(&s, title).is_some(), "nothing above is deleted");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn filmcraft_s_clear_with_the_magnet_on_ends_like_shnyp_s_delete() {
    if !have_ffmpeg() {
        return;
    }
    let mut v1 = video("clear-shnyp");
    let mut v2 = video("clear-fc");
    ok(&mut v1.s, "shnyp.video.delete", json!({"clips": [v1.b]}));
    ok(&mut v2.s, "edit.clear", json!({"clips": [v2.b]}));
    assert_eq!(shape(&mut v1.s), shape(&mut v2.s));
    let _ = std::fs::remove_dir_all(&v1.d);
    let _ = std::fs::remove_dir_all(&v2.d);
}
