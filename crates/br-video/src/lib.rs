//! BR's video commands on FilmCraft's engine: CapCut's one-tap results for the Schnitt page,
//! the CLI and `br mcp`. Each command is one undo step (FilmCraft's `edit_group`), built from
//! FilmCraft's own commands, so everything it does can be refined in FilmCraft's panels.
//!
//! | Command | What it does |
//! |---|---|
//! | `br.video.new` | a new video in a social format (9:16, 16:9, 1:1, 4:5) |
//! | `br.video.add` | import files and put them one after another at the end of the main track |
//! | `br.video.split` | cut every clip under the playhead (Add Edit) |
//! | `br.video.delete` | delete the selected clips and close the gap (Ripple Delete) |
//! | `br.video.speed` | play the selected clips faster or slower, the gap closes or opens |
//! | `br.video.detachAudio` | separate picture and sound of the selected clips |
//! | `br.video.text` | a title at the playhead |
//! | `br.video.captions` | captions from an SRT/VTT/… file |
//! | `br.video.autoCaptions` | captions from the speech in the video (Whisper, see [`speech`]) |
//! | `br.video.music` | a music file on its own track, as long as the video, lowered under speech |
//! | `br.video.filter` | a look on the selected clips (all without a selection), replacing the last |
//! | `br.video.format` | another shape (9:16, 16:9, 1:1, 4:5); the clips fit or fill it |
//! | `br.video.transition` | the default transition (a dissolve) at the cuts of the selected clips |
//! | `br.video.export` | an MP4 in a preset for TikTok/Reels/Shorts, YouTube, … |
//! | `br.video.state` | the video at a glance |

use serde_json::{Value, json};

pub use filmcraft_engine::Session;

pub mod speech;

/// A video's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// 9:16, 1080×1920: TikTok, Reels, Shorts, Stories.
    Vertical,
    /// 16:9, 1920×1080: YouTube, TV.
    Wide,
    /// 1:1, 1080×1080.
    Square,
    /// 4:5, 1080×1350: Instagram feed.
    Portrait,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Vertical, Format::Wide, Format::Square, Format::Portrait];

    pub fn key(self) -> &'static str {
        match self {
            Format::Vertical => "9:16",
            Format::Wide => "16:9",
            Format::Square => "1:1",
            Format::Portrait => "4:5",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Vertical => "Hochformat 9:16",
            Format::Wide => "Querformat 16:9",
            Format::Square => "Quadrat 1:1",
            Format::Portrait => "Porträt 4:5",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Format::Vertical => "TikTok, Reels, Shorts, Stories",
            Format::Wide => "YouTube, Fernsehen, Präsentationen",
            Format::Square => "Feed-Beiträge",
            Format::Portrait => "Instagram-Feed",
        }
    }

    pub fn size(self) -> (u32, u32) {
        match self {
            Format::Vertical => (1080, 1920),
            Format::Wide => (1920, 1080),
            Format::Square => (1080, 1080),
            Format::Portrait => (1080, 1350),
        }
    }

    pub fn find(key: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.key() == key)
    }

    /// The format with this frame size, if it is one of BR's.
    pub fn of_size(w: u32, h: u32) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.size() == (w, h))
    }
}

/// An export preset: BR's name for one of FilmCraft's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportPreset {
    pub key: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
    /// FilmCraft's preset.
    pub filmcraft: &'static str,
    /// The file's extension.
    pub ext: &'static str,
}

pub const EXPORT_PRESETS: [ExportPreset; 5] = [
    ExportPreset {
        key: "video",
        label: "Wie das Video",
        detail: "MP4 (H.264) in Größe und Bildrate des Videos",
        filmcraft: "Match Source – Adaptive High Bitrate",
        ext: "mp4",
    },
    ExportPreset {
        key: "social",
        label: "TikTok, Reels, Shorts",
        detail: "MP4 9:16, 1080 × 1920, füllend skaliert",
        filmcraft: "Social Vertical 1080×1920",
        ext: "mp4",
    },
    ExportPreset { key: "youtube", label: "YouTube 1080p", detail: "MP4 16:9, 1920 × 1080", filmcraft: "YouTube 1080p Full HD", ext: "mp4" },
    ExportPreset { key: "youtube4k", label: "YouTube 4K", detail: "MP4 16:9, 3840 × 2160", filmcraft: "YouTube 2160p 4K Ultra HD", ext: "mp4" },
    ExportPreset {
        key: "prores",
        label: "Master (ProRes 422 HQ)",
        detail: "MOV für die Weiterbearbeitung",
        filmcraft: "Apple ProRes 422 HQ",
        ext: "mov",
    },
];

