//! BR's video commands end to end on files like a phone makes them, written by `ffmpeg` (the
//! tests are skipped where it is missing, as FilmCraft's own workflow tests do): edit a 9:16
//! video, export it, and check the file with `ffprobe`.

use std::path::{Path, PathBuf};
use std::process::Command;

use br_video::Session;
use serde_json::{Value, json};

fn have(tool: &str) -> bool {
    Command::new(tool).arg("-version").output().is_ok_and(|o| o.status.success())
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("br-video-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ffmpeg(args: &[&str], out: &Path) {
    let o = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(out).output().unwrap();
    assert!(o.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

/// A phone clip: 1080×1920 H.264 at a variable frame rate (every 7th frame dropped, as phones do
/// under load), AAC sound.
fn phone_h264(path: &Path, seconds: u32) {
    let d = seconds.to_string();
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=1080x1920:rate=30:duration={d}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={d}"),
            "-vf",
            "select='not(eq(mod(n\\,7)\\,3))'",
            "-fps_mode",
            "vfr",
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

/// An iPhone-style clip: HEVC Main 10, HLG, recorded sideways and turned upright by its rotation
/// metadata, in a MOV.
fn iphone_hevc(path: &Path) {
    ffmpeg(
        &[
            "-display_rotation",
            "90",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=30:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=330:duration=2",
            "-c:v",
            "libx265",
            "-pix_fmt",
            "yuv420p10le",
            "-tag:v",
            "hvc1",
            "-x265-params",
            "log-level=error:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc",
            "-c:a",
            "aac",
            "-shortest",
        ],
        path,
    );
}

fn music(path: &Path) {
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=220:duration=12", "-c:a", "aac"], path);
}

fn probe(path: &Path) -> Value {
    let o = Command::new("ffprobe").args(["-v", "error", "-show_streams", "-show_format", "-of", "json"]).arg(path).output().unwrap();
    assert!(o.status.success(), "ffprobe: {}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

fn stream<'a>(p: &'a Value, kind: &str) -> &'a Value {
    p["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == kind).unwrap_or_else(|| panic!("no {kind} stream: {p}"))
}

fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    br_video::execute(s, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn a_vertical_video_from_phone_clips_with_title_captions_and_music() {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("ffmpeg/ffprobe missing: skipped");
        return;
    }
    let d = dir("vertical");
    let (a, b, m) = (d.join("a.mp4"), d.join("b.mp4"), d.join("music.m4a"));
    phone_h264(&a, 4);
    phone_h264(&b, 3);
    music(&m);
    let srt = d.join("captions.srt");
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:02,000\nHallo zusammen\n\n2\n00:00:02,500 --> 00:00:04,000\nWillkommen\n").unwrap();

    let mut s = Session::default();
    let st = ok(&mut s, "br.video.new", json!({"format": "9:16"}));
    assert_eq!((st["width"].as_u64(), st["height"].as_u64(), st["format"].as_str()), (Some(1080), Some(1920), Some("9:16")), "{st}");

    let undo0 = s.history.undo.len();
    let st = ok(&mut s, "br.video.add", json!({"paths": [a.to_string_lossy(), b.to_string_lossy()]}));
    assert_eq!(s.history.undo.len(), undo0 + 1, "adding two clips is one undo step");
    assert!((st["seconds"].as_f64().unwrap() - 7.0).abs() < 0.2, "{st}");

    // cut at 2 s and ripple-delete the second piece of the first clip: 2 s shorter
    ok(&mut s, "br.video.split", json!({"seconds": 2.0}));
    ok(&mut s, "playhead.set", json!({"seconds": 2.5}));
    let seq = s.execute("sequence.inspect", json!({})).unwrap();
    let clip = seq["video"][0]["items"][1]["clip"].as_u64().unwrap_or_else(|| panic!("second clip on the main track: {seq}"));
    ok(&mut s, "br.video.delete", json!({"clips": [clip]}));
    let st = ok(&mut s, "br.video.state", json!({}));
    assert!((st["seconds"].as_f64().unwrap() - 5.0).abs() < 0.2, "{st}");

    ok(&mut s, "br.video.text", json!({"text": "Mein Video", "seconds": 3.0, "at": 0.0}));
    ok(&mut s, "br.video.captions", json!({"path": srt.to_string_lossy()}));
    let undo1 = s.history.undo.len();
    ok(&mut s, "br.video.music", json!({"path": m.to_string_lossy()}));
    assert_eq!(s.history.undo.len(), undo1 + 1, "music with ducking is one undo step");

    let out = d.join("tiktok.mp4");
    let r = ok(&mut s, "br.video.export", json!({"preset": "social", "path": out.to_string_lossy(), "wait": true}));
    assert!(out.exists(), "export wrote nothing: {r}");
    let p = probe(&out);
    let (v, au) = (stream(&p, "video"), stream(&p, "audio"));
    assert_eq!((v["codec_name"].as_str(), v["width"].as_u64(), v["height"].as_u64()), (Some("h264"), Some(1080), Some(1920)), "{p}");
    assert_eq!(au["codec_name"].as_str(), Some("aac"));
    let dur: f64 = p["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((dur - 5.0).abs() < 0.1, "duration {dur}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_iphone_hevc_clip_is_imported_upright_and_exported_completely() {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("ffmpeg/ffprobe missing: skipped");
        return;
    }
    let d = dir("hevc");
    let clip = d.join("IMG_0001.MOV");
    iphone_hevc(&clip);
    let mut s = Session::default();
    // no video yet: the first clip's shape makes one (upright: 1080 wide, 1920 high)
    let st = ok(&mut s, "br.video.add", json!({"paths": [clip.to_string_lossy()]}));
    assert_eq!((st["width"].as_u64(), st["height"].as_u64()), (Some(1080), Some(1920)), "rotation metadata honoured: {st}");
    let out = d.join("out.mp4");
    ok(&mut s, "br.video.export", json!({"preset": "video", "path": out.to_string_lossy(), "wait": true}));
    let p = probe(&out);
    let dur: f64 = p["format"]["duration"].as_str().unwrap().parse().unwrap();
    // FilmCraft #480: on Linux an export of iPhone HEVC stopped getting frames
    assert!((dur - 2.0).abs() < 0.1, "exported completely: {dur} s");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn mistakes_are_reported_and_change_nothing() {
    let mut s = Session::default();
    assert!(br_video::execute(&mut s, "br.video.new", &json!({"format": "3:2"})).unwrap_err().contains("unknown format"));
    assert!(br_video::execute(&mut s, "br.video.add", &json!({})).unwrap_err().contains("paths"));
    assert!(br_video::execute(&mut s, "br.video.export", &json!({"preset": "nope"})).is_err());
    let undo = s.history.undo.len();
    assert!(br_video::execute(&mut s, "br.video.add", &json!({"paths": ["/nonexistent/x.mp4"]})).is_err());
    assert_eq!(s.history.undo.len(), undo, "a failed add leaves no step behind");
    assert_eq!(br_video::state(&mut s).unwrap(), json!({"video": null}));
}
