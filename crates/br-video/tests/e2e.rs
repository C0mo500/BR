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

#[test]
fn a_new_shape_fits_the_clips_and_dissolves_go_only_between_clips() {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("ffmpeg/ffprobe missing: skipped");
        return;
    }
    let d = dir("format");
    let (a, b) = (d.join("a.mp4"), d.join("b.mp4"));
    phone_h264(&a, 3);
    phone_h264(&b, 2);
    let mut s = Session::default();
    ok(&mut s, "br.video.add", json!({"paths": [a.to_string_lossy(), b.to_string_lossy()]}));

    let undo = s.history.undo.len();
    let st = ok(&mut s, "br.video.format", json!({"format": "16:9"}));
    assert_eq!((st["format"].as_str(), st["width"].as_u64(), st["height"].as_u64()), (Some("16:9"), Some(1920), Some(1080)), "{st}");
    assert_eq!(s.history.undo.len(), undo + 1, "a new shape is one undo step");

    let undo = s.history.undo.len();
    let r = ok(&mut s, "br.video.transition", json!({}));
    assert_eq!(r["transitions"], 1, "two clips have one cut between them: {r}");
    assert_eq!(s.history.undo.len(), undo + 1);
    let seq = s.execute("sequence.inspect", json!({})).unwrap();
    let tr = &seq["video"][0]["transitions"];
    assert_eq!(tr.as_array().map(Vec::len), Some(1), "{seq}");
    assert!(tr[0]["from"].is_u64() && tr[0]["to"].is_u64(), "a dissolve between two clips, not from black: {tr}");
    assert!(br_video::execute(&mut s, "br.video.transition", &json!({})).is_err(), "the cut has its transition already");

    // a look replaces the one before; without a selection it goes on every clip
    s.state.selection.clear();
    let r = ok(&mut s, "br.video.filter", json!({"preset": "Golden Dusk"}));
    assert_eq!(r["clips"], 2, "no selection: every clip: {r}");
    let r = ok(&mut s, "br.video.filter", json!({"preset": "Faded Print"}));
    assert_eq!(r["replaced"], 2, "{r}");
    let lumetri = |s: &mut Session| {
        let seq = s.execute("sequence.inspect", json!({})).unwrap();
        seq["video"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["effects"].as_array().unwrap().iter().filter(|e| e["effect"] == "lumetri").count())
            .collect::<Vec<_>>()
    };
    let before = lumetri(&mut s);
    ok(&mut s, "br.video.filter", json!({"preset": "none"}));
    let after = lumetri(&mut s);
    assert!(after.iter().zip(&before).all(|(a, b)| a + 1 == *b), "one look less per clip: {before:?} → {after:?}");

    let out = d.join("wide.mp4");
    ok(&mut s, "br.video.export", json!({"preset": "video", "path": out.to_string_lossy(), "wait": true}));
    let p = probe(&out);
    let v = stream(&p, "video");
    assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(1920), Some(1080)), "{p}");
    let _ = std::fs::remove_dir_all(&d);
}

fn words(list: &[(&str, f64, f64)]) -> filmcraft_project::Transcript {
    let tick = |s: f64| filmcraft_time::Tick::from_seconds_f64(s);
    filmcraft_project::Transcript {
        language: "de".into(),
        words: list.iter().map(|(t, a, b)| filmcraft_project::transcript::Word::new(*t, tick(*a), tick(*b))).collect(),
        ..Default::default()
    }
}

