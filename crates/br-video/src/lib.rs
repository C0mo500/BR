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
    CommandInfo {
        id: "br.video.commands",
        params: r#"{"query":str?}"#,
        summary: "FilmCraft's commands (id, label, menu, parameters) whose id or label contains `query`",
    },
];

/// The picture of the video at `seconds` (at the playhead without), at most `max_side` pixels on
/// its long side: (width, height, RGBA8 pixels).
pub fn frame(s: &Session, seconds: Option<f64>, max_side: u32) -> Result<(u32, u32, Vec<u8>), String> {
    let q = s.active_sequence().ok_or("no video yet")?;
    let long = q.settings.width.max(q.settings.height).max(1);
    let scale = (max_side.max(16) as f32 / long as f32).min(1.0);
    let t = match seconds {
        Some(sec) if sec.is_finite() && sec >= 0.0 => filmcraft_time::Tick::from_seconds_f64(sec),
        Some(sec) => return Err(format!("`seconds` must be 0 or more, not {sec}")),
        None => s.playhead(),
    };
    let img = s.try_render_program_at(scale, t).map_err(|e| e.to_string())?;
    Ok((img.w as u32, img.h as u32, img.to_rgba8()))
}

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

/// Open files in the video project by their kind: a video project (the first) is opened, videos,
/// sound and stills are added (`br.video.add`), subtitle files become captions. Paths are made
/// absolute first (FilmCraft keeps them as given, and a project saved with relative ones loses its
/// media when opened from another folder). Returns the video's state.
pub fn open_files(s: &mut Session, paths: &[String]) -> Result<Value, String> {
    let abs = |p: &String| std::path::absolute(p).map(|a| a.to_string_lossy().into_owned()).unwrap_or_else(|_| p.clone());
    let paths: Vec<String> = expand_folders(&paths.iter().map(abs).collect::<Vec<_>>());
    let kind = |p: &String| file_kind(p).unwrap_or("");
    if let Some(project) = paths.iter().find(|p| kind(p) == "project") {
        run(s, "file.open", json!({"path": project}))?;
    }
    let media: Vec<&String> = paths.iter().filter(|p| matches!(kind(p), "video" | "audio" | "image")).collect();
    let mut st = if media.is_empty() { state(s)? } else { execute(s, "br.video.add", &json!({"paths": media}))? };
    for subs in paths.iter().filter(|p| kind(p) == "caption") {
        execute(s, "br.video.captions", &json!({"path": subs}))?;
        st = state(s)?;
    }
    Ok(st)
}

/// The hidden file an export to `path` is written to until it is complete.
pub fn part_path(path: &std::path::Path) -> std::path::PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Video.mp4".into());
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name.as_str(), "mp4"));
    path.with_file_name(format!(".{stem}.br-part.{ext}"))
}

/// An export job's end: `None` while it runs; when it is done the part file becomes `path`
/// (`Ok`), or is removed (`Err`: failed or cancelled).
pub fn settle_export(s: &Session, job: u64, part: &std::path::Path, path: &std::path::Path) -> Option<Result<(), String>> {
    let result = match s.jobs.iter().find(|j| j.id == job) {
        Some(j) => j.result.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|r| r.as_ref().map(|_| ()).map_err(Clone::clone))?,
        // FilmCraft let go of the job: the file tells
        None if part.exists() => Ok(()),
        None => Err("abgebrochen".into()),
    };
    Some(match result {
        Ok(()) => std::fs::rename(part, path).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) => {
            let _ = std::fs::remove_file(part);
            Err(if e == "cancelled" { "abgebrochen".into() } else { e })
        }
    })
}

