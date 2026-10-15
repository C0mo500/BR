//! BR's video commands on FilmCraft's engine: CapCut's one-tap results for the Schnitt page,
//! the CLI and `br mcp`. Each command is one undo step (FilmCraft's `edit_group`), built from
//! FilmCraft's own commands, so everything it does can be refined in FilmCraft's panels.
//!
//! The video is built CapCut's way on FilmCraft's sequence: a magnetic main track ([`magnet`],
//! [`main_track`]) and titles, overlays, sound effects and captions attached to its clips
//! ([`anchors`]), also when the edit is made with FilmCraft's own tools ([`reconcile`]).
//!
//! | Command | What it does |
//! |---|---|
//! | `br.video.new` | a new video in a social format (9:16, 16:9, 1:1, 4:5) |
//! | `br.video.add` | import files and put them one after another on the main track (at its end or a cut) |
//! | `br.video.place` | media where they are dropped: main track at a cut, overlay, sound, captions |
//! | `br.video.split` | cut the selected clip (else the main clip) at the playhead |
//! | `br.video.delete` | delete the selected clips; the main track closes up, what hung on them goes too |
//! | `br.video.speed` | play the selected clips faster or slower, the rest moves up or on |
//! | `br.video.move` | reorder the main track, or move a clip onto or off it |
//! | `br.video.trim` | move a clip's edge (main clips ripple) |
//! | `br.video.duplicate` | a copy of the selected clips right after them |
//! | `br.video.pack` | close the main track's gaps |
//! | `br.video.magnet`, `br.video.linkage` | CapCut's switches: magnetic main track, attached clips follow |
//! | `br.video.anchors`, `br.video.anchor` | what hangs on which main clip; detach or attach |
//! | `br.video.detachAudio` | separate picture and sound of the selected clips |
//! | `br.video.text` | a title at the playhead |
//! | `br.video.captions` | captions from an SRT/VTT/… file |
//! | `br.video.autoCaptions` | captions from the speech in the video (Whisper, see [`speech`]) |
//! | `br.video.music` | a music file on its own track, as long as the video, lowered under speech |
//! | `br.video.filter` | a look on the selected clips (all without a selection), replacing the last |
//! | `br.video.transform`, `br.video.fit` | where a picture is in the frame; fit, fill, reset |
//! | `br.video.background` | fill the frame behind the main clips: blurred, a colour or a picture |
//! | `br.video.reframe` | pictures of another shape fill the frame and follow their subject |
//! | `br.video.curve`, `br.video.freeze`, `br.video.reverse` | speed curves (baked), freeze frames, backwards |
//! | `br.video.beats` | the music's beats as markers the timelines snap to |
//! | `br.video.template.*` | video templates: list, new (placeholders), fill with media, save the video as one |
//! | `br.video.format` | another shape (9:16, 16:9, 1:1, 4:5); the clips fit or fill it |
//! | `br.video.transition` | the default transition (a dissolve) at the cuts of the selected clips |
//! | `br.video.export` | an MP4 in a preset for TikTok/Reels/Shorts, YouTube, … |
//! | `br.video.inspect` | the inspector's rows of the selected clip, caption or transition (Basis, Mehr) |
//! | `br.video.set`, `br.video.keyframe` | set a row or any effect parameter; keyframes at the playhead |
//! | `br.video.effect`, `br.video.schema` | add, switch off, reset or remove effects; FilmCraft's effect schemas |
//! | `br.video.state` | the video at a glance |

use serde_json::{Value, json};

pub use filmcraft_engine::Session;