#[test]
fn automatic_captions_come_from_the_speech_as_one_step() {
    if !have("ffmpeg") {
        eprintln!("ffmpeg missing: skipped");
        return;
    }
    let d = dir("captions");
    let a = d.join("talk.mp4");
    phone_h264(&a, 4);
    let mut s = Session::default();
    let opts = br_video::speech::CaptionOptions::default();
    assert!(br_video::speech::start(&s, &opts).err().unwrap().contains("noch kein Video"));
    ok(&mut s, "br.video.add", json!({"paths": [a.to_string_lossy()]}));
    s.transcriber = Some(std::sync::Arc::new(filmcraft_speech::FixedTranscriber {
        transcript: words(&[("Hallo", 0.5, 0.9), ("zusammen,", 0.9, 1.4), ("willkommen", 2.0, 2.6), ("hier.", 2.6, 3.0)]),
        id: "test".into(),
    }));

    // in the background, with progress, then into the project
    let undo = s.history.undo.len();
    let job = br_video::speech::start(&s, &opts).unwrap();
    let seq = job.sequence;
    let t0 = std::time::Instant::now();
    let done = loop {
        if let Some(r) = job.poll() {
            break r.unwrap();
        }
        assert!(t0.elapsed().as_secs() < 60, "recognition never finished: {:?}", job.progress());
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let r = br_video::speech::apply(&mut s, seq, done).unwrap();
    assert!(r["captions"].as_u64().unwrap() >= 1, "{r}");
    assert_eq!(s.history.undo.len(), undo + 1, "transcript and captions are one undo step");
    assert_eq!(s.history.undo.last().map(|u| u.0.as_str()), Some("Auto-Untertitel"));
    let q = s.active_sequence().unwrap();
    assert_eq!(q.caption_tracks.len(), 1);
    let text: Vec<String> = q.caption_tracks[0].captions.iter().map(|c| c.text.clone()).collect();
    assert!(text.join(" ").contains("Hallo zusammen,"), "{text:?}");

    // the same through the command, as the CLI and MCP run it
    ok(&mut s, "br.video.autoCaptions", json!({"language": "de"}));
    assert_eq!(s.active_sequence().unwrap().caption_tracks.len(), 2);
    let _ = std::fs::remove_dir_all(&d);
}

/// A recogniser that only stops when it is cancelled.
struct Endless;

impl filmcraft_speech::Transcriber for Endless {
    fn id(&self) -> String {
        "endless".into()
    }
    fn transcribe(
        &self,
        _: &[f32],
        _: &filmcraft_speech::Options,
        progress: filmcraft_speech::ProgressFn,
    ) -> Result<filmcraft_project::Transcript, filmcraft_speech::SpeechError> {
        loop {
            if !progress(0.3, "") {
                return Err(filmcraft_speech::SpeechError::Cancelled);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

#[test]
fn automatic_captions_can_be_cancelled_and_need_sound() {
    if !have("ffmpeg") {
        eprintln!("ffmpeg missing: skipped");
        return;
    }
    let d = dir("cancel");
    let (talk, mute) = (d.join("talk.mp4"), d.join("mute.mp4"));
    phone_h264(&talk, 2);
    ffmpeg(&["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=2", "-c:v", "libx264", "-pix_fmt", "yuv420p"], &mute);

    let mut s = Session::default();
    ok(&mut s, "br.video.add", json!({"paths": [mute.to_string_lossy()]}));
    s.transcriber = Some(std::sync::Arc::new(Endless));
    let e = br_video::speech::start(&s, &Default::default()).err().unwrap();
    assert!(e.contains("keinen Ton"), "{e}");

    ok(&mut s, "br.video.add", json!({"paths": [talk.to_string_lossy()]}));
    let undo = s.history.undo.len();
    let job = br_video::speech::start(&s, &Default::default()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    job.cancel();
    assert_eq!(job.wait().unwrap_err(), "abgebrochen");
    assert_eq!(s.history.undo.len(), undo, "a cancelled recognition changes nothing");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn files_are_told_apart_by_kind() {
    for (f, video) in [
        ("/x/IMG_0001.MOV", true),
        ("clip.mp4", true),
        ("song.M4A", true),
        ("cut.fcproj", true),
        ("subs.srt", true),
        ("photo.jpg", false),
        ("raw.DNG", false),
        ("notes.txt", false),
    ] {
        assert_eq!(br_video::is_video_file(f), video, "{f}");
    }
    assert_eq!(br_video::file_kind("a.webm"), Some("video"));
    assert_eq!(br_video::file_kind("b.heic"), None);
}