/// Expand folders into the media files in them (sorted by name, at most 1000 per folder, not
/// into hidden folders); files stay as they are.
pub fn expand_folders(paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in paths {
        let path = std::path::Path::new(p);
        if !path.is_dir() {
            out.push(p.clone());
            continue;
        }
        let mut found: Vec<String> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.') && e.path().is_file())
            .map(|e| e.path().to_string_lossy().into_owned())
            .filter(|f| matches!(file_kind(f), Some("video" | "audio" | "image" | "caption")))
            .collect();
        found.sort();
        found.truncate(1000);
        out.extend(found);
    }
    out
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

/// The clips a command works on: `clips`, else the timeline's selection.
fn chosen_clips(s: &Session, p: &Value) -> Vec<u64> {
    match p.get("clips") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_u64).collect(),
        _ => s.state.selection.iter().map(|c| c.0).collect(),
    }
}

/// Put imported items into the active sequence (a new one in the first item's shape when there
/// is none): one after another at the end (of the picture and the sound, whichever is later) or
/// inserted at the playhead. Pictures go on V1 with their sound on A1; sound alone on A1.
fn place_media(s: &mut Session, items: &[u64], at_playhead: bool) -> filmcraft_engine::Result<()> {
    if s.active_sequence().is_none() {
        s.execute("file.newSequence", json!({"name": "Video", "fromItems": items}))?;
        return Ok(());
    }
    let seq = s.execute("sequence.inspect", json!({}))?;
    let mut time = if at_playhead { playhead(&seq) } else { track_end(main_track(&seq)).max(track_end(tracks(&seq, "audio").next())) };
    for item in items {
        let (has_video, _) = s.project.source_streams(filmcraft_project::ItemId(*item));
        let mut q = json!({"item": item, "time": time, "insert": at_playhead});
        if has_video {
            q["track"] = json!("V1");
        } else {
            q["audioTrack"] = json!("A1");
        }
        let placed = s.execute("timeline.place", q)?;
        let end = ids(&placed, "clips")
            .iter()
            .filter_map(|c| s.active_sequence()?.find_item(filmcraft_project::ClipId(*c)).map(|(_, it)| it.end().0))
            .max();
        if let Some(end) = end {
            time = time.max(end);
        }
    }
    Ok(())
}