pub mod anchors;
pub mod beats;
pub mod frame;
pub mod inspect;
pub mod magnet;
pub mod main_track;
pub mod place;
pub mod reconcile;
pub mod speech;
pub mod speed;
pub mod template;

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
        params: r#"{"paths":[str],"at":"end|playhead"|seconds="end"}"#,
        summary: "Import files and put them one after another on the main track: at its end, or at the cut nearest to the playhead (or `at` seconds); nothing is cut in two",
    },
    CommandInfo {
        id: "br.video.place",
        params: r#"{"paths":[str]?,"items":[id]?,"target":"auto|main|overlay|audio"="auto","seconds":f64?,"track":"V2|A3|main"|id?,"position":[x,y]?,"sourceIn":f64?,"duration":f64?}"#,
        summary: "Put files (or project items) where they were dropped: a picture onto the main track at the cut nearest `seconds` (default: the playhead), above it as an overlay on a free track at `position` (0…1 of the picture), sound alone on a free sound track (long: music, short: a sound effect), subtitles as captions; one undo step",
    },
    CommandInfo {
        id: "br.video.split",
        params: r#"{"clip":id?,"seconds":f64?}"#,
        summary: "Cut a clip (the selected one, else the main clip) at the playhead (or `seconds`); titles and music stay whole",
    },
    CommandInfo {
        id: "br.video.delete",
        params: r#"{"clips":[id]?}"#,
        summary: "Delete the selected clips; the main track closes up (magnet) and what hangs on a deleted main clip goes too",
    },
    CommandInfo {
        id: "br.video.speed",
        params: r#"{"speed":f64,"clips":[id]?}"#,
        summary: "Speed factor of the selected clips (2 = twice as fast); the main track moves up or on, attached clips follow",
    },
    CommandInfo {
        id: "br.video.move",
        params: r#"{"clip":id,"index":n?,"seconds":f64?,"track":"main|overlay|V2|A3|…"|id?}"#,
        summary: "Move a clip: to place `index` of the main track (or the cut nearest `seconds`), off it onto an overlay track, or any clip to `seconds` on a track",
    },
    CommandInfo {
        id: "br.video.trim",
        params: r#"{"clip":id,"edge":"in|out","seconds":f64|"frames":n}"#,
        summary: "Move a clip's edge (+ later, − earlier); a main clip ripples with the magnet on, its attached clips follow",
    },
    CommandInfo {
        id: "br.video.duplicate",
        params: r#"{"clips":[id]?}"#,
        summary: "Duplicate clips: a main clip's copy comes right after it (the rest moves on), any other clip's right after it on a free track",
    },
    CommandInfo { id: "br.video.pack", params: "{}", summary: "Close the gaps of the main track; attached clips go along" },
    CommandInfo {
        id: "br.video.magnet",
        params: r#"{"on":bool}"#,
        summary: "The magnetic main track (CapCut): no gaps, deleting closes up, inserting pushes on. Turning it on closes the gaps",
    },
    CommandInfo {
        id: "br.video.linkage",
        params: r#"{"on":bool}"#,
        summary: "Titles, overlays, sound effects and captions follow the main clip they start on (off: FilmCraft's own behaviour)",
    },
    CommandInfo { id: "br.video.anchors", params: "{}", summary: "What hangs on which main clip (clip, kind, parent, offset in seconds)" },
    CommandInfo {
        id: "br.video.anchor",
        params: r#"{"clips":[id]?,"to":id|"auto"|null}"#,
        summary: "Attach clips to a main clip (`to`), detach them (null) or let them follow the clip they start on again (auto)",
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
        params: r#"{"language":"de|en|…|auto"?,"model":"whisper-tiny|whisper-base|whisper-small"?,"wait":bool=true}"#,
        summary: "Captions from the speech in the video (Whisper; the model is downloaded the first time). wait:false (window, br mcp) runs it in the background: see br.video.jobs",
    },
    CommandInfo {
        id: "br.video.transform",
        params: r#"{"clips":[id]?,"position":[x,y]?,"scale":f64?,"rotation":f64?,"anchor":[x,y]?,"opacity":f64?,"keyframe":bool?,"gesture":str?}"#,
        summary: "Where the selected pictures are: position (frame pixels), scale (%), rotation (degrees), anchor, opacity; at the playhead when they animate (keyframe:true writes one). One undo step; calls with the same `gesture` are one",
    },
    CommandInfo {
        id: "br.video.fit",
        params: r#"{"clips":[id]?,"mode":"fit|fill|reset"="fit"}"#,
        summary: "Fit the selected pictures into the frame, fill it with them, or put their Motion back",
    },
    CommandInfo {
        id: "br.video.background",
        params: r##"{"mode":"blur|color|image|none"="blur","clips":[id]?,"amount":0..100=50,"color":"#rrggbb"?,"path":str?}"##,
        summary: "Fill the frame behind the main clips (chosen, else selected, else all): the clip itself blurred, a colour or a picture, on a track below the main track that follows the clip; none takes it away",
    },
    CommandInfo {
        id: "br.video.reframe",
        params: r#"{"clips":[id]?,"on":bool=true,"speed":"slow|default|fast"="default"}"#,
        summary: "Pictures of another shape than the frame fill it and follow their subject (Auto Reframe); on:false: they fit again",
    },
    CommandInfo {
        id: "br.video.curve",
        params: r#"{"preset":"montage|held|bullet|sprung|blitzRein|blitzRaus|none","clips":[id]?}"#,
        summary: "A speed curve on the selected clips (else the main clip at the playhead), baked into pieces of constant speed (one undo step); the video moves up or on; none puts the clip back together",
    },
    CommandInfo {
        id: "br.video.freeze",
        params: r#"{"seconds":f64=3,"at":f64?,"clip":id?}"#,
        summary: "Freeze the picture at the playhead (or `at`) for `seconds`; the rest of the clip and the video move on",
    },
    CommandInfo {
        id: "br.video.reverse",
        params: r#"{"clips":[id]?}"#,
        summary: "Play the selected clips backwards (again forwards); a speed curve goes along",
    },
    CommandInfo {
        id: "br.video.beats",
        params: r#"{"clip":id?,"mode":"beats|bars|half|onsets|none"="beats"}"#,
        summary: "Markers on the beats of the music (or `clip`'s sound): every beat, every bar, every half beat or every hit; the timelines snap to them; none removes them",
    },
    CommandInfo {
        id: "br.video.template.list",
        params: r#"{"dir":str?}"#,
        summary: "The video templates: BR's and the user's (id, name, format, slots, length, whether they cut on beats); `dir`: another folder of the user's",
    },
    CommandInfo {
        id: "br.video.template.new",
        params: r#"{"template":id|name|path,"music":str?,"dir":str?}"#,
        summary: "A new video from a template: grey placeholders, titles, transitions, look; with `music` under it (slots in beats cut on its beats). One undo step",
    },
    CommandInfo {
        id: "br.video.template.fill",
        params: r#"{"paths":[str]?,"items":[id]?,"fit":"fill|fit"="fill","all":bool?}"#,
        summary: "Put media into the template's empty placeholders, in order (a clip too short for its slot plays slower); the template's curves and background follow. One undo step",
    },
    CommandInfo {
        id: "br.video.template.save",
        params: r#"{"name":str,"description":str?,"dir":str?}"#,
        summary: "Keep the video's shape as a template of your own: format, clip lengths with curves and transitions, titles, background",
    },
    CommandInfo {
        id: "br.video.format",
        params: r#"{"format":"9:16|16:9|1:1|4:5","fit":"fit|fill|reframe"=fit}"#,
        summary: "Change the video's shape; its clips fit into it, fill it, or fill it following their subject (reframe); backgrounds always fill it",
    },
    CommandInfo {
        id: "br.video.transition",
        params: r#"{"clips":[id]?,"effect":"cross_dissolve|dip_to_black|…"?}"#,
        summary: "A transition (default: a dissolve) at the cuts between the selected clips (every cut of the main track without a selection)",
    },
    CommandInfo {
        id: "br.video.inspect",
        params: r#"{"clip":id?,"caption":id?,"transition":id?,"seconds":f64?,"level":"basis|mehr"="basis"}"#,
        summary: "What the selected clip (caption, transition) is made of, as the inspector shows it: Basis rows (key, German label, type, value, range, keyframes) per kind; level mehr adds every effect with every parameter and its keyframes",
    },
    CommandInfo {
        id: "br.video.set",
        params: r#"{"row":"transform.scale|adjust.exposure|audio.volume|text.text|…"?,"effect":index|id|"adjust"?,"param":str?,"value":any,"clip":id?,"caption":id?,"transition":id?,"seconds":f64?,"keyframe":bool?,"gesture":str?}"#,
        summary: "Set an inspector row (see br.video.inspect) or any effect parameter, checked against its schema; keyframe:true writes a keyframe at the playhead (or `seconds`); calls with the same `gesture` are one undo step",
    },
    CommandInfo {
        id: "br.video.keyframe",
        params: r#"{"row":key|[key]?,"effect":index|id|"adjust"?,"param":str?,"op":"toggle|add|remove|clear|ease|prev|next"="toggle","seconds":f64?,"interpolation":"linear|hold|bezier|easeIn|easeOut"?,"all":bool?}"#,
        summary: "Keyframes of rows or parameters at the playhead: set or remove one (several rows: one step), stop the animation (clear), ease them, or go to the previous/next one",
    },
    CommandInfo {
        id: "br.video.effect",
        params: r#"{"op":"add|remove|enable|disable|reset","effect":id|name|index|"adjust","param":str?,"clip":id?,"on":id?}"#,
        summary: "Add an effect to the selected clip (an audio effect goes on its sound), or remove, switch off/on or reset one (`on`: the clip it is on, e.g. the linked sound)",
    },
    CommandInfo {
        id: "br.video.schema",
        params: r#"{"effect":id|name?,"kind":"video|audio|videoTransition|audioTransition"?,"detail":bool?}"#,
        summary: "FilmCraft's effects: one effect's parameters (type, range, slider range, unit, default, group, animatable) or the list",
    },
    CommandInfo { id: "br.video.state", params: "{}", summary: "The video at a glance: format, length, clips, playhead" },
    CommandInfo {
        id: "br.video.jobs",
        params: "{}",
        summary: "The exports (and, in a window or br mcp, the caption recognition) running or done: progress, errors",
    },
    CommandInfo { id: "br.video.cancel", params: r#"{"job":id?,"captions":bool?}"#, summary: "Stop an export (`job`) or the caption recognition" },
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
    // files that could not be read stay in the answer
    let errors = st.get("errors").cloned();
    for subs in paths.iter().filter(|p| kind(p) == "caption") {
        execute(s, "br.video.captions", &json!({"path": subs}))?;
        st = state(s)?;
    }
    if let Some(e) = errors {
        st["errors"] = e;
    }
    Ok(st)
}