pub fn export_preset(key: &str) -> Option<&'static ExportPreset> {
    EXPORT_PRESETS.iter().find(|p| p.key == key)
}

/// One of BR's video commands, for listings (`br commands`, MCP).
pub struct CommandInfo {
    pub id: &'static str,
    pub params: &'static str,
    pub summary: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        id: "br.video.new",
        params: r#"{"format":"9:16|16:9|1:1|4:5","fps":f64=30,"name":str?}"#,
        summary: "A new video in a social format",
    },
    CommandInfo {
        id: "br.video.add",
        params: r#"{"paths":[str],"at":"end|playhead"="end"}"#,
        summary: "Import files and put them one after another on the main track (at its end or the playhead)",
    },
    CommandInfo { id: "br.video.split", params: r#"{"seconds":f64?}"#, summary: "Cut every clip under the playhead (or at `seconds`)" },
    CommandInfo { id: "br.video.delete", params: r#"{"clips":[id]?}"#, summary: "Delete the selected clips and close the gap" },
    CommandInfo {
        id: "br.video.speed",
        params: r#"{"speed":f64,"clips":[id]?}"#,
        summary: "Speed factor of the selected clips (2 = twice as fast), the gap follows",
    },
    CommandInfo { id: "br.video.detachAudio", params: r#"{"clips":[id]?}"#, summary: "Separate picture and sound of the selected clips" },
    CommandInfo { id: "br.video.text", params: r#"{"text":str,"seconds":f64=3,"at":f64?}"#, summary: "A title at the playhead (or at `at` seconds)" },
    CommandInfo { id: "br.video.captions", params: r#"{"path":str}"#, summary: "Captions from a subtitle file (SRT, VTT, SCC, STL, TTML, …)" },
    CommandInfo {
        id: "br.video.music",
        params: r#"{"path":str,"duck":bool=true}"#,
        summary: "Music on its own track from the start, cut to the video's length, lowered under speech",
    },
    CommandInfo {
        id: "br.video.filter",
        params: r#"{"preset":str|"none","clips":[id]?}"#,
        summary: "A look (see lumetri.presets) on the selected clips, all without a selection; replaces the look applied before",
    },
    CommandInfo {
        id: "br.video.export",
        params: r#"{"preset":"video|social|youtube|youtube4k|prores"="video","path":str?,"wait":bool=false}"#,
        summary: "Export the video (MP4 unless ProRes); `wait` blocks until written",
    },
    CommandInfo {
        id: "br.video.autoCaptions",
        params: r#"{"language":"de|en|…|auto"?,"model":"whisper-tiny|whisper-base|whisper-small"?}"#,
        summary: "Captions from the speech in the video (Whisper; the model is downloaded the first time)",
    },
    CommandInfo {
        id: "br.video.format",
        params: r#"{"format":"9:16|16:9|1:1|4:5","fit":"fit|fill"=fit}"#,
        summary: "Change the video's shape; its clips fit into it (or fill it)",
    },
    CommandInfo {
        id: "br.video.transition",
        params: r#"{"clips":[id]?}"#,
        summary: "A dissolve at the cuts between the selected clips (every cut of the main track without a selection)",
    },
    CommandInfo { id: "br.video.state", params: "{}", summary: "The video at a glance: format, length, clips, playhead" },
];

/// Where videos are exported: `$BR_EXPORTS` (as photos; a hosted BR serves that folder), else
/// the system's videos folder ▸ BR Exports.
pub fn export_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("BR_EXPORTS").filter(|d| !d.is_empty()) {
        return d.into();
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join(if cfg!(target_os = "macos") { "Movies" } else { "Videos" }).join("BR Exports")
}

/// A file in `dir` named after the video that is not there yet: `Video.mp4`, `Video 2.mp4`, …
pub fn export_path(dir: &std::path::Path, name: &str, preset: &ExportPreset) -> std::path::PathBuf {
    let clean: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let stem = match clean.trim().trim_matches('.') {
        "" => "Video",
        s => s,
    };
    (1..)
        .map(|n| dir.join(if n == 1 { format!("{stem}.{}", preset.ext) } else { format!("{stem} {n}.{}", preset.ext) }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("{stem}.{}", preset.ext)))
}

/// What the media picker offers: video, sound and still images.
pub fn media_extensions() -> Vec<&'static str> {
    filmcraft_media::VIDEO_EXTENSIONS.iter().chain(filmcraft_media::AUDIO_EXTENSIONS).chain(filmcraft_media::STILL_EXTENSIONS).copied().collect()
}

/// Sound files (music).
pub fn audio_extensions() -> &'static [&'static str] {
    filmcraft_media::AUDIO_EXTENSIONS
}

/// What kind of file FilmCraft makes of `path`, by its name: `video`, `audio`, `image`,
/// `project` (`.fcproj`, EDL, XML, FCPXML, OTIO) or `caption` (SRT, VTT, SCC).
pub fn file_kind(path: &str) -> Option<&'static str> {
    let name = std::path::Path::new(path).file_name().map_or(path.into(), |n| n.to_string_lossy());
    filmcraft_engine::media_browser::kind_of(&name)
}

/// Files that belong on the video pages rather than in the photo library: video, sound, video
/// projects and subtitle files.
pub fn is_video_file(path: &str) -> bool {
    matches!(file_kind(path), Some("video" | "audio" | "project" | "caption"))
}

/// The project holds nothing yet: no media, no sequence (the start screen's moment).
pub fn is_empty(s: &Session) -> bool {
    s.project.items.values().all(|i| !matches!(i.kind, filmcraft_project::ItemKind::Media(_) | filmcraft_project::ItemKind::Sequence(_)))
}

/// The clips selected in the timeline.
pub fn selection(s: &Session) -> usize {
    s.state.selection.len()
}

/// The video clips of the active sequence that show media (not titles or graphics).
fn media_video_clips(s: &Session, seq: &Value) -> Vec<u64> {
    tracks(seq, "video")
        .flat_map(items)
        .filter(|c| {
            c.get("item")
                .and_then(Value::as_u64)
                .and_then(|i| s.project.item(filmcraft_project::ItemId(i)))
                .is_some_and(|i| matches!(i.kind, filmcraft_project::ItemKind::Media(_) | filmcraft_project::ItemKind::Subclip { .. }))
        })
        .filter_map(|c| c.get("clip").and_then(Value::as_u64))
        .collect()
}

pub fn is_video(id: &str) -> bool {
    id.starts_with("br.video.")
}

fn err(cmd: &str, msg: impl std::fmt::Display) -> String {
    format!("{cmd}: {msg}")
}

fn run(s: &mut Session, id: &str, p: Value) -> Result<Value, String> {
    s.execute(id, p).map_err(|e| e.to_string())
}

fn str_param<'a>(cmd: &str, p: &'a Value, key: &str) -> Result<&'a str, String> {
    p.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| err(cmd, format!("needs `{key}`")))
}