/// Delete `chosen` (and the clips linked to them) and close the gap on their tracks only, as
/// CapCut does: music, titles and captions on other tracks keep their place (FilmCraft refuses a
/// ripple delete that sync-locked tracks with something in the gap could not follow). What then
/// runs past the end of the picture is cut at its end.
fn ripple_delete(s: &mut Session, chosen: &[u64]) -> filmcraft_engine::Result<()> {
    use filmcraft_project::TrackId;
    let q = s.active_sequence().ok_or(filmcraft_engine::EngineError::NoSequence)?;
    let links: Vec<u64> = q.all_tracks().flat_map(|t| t.items.iter()).filter(|it| chosen.contains(&it.id.0)).filter_map(|it| it.link).collect();
    let affected: Vec<TrackId> = q
        .all_tracks()
        .filter(|t| t.items.iter().any(|it| chosen.contains(&it.id.0) || it.link.is_some_and(|l| links.contains(&l))))
        .map(|t| t.id)
        .collect();
    let held: Vec<TrackId> = q.all_tracks().filter(|t| t.sync_lock && !affected.contains(&t.id)).map(|t| t.id).collect();
    s.edit_sequence("Löschen", |q, _, _| {
        for t in q.all_tracks_mut().filter(|t| held.contains(&t.id)) {
            t.sync_lock = false;
        }
        Ok(())
    })?;
    let deleted = s.execute("edit.rippleDelete", json!({"clips": chosen}));
    s.edit_sequence("Löschen", |q, _, _| {
        for t in q.all_tracks_mut().filter(|t| held.contains(&t.id)) {
            t.sync_lock = true;
        }
        // nothing outlasts the picture (music would play over black)
        let end = q.video_tracks.first().and_then(|t| t.items.iter().map(|it| it.end()).max());
        if let Some(end) = end.filter(|e| e.0 > 0) {
            for t in q.all_tracks_mut().filter(|t| !affected.contains(&t.id)) {
                for it in t.items.iter_mut().filter(|it| it.start < end && it.end() > end) {
                    it.duration = end - it.start;
                }
            }
        }
        Ok(())
    })?;
    deleted.map(|_| ())
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
            let errors = s
                .edit_group("Medien hinzufügen", |s| {
                    let imported = s.execute("file.import", json!({"paths": paths}))?;
                    let items = ids(&imported, "items");
                    if items.is_empty() {
                        let why: Vec<String> = imported["errors"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|e| e.as_str().map_or_else(|| e.to_string(), str::to_string))
                            .collect();
                        return Err(filmcraft_engine::EngineError::Other(if why.is_empty() {
                            "keine dieser Dateien ließ sich lesen".into()
                        } else {
                            why.join("; ")
                        }));
                    }
                    // clips of another shape fit into the frame, as in CapCut (FilmCraft's own
                    // default places them at 100 %); the preference is the user's again afterwards
                    let saved = std::mem::replace(&mut s.prefs.media.default_media_scaling, "setToFrameSize".into());
                    let placed = place_media(s, &items, at_playhead);
                    s.prefs.media.default_media_scaling = saved;
                    placed?;
                    Ok(imported.get("errors").cloned().unwrap_or(json!([])))
                })
                .map_err(|e| e.to_string())?;
            let mut st = state(s)?;
            // files that could not be read are said, the others are in
            if errors.as_array().is_some_and(|e| !e.is_empty()) {
                st["errors"] = errors;
            }
            Ok(st)
        }
        "br.video.split" => {
            let mut q = json!({});
            if let Some(sec) = opt_f64(id, p, "seconds")? {
                q["time"] = json!((sec * filmcraft_time::TICKS_PER_SECOND as f64).round() as i64);
            }
            run(s, "sequence.addEdit", q)
        }
        "br.video.delete" => {
            let chosen = chosen_clips(s, p);
            if chosen.is_empty() {
                return Err(err(id, "erst die Clips auswählen, die weg sollen"));
            }
            s.edit_group("Löschen", |s| ripple_delete(s, &chosen)).map_err(|e| e.to_string())?;
            state(s)
        }
        "br.video.speed" => {
            let speed = opt_f64(id, p, "speed")?.ok_or_else(|| err(id, "needs `speed`"))?;
            if !(0.01..=100.0).contains(&speed) {
                return Err(err(id, "`speed` must be between 0.01 and 100"));
            }
            let chosen = chosen_clips(s, p);
            if chosen.is_empty() {
                return Err(err(id, "erst die Clips auswählen, deren Tempo sich ändern soll"));
            }
            // a reversed clip stays reversed (FilmCraft's command sets the direction too)
            let q = s.active_sequence().ok_or_else(|| err(id, "noch kein Video"))?;
            let (backward, forward): (Vec<u64>, Vec<u64>) =
                chosen.iter().partition(|c| q.find_item(filmcraft_project::ClipId(**c)).is_some_and(|(_, it)| it.reverse));
            s.edit_group(&format!("Tempo {speed}×"), |s| {
                for (clips, reverse) in [(forward, false), (backward, true)] {
                    if !clips.is_empty() {
                        s.execute("clip.speedDuration", json!({"clips": clips, "speed": speed * 100.0, "ripple": true, "reverse": reverse}))?;
                    }
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
            state(s)
        }
        "br.video.detachAudio" => {
            // only ever unlinks (FilmCraft's clip.link toggles, and links unlinked clips)
            let chosen = chosen_clips(s, p);
            if chosen.is_empty() {
                return Err(err(id, "erst die Clips auswählen, deren Ton getrennt werden soll"));
            }
            let linked = s
                .active_sequence()
                .is_some_and(|q| q.all_tracks().flat_map(|t| t.items.iter()).any(|it| chosen.contains(&it.id.0) && it.link.is_some()));
            if !linked {
                return Err(err(id, "Bild und Ton dieser Clips sind schon getrennt"));
            }
            let n = s
                .edit_sequence("Ton trennen", |q, _, _| {
                    let links: Vec<u64> =
                        q.all_tracks().flat_map(|t| t.items.iter()).filter(|it| chosen.contains(&it.id.0)).filter_map(|it| it.link).collect();
                    let mut n = 0;
                    for it in q.all_tracks_mut().flat_map(|t| t.items.iter_mut()) {
                        if it.link.is_some_and(|l| links.contains(&l)) {
                            it.link = None;
                            n += 1;
                        }
                    }
                    Ok(n)
                })
                .map_err(|e| e.to_string())?;
            Ok(json!({"detached": n}))
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
                    return Err(filmcraft_engine::EngineError::Other(format!("{path} ließ sich nicht lesen")));
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
                    // lowered under any sound of the video, not only under clips typed as dialogue
                    // (FilmCraft's default; the video's own sound has no type)
                    let ids: Vec<filmcraft_project::ClipId> = clips.iter().copied().map(filmcraft_project::ClipId).collect();
                    s.edit_sequence("Musik", |q, _, _| {
                        for it in q.all_tracks_mut().flat_map(|t| t.items.iter_mut()).filter(|it| ids.contains(&it.id)) {
                            if let Some(e) = it.essential.as_mut() {
                                e.ducking.enabled = true;
                                e.ducking.against_untyped = true;
                            }
                        }
                        Ok(())
                    })?;
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
                return Err(err(id, "keine Videoclips, auf die ein Filter passt"));
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
                return Err(err(id, "noch kein Video: erst br.video.new oder br.video.add"));
            }
            let (w, h) = format.size();
            s.edit_group(&format!("Format {}", format.key()), |s| {
                // titles, graphics and nested sequences move and scale with the frame
                s.execute("sequence.settings", json!({"width": w, "height": h, "scaleMotion": true}))?;
                let seq = s.execute("sequence.inspect", json!({}))?;
                // an animated (keyframed) position or scale is the user's: it stays
                let animated = |c: &u64| {
                    s.active_sequence()
                        .and_then(|q| q.find_item(filmcraft_project::ClipId(*c)))
                        .is_some_and(|(_, it)| it.effect("motion").is_some_and(|m| m.params.values().any(|p| !p.keyframes.is_empty())))
                };
                let clips: Vec<u64> = media_video_clips(s, &seq).into_iter().filter(|c| !animated(c)).collect();
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
                return Err(err(id, "kein Schnitt zwischen zwei Clips ohne Übergang"));
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
            // waits for the file unless asked not to (the window runs it in the background)
            let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(true);
            // written under a hidden name first, the file appears only when it is complete (a
            // download page or a player never sees half an MP4; a cancelled one leaves nothing)
            let part = part_path(&path);
            let _ = std::fs::remove_file(&part);
            // captions on screen go into the picture (an MP4 has no other place for them)
            let burn = p
                .get("burnCaptions")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| s.active_sequence().is_some_and(|q| q.caption_tracks.iter().any(|t| t.enabled && !t.captions.is_empty())));
            // the whole video, whatever In and Out marks are set
            let range = p.get("range").cloned().unwrap_or(json!("entire"));
            let Some(part_str) = part.to_str() else {
                return Err(err(id, format!("{}: der Ordnername ist kein gültiger Text (UTF-8)", path.display())));
            };
            let mut r =
                run(s, "export.quick", json!({"preset": preset.filmcraft, "path": part_str, "wait": wait, "burnCaptions": burn, "range": range}))?;
            r["path"] = json!(path.to_string_lossy());
            r["part"] = json!(part_str);
            if wait {
                // a waited-for export that failed is a failure (FilmCraft keeps the error in its job)
                let job = r.get("job").and_then(Value::as_u64).unwrap_or(0);
                match settle_export(s, job, &part, &path) {
                    Some(Ok(())) | None => {}
                    Some(Err(e)) => return Err(err(id, e)),
                }
            }
            Ok(r)
        }
        "br.video.state" => state(s),
        "br.video.commands" => {
            let query = p.get("query").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let list: Vec<Value> = filmcraft_engine::command_specs()
                .iter()
                .filter(|c| query.is_empty() || c.id.to_lowercase().contains(&query) || c.label.to_lowercase().contains(&query))
                .map(|c| json!({"id": c.id, "label": c.label, "menu": c.menu, "params": c.params}))
                .collect();
            Ok(json!(list))
        }
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