/// FilmCraft's export jobs of this session: `{job, label, progress (0..1), etaSeconds, done,
/// error}`.
pub fn jobs(s: &Session) -> Value {
    Value::Array(
        s.jobs
            .iter()
            .map(|j| {
                let v = j.to_json();
                let error = v["result"].get("error").cloned().unwrap_or(Value::Null);
                json!({"job": j.id, "label": j.label, "progress": v["progress"], "etaSeconds": v["etaSeconds"], "done": v["finished"], "error": error})
            })
            .collect(),
    )
}

/// The hidden file an export to `path` is written to until it is complete.
pub fn part_path(path: &std::path::Path) -> std::path::PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Video.mp4".into());
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name.as_str(), "mp4"));
    path.with_file_name(format!(".{stem}.br-part.{ext}"))
}

/// When an export ends, its part file becomes the export (or is removed when it failed or was
/// cancelled). A thread watches the job, so this happens whether or not anyone asks again.
fn watch_export(s: &Session, job: u64, part: std::path::PathBuf, path: std::path::PathBuf) {
    let Some(result) = s.jobs.iter().find(|j| j.id == job).map(|j| j.result.clone()) else { return };
    let shown = path.display().to_string();
    let watched = std::thread::Builder::new().name("br-export-watch".into()).spawn(move || {
        loop {
            let done = result.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|r| r.is_ok());
            match done {
                Some(true) => {
                    if let Err(e) = std::fs::rename(&part, &path) {
                        log::error!("export {}: {e}", path.display());
                    }
                    return;
                }
                Some(false) => {
                    let _ = std::fs::remove_file(&part);
                    return;
                }
                None => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
    });
    if let Err(e) = watched {
        log::error!("export {shown}: no thread to finish it: {e}");
    }
}

/// An export job's end: `None` while it runs (or its file is still being put in place), then
/// `Ok` when the file is there, `Err` when it failed or was cancelled.
pub fn settle_export(s: &Session, job: u64, part: &std::path::Path, path: &std::path::Path) -> Option<Result<(), String>> {
    let result = match s.jobs.iter().find(|j| j.id == job) {
        Some(j) => j.result.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|r| r.as_ref().map(|_| ()).map_err(Clone::clone))?,
        None if part.exists() => return None,
        None if path.exists() => Ok(()),
        None => Err("abgebrochen".into()),
    };
    // the watcher has not moved the file yet
    if part.exists() {
        return None;
    }
    Some(match result {
        Ok(()) if path.exists() => Ok(()),
        Ok(()) => Err(format!("{} wurde nicht geschrieben", path.display())),
        Err(e) => Err(if e == "cancelled" { "abgebrochen".into() } else { e }),
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
/// is none): one after another at the end of the main track, or inserted at the cut of the main
/// track nearest to `at` (nothing is cut in two). Pictures go on the main track with their sound;
/// sound alone on a free sound track.
pub(crate) fn place_media(s: &mut Session, items: &[u64], at: Option<filmcraft_time::Tick>) -> filmcraft_engine::Result<()> {
    if s.active_sequence().is_none() {
        // the pictures make the video, in the first one's shape; sound alone goes under them
        let (pictures, sounds): (Vec<u64>, Vec<u64>) = items.iter().partition(|i| s.project.source_streams(filmcraft_project::ItemId(**i)).0);
        if pictures.is_empty() {
            s.execute("file.newSequence", json!({"name": "Video", "fromItems": items}))?;
            return prepare(s);
        }
        s.execute("file.newSequence", json!({"name": "Video", "fromItems": pictures}))?;
        prepare(s)?;
        if !sounds.is_empty() {
            s.set_playhead(filmcraft_time::Tick::ZERO);
            magnet::insert(s, &sounds, None)?;
        }
        return Ok(());
    }
    magnet::insert(s, items, at)?;
    Ok(())
}

/// A video BR has just made: CapCut's magnetic main track and linkage on, so every track but the
/// main one (and its sound) follows BR's anchors instead of Premiere's sync lock (titles, overlays
/// and music are moved by their main clip, never cut by an insert). Part of the caller's undo
/// step. A sequence BR did not make keeps FilmCraft's behaviour until the switches are turned on.
fn prepare(s: &mut Session) -> filmcraft_engine::Result<()> {
    if s.active_sequence().is_some_and(main_track::is_marked) {
        return Ok(());
    }
    s.edit_sequence("Video", |q, _, _| {
        main_track::capcut(q);
        Ok(())
    })
}

/// The clips a command works on: `clips`, else the selection; never a background (it belongs to
/// its main clip, which is taken instead).
pub(crate) fn chosen_or_selected(s: &Session, p: &Value) -> Vec<filmcraft_project::ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let mut out: Vec<filmcraft_project::ClipId> = Vec::new();
    for c in chosen_clips(s, p).into_iter().map(filmcraft_project::ClipId) {
        let c = match q.find_item(c) {
            Some((_, it)) if main_track::is_background(it) => match main_track::main_of(q, c) {
                Some(m) => m,
                None => continue,
            },
            Some(_) => c,
            None => continue,
        };
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// The index of a video track above the main track that is free from `t` for `len` (a new one on
/// top when none is), for a title or an overlay.
fn overlay_track(s: &mut Session, t: filmcraft_time::Tick, len: filmcraft_time::Tick) -> filmcraft_engine::Result<usize> {
    let q = s.active_sequence().ok_or(filmcraft_engine::EngineError::NoSequence)?;
    let main = main_track::main_track_id(q);
    let from = q.video_tracks.iter().position(|tr| Some(tr.id) == main).map_or(0, |i| i + 1);
    let range = filmcraft_time::TimeRange::new(t, len);
    let free = q
        .video_tracks
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, tr)| !tr.locked && !main_track::is_background_track(tr) && tr.items.iter().all(|i| !i.range().overlaps(&range)))
        .map(|(i, _)| i);
    if let Some(i) = free {
        return Ok(i);
    }
    s.execute("sequence.addTracks", json!({"video": 1, "audio": 0}))?;
    let n = s.active_sequence().map(|q| q.video_tracks.len()).unwrap_or(1);
    Ok(n - 1)
}

/// Seconds as ticks.
fn ticks(sec: f64) -> filmcraft_time::Tick {
    filmcraft_time::Tick((sec * filmcraft_time::TICKS_PER_SECOND as f64).round() as i64)
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
pub(crate) fn ids(v: &Value, key: &str) -> Vec<u64> {
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

fn playhead(seq: &Value) -> i64 {
    seq.get("playhead").and_then(Value::as_i64).unwrap_or(0)
}

/// Run one of BR's video commands, or any of FilmCraft's (`timeline.razor`, `lumetri.setLook`, …).
/// For a session that is also edited otherwise (a window), use [`execute_with`] and its watch.
pub fn execute(s: &mut Session, id: &str, p: &Value) -> Result<Value, String> {
    let mut w = reconcile::Watch::new(s);
    execute_with(s, &mut w, id, p)
}

/// [`execute`] for a session with a [`reconcile::Watch`]: edits made since with FilmCraft's own
/// tools are followed first (see [`reconcile`]), a FilmCraft command's edit is followed in its
/// step, and BR's own steps are never taken for FilmCraft's.
pub fn execute_with(s: &mut Session, w: &mut reconcile::Watch, id: &str, p: &Value) -> Result<Value, String> {
    let p = if p.is_null() { &json!({}) } else { p };
    reconcile::catch_up(s, w);
    if !is_video(id) {
        // FilmCraft's own command: what hangs on the main clips it changed follows, in its step
        let r = run(s, id, p.clone());
        reconcile::catch_up(s, w);
        return r;
    }
    // a dynamic trim running in FilmCraft ends first, as its own step (not inside BR's)
    if s.trim_play.dynamic.is_some() {
        let _ = s.execute("trim.shuttleStop", json!({}));
        reconcile::catch_up(s, w);
    }
    let before = s.project.clone();
    let tracks = reconcile::track_ids(s);
    let r = execute_br(s, id, p);
    if r.is_ok() {
        reconcile::unlock_new_tracks(s, &tracks);
    }
    // BR edits outside FilmCraft's execute: playback and autosave hear of them here
    if !std::sync::Arc::ptr_eq(&before, &s.project) {
        s.sync_persistence();
        s.previews.live.publish_project(s.project.clone());
    }
    w.look(s);
    r
}

pub(crate) fn execute_br(s: &mut Session, id: &str, p: &Value) -> Result<Value, String> {
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
            s.edit_group("Neues Video", |s| {
                s.execute("file.newSequence", json!({"name": name, "width": w, "height": h, "fps": fps}))?;
                prepare(s)
            })
            .map_err(|e| e.to_string())?;
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
            let at = match p.get("at") {
                None | Some(Value::Null) => None,
                Some(Value::String(w)) if w == "end" => None,
                Some(Value::String(w)) if w == "playhead" => Some(s.playhead()),
                Some(v) if v.as_f64().is_some_and(|x| x.is_finite() && x >= 0.0) => v.as_f64().map(ticks),
                Some(other) => return Err(err(id, format!("`at` is end, playhead or seconds, not `{other}`"))),
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
                    let placed = place_media(s, &items, at);
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
            let q = s.active_sequence().ok_or_else(|| err(id, "noch kein Video"))?;
            let t = opt_f64(id, p, "seconds")?.map(ticks).unwrap_or_else(|| s.playhead());
            // the chosen clip, else the selected one under the cut, else the main clip there:
            // never every track (titles and music stay whole)
            let under = |c: &filmcraft_project::ClipId| q.find_item(*c).is_some_and(|(_, it)| it.start < t && t < it.end());
            let clip = match p.get("clip").and_then(Value::as_u64) {
                Some(c) => Some(filmcraft_project::ClipId(c)),
                None => chosen_or_selected(s, &json!({})).into_iter().find(under).or_else(|| magnet::main_clip_at(q, t).filter(under)),
            };
            let clip = clip.ok_or_else(|| err(id, "hier liegt kein Clip, der sich teilen ließe"))?;
            magnet::split(s, clip, t).map_err(|e| err(id, e))?;
            state(s)
        }
        "br.video.delete" => {
            let chosen = chosen_or_selected(s, p);
            if chosen.is_empty() {
                return Err(err(id, "erst die Clips auswählen, die weg sollen"));
            }
            let r = magnet::delete(s, &chosen).map_err(|e| err(id, e))?;
            let mut st = state(s)?;
            st["deleted"] = r["deleted"].clone();
            st["followed"] = r["followed"].clone();
            Ok(st)
        }
        "br.video.speed" => {
            let speed = opt_f64(id, p, "speed")?.ok_or_else(|| err(id, "needs `speed`"))?;
            if !(0.01..=100.0).contains(&speed) {
                return Err(err(id, "`speed` must be between 0.01 and 100"));
            }
            let chosen = chosen_or_selected(s, p);
            if chosen.is_empty() {
                return Err(err(id, "erst die Clips auswählen, deren Tempo sich ändern soll"));
            }
            magnet::speed(s, &chosen, speed).map_err(|e| err(id, e))?;
            state(s)
        }
        "br.video.move" => {
            let clip = p.get("clip").and_then(Value::as_u64).map(filmcraft_project::ClipId).ok_or_else(|| err(id, "needs `clip`"))?;
            let q = s.active_sequence().ok_or_else(|| err(id, "noch kein Video"))?;
            let (own, start) = q.find_item(clip).map(|(t, it)| (t, it.start)).ok_or_else(|| err(id, format!("no clip {}", clip.0)))?;
            let main = main_track::main_track_id(q);
            let index = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
            let at = opt_f64(id, p, "seconds")?.map(|x| ticks(x.max(0.0)));
            // the track asked for: None = the main track, Some(None) = a free overlay track
            let track: Option<Option<filmcraft_project::TrackId>> = match p.get("track") {
                None | Some(Value::Null) if index.is_some() => None,
                None | Some(Value::Null) => {
                    if Some(own) == main {
                        None
                    } else {
                        Some(Some(own))
                    }
                }
                Some(Value::String(t)) if t == "main" => None,
                Some(Value::String(t)) if t == "overlay" || t == "new" => Some(None),
                Some(Value::String(t)) => {
                    let n = t.get(1..).and_then(|n| n.parse::<usize>().ok()).filter(|n| *n > 0);
                    let list = match t.chars().next() {
                        Some('V') => &q.video_tracks,
                        Some('A') => &q.audio_tracks,
                        _ => return Err(err(id, format!("`track` is main, overlay, V2, A3, … or a track id, not `{t}`"))),
                    };
                    let tid = n.and_then(|n| list.get(n - 1)).map(|t| t.id).ok_or_else(|| err(id, format!("no track {t}")))?;
                    if Some(tid) == main { None } else { Some(Some(tid)) }
                }
                Some(v) => {
                    let tid = filmcraft_project::TrackId(v.as_u64().ok_or_else(|| err(id, "`track` is main, overlay, V2, A3, … or a track id"))?);
                    if Some(tid) == main { None } else { Some(Some(tid)) }
                }
            };
            let to = match (track, index, at) {
                (None, Some(i), _) => magnet::To::Index(i),
                (None, None, Some(t)) => magnet::To::Main(t),
                (None, None, None) => return Err(err(id, "needs `index` or `seconds`: where on the main track")),
                (Some(t), _, at) => magnet::To::Track(t, at.unwrap_or(start)),
            };
            let r = magnet::move_clip(s, clip, to).map_err(|e| err(id, e))?;
            let mut st = state(s)?;
            st["followed"] = r;
            Ok(st)
        }
        "br.video.trim" => {
            let clip = p.get("clip").and_then(Value::as_u64).map(filmcraft_project::ClipId).ok_or_else(|| err(id, "needs `clip`"))?;
            let edge = match p.get("edge").and_then(Value::as_str) {
                Some("in" | "start") => filmcraft_edit::Edge::In,
                Some("out" | "end") => filmcraft_edit::Edge::Out,
                _ => return Err(err(id, "`edge` is in or out")),
            };
            let q = s.active_sequence().ok_or_else(|| err(id, "noch kein Video"))?;
            let delta = match (opt_f64(id, p, "seconds")?, p.get("frames").and_then(Value::as_i64)) {
                (Some(x), _) => ticks(x),
                (None, Some(f)) => filmcraft_time::Tick(q.settings.frame_rate.frame_duration().0 * f),
                _ => return Err(err(id, "needs `seconds` (or `frames`): how far the edge moves, + later, − earlier")),
            };
            let r = magnet::trim(s, clip, edge, delta).map_err(|e| err(id, e))?;
            let mut st = state(s)?;
            st["trimmed"] = r;
            Ok(st)
        }
        "br.video.duplicate" => {
            let chosen = chosen_or_selected(s, p);
            if chosen.is_empty() {
                return Err(err(id, "erst die Clips auswählen, die doppelt sein sollen"));
            }
            let r = magnet::duplicate(s, &chosen).map_err(|e| err(id, e))?;
            let mut st = state(s)?;
            st["copies"] = r["copies"].clone();
            Ok(st)
        }
        "br.video.pack" => {
            if s.active_sequence().is_none() {
                return Err(err(id, "noch kein Video"));
            }
            magnet::pack(s).map_err(|e| err(id, e))?;
            state(s)
        }
        "br.video.magnet" | "br.video.linkage" => {
            let on = p.get("on").and_then(Value::as_bool).ok_or_else(|| err(id, "needs `on` (true or false)"))?;
            if s.active_sequence().is_none() {
                return Err(err(id, "noch kein Video"));
            }
            let key = if id == "br.video.magnet" { "magnet" } else { "linkage" };
            let label = match (key, on) {
                ("magnet", true) => "Magnet an",
                ("magnet", false) => "Magnet aus",
                (_, true) => "Verknüpfung an",
                _ => "Verknüpfung aus",
            };
            s.edit_group(label, |s| {
                s.edit_sequence(label, |q, _, _| {
                    main_track::set_flag(q, key, on);
                    Ok(())
                })?;
                // the magnet closes the main track's gaps when it comes on, as in CapCut
                if key == "magnet" && on {
                    magnet::pack(s)?;
                }
                Ok(())
            })
            .map_err(|e| err(id, e))?;
            state(s)
        }
        "br.video.anchors" => {
            let q = s.active_sequence().ok_or_else(|| err(id, "noch kein Video"))?;
            let list: Vec<Value> = anchors::derive(q)
                .iter()
                .map(|a| {
                    let kind = match a.kind {
                        anchors::Kind::Overlay => "overlay",
                        anchors::Kind::Sound => "sound",
                        anchors::Kind::Caption => "caption",
                    };
                    json!({
                        "clip": a.child.0,
                        "kind": kind,
                        "parent": a.parent_clip().map(|c| c.0),
                        "end": a.parent == anchors::Parent::End,
                        "offset": seconds((a.start - a.parent_start).0),
                    })
                })
                .collect();
            let st = main_track::settings(q);
            Ok(json!({"magnet": st.magnet, "linkage": st.linkage, "anchors": list}))
        }
        "br.video.anchor" => {
            let chosen = chosen_clips(s, p);
            if chosen.is_empty() {
                return Err(err(id, "needs `clips` (or a selection)"));
            }
            let to = match p.get("to") {
                Some(Value::String(a)) if a == "auto" => None,
                None | Some(Value::Null) => Some(None),
                Some(v) => Some(Some(filmcraft_project::ClipId(v.as_u64().ok_or_else(|| err(id, "`to` is a main clip id, auto or null"))?))),
            };
            if let Some(Some(parent)) = to
                && !s.active_sequence().is_some_and(|q| magnet::is_main(q, parent))
            {
                return Err(err(id, format!("{} is no clip of the main track", parent.0)));
            }
            let label = match to {
                None => "Automatisch anhängen",
                Some(None) => "Lösen",
                Some(Some(_)) => "Anhängen",
            };
            let n = s
                .edit_sequence(label, |q, _, _| Ok(chosen.iter().filter(|c| anchors::set_exception(q, filmcraft_project::ClipId(**c), to)).count()))
                .map_err(|e| err(id, e))?;
            Ok(json!({"changed": n}))
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
                .edit_sequence("Ton trennen", |q, ctx, _| {
                    let links: Vec<u64> =
                        q.all_tracks().flat_map(|t| t.items.iter()).filter(|it| chosen.contains(&it.id.0)).filter_map(|it| it.link).collect();
                    let mut n = 0;
                    for it in q.audio_tracks.iter_mut().flat_map(|t| t.items.iter_mut()) {
                        if it.link.is_some_and(|l| links.contains(&l)) {
                            it.link = None;
                            n += 1;
                        }
                    }
                    // a picture and its filled background stay together, under a link of their own
                    for l in links {
                        let pictures: Vec<filmcraft_project::ClipId> =
                            q.video_tracks.iter().flat_map(|t| &t.items).filter(|it| it.link == Some(l)).map(|it| it.id).collect();
                        let fresh = (pictures.len() > 1).then(|| ctx.alloc());
                        for c in pictures {
                            if let Some((_, it)) = q.find_item_mut(c) {
                                it.link = fresh;
                                n += usize::from(fresh.is_none());
                            }
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
            if s.active_sequence().is_none() {
                return Err(err(id, "noch kein Video"));
            }
            let t = opt_f64(id, p, "at")?.map(|x| ticks(x.max(0.0))).unwrap_or_else(|| s.playhead());
            s.edit_group("Text", |s| {
                // on an overlay track that is free there (FilmCraft would take the main track
                // where nothing else is, e.g. after the end), centred in the picture
                let track = overlay_track(s, t, ticks(secs))?;
                let r = s.execute("graphics.newText", json!({"text": text, "seconds": secs, "newClip": true, "time": t.0, "track": track}))?;
                if let Some(clip) = r.get("clip").and_then(Value::as_u64) {
                    s.execute("graphics.set", json!({"clip": clip, "layer": 0, "props": {"align": "center"}}))?;
                }
                Ok(r)
            })
            .map_err(|e| err(id, e))
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
                let end = s.active_sequence().map_or(0, |q| magnet::picture_end(q).0);
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
            let mode = p.get("fit").and_then(Value::as_str).unwrap_or("fit");
            if !matches!(mode, "fit" | "fill" | "reframe") {
                return Err(err(id, format!("`fit` is fit, fill or reframe, not `{mode}`")));
            }
            if s.active_sequence().is_none() {
                return Err(err(id, "noch kein Video: erst br.video.new oder br.video.add"));
            }
            let (w, h) = format.size();
            let before = s.active_sequence().cloned().ok_or_else(|| err(id, "no video yet"))?;
            s.edit_group(&format!("Format {}", format.key()), |s| {
                // the picture scales with the frame (FilmCraft fits the old frame into the new one);
                // titles and shapes are laid out for the new frame
                s.execute("sequence.settings", json!({"width": w, "height": h, "scaleMotion": true}))?;
                frame::refit_graphics(s, &before)?;
                // an animated (keyframed) position or scale is the user's: it stays
                let animated = |c: &u64| {
                    s.active_sequence()
                        .and_then(|q| q.find_item(filmcraft_project::ClipId(*c)))
                        .is_some_and(|(_, it)| it.effect("motion").is_some_and(|m| m.params.values().any(|p| !p.keyframes.is_empty())))
                };
                let clips: Vec<u64> = frame::to_fit(s).into_iter().filter(|c| !animated(c)).collect();
                if mode == "reframe" {
                    let ids: Vec<filmcraft_project::ClipId> = clips.iter().copied().map(filmcraft_project::ClipId).collect();
                    frame::reframe_clips(s, &ids, true, 1)?;
                }
                // what does not reframe fits or fills
                let rest = frame::to_fit(s);
                let clips: Vec<u64> = clips.into_iter().filter(|c| rest.contains(c)).collect();
                if !clips.is_empty() {
                    s.execute(if mode == "fill" { "clip.fillFrame" } else { "clip.fitToFrame" }, json!({"clips": clips}))?;
                }
                // backgrounds fill the new frame; Auto Reframe frames for it
                frame::after_format(s)
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
            let (mut main, has_one): (Vec<(u64, i64, i64)>, Vec<u64>) = {
                let q = s.active_sequence().ok_or_else(|| err(id, "no video yet"))?;
                let track = main_track::main_track_id(q).and_then(|t| q.track(t));
                let main = track.map(|t| t.items.iter().map(|it| (it.id.0, it.start.0, it.end().0)).collect()).unwrap_or_default();
                // a cut that has a transition keeps it
                let has_one = track.map(|t| t.transitions.iter().filter_map(|x| x.from.map(|c| c.0)).collect()).unwrap_or_default();
                (main, has_one)
            };
            main.sort_by_key(|c| c.1);
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
            let effect = p.get("effect").and_then(Value::as_str).map(str::to_string);
            if let Some(e) = &effect
                && filmcraft_project::vtransition::find_transition(e, filmcraft_project::EffectKind::VideoTransition).is_none()
            {
                return Err(err(id, format!("unknown transition `{e}` (br.video.schema kind videoTransition)")));
            }
            s.edit_group("Übergänge", |s| {
                for clip in cuts {
                    let mut q = json!({"clip": clip, "edge": "out"});
                    if let Some(e) = &effect {
                        q["effect"] = json!(e);
                    }
                    s.execute("sequence.applyVideoTransition", q)?;
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
            Ok(json!({"transitions": n}))
        }
        "br.video.autoCaptions" => {
            if p.get("wait").and_then(Value::as_bool) == Some(false) {
                return Err(err(id, "wait:false needs something that keeps running (the window or br mcp)"));
            }
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
            // the whole video, whatever In and Out marks are set, up to the end of its picture
            // (music that runs on is not played over black)
            let (end, all) = s.active_sequence().map(|q| (magnet::picture_end(q), q.duration())).unwrap_or_default();
            let range = p.get("range").cloned().unwrap_or(json!("entire"));
            let (range, from, to) = if p.get("range").is_none() && end.0 > 0 && end < all {
                (json!("custom"), Some(0.0), Some(seconds(end.0)))
            } else {
                (range, None, None)
            };
            let Some(part_str) = part.to_str() else {
                return Err(err(id, format!("{}: der Ordnername ist kein gültiger Text (UTF-8)", path.display())));
            };
            let mut q = json!({"preset": preset.filmcraft, "path": part_str, "wait": wait, "burnCaptions": burn, "range": range});
            if let (Some(a), Some(b)) = (from, to) {
                q["startSeconds"] = json!(a);
                q["endSeconds"] = json!(b);
            }
            let mut r = run(s, "export.quick", q)?;
            r["path"] = json!(path.to_string_lossy());
            r["part"] = json!(part_str);
            let job = r.get("job").and_then(Value::as_u64).unwrap_or(0);
            if !wait {
                watch_export(s, job, part, path);
                return Ok(r);
            }
            // a waited-for export that failed is a failure (FilmCraft keeps the error in its job)
            let failed =
                s.jobs.iter().find(|j| j.id == job).and_then(|j| j.result.lock().unwrap_or_else(|e| e.into_inner()).clone()).and_then(Result::err);
            if let Some(e) = failed {
                let _ = std::fs::remove_file(&part);
                return Err(err(id, if e == "cancelled" { "abgebrochen".to_string() } else { e }));
            }
            std::fs::rename(&part, &path).map_err(|e| err(id, format!("{}: {e}", path.display())))?;
            Ok(r)
        }
        "br.video.state" => state(s),
        "br.video.inspect" => inspect::inspect(s, p),
        "br.video.template.list" => Ok(template::list(p)),
        "br.video.template.new" => template::new(s, p).map_err(|e| err(id, e)),
        "br.video.template.fill" => template::fill(s, p).map_err(|e| err(id, e)),
        "br.video.template.save" => template::save(s, p).map_err(|e| err(id, e)),
        "br.video.curve" => speed::curve(s, p).map_err(|e| err(id, e)),
        "br.video.freeze" => speed::freeze(s, p).map_err(|e| err(id, e)),
        "br.video.reverse" => speed::reverse(s, p).map_err(|e| err(id, e)),
        "br.video.beats" => beats::beats(s, p).map_err(|e| err(id, e)),
        "br.video.transform" => frame::transform(s, p).map_err(|e| err(id, e)),
        "br.video.fit" => frame::fit(s, p).map_err(|e| err(id, e)),
        "br.video.background" => frame::background(s, p).map_err(|e| err(id, e)),
        "br.video.reframe" => frame::reframe(s, p).map_err(|e| err(id, e)),
        "br.video.place" => place::place(s, p).map_err(|e| err(id, e)),
        "br.video.set" => inspect::set(s, p).map_err(|e| err(id, e)),
        "br.video.keyframe" => inspect::keyframe(s, p).map_err(|e| err(id, e)),
        "br.video.effect" => inspect::effect(s, p).map_err(|e| err(id, e)),
        "br.video.schema" => inspect::schema(p).map_err(|e| err(id, e)),
        "br.video.jobs" => Ok(json!({"exports": jobs(s)})),
        "br.video.cancel" => {
            let job = p.get("job").and_then(Value::as_u64).ok_or_else(|| err(id, "needs `job` (from br.video.jobs)"))?;
            run(s, "jobs.cancel", json!({"job": job}))
        }
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
    // the video's length: its picture's (what an export holds), else everything's
    let end =
        s.active_sequence().map(|q| magnet::picture_end(q).0).filter(|e| *e > 0).or_else(|| seq.get("duration").and_then(Value::as_i64)).unwrap_or(0);
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
