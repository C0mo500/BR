//! The picture in the frame: a filled background (blurred, a colour) on a track below the main
//! track that follows its main clip through delete, trim, speed, split, undo and a change of
//! format; Auto Reframe for pictures of another shape; transform and fit. Clips are made with
//! `ffmpeg` (skipped without it, unless BR_REQUIRE_FFMPEG is set).

use std::path::{Path, PathBuf};
use std::process::Command;

use br_video::{Session, main_track};
use filmcraft_project::ClipId;
use serde_json::{Value, json};

fn have_ffmpeg() -> bool {
    let found = Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    assert!(found || std::env::var_os("BR_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (BR_REQUIRE_FFMPEG is set)");
    found
}

/// A landscape clip with sound (640×360), `seconds` long.
fn clip(d: &Path, name: &str, seconds: u32) -> String {
    let p = d.join(name);
    let dur = seconds.to_string();
    let o = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size=640x360:rate=30:duration={dur}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={dur}"))
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&p)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    p.to_string_lossy().into_owned()
}

fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    br_video::execute(s, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
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

/// A 9:16 video of two landscape clips: a (4 s), b (3 s).
fn video(name: &str) -> Video {
    let d = std::env::temp_dir().join(format!("br-frame-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (a, b) = (clip(&d, "a.mp4", 4), clip(&d, "b.mp4", 3));
    let mut s = Session::default();
    ok(&mut s, "br.video.new", json!({"format": "9:16"}));
    ok(&mut s, "br.video.add", json!({"paths": [a, b]}));
    // nothing selected: commands without clips work on all of them
    ok(&mut s, "timeline.select", json!({"clips": []}));
    let m = main_track::main_clips(s.active_sequence().unwrap()).iter().map(|it| it.id.0).collect::<Vec<_>>();
    Video { a: m[0], b: m[1], s, d }
}

/// The backgrounds: (clip, start, end, link).
fn backgrounds(s: &Session) -> Vec<(u64, f64, f64, Option<u64>)> {
    let q = s.active_sequence().unwrap();
    let mut v: Vec<(u64, f64, f64, Option<u64>)> = q
        .video_tracks
        .iter()
        .flat_map(|t| &t.items)
        .filter(|it| main_track::is_background(it))
        .map(|it| (it.id.0, it.start.seconds(), it.end().seconds(), it.link))
        .collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v
}

fn link_of(s: &Session, c: u64) -> Option<u64> {
    s.active_sequence().unwrap().find_item(ClipId(c)).and_then(|(_, it)| it.link)
}

fn scale_of(s: &Session, c: u64) -> f64 {
    let q = s.active_sequence().unwrap();
    q.find_item(ClipId(c)).unwrap().1.effect("motion").unwrap().params["scale"].value.as_f64().unwrap()
}

/// How much of the top tenth of the picture at `secs` is not black (0…1).
fn top_lit(s: &Session, secs: f64) -> f64 {
    let (w, h, px) = br_video::frame(s, Some(secs), 192).unwrap();
    let rows = (h / 10) as usize;
    let mut lit = 0;
    for y in 0..rows {
        for x in 0..w as usize {
            let i = (y * w as usize + x) * 4;
            if px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32 > 40 {
                lit += 1;
            }
        }
    }
    lit as f64 / (rows * w as usize) as f64
}

#[test]
fn a_blurred_background_fills_the_frame_and_follows_its_clip() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("blur");
    let s = &mut v.s;
    assert!(top_lit(s, 1.0) < 0.05, "a landscape clip in a 9:16 frame leaves the top black");
    let main_before = main_track::main_track_id(s.active_sequence().unwrap());
    let undo = s.history.undo.len();
    ok(s, "br.video.background", json!({"mode": "blur", "amount": 60}));
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let q = s.active_sequence().unwrap();
    assert_eq!(main_track::main_track_id(q), main_before, "the main track stays the main track");
    let bg_track = q.video_tracks.iter().position(main_track::is_background_track).expect("a background track");
    let main_at = q.video_tracks.iter().position(|t| Some(t.id) == main_before).unwrap();
    assert!(bg_track < main_at, "below the main track");
    let bgs = backgrounds(s);
    assert_eq!(bgs.len(), 2);
    assert!(close(bgs[0].1, 0.0) && close(bgs[0].2, 4.0) && close(bgs[1].1, 4.0) && close(bgs[1].2, 7.0), "{bgs:?}");
    assert_eq!(bgs[0].3, link_of(s, v.a), "linked with its clip");
    assert!(scale_of(s, bgs[0].0) > scale_of(s, v.a) * 2.0, "it fills, the clip fits: {} vs {}", scale_of(s, bgs[0].0), scale_of(s, v.a));
    assert!(top_lit(s, 1.0) > 0.9, "the frame is filled");
    // the inspector shows it
    let i = ok(s, "br.video.inspect", json!({"clip": v.a}));
    let rows: Vec<&Value> = i["sections"].as_array().unwrap().iter().flat_map(|x| x["rows"].as_array().unwrap()).collect();
    assert_eq!(rows.iter().find(|r| r["key"] == "background.mode").unwrap()["value"], "blur");
    assert!(rows.iter().any(|r| r["key"] == "transform.reframe"), "a landscape clip can reframe");
    // trimmed, sped up, split: the background goes along
    ok(s, "br.video.trim", json!({"clip": v.a, "edge": "out", "seconds": -1.0}));
    let bgs = backgrounds(s);
    assert!(close(bgs[0].2, 3.0) && close(bgs[1].1, 3.0), "{bgs:?}");
    ok(s, "br.video.speed", json!({"clips": [v.b], "speed": 2.0}));
    let bgs = backgrounds(s);
    assert!(close(bgs[1].2, 4.5), "{bgs:?}");
    ok(s, "br.video.split", json!({"clip": v.a, "seconds": 1.0}));
    assert_eq!(backgrounds(s).len(), 3, "split with it: {:?}", backgrounds(s));
    // a new clip goes onto the main track, not onto the backgrounds
    let c = clip(&v.d, "c.mp4", 2);
    ok(s, "br.video.add", json!({"paths": [c]}));
    let q = s.active_sequence().unwrap();
    assert_eq!(main_track::main_clips(q).len(), 4);
    assert_eq!(backgrounds(s).len(), 3);
    // deleting a main clip takes its background along; one undo brings both back
    let first = main_track::main_clips(s.active_sequence().unwrap())[0].id.0;
    ok(s, "br.video.delete", json!({"clips": [first]}));
    assert_eq!(backgrounds(s).len(), 2);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(backgrounds(s).len(), 3);
    // separating picture and sound keeps the background with its picture
    let a_first = main_track::main_clips(s.active_sequence().unwrap())[0].id.0;
    ok(s, "br.video.detachAudio", json!({"clips": [a_first]}));
    let l = link_of(s, a_first);
    assert!(l.is_some() && backgrounds(s).iter().any(|b| b.3 == l), "still linked to its background");
    let q = s.active_sequence().unwrap();
    assert!(q.audio_tracks.iter().flat_map(|t| &t.items).all(|it| it.link != l), "the sound is apart");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn colour_and_none_and_the_format_keeps_the_frame_filled() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("colour");
    let s = &mut v.s;
    ok(s, "br.video.background", json!({"mode": "color", "color": "#ff0000", "clips": [v.b]}));
    let bgs = backgrounds(s);
    assert_eq!(bgs.len(), 1);
    assert_eq!(bgs[0].3, link_of(s, v.b));
    let (w, _, px) = br_video::frame(s, Some(5.0), 192).unwrap();
    let _ = w;
    assert!(px[0] > 200 && px[1] < 40 && px[2] < 40, "red at the top: {:?}", &px[..4]);
    // another format: the background fills the new frame, the clips fit it
    ok(s, "br.video.background", json!({"mode": "blur", "clips": [v.a]}));
    ok(s, "br.video.format", json!({"format": "1:1"}));
    let blur_bg = backgrounds(s)[0].0;
    assert!(scale_of(s, blur_bg) > scale_of(s, v.a) * 1.5, "fills 1:1: {} vs {}", scale_of(s, blur_bg), scale_of(s, v.a));
    assert!(top_lit(s, 1.0) > 0.9);
    // none takes them away, one step
    let undo = s.history.undo.len();
    ok(s, "br.video.background", json!({"mode": "none", "clips": [v.a, v.b]}));
    assert!(backgrounds(s).is_empty());
    assert_eq!(s.history.undo.len(), undo + 1);
    assert!(br_video::execute(s, "br.video.background", &json!({"mode": "color", "color": "rot"})).is_err());
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn reframe_fills_the_frame_and_follows_a_change_of_format() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("reframe");
    let s = &mut v.s;
    let aspect = |s: &Session, c: u64| {
        let q = s.active_sequence().unwrap();
        let it = q.find_item(ClipId(c)).unwrap().1;
        it.effect("auto_reframe").map(|e| e.params["aspect"].value.as_f64().unwrap() as u32)
    };
    let fitted = scale_of(s, v.a);
    ok(s, "br.video.reframe", json!({}));
    assert_eq!(aspect(s, v.a), Some(1), "Vertical 9:16 (never `Sequence`)");
    assert!(scale_of(s, v.a) > fitted * 2.0, "it fills");
    assert!(top_lit(s, 1.0) > 0.9);
    ok(s, "br.video.format", json!({"format": "1:1"}));
    assert_eq!(aspect(s, v.a), Some(2), "Square 1:1 after the change");
    // fit: reframing ends
    ok(s, "br.video.reframe", json!({"on": false}));
    assert_eq!(aspect(s, v.a), None);
    // a format change with reframe
    ok(s, "br.video.format", json!({"format": "9:16", "fit": "reframe"}));
    assert_eq!(aspect(s, v.b), Some(1));
    // the same shape has nothing to reframe
    ok(s, "br.video.format", json!({"format": "16:9", "fit": "reframe"}));
    assert_eq!(aspect(s, v.a), None, "640×360 is 16:9");
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn transform_and_fit_are_one_step_each() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("transform");
    let s = &mut v.s;
    let undo = s.history.undo.len();
    ok(s, "br.video.transform", json!({"clips": [v.a], "position": [300, 500], "scale": 120, "rotation": 15, "opacity": 80}));
    assert_eq!(s.history.undo.len(), undo + 1);
    let q = s.active_sequence().unwrap();
    let it = q.find_item(ClipId(v.a)).unwrap().1;
    let m = it.effect("motion").unwrap();
    assert_eq!(m.params["scale"].value.as_f64(), Some(120.0));
    assert_eq!(m.params["rotation"].value.as_f64(), Some(15.0));
    let pos = m.params["position"].value.as_vec2().unwrap();
    assert!(close(pos.x, 300.0) && close(pos.y, 500.0));
    assert_eq!(it.effect("opacity").unwrap().params["opacity"].value.as_f64(), Some(80.0));
    for x in [130, 140, 150] {
        ok(s, "br.video.transform", json!({"clips": [v.a], "scale": x, "gesture": "drag1"}));
    }
    assert_eq!(s.history.undo.len(), undo + 2, "a drag is one step");
    ok(s, "br.video.fit", json!({"clips": [v.a], "mode": "fill"}));
    assert!(scale_of(s, v.a) > 500.0);
    ok(s, "br.video.fit", json!({"clips": [v.a], "mode": "reset"}));
    assert_eq!(scale_of(s, v.a), 100.0);
    assert!(br_video::execute(s, "br.video.transform", &json!({"clips": [v.a]})).is_err());
    let _ = std::fs::remove_dir_all(&v.d);
}

#[test]
fn a_title_stays_in_the_frame_when_the_format_changes() {
    if !have_ffmpeg() {
        return;
    }
    let mut v = video("title");
    let s = &mut v.s;
    let t = ok(s, "br.video.text", json!({"text": "Hallo Welt", "seconds": 2.0, "at": 0.5}))["clip"].as_u64().unwrap();
    for f in ["16:9", "1:1", "4:5", "9:16"] {
        ok(s, "br.video.format", json!({"format": f}));
        let g = ok(s, "graphics.list", json!({"clip": t}));
        let (w, h) = (g["canvas"][0].as_f64().unwrap(), g["canvas"][1].as_f64().unwrap());
        let q = s.active_sequence().unwrap();
        assert_eq!((w as u32, h as u32), (q.settings.width, q.settings.height), "{f}: the title's canvas is the frame: {g}");
        let quad = &g["layers"][0]["quad"];
        let pts: Vec<(f64, f64)> = quad.as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
        assert!(pts.iter().all(|(x, y)| *x >= 0.0 && *x <= w && *y >= 0.0 && *y <= h), "{f}: inside the frame: {pts:?} in {w}×{h}");
        let cx = pts.iter().map(|p| p.0).sum::<f64>() / pts.len() as f64;
        assert!((cx - w / 2.0).abs() < w * 0.1, "{f}: still in the middle: {cx} of {w}");
    }
    let _ = std::fs::remove_dir_all(&v.d);
}