fn opt_f64(cmd: &str, p: &Value, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|x| x.is_finite()).map(Some).ok_or_else(|| err(cmd, format!("`{key}` must be a number"))),
    }
}

/// The ids of a list of numbers in `v[key]`.
fn ids(v: &Value, key: &str) -> Vec<u64> {
    v.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default()
}

/// The `clips` parameter, passed on as it is (FilmCraft then uses the selection without it).
fn clips_param(p: &Value) -> Value {
    match p.get("clips") {
        Some(c) if !c.is_null() => json!({"clips": c}),
        _ => json!({}),
    }
}

fn seconds(t: i64) -> f64 {
    t as f64 / filmcraft_time::TICKS_PER_SECOND as f64
}

/// The active sequence as FilmCraft's `sequence.inspect` describes it.
fn inspect(s: &mut Session) -> Result<Value, String> {
    run(s, "sequence.inspect", json!({}))
}

/// The tracks of one kind (`video` or `audio`) in a `sequence.inspect` answer.
fn tracks<'a>(seq: &'a Value, kind: &str) -> impl Iterator<Item = &'a Value> {
    seq.get(kind).and_then(Value::as_array).into_iter().flatten()
}

/// The clips on a track.
fn items(track: &Value) -> impl Iterator<Item = &Value> {
    track.get("items").and_then(Value::as_array).into_iter().flatten()
}

/// Where the last clip of a track ends, in ticks (0 for an empty or missing track).
fn track_end(track: Option<&Value>) -> i64 {
    track.map(|t| items(t).filter_map(|c| c.get("end").and_then(Value::as_i64)).max().unwrap_or(0)).unwrap_or(0)
}

