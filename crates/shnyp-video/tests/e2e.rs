//! shnyp's video commands end to end on files like a phone makes them, written by `ffmpeg` (the
//! tests are skipped where it is missing, as FilmCraft's own workflow tests do): edit a 9:16
//! video, export it, and check the file with `ffprobe`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use shnyp_video::Session;

fn have(tool: &str) -> bool {
    let found = Command::new(tool).arg("-version").output().is_ok_and(|o| o.status.success());
    // in CI (SHNYP_REQUIRE_FFMPEG) the video tests must run, not skip
    assert!(found || std::env::var_os("SHNYP_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (SHNYP_REQUIRE_FFMPEG is set)");
    found
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("shnyp-video-{name}-{}", std::process::id()));
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
    shnyp_video::execute(s, id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
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
    let st = ok(&mut s, "shnyp.video.new", json!({"format": "9:16"}));
    assert_eq!((st["width"].as_u64(), st["height"].as_u64(), st["format"].as_str()), (Some(1080), Some(1920), Some("9:16")), "{st}");

    let undo0 = s.history.undo.len();
    let st = ok(&mut s, "shnyp.video.add", json!({"paths": [a.to_string_lossy(), b.to_string_lossy()]}));
    assert_eq!(s.history.undo.len(), undo0 + 1, "adding two clips is one undo step");
    assert!((st["seconds"].as_f64().unwrap() - 7.0).abs() < 0.2, "{st}");

    // cut at 2 s and ripple-delete the second piece of the first clip: 2 s shorter
    ok(&mut s, "shnyp.video.split", json!({"seconds": 2.0}));
    ok(&mut s, "playhead.set", json!({"seconds": 2.5}));
    let seq = s.execute("sequence.inspect", json!({})).unwrap();
    let clip = seq["video"][0]["items"][1]["clip"].as_u64().unwrap_or_else(|| panic!("second clip on the main track: {seq}"));
    ok(&mut s, "shnyp.video.delete", json!({"clips": [clip]}));
    let st = ok(&mut s, "shnyp.video.state", json!({}));
    assert!((st["seconds"].as_f64().unwrap() - 5.0).abs() < 0.2, "{st}");

    ok(&mut s, "shnyp.video.text", json!({"text": "Mein Video", "seconds": 3.0, "at": 0.0}));
    ok(&mut s, "shnyp.video.captions", json!({"path": srt.to_string_lossy()}));
    let undo1 = s.history.undo.len();
    ok(&mut s, "shnyp.video.music", json!({"path": m.to_string_lossy()}));
    assert_eq!(s.history.undo.len(), undo1 + 1, "music with ducking is one undo step");
    // the music is on its own track, as long as the video, and lowered where the clips have sound
    let q = s.active_sequence().unwrap();
    let music = q
        .audio_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .find(|it| it.essential.as_ref().is_some_and(|e| e.kind == filmcraft_project::AudioType::Music))
        .expect("a music clip");
    let picture_end = q.video_tracks[0].items.iter().map(|i| i.end()).max().unwrap();
    assert!(music.end() <= picture_end, "never longer than the picture");
    let keys = music.effect("volume").and_then(|v| v.params.get("level")).map_or(0, |p| p.keyframes.len());
    assert!(keys >= 2, "ducked under the video's sound: {keys} volume keyframes");

    let out = d.join("tiktok.mp4");
    let r = ok(&mut s, "shnyp.video.export", json!({"preset": "social", "path": out.to_string_lossy(), "wait": true}));
    assert!(out.exists(), "export wrote nothing: {r}");
    assert!(!shnyp_video::part_path(&out).exists(), "the hidden part file became the export");
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
    let st = ok(&mut s, "shnyp.video.add", json!({"paths": [clip.to_string_lossy()]}));
    assert_eq!((st["width"].as_u64(), st["height"].as_u64()), (Some(1080), Some(1920)), "rotation metadata honoured: {st}");
    let out = d.join("out.mp4");
    ok(&mut s, "shnyp.video.export", json!({"preset": "video", "path": out.to_string_lossy(), "wait": true}));
    let p = probe(&out);
    let dur: f64 = p["format"]["duration"].as_str().unwrap().parse().unwrap();
    // FilmCraft #480: on Linux an export of iPhone HEVC stopped getting frames
    assert!((dur - 2.0).abs() < 0.1, "exported completely: {dur} s");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn mistakes_are_reported_and_change_nothing() {
    let mut s = Session::default();
    assert!(shnyp_video::execute(&mut s, "shnyp.video.new", &json!({"format": "3:2"})).unwrap_err().contains("unknown format"));
    assert!(shnyp_video::execute(&mut s, "shnyp.video.add", &json!({})).unwrap_err().contains("paths"));
    assert!(shnyp_video::execute(&mut s, "shnyp.video.export", &json!({"preset": "nope"})).is_err());
    let undo = s.history.undo.len();
    assert!(shnyp_video::execute(&mut s, "shnyp.video.add", &json!({"paths": ["/nonexistent/x.mp4"]})).is_err());
    assert_eq!(s.history.undo.len(), undo, "a failed add leaves no step behind");
    assert_eq!(shnyp_video::state(&mut s).unwrap(), json!({"video": null}));
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
    ok(&mut s, "shnyp.video.add", json!({"paths": [a.to_string_lossy(), b.to_string_lossy()]}));

    let undo = s.history.undo.len();
    let st = ok(&mut s, "shnyp.video.format", json!({"format": "16:9"}));
    assert_eq!((st["format"].as_str(), st["width"].as_u64(), st["height"].as_u64()), (Some("16:9"), Some(1920), Some(1080)), "{st}");
    assert_eq!(s.history.undo.len(), undo + 1, "a new shape is one undo step");

    let undo = s.history.undo.len();
    let r = ok(&mut s, "shnyp.video.transition", json!({}));
    assert_eq!(r["transitions"], 1, "two clips have one cut between them: {r}");
    assert_eq!(s.history.undo.len(), undo + 1);
    let seq = s.execute("sequence.inspect", json!({})).unwrap();
    let tr = &seq["video"][0]["transitions"];
    assert_eq!(tr.as_array().map(Vec::len), Some(1), "{seq}");
    assert!(tr[0]["from"].is_u64() && tr[0]["to"].is_u64(), "a dissolve between two clips, not from black: {tr}");
    assert!(shnyp_video::execute(&mut s, "shnyp.video.transition", &json!({})).is_err(), "the cut has its transition already");

    // a look replaces the one before; without a selection it goes on every clip
    s.state.selection.clear();
    let r = ok(&mut s, "shnyp.video.filter", json!({"preset": "Golden Dusk"}));
    assert_eq!(r["clips"], 2, "no selection: every clip: {r}");
    let r = ok(&mut s, "shnyp.video.filter", json!({"preset": "Faded Print"}));
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
    ok(&mut s, "shnyp.video.filter", json!({"preset": "none"}));
    let after = lumetri(&mut s);
    assert!(after.iter().zip(&before).all(|(a, b)| a + 1 == *b), "one look less per clip: {before:?} → {after:?}");

    let out = d.join("wide.mp4");
    ok(&mut s, "shnyp.video.export", json!({"preset": "video", "path": out.to_string_lossy(), "wait": true}));
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
    let opts = shnyp_video::speech::CaptionOptions::default();
    assert!(shnyp_video::speech::start(&s, &opts).err().unwrap().contains("noch kein Video"));
    ok(&mut s, "shnyp.video.add", json!({"paths": [a.to_string_lossy()]}));
    s.transcriber = Some(std::sync::Arc::new(filmcraft_speech::FixedTranscriber {
        transcript: words(&[("Hallo", 0.5, 0.9), ("zusammen,", 0.9, 1.4), ("willkommen", 2.0, 2.6), ("hier.", 2.6, 3.0)]),
        id: "test".into(),
    }));

    // in the background, with progress, then into the project
    let undo = s.history.undo.len();
    let job = shnyp_video::speech::start(&s, &opts).unwrap();
    let seq = job.sequence;
    let t0 = std::time::Instant::now();
    let done = loop {
        if let Some(r) = job.poll() {
            break r.unwrap();
        }
        assert!(t0.elapsed().as_secs() < 60, "recognition never finished: {:?}", job.progress());
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let r = shnyp_video::speech::apply(&mut s, seq, done).unwrap();
    assert!(r["captions"].as_u64().unwrap() >= 1, "{r}");
    assert_eq!(s.history.undo.len(), undo + 1, "transcript and captions are one undo step");
    assert_eq!(s.history.undo.last().map(|u| u.0.as_str()), Some("Auto-Untertitel"));
    let q = s.active_sequence().unwrap();
    assert_eq!(q.caption_tracks.len(), 1);
    let text: Vec<String> = q.caption_tracks[0].captions.iter().map(|c| c.text.clone()).collect();
    assert!(text.join(" ").contains("Hallo zusammen,"), "{text:?}");

    // the same through the command, as the CLI and MCP run it: it replaces the first run's track
    ok(&mut s, "shnyp.video.autoCaptions", json!({"language": "de"}));
    assert_eq!(s.active_sequence().unwrap().caption_tracks.len(), 1, "one track of automatic captions");
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
    ok(&mut s, "shnyp.video.add", json!({"paths": [mute.to_string_lossy()]}));
    s.transcriber = Some(std::sync::Arc::new(Endless));
    let e = shnyp_video::speech::start(&s, &Default::default()).err().unwrap();
    assert!(e.contains("keinen Ton"), "{e}");

    ok(&mut s, "shnyp.video.add", json!({"paths": [talk.to_string_lossy()]}));
    let undo = s.history.undo.len();
    let job = shnyp_video::speech::start(&s, &Default::default()).unwrap();
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
        assert_eq!(shnyp_video::is_video_file(f), video, "{f}");
    }
    assert_eq!(shnyp_video::file_kind("a.webm"), Some("video"));
    assert_eq!(shnyp_video::file_kind("b.heic"), None);
}

fn landscape(path: &Path, seconds: u32) {
    let d = seconds.to_string();
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=1920x1080:rate=30:duration={d}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=550:duration={d}"),
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

fn clip_scale(s: &Session, clip: u64) -> f64 {
    let (_, it) = s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(clip)).unwrap();
    match it.effect("motion").and_then(|m| m.params.get("scale")).map(|p| &p.value) {
        Some(filmcraft_project::ParamValue::Float(f)) => *f,
        other => panic!("scale: {other:?}"),
    }
}

/// CapCut's order of work: clips (of every shape and sound alone), music, a title, then cutting.
#[test]
fn mixed_media_music_and_titles_then_cuts_still_work() {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("ffmpeg/ffprobe missing: skipped");
        return;
    }
    let d = dir("capcut-order");
    let (a, wide, voice, m) = (d.join("a.mp4"), d.join("wide.mp4"), d.join("voice.wav"), d.join("music.m4a"));
    phone_h264(&a, 3);
    landscape(&wide, 2);
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=300:duration=2"], &voice);
    music(&m);
    let mut s = Session::default();
    ok(&mut s, "shnyp.video.add", json!({"paths": [a.to_string_lossy()]}));
    // a landscape clip and a sound file in one go, into the vertical video
    let undo = s.history.undo.len();
    let st = ok(&mut s, "shnyp.video.add", json!({"paths": [wide.to_string_lossy(), voice.to_string_lossy()]}));
    assert_eq!(s.history.undo.len(), undo + 1);
    assert!((st["seconds"].as_f64().unwrap() - 5.0).abs() < 0.2, "3 s + 2 s picture, the sound alone at the playhead: {st}");
    let q = s.active_sequence().unwrap();
    let wide_clip = q.video_tracks[0].items[1].id.0;
    assert!((clip_scale(&s, wide_clip) - 56.3).abs() < 0.2, "16:9 fits into 9:16: {}", clip_scale(&s, wide_clip));
    assert_eq!(s.prefs.media.default_media_scaling, "none", "the user's preference is untouched");

    ok(&mut s, "shnyp.video.music", json!({"path": m.to_string_lossy()}));
    ok(&mut s, "shnyp.video.text", json!({"text": "Titel", "at": 0.0, "seconds": 6.0}));
    // delete the first clip: the picture closes up; the title over the whole video (it started on
    // that clip but runs far past it) and the music stay
    let first = s.active_sequence().unwrap().video_tracks[0].items[0].id.0;
    let undo = s.history.undo.len();
    let st = ok(&mut s, "shnyp.video.delete", json!({"clips": [first]}));
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let q = s.active_sequence().unwrap();
    let picture_end = q.video_tracks[0].items.iter().map(|i| i.end()).max().unwrap();
    let music = q
        .audio_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .find(|it| it.essential.as_ref().is_some_and(|e| e.kind == filmcraft_project::AudioType::Music));
    assert!(music.is_some_and(|m| m.end() > picture_end), "the music stays whole (the export ends with the picture)");
    assert!(q.video_tracks.iter().flat_map(|t| t.items.iter()).any(|it| it.name.contains("Titel")), "the long title stays");
    // the main track and its sound keep Premiere's sync lock; titles and music follow shnyp's anchors
    let main = shnyp_video::main_track::main_track_id(q).unwrap();
    let main_audio = shnyp_video::main_track::main_audio_track(q).unwrap();
    for t in q.all_tracks() {
        assert_eq!(t.sync_lock, t.id == main || t.id == main_audio, "sync lock of {}", t.name);
    }
    // the sound that started on the first clip went with it; the video lasts as long as the
    // title still runs
    let sounds = q
        .audio_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|it| it.essential.as_ref().is_some_and(|e| e.kind == filmcraft_project::AudioType::Sfx))
        .count();
    assert_eq!(sounds, 0, "the short sound went with its clip");
    assert!((st["seconds"].as_f64().unwrap() - 6.0).abs() < 0.2, "{st}");

    // speed keeps a reversed clip reversed
    let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id.0;
    s.execute("clip.speedDuration", json!({"clips": [clip], "speed": 100.0, "reverse": true})).unwrap();
    ok(&mut s, "shnyp.video.speed", json!({"clips": [clip], "speed": 2.0}));
    let (_, it) = s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(clip)).unwrap();
    assert!(it.reverse && (it.speed - 2.0).abs() < 1e-6, "reverse {} speed {}", it.reverse, it.speed);

    // detaching only ever unlinks
    let linked = s.active_sequence().unwrap().video_tracks[0].items.iter().find(|i| i.link.is_some()).map(|i| i.id.0).unwrap();
    let r = ok(&mut s, "shnyp.video.detachAudio", json!({"clips": [linked]}));
    assert_eq!(r["detached"], 2, "{r}");
    assert!(shnyp_video::execute(&mut s, "shnyp.video.detachAudio", &json!({"clips": [linked]})).is_err(), "nothing to detach any more");

    // In and Out marks don't shorten the export
    let whole = ok(&mut s, "shnyp.video.state", json!({}))["seconds"].as_f64().unwrap();
    s.execute("playhead.set", json!({"seconds": 0.2})).unwrap();
    s.execute("markers.markIn", json!({})).unwrap();
    s.execute("playhead.set", json!({"seconds": 0.6})).unwrap();
    s.execute("markers.markOut", json!({})).unwrap();
    let out = d.join("all.mp4");
    ok(&mut s, "shnyp.video.export", json!({"preset": "video", "path": out.to_string_lossy()}));
    let dur: f64 = probe(&out)["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((dur - whole).abs() < 0.15 && whole > 0.8, "the whole video ({whole} s), not the marked 0.4 s: {dur}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn captions_skip_the_music_and_are_burned_into_the_export() {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("ffmpeg/ffprobe missing: skipped");
        return;
    }
    let d = dir("burn");
    let (a, m) = (d.join("talk.mp4"), d.join("music.m4a"));
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:size=320x240:rate=30:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
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
    music(&m);
    let mut s = Session::default();
    ok(&mut s, "shnyp.video.add", json!({"paths": [a.to_string_lossy()]}));
    ok(&mut s, "shnyp.video.music", json!({"path": m.to_string_lossy()}));
    s.transcriber = Some(std::sync::Arc::new(filmcraft_speech::FixedTranscriber {
        transcript: words(&[("GROSSER", 0.2, 0.8), ("TEXT", 0.8, 1.6)]),
        id: "test".into(),
    }));
    let job = shnyp_video::speech::start(&s, &Default::default()).unwrap();
    let seq = job.sequence;
    let done = job.wait().unwrap();
    assert_eq!(done.len(), 1, "the speech clip only, not the music");
    shnyp_video::speech::apply(&mut s, seq, done).unwrap();

    let (with, without) = (d.join("with.mp4"), d.join("without.mp4"));
    ok(&mut s, "shnyp.video.export", json!({"preset": "video", "path": with.to_string_lossy()}));
    ok(&mut s, "shnyp.video.export", json!({"preset": "video", "path": without.to_string_lossy(), "burnCaptions": false}));
    // the frame at 1 s differs where the caption is drawn
    let frame = |p: &Path| {
        let o = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-ss", "1", "-i"])
            .arg(p)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "gray", "-"])
            .output()
            .unwrap();
        o.stdout
    };
    let (fa, fb) = (frame(&with), frame(&without));
    assert_eq!(fa.len(), fb.len());
    let differing = fa.iter().zip(&fb).filter(|(x, y)| x.abs_diff(**y) > 40).count();
    assert!(differing > 200, "captions are in the picture: {differing} pixels differ");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_failed_or_cancelled_export_leaves_no_file() {
    if !have("ffmpeg") {
        return;
    }
    let d = dir("failed-export");
    let a = d.join("a.mp4");
    phone_h264(&a, 2);
    let mut s = Session::default();
    ok(&mut s, "shnyp.video.add", json!({"paths": [a.to_string_lossy()]}));
    // a folder that cannot be written: the waited-for export is an error, not "ok"
    let blocked = d.join("blocked");
    std::fs::write(&blocked, b"a file, not a folder").unwrap();
    let out = blocked.join("x.mp4");
    let e = shnyp_video::execute(&mut s, "shnyp.video.export", &json!({"preset": "video", "path": out.to_string_lossy()})).unwrap_err();
    assert!(!e.is_empty());
    assert!(!out.exists() && !shnyp_video::part_path(&out).exists());
    // started in the background and cancelled: nothing is left behind
    let out = d.join("cancel.mp4");
    let r = ok(&mut s, "shnyp.video.export", json!({"preset": "youtube4k", "path": out.to_string_lossy(), "wait": false}));
    let job = r["job"].as_u64().unwrap();
    let part = std::path::PathBuf::from(r["part"].as_str().unwrap());
    s.execute("jobs.cancel", json!({"job": job})).unwrap();
    let t0 = std::time::Instant::now();
    let end = loop {
        if let Some(end) = shnyp_video::settle_export(&s, job, &part, &out) {
            break end;
        }
        assert!(t0.elapsed().as_secs() < 120);
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    // a cancel that came too late is a finished file; either way nothing half-written stays
    assert!(end.is_err() == !out.exists(), "{end:?}");
    assert!(!part.exists());
    let _ = std::fs::remove_dir_all(&d);
}
