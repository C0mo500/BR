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
//! | `br.video.music` | a music file on its own track, as long as the video, lowered under speech |
//! | `br.video.filter` | a Lumetri look on the selected clips |
//! | `br.video.export` | an MP4 in a preset for TikTok/Reels/Shorts, YouTube, … |
//! | `br.video.state` | the video at a glance |

use serde_json::{Value, json};

pub use filmcraft_engine::Session;

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
}

pub const EXPORT_PRESETS: [ExportPreset; 5] = [
    ExportPreset {
        key: "video",
        label: "Wie das Video",
        detail: "MP4 (H.264) in Größe und Bildrate des Videos",
        filmcraft: "Match Source – Adaptive High Bitrate",
    },
    ExportPreset {
        key: "social",
        label: "TikTok, Reels, Shorts",
        detail: "MP4 9:16, 1080 × 1920, füllend skaliert",
        filmcraft: "Social Vertical 1080×1920",
    },
    ExportPreset { key: "youtube", label: "YouTube 1080p", detail: "MP4 16:9, 1920 × 1080", filmcraft: "YouTube 1080p Full HD" },
    ExportPreset { key: "youtube4k", label: "YouTube 4K", detail: "MP4 16:9, 3840 × 2160", filmcraft: "YouTube 2160p 4K Ultra HD" },
    ExportPreset { key: "prores", label: "Master (ProRes 422 HQ)", detail: "MOV für die Weiterbearbeitung", filmcraft: "Apple ProRes 422 HQ" },
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
        params: r#"{"preset":str,"clips":[id]?}"#,
        summary: "A Lumetri look (see lumetri.presets) on the selected clips",
    },
    CommandInfo {
        id: "br.video.export",
        params: r#"{"preset":"video|social|youtube|youtube4k|prores"="video","path":str?,"wait":bool=false}"#,
        summary: "Export the video (MP4 unless ProRes); `wait` blocks until written",
    },
    CommandInfo { id: "br.video.state", params: "{}", summary: "The video at a glance: format, length, clips, playhead" },
];

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
            let preset = str_param(id, p, "preset")?.to_string();
            let mut q = clips_param(p);
            q["name"] = json!(preset);
            run(s, "lumetri.applyPreset", q)
        }
        "br.video.export" => {
            let key = p.get("preset").and_then(Value::as_str).unwrap_or("video");
            let preset = export_preset(key).ok_or_else(|| err(id, format!("unknown preset `{key}`: video, social, youtube, youtube4k or prores")))?;
            let mut q = json!({"preset": preset.filmcraft, "wait": p.get("wait").and_then(Value::as_bool).unwrap_or(false)});
            if let Some(path) = p.get("path").and_then(Value::as_str) {
                q["path"] = json!(path);
            }
            run(s, "export.quick", q)
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