/// The main track: the first video track.
fn main_track(seq: &Value) -> Option<&Value> {
    tracks(seq, "video").next()
}

fn playhead(seq: &Value) -> i64 {
    seq.get("playhead").and_then(Value::as_i64).unwrap_or(0)
}

/// Run one of BR's video commands, or any of FilmCraft's (`timeline.razor`, `lumetri.setLook`, …).
pub fn execute(s: &mut Session, id: &str, p: &Value) -> Result<Value, String> {
    let p = if p.is_null() { &json!({}) } else { p };
    if !is_video(id) {
        return run(s, id, p.clone());
    }
    match id {
        "br.video.new" => {
            let key = p.get("format").and_then(Value::as_str).unwrap_or("16:9");
            let format = Format::find(key).ok_or_else(|| err(id, format!("unknown format `{key}`: 9:16, 16:9, 1:1 or 4:5")))?;
            let fps = opt_f64(id, p, "fps")?.unwrap_or(30.0);
            if !(1.0..=240.0).contains(&fps) {
                return Err(err(id, "`fps` must be between 1 and 240"));
            }
            let name = p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("Video {}", format.key()));
            let (w, h) = format.size();
            run(s, "file.newSequence", json!({"name": name, "width": w, "height": h, "fps": fps}))?;
            state(s)
        }
        "br.video.add" => {
            let paths: Vec<String> = match p.get("paths") {
                Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                Some(Value::String(one)) => vec![one.clone()],
                _ => Vec::new(),
            };
            if paths.is_empty() {
                return Err(err(id, "needs `paths`"));
            }
            let at_playhead = match p.get("at").and_then(Value::as_str) {
                None | Some("end") => false,
                Some("playhead") => true,
                Some(other) => return Err(err(id, format!("`at` is end or playhead, not `{other}`"))),
            };
            s.edit_group("Medien hinzufügen", |s| {
                let imported = s.execute("file.import", json!({"paths": paths}))?;
                let items = ids(&imported, "items");
                if items.is_empty() {
                    return Err(filmcraft_engine::EngineError::BadParams { cmd: "file.import".into(), msg: format!("nothing imported: {imported}") });
                }
                // a project without a video: one in the first clip's shape, with the clips in it
                if s.active_sequence().is_none() {
                    return s.execute("file.newSequence", json!({"name": "Video", "fromItems": items}));
                }
                let seq = s.execute("sequence.inspect", json!({}))?;
                let time = if at_playhead { playhead(&seq) } else { track_end(main_track(&seq)) };
                s.execute("timeline.place", json!({"items": items, "track": "V1", "time": time, "insert": at_playhead}))
            })
            .map_err(|e| e.to_string())?;
            state(s)
        }
        "br.video.split" => {
            let mut q = json!({});
            if let Some(sec) = opt_f64(id, p, "seconds")? {
                q["time"] = json!((sec * filmcraft_time::TICKS_PER_SECOND as f64).round() as i64);
            }
            run(s, "sequence.addEdit", q)
        }
        "br.video.delete" => run(s, "edit.rippleDelete", clips_param(p)),
        "br.video.speed" => {
            let speed = opt_f64(id, p, "speed")?.ok_or_else(|| err(id, "needs `speed`"))?;
            if !(0.01..=100.0).contains(&speed) {
                return Err(err(id, "`speed` must be between 0.01 and 100"));
            }
            let mut q = clips_param(p);
            q["speed"] = json!(speed * 100.0);
            q["ripple"] = json!(true);
            run(s, "clip.speedDuration", q)
        }
        "br.video.detachAudio" => {
            // clip.link unlinks clips that are linked (and links them otherwise)
            run(s, "clip.link", clips_param(p))
        }
        "br.video.text" => {
            let text = str_param(id, p, "text")?.to_string();
            let secs = opt_f64(id, p, "seconds")?.unwrap_or(3.0);
            if !(0.04..=3600.0).contains(&secs) {
                return Err(err(id, "`seconds` must be between 0.04 and 3600"));
            }
            let mut q = json!({"text": text, "seconds": secs, "newClip": true});
            if let Some(at) = opt_f64(id, p, "at")? {
                q["time"] = json!((at * filmcraft_time::TICKS_PER_SECOND as f64).round() as i64);
            }
            run(s, "graphics.newText", q)
        }
        "br.video.captions" => {
            let path = str_param(id, p, "path")?.to_string();
            run(s, "captions.import", json!({"path": path}))
        }
        "br.video.music" => {
            let path = str_param(id, p, "path")?.to_string();
            let duck = p.get("duck").and_then(Value::as_bool).unwrap_or(true);
            s.edit_group("Musik", |s| {
                let imported = s.execute("file.import", json!({"paths": [path]}))?;
                let items = ids(&imported, "items");
                let Some(item) = items.first() else {
                    return Err(filmcraft_engine::EngineError::BadParams { cmd: "file.import".into(), msg: format!("nothing imported: {imported}") });
                };
                // as long as the video (the main track), never longer: music must not add black
                let end = track_end(main_track(&s.execute("sequence.inspect", json!({}))?));
                let mut q = json!({"item": item, "audioTrack": "new", "time": 0, "video": false});
                if end > 0 {
                    q["duration"] = json!(end);
                }
                let placed = s.execute("timeline.place", q)?;
                let clips = ids(&placed, "clips");
                s.execute("essentialSound.setType", json!({"clips": clips, "type": "music"}))?;
                if duck {
                    s.execute("essentialSound.generateDucking", json!({"clips": clips}))?;
                }
                Ok(placed)
            })
            .map_err(|e| e.to_string())?;
            state(s)
        }
        "br.video.filter" => {
            // CapCut's filters replace each other: a look applied unchanged gives way to the new
            // one; a grade changed by hand (or any other Lumetri) stays
            let name = p.get("preset").and_then(Value::as_str).unwrap_or("");
            let remove_only = name.is_empty() || name == "none";
            if !remove_only && filmcraft_render::lumetri_presets::find(name).is_none() {
                return Err(err(id, format!("unknown look `{name}` (see lumetri.presets)")));
            }
            if s.active_sequence().is_none() {
                return Err(err(id, "no video yet"));
            }
            let clips: Vec<u64> = match p.get("clips") {
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_u64).collect(),
                _ if !s.state.selection.is_empty() => s.state.selection.iter().map(|c| c.0).collect(),
                _ => {
                    let seq = inspect(s)?;
                    media_video_clips(s, &seq)
                }
            };
            if clips.is_empty() {
                return Err(err(id, "no video clips"));
            }
            let looks: Vec<_> = filmcraft_render::lumetri_presets::presets().iter().map(|l| l.instance()).collect();
            let label = if remove_only { "Filter entfernen".to_string() } else { format!("Filter {name}") };
            s.edit_group(&label, |s| {
                let ids: Vec<filmcraft_project::ClipId> = clips.iter().copied().map(filmcraft_project::ClipId).collect();
                let removed = s.edit_sequence("Filter entfernen", |q, _, _| {
                    let mut n = 0;
                    for it in q.video_tracks.iter_mut().flat_map(|t| t.items.iter_mut()).filter(|it| ids.contains(&it.id)) {
                        let before = it.effects.len();
                        it.effects.retain(|e| !looks.contains(e));
                        n += before - it.effects.len();
                    }
                    Ok(n)
                })?;
                if remove_only {
                    return Ok(json!({"removed": removed}));
                }
                let mut r = s.execute("lumetri.applyPreset", json!({"name": name, "clips": clips}))?;
                r["replaced"] = json!(removed);
                Ok(r)
            })
            .map_err(|e| e.to_string())
        }
        "br.video.format" => {
            let key = str_param(id, p, "format")?;
            let format = Format::find(key).ok_or_else(|| err(id, format!("unknown format `{key}`: 9:16, 16:9, 1:1 or 4:5")))?;
            let fill = match p.get("fit").and_then(Value::as_str) {
                None | Some("fit") => false,
                Some("fill") => true,
                Some(other) => return Err(err(id, format!("`fit` is fit or fill, not `{other}`"))),
            };
            if s.active_sequence().is_none() {
                return Err(err(id, "no video yet: br.video.new or br.video.add first"));
            }
            let (w, h) = format.size();
            s.edit_group(&format!("Format {}", format.key()), |s| {
                s.execute("sequence.settings", json!({"width": w, "height": h}))?;
                let seq = s.execute("sequence.inspect", json!({}))?;
                let clips = media_video_clips(s, &seq);
                if !clips.is_empty() {
                    s.execute(if fill { "clip.fillFrame" } else { "clip.fitToFrame" }, json!({"clips": clips}))?;
                }
                Ok(Value::Null)
            })
            .map_err(|e| e.to_string())?;
            state(s)
        }
        "br.video.transition" => {
            if s.active_sequence().is_none() {
                return Err(err(id, "no video yet"));
            }
            let chosen: Vec<u64> = match p.get("clips") {
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_u64).collect(),
                _ => s.state.selection.iter().map(|c| c.0).collect(),
            };
            // the cuts of the main track: where one clip ends and the next begins
            let seq = inspect(s)?;
            let mut main: Vec<(u64, i64, i64)> = main_track(&seq)
                .into_iter()
                .flat_map(items)
                .filter_map(|c| Some((c.get("clip")?.as_u64()?, c.get("start")?.as_i64()?, c.get("end")?.as_i64()?)))
                .collect();
            main.sort_by_key(|c| c.1);
            // a cut that has a transition keeps it
            let has_one: Vec<u64> = main_track(&seq)
                .and_then(|t| t.get("transitions"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|x| x.get("from").and_then(Value::as_u64))
                .collect();
            let cuts: Vec<u64> = main
                .windows(2)
                .filter(|w| w[0].2 == w[1].1 && (chosen.is_empty() || chosen.contains(&w[0].0) || chosen.contains(&w[1].0)))
                .map(|w| w[0].0)
                .filter(|a| !has_one.contains(a))
                .collect();
            if cuts.is_empty() {
                return Err(err(id, "no cut between two clips without a transition there"));
            }
            let n = cuts.len();
            s.edit_group("Übergänge", |s| {
                for clip in cuts {
                    s.execute("sequence.applyVideoTransition", json!({"clip": clip, "edge": "out"}))?;
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
            Ok(json!({"transitions": n}))
        }
        "br.video.autoCaptions" => {
            let opts = speech::CaptionOptions {
                language: p.get("language").and_then(Value::as_str).map(str::to_string),
                model: p.get("model").and_then(Value::as_str).map(str::to_string),
            };
            let job = speech::start(s, &opts).map_err(|e| err(id, e))?;
            let seq = job.sequence;
            let done = job.wait().map_err(|e| err(id, e))?;
            speech::apply(s, seq, done).map_err(|e| err(id, e))
        }
        "br.video.export" => {
            let key = p.get("preset").and_then(Value::as_str).unwrap_or("video");
            let preset = export_preset(key).ok_or_else(|| err(id, format!("unknown preset `{key}`: video, social, youtube, youtube4k or prores")))?;
            if s.active_sequence().is_none() {
                return Err(err(id, "no video yet"));
            }
            // no path: BR's export folder, named after the video
            let path = match p.get("path").and_then(Value::as_str) {
                Some(path) => std::path::PathBuf::from(path),
                None => {
                    let dir = export_dir();
                    std::fs::create_dir_all(&dir).map_err(|e| err(id, format!("{}: {e}", dir.display())))?;
                    let name = s.state.active_sequence.and_then(|q| s.project.item(q)).map(|i| i.name.clone()).unwrap_or_default();
                    export_path(&dir, &name, preset)
                }
            };
            let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(false);
            run(s, "export.quick", json!({"preset": preset.filmcraft, "path": path.to_string_lossy(), "wait": wait}))
        }
        "br.video.state" => state(s),
        _ => Err(format!("unknown command `{id}`: see br.video.* in `br commands`")),
    }
}

/// The video at a glance.
pub fn state(s: &mut Session) -> Result<Value, String> {
    if s.active_sequence().is_none() {
        return Ok(json!({"video": null}));
    }
    let seq = inspect(s)?;
    let size = |k: &str| seq.pointer(&format!("/settings/{k}")).and_then(Value::as_u64).unwrap_or(0) as u32;
    let (w, h) = (size("width"), size("height"));
    let video: usize = tracks(&seq, "video").map(|t| items(t).count()).sum();
    let audio: usize = tracks(&seq, "audio").map(|t| items(t).count()).sum();
    let end = seq.get("duration").and_then(Value::as_i64).unwrap_or(0);
    Ok(json!({
        "name": seq.get("name"),
        "format": Format::of_size(w, h).map(Format::key),
        "width": w,
        "height": h,
        "seconds": seconds(end),
        "videoClips": video,
        "audioClips": audio,
        "playhead": seconds(playhead(&seq)),
        "undo": s.history.undo.last().map(|u| u.0.clone()),
    }))
}
