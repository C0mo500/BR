//! `br mcp`: BR for AI agents over the Model Context Protocol: photos (LightCraft) and video
//! (FilmCraft) in one server.
//!
//! LightCraft already ships a complete MCP server (`lightcraft-mcp`): import, query, develop,
//! render and export tools, plus one `cmd_*` tool per engine command. BR keeps all of it and adds:
//!
//! - [`BrBackend`]: LightCraft's in-process [`Headless`] backend with the `br.*` commands of
//!   [`br_photo::commands`] routed in, so they also work through `command_run` and, with `--full`,
//!   as `cmd_br_*` tools; and a FilmCraft session for the video: `br.video.*` ([`br_video`]) and
//!   every FilmCraft command as `film.<id>`.
//! - Typed `br_video_*` tools for the video (CapCut's steps: add, cut, text, captions, music,
//!   filter, format, export, a preview image) and `film_commands` / `film_run` for all of
//!   FilmCraft (timeline, Lumetri, keyframes, Essential Sound, …).
//! - A front ([`serve`], [`BrServer`]) for LightCraft's [`Server`]: it names the server `br`, puts
//!   BR's instructions first and lists typed `br_*` tools (proper schemas, a preview image) ahead
//!   of LightCraft's own. A `br_*` call is checked against its schema and handed to LightCraft's
//!   server as the matching `command_run` (or `render_preview`) call, so everything, BR's tools
//!   included, runs through LightCraft's own request handling: id checks, protocol versions,
//!   progress notifications, cancellation and `ping` during long exports.
//!
//! The protocol is newline-delimited JSON-RPC 2.0 over stdio.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};

use br_photo::BrState;
use br_photo::adjust::Slider;
use br_photo::commands;
use br_photo::crop::Format;
use br_photo::export::Preset;
use lightcraft_mcp::{Backend, Headless, ProgressHook, Server};
use serde_json::{Map, Value, json};

mod window;
pub use window::Window;

/// Prefix of BR's own tools.
pub const TOOL_PREFIX: &str = "br_";

/// What `br mcp` tells the agent first.
pub const INSTRUCTIONS: &str = "BR is a media suite: photos on LightCraft's engine and video on FilmCraft's (a full editor: \
timeline, colour, sound, captions, export). VIDEO: the br_video_* tools work on one video project. A typical edit: \
`br_video_add` the clips (the first one's shape makes the video, or `br_video_new` with a format first), `br_video_split` / \
`br_video_delete` / `br_video_speed` to cut, `br_video_text`, `br_video_auto_captions` (speech recognition) or \
`br_video_captions` (SRT file), `br_video_music` (lowered under speech), `br_video_filter` (a look), `br_video_format` \
(9:16, 16:9, 1:1, 4:5), `br_video_preview` to look at a frame, `br_video_export` (social, youtube, youtube4k, video, \
prores). Each step is one undo step (`br_video_undo`). Long work (automatic captions, exporting a long video) can run in \
the background with `wait: false`: `br_video_jobs` tells how far it is, `br_video_cancel` stops it. Everything else FilmCraft can do: `film_commands` to find a \
command, `film_run` to run it (`sequence.inspect` describes the timeline; times are in ticks, 254016000000 per \
second). PHOTOS: start with the br_* tools: they work on the active photo unless you pass `id`. A typical edit: `import` the files (or \
`query_photos` / `select_photos` to pick one), `br_looks` then `br_look` for a style, `br_adjust` for light, contrast, \
color, warmth, sharpness and vignette (-100..100, 0 = unchanged), `br_crop` for a format (1:1, 4:5, 9:16, 16:9), \
`br_preview` to look at the result, and `br_export` with a preset (instagram 1080x1350, story 1080x1920, square, wide, \
full). Every edit is one undo step (`br_undo`). For finer control use LightCraft's tools below (`list_controls`, \
`set_develop`, `command_list`, `command_run`).";

/// LightCraft's headless backend plus BR's `br.*` commands, and the video project.
pub struct BrBackend {
    pub inner: Headless,
    pub state: BrState,
    /// The video project (FilmCraft): `br.video.*` and `film.<id>` run on it.
    pub video: br_video::Session,
    /// Caption recognition started with `wait: false`.
    pub captions: br_video::speech::Background,
    /// How the session was opened, for the agent ("library at …", "demo photos", …).
    pub origin: String,
}

impl BrBackend {
    pub fn new(inner: Headless, origin: impl Into<String>) -> Self {
        Self { inner, state: BrState::default(), video: br_video::Session::default(), captions: Default::default(), origin: origin.into() }
    }

    /// Render the video's frame for `br_video_preview` into a PNG at `path`.
    fn video_frame(&mut self, id: u64, size: u64, path: &str) -> Result<Value, String> {
        let ms = id - VIDEO_FRAME;
        let seconds = (ms > 0).then(|| (ms - 1) as f64 / 1000.0);
        let (w, h, rgba) = br_video::frame(&self.video, seconds, size.clamp(16, 4096) as u32)?;
        let img = lightcraft_codecs::EncodeImage::new(w, h, 4, lightcraft_codecs::Samples::U8(&rgba));
        let png = lightcraft_codecs::encode_png(&img, &lightcraft_codecs::EncodeMeta::default()).map_err(|e| e.to_string())?;
        std::fs::write(path, png).map_err(|e| format!("{path}: {e}"))?;
        let st = br_video::state(&mut self.video)?;
        Ok(json!({"width": w, "height": h, "video": st["name"], "seconds": seconds.unwrap_or(st["playhead"].as_f64().unwrap_or(0.0))}))
    }
}

/// `br_video_preview` goes through LightCraft's `render_preview` (which turns a rendered file
/// into an image for the agent): with a photo id from here up, BR renders the video's frame
/// instead, at (id − this − 1) milliseconds (at the playhead for exactly this). Above any photo id
/// LightCraft gives out, and below 2^53 so JSON numbers keep it exact.
pub const VIDEO_FRAME: u64 = 1 << 52;

impl Backend for BrBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if matches!(method, "engine.execute" | "ui.menu.invoke" | "command")
            && let Some(id) = params.get("command").or(params.get("id")).and_then(Value::as_str)
        {
            let p = params.get("params").cloned().unwrap_or(Value::Null);
            // finished background recognition goes into the project first
            self.captions.poll(&mut self.video);
            match id {
                "br.video.autoCaptions" if p.get("wait").and_then(Value::as_bool) == Some(false) => {
                    let opts = br_video::speech::CaptionOptions {
                        language: p.get("language").and_then(Value::as_str).map(str::to_string),
                        model: p.get("model").and_then(Value::as_str).map(str::to_string),
                    };
                    self.captions.start(&self.video, &opts)?;
                    return Ok(json!({"started": true, "poll": "br_video_jobs"}));
                }
                "br.video.jobs" => return Ok(json!({"exports": br_video::jobs(&self.video), "captions": self.captions.status()})),
                "br.video.cancel" if p.get("captions").and_then(Value::as_bool) == Some(true) => {
                    return Ok(json!({"cancelled": self.captions.cancel()}));
                }
                _ => {}
            }
            if br_video::is_video(id) {
                return br_video::execute(&mut self.video, id, &p);
            }
            if let Some(film) = id.strip_prefix("film.") {
                return br_video::execute(&mut self.video, film, &p);
            }
            if commands::is_br(id) {
                let id = id.to_string();
                return commands::execute(&mut self.inner.session, &mut self.state, &id, &p).map_err(|e| e.to_string());
            }
        }
        if method == "ui.render"
            && let Some(id) = params.get("id").and_then(Value::as_u64).filter(|i| *i >= VIDEO_FRAME)
        {
            let size = params.get("size").and_then(Value::as_u64).unwrap_or(1024);
            let path = params.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
            return self.video_frame(id, size, &path);
        }
        let mut r = self.inner.call(method, params)?;
        if method == "engine.commands"
            && let Value::Array(a) = &mut r
        {
            a.extend(commands::list(&self.inner.session));
        }
        Ok(r)
    }

    fn has_ui(&self) -> bool {
        self.inner.has_ui()
    }

    fn describe(&self) -> String {
        format!("BR headless, {}", self.origin)
    }

    fn set_progress(&mut self, hook: Option<ProgressHook>) {
        self.inner.set_progress(hook);
    }
}

fn id_prop() -> Value {
    json!({"type": "integer", "description": "Photo id (default: the active photo)"})
}

fn tool(name: &str, title: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
    // edits replace what the photo had (undoable, but a client asking before "destructive" calls
    // should ask); an export only writes a new file
    let destructive = !read_only && !matches!(name, "br_export" | "br_video_export" | "br_video_preview");
    let idempotent = read_only || matches!(name, "br_adjust" | "br_crop" | "br_reset");
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false},
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": destructive, "idempotentHint": idempotent, "openWorldHint": false},
    })
}

/// BR's typed tools.
pub fn tools() -> Vec<Value> {
    let mut sliders = Map::new();
    for k in Slider::ALL {
        sliders.insert(
            k.key().into(),
            json!({"type": "number", "minimum": -100, "maximum": 100, "description": format!("{} ({}): {}", k.label(), k.key(), k.hint())}),
        );
    }
    sliders.insert("id".into(), id_prop());
    let formats: Vec<&str> = Format::ALL.iter().map(|f| f.key()).collect();
    let presets: Vec<&str> = Preset::ALL.iter().map(|p| p.key()).collect();
    let preset_doc: Vec<String> = Preset::ALL.iter().map(|p| format!("{} = {}", p.key(), p.detail())).collect();
    vec![
        tool(
            "br_state",
            "Photo state",
            "The simple view of a photo: file, size, crop format, the look and its amount, the six slider values, and what undo/redo would do.",
            json!({"id": id_prop()}),
            &[],
            true,
        ),
        tool("br_looks", "List looks", "All looks (filter presets): [{id, name, group}].", json!({}), &[], true),
        tool(
            "br_look",
            "Apply look",
            "Apply a look (a filter preset) to a photo. If the photo's latest edit is a look, it is replaced (the same look at a new \
             amount, or another look), so looks never stack and stay one undo step. Amount 0 takes the look off.",
            json!({
                "look": {"type": "string", "description": "Look id or name from br_looks, e.g. lc.golden-hour or \"Golden Hour\""},
                "amount": {"type": "number", "minimum": 0, "maximum": 200, "description": "Strength in percent (default 100, or the look's current amount)"},
                "id": id_prop(),
            }),
            &["look"],
            false,
        ),
        tool(
            "br_adjust",
            "Adjust",
            "Set the simple sliders (absolute values, -100..100, 0 = unchanged). Only the sliders you pass change. One undo step.",
            Value::Object(sliders),
            &[],
            false,
        ),
        tool("br_auto", "Auto", "Automatic exposure, contrast and tone from the photo's own statistics.", json!({"id": id_prop()}), &[], false),
        tool(
            "br_crop",
            "Format",
            "Crop to a format: the largest centred crop in the exact ratio and orientation (9:16 is portrait), in whole pixels. \
             `original` removes the crop.",
            json!({"format": {"type": "string", "enum": formats}, "id": id_prop()}),
            &["format"],
            false,
        ),
        tool(
            "br_export",
            "Export",
            &format!(
                "Export a photo as JPEG with a preset; the photo's own edit is not changed. Presets: {}. Writes to `path` (exact file), \
                 `dir` (named <photo>-<preset>.jpg), or BR's export folder. Returns the written files.",
                preset_doc.join(", ")
            ),
            json!({
                "preset": {"type": "string", "enum": presets},
                "dir": {"type": "string", "description": "Folder to write to"},
                "path": {"type": "string", "description": "Exact output file (.jpg)"},
                "id": id_prop(),
            }),
            &["preset"],
            false,
        ),
        tool(
            "br_preview",
            "Preview",
            "Render the photo as it is edited (crop applied) and return it as an image.",
            json!({
                "id": id_prop(),
                "max_side": {"type": "integer", "minimum": 64, "maximum": 2048, "description": "Longest side in pixels (default 1024)"},
                "format": {"type": "string", "enum": ["jpeg", "png"], "description": "Image format (default jpeg)"},
            }),
            &[],
            true,
        ),
        tool(
            "br_reset",
            "Reset all",
            "Back to how the photo was imported: look, sliders, format and Pro edits.",
            json!({"id": id_prop()}),
            &[],
            false,
        ),
        tool("br_undo", "Undo", "Undo the last edit.", json!({}), &[], false),
        tool("br_redo", "Redo", "Redo the last undone edit.", json!({}), &[], false),
    ]
}

/// Every tool BR adds: photo, then video.
pub fn all_tools() -> Vec<Value> {
    let mut t = tools();
    t.extend(video_tools());
    t
}

/// The video tools and the command each runs (`br.video.*`, or FilmCraft's as `film.<id>`).
const VIDEO_TOOLS: &[(&str, &str)] = &[
    ("br_video_state", "br.video.state"),
    ("br_video_new", "br.video.new"),
    ("br_video_add", "br.video.add"),
    ("br_video_split", "br.video.split"),
    ("br_video_delete", "br.video.delete"),
    ("br_video_speed", "br.video.speed"),
    ("br_video_move", "br.video.move"),
    ("br_video_trim", "br.video.trim"),
    ("br_video_duplicate", "br.video.duplicate"),
    ("br_video_pack", "br.video.pack"),
    ("br_video_magnet", "br.video.magnet"),
    ("br_video_linkage", "br.video.linkage"),
    ("br_video_anchors", "br.video.anchors"),
    ("br_video_anchor", "br.video.anchor"),
    ("br_video_inspect", "br.video.inspect"),
    ("br_video_set", "br.video.set"),
    ("br_video_keyframe", "br.video.keyframe"),
    ("br_video_effect", "br.video.effect"),
    ("br_video_schema", "br.video.schema"),
    ("br_video_detach_audio", "br.video.detachAudio"),
    ("br_video_text", "br.video.text"),
    ("br_video_captions", "br.video.captions"),
    ("br_video_auto_captions", "br.video.autoCaptions"),
    ("br_video_music", "br.video.music"),
    ("br_video_looks", "film.lumetri.presets"),
    ("br_video_filter", "br.video.filter"),
    ("br_video_format", "br.video.format"),
    ("br_video_transition", "br.video.transition"),
    ("br_video_export", "br.video.export"),
    ("br_video_jobs", "br.video.jobs"),
    ("br_video_cancel", "br.video.cancel"),
    ("br_video_undo", "film.edit.undo"),
    ("br_video_redo", "film.edit.redo"),
    ("film_commands", "br.video.commands"),
];

fn clips_prop() -> Value {
    json!({"type": "array", "items": {"type": "integer"}, "description": "Clip ids (default: the clips selected in the timeline); film_run sequence.inspect lists them"})
}

/// BR's typed video tools.
pub fn video_tools() -> Vec<Value> {
    let formats: Vec<&str> = br_video::Format::ALL.iter().map(|f| f.key()).collect();
    let presets: Vec<&str> = br_video::EXPORT_PRESETS.iter().map(|p| p.key).collect();
    let preset_doc: Vec<String> = br_video::EXPORT_PRESETS.iter().map(|p| format!("{} = {} ({})", p.key, p.label, p.detail)).collect();
    vec![
        tool(
            "br_video_state",
            "Video state",
            "The video at a glance: name, format, size, length in seconds, clip counts, playhead, what undo would do. {\"video\": null} before there is one.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "br_video_new",
            "New video",
            "Start a new, empty video in a format: 9:16 (1080x1920, TikTok/Reels/Shorts), 16:9 (1920x1080), 1:1, 4:5. Not needed before br_video_add, which makes a video in the first clip's shape.",
            json!({"format": {"type": "string", "enum": formats}, "fps": {"type": "number", "minimum": 1, "maximum": 240}, "name": {"type": "string"}}),
            &["format"],
            false,
        ),
        tool(
            "br_video_add",
            "Add media",
            "Import video, sound or image files. Pictures go one after another on the main track: at its end, or inserted at the cut nearest to the playhead (`at`: playhead) or to `at` seconds; nothing is cut in two. Sound alone goes on a sound track at the playhead (20 s or longer: as music). One undo step.",
            json!({"paths": {"type": "array", "items": {"type": "string"}, "description": "Files (absolute paths)"}, "at": {"description": "end (default), playhead, or seconds", "oneOf": [{"type": "string", "enum": ["end", "playhead"]}, {"type": "number", "minimum": 0}]}}),
            &["paths"],
            false,
        ),
        tool(
            "br_video_split",
            "Split",
            "Cut a clip in two at the playhead (or at `seconds`): `clip`, else the selected clip there, else the main track's clip there. Its sound and background are cut with it; titles and music over it stay whole.",
            json!({"clip": {"type": "integer"}, "seconds": {"type": "number", "minimum": 0}}),
            &[],
            false,
        ),
        tool(
            "br_video_delete",
            "Delete",
            "Delete clips. A main-track clip goes with its sound; with the magnet on the main track closes up, and what hangs on the clip (titles, overlays, sound effects, captions) goes too. One undo step brings all back.",
            json!({"clips": clips_prop()}),
            &[],
            false,
        ),
        tool(
            "br_video_move",
            "Move clip",
            "Move a clip. On the main track: to place `index` (0 = first) or to the cut nearest `seconds`; what hangs on it moves along. `track`: overlay (off the main track onto a free overlay track at `seconds`), main (onto the main track), V2/A3/… or a track id (any clip to `seconds` there, onto a free track if taken).",
            json!({"clip": {"type": "integer"}, "index": {"type": "integer", "minimum": 0}, "seconds": {"type": "number", "minimum": 0}, "track": {"description": "main, overlay, V2, A3, … or a track id", "oneOf": [{"type": "string"}, {"type": "integer"}]}}),
            &["clip"],
            false,
        ),
        tool(
            "br_video_trim",
            "Trim",
            "Move a clip's edge by `seconds` (+ later, − earlier): edge in = its start, out = its end. A main-track clip ripples with the magnet on (its start stays, what follows moves up or on with what hangs on it); captions follow the picture.",
            json!({"clip": {"type": "integer"}, "edge": {"type": "string", "enum": ["in", "out"]}, "seconds": {"type": "number"}, "frames": {"type": "integer"}}),
            &["clip", "edge"],
            false,
        ),
        tool(
            "br_video_duplicate",
            "Duplicate",
            "Copy clips: a main-track clip's copy (with its sound) comes right after it and what follows moves on; any other clip's copy right after it, on a free track if needed.",
            json!({"clips": clips_prop()}),
            &[],
            false,
        ),
        tool("br_video_pack", "Close gaps", "Close every gap of the main track; what hangs on its clips moves along.", json!({}), &[], false),
        tool(
            "br_video_magnet",
            "Magnetic main track",
            "CapCut's magnetic main track: on, the main track has no gaps (deleting closes up, inserting pushes on, trimming ripples); turning it on closes the gaps. Off: gaps stay.",
            json!({"on": {"type": "boolean"}}),
            &["on"],
            false,
        ),
        tool(
            "br_video_linkage",
            "Linkage",
            "Titles, overlays, sound effects and captions follow the main-track clip they start on (also after edits made with film_run). Off: nothing follows (FilmCraft's own behaviour).",
            json!({"on": {"type": "boolean"}}),
            &["on"],
            false,
        ),
        tool(
            "br_video_anchors",
            "What hangs on what",
            "The magnet and linkage switches and every attached clip: {clip, kind (overlay, sound, caption), parent (main clip id; null with end: true for what follows the end), offset in seconds}.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "br_video_anchor",
            "Attach or detach",
            "Attach clips to a main-track clip (`to`: its id), detach them so they stay put (`to`: null), or let them follow the clip they start on again (`to`: auto).",
            json!({"clips": clips_prop(), "to": {"description": "a main clip id, auto or null", "oneOf": [{"type": "integer"}, {"type": "string", "enum": ["auto"]}, {"type": "null"}]}}),
            &[],
            false,
        ),
        tool(
            "br_video_inspect",
            "Inspector",
            "What a clip, caption or transition is made of, as BR's inspector shows it (default: the selection). `sections` are the Basis rows for its kind (video, picture, text, shape, sound, music, caption, transition): key (e.g. transform.scale, adjust.exposure, audio.volume, text.text), German label, type, value at the playhead (or `seconds`), range, slider range, unit, animatable, animated, atKeyframe. level mehr adds `effects`: every effect with every parameter (id, group, value, keyframes with seconds). `linked` names a picture's sound clip.",
            json!({"clip": {"type": "integer"}, "caption": {"type": "integer"}, "transition": {"type": "integer"}, "seconds": {"type": "number", "minimum": 0}, "level": {"type": "string", "enum": ["basis", "mehr"]}}),
            &[],
            true,
        ),
        tool(
            "br_video_set",
            "Set a value",
            "Set an inspector row (`row`: a key from br_video_inspect) or any effect parameter (`effect`: index from br_video_inspect level mehr, an effect id, or adjust for the clip's grade; `param`: its id). The value is held to the schema (numbers clamped, choices by name or index, colours #rrggbb[aa]). keyframe: true writes a keyframe at the playhead (or `seconds`). One undo step; calls with the same `gesture` string fold into one (a drag). Returns the row as it is now.",
            json!({"row": {"type": "string"}, "effect": {"oneOf": [{"type": "integer"}, {"type": "string"}]}, "param": {"type": "string"}, "value": {}, "clip": {"type": "integer"}, "caption": {"type": "integer"}, "transition": {"type": "integer"}, "seconds": {"type": "number", "minimum": 0}, "keyframe": {"type": "boolean"}, "gesture": {"type": "string"}}),
            &["value"],
            false,
        ),
        tool(
            "br_video_keyframe",
            "Keyframes",
            "Keyframes at the playhead (or `seconds`) of inspector rows (`row`: one key or a list, e.g. all transform rows, in one step) or of an effect parameter (`effect` + `param`): toggle (default), add, remove, clear (stop animating, keep the value there), ease (`interpolation`, `all` for every keyframe), prev / next (move the playhead to the neighbouring keyframe).",
            json!({"row": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]}, "effect": {"oneOf": [{"type": "integer"}, {"type": "string"}]}, "param": {"type": "string"}, "op": {"type": "string", "enum": ["toggle", "add", "remove", "clear", "ease", "prev", "next"]}, "seconds": {"type": "number", "minimum": 0}, "interpolation": {"type": "string", "enum": ["linear", "hold", "bezier", "autoBezier", "continuousBezier", "easeIn", "easeOut"]}, "all": {"type": "boolean"}, "clip": {"type": "integer"}}),
            &[],
            false,
        ),
        tool(
            "br_video_effect",
            "Effects",
            "Add an effect to a clip (`effect`: an id or name from br_video_schema; an audio effect on a picture goes on its sound) and get its index, or remove, disable, enable or reset one (`effect`: its index or id; `param` resets one parameter; `on`: the clip it is on when that is the linked sound).",
            json!({"op": {"type": "string", "enum": ["add", "remove", "enable", "disable", "reset"]}, "effect": {"oneOf": [{"type": "integer"}, {"type": "string"}]}, "param": {"type": "string"}, "clip": {"type": "integer"}, "on": {"type": "integer"}}),
            &["effect"],
            false,
        ),
        tool(
            "br_video_schema",
            "Effect schemas",
            "FilmCraft's effects: with `effect` its parameters (id, label, type, min/max, slider range, unit, decimals, default, group, animatable); without, the list (filter by `kind`; detail: true for all parameters).",
            json!({"effect": {"type": "string"}, "kind": {"type": "string", "enum": ["video", "audio", "videoTransition", "audioTransition"]}, "detail": {"type": "boolean"}}),
            &[],
            true,
        ),
        tool(
            "br_video_speed",
            "Speed",
            "Play clips faster (2 = twice as fast) or slower (0.5); the clips after them move up or back.",
            json!({"speed": {"type": "number", "minimum": 0.01, "maximum": 100}, "clips": clips_prop()}),
            &["speed"],
            false,
        ),
        tool(
            "br_video_detach_audio",
            "Detach audio",
            "Separate picture and sound of clips (on linked clips; on unlinked ones it links them again).",
            json!({"clips": clips_prop()}),
            &[],
            false,
        ),
        tool(
            "br_video_text",
            "Text",
            "A title at the playhead (or at `at` seconds), `seconds` long (default 3).",
            json!({"text": {"type": "string"}, "seconds": {"type": "number", "minimum": 0.04, "maximum": 3600}, "at": {"type": "number", "minimum": 0}}),
            &["text"],
            false,
        ),
        tool(
            "br_video_captions",
            "Captions from a file",
            "Captions from a subtitle file (SRT, VTT, SCC, STL, TTML, …).",
            json!({"path": {"type": "string"}}),
            &["path"],
            false,
        ),
        tool(
            "br_video_auto_captions",
            "Automatic captions",
            "Recognise the speech in the video (Whisper, on this computer) and make captions from it, one undo step. The first time the model is downloaded (about 150 MB). Takes a while for long videos: with wait false it runs in the background (br_video_jobs shows how far it is, br_video_cancel stops it).",
            json!({
                "language": {"type": "string", "description": "Spoken language as an ISO code (de, en, fr, …) or auto (default)"},
                "model": {"type": "string", "enum": ["whisper-tiny", "whisper-base", "whisper-small"], "description": "Default: the one in FilmCraft's settings (whisper-base)"},
                "wait": {"type": "boolean", "description": "Wait for the captions (default true); false: start and return"},
            }),
            &[],
            false,
        ),
        tool(
            "br_video_music",
            "Music",
            "A music file on its own track under the video, as long as the video, set as music and lowered under speech (`duck`, default true). One undo step.",
            json!({"path": {"type": "string"}, "duck": {"type": "boolean"}}),
            &["path"],
            false,
        ),
        tool(
            "br_video_looks",
            "List video looks",
            "FilmCraft's Lumetri looks for br_video_filter: [{name, folder, description}].",
            json!({}),
            &[],
            true,
        ),
        tool(
            "br_video_filter",
            "Filter",
            "A look on clips (default: the selected ones, else every clip), replacing the look applied before; `none` removes it.",
            json!({"preset": {"type": "string", "description": "A name from br_video_looks, or none"}, "clips": clips_prop()}),
            &["preset"],
            false,
        ),
        tool(
            "br_video_format",
            "Format",
            "Change the video's shape; its clips fit into the new frame (`fit`) or fill it (`fill`, cropping the edges).",
            json!({"format": {"type": "string", "enum": formats}, "fit": {"type": "string", "enum": ["fit", "fill"]}}),
            &["format"],
            false,
        ),
        tool(
            "br_video_transition",
            "Transition",
            "A dissolve at the cuts between the given (or selected) clips; without either, at every cut of the main track. Cuts that have one keep it.",
            json!({"clips": clips_prop()}),
            &[],
            false,
        ),
        tool(
            "br_video_export",
            "Export video",
            &format!(
                "Write the video as a file. Presets: {}. To `path`, else into BR's export folder named after the video. Waits until the file is written (`wait`: false starts it in the background).",
                preset_doc.join("; ")
            ),
            json!({"preset": {"type": "string", "enum": presets}, "path": {"type": "string"}, "wait": {"type": "boolean"}}),
            &["preset"],
            false,
        ),
        tool(
            "br_video_preview",
            "Video preview",
            "A frame of the video as an image: at the playhead, or at `seconds`.",
            json!({
                "seconds": {"type": "number", "minimum": 0},
                "max_side": {"type": "integer", "minimum": 64, "maximum": 2048, "description": "Longest side in pixels (default 1024)"},
                "format": {"type": "string", "enum": ["jpeg", "png"], "description": "Image format (default jpeg)"},
            }),
            &[],
            true,
        ),
        tool(
            "br_video_jobs",
            "Video jobs",
            "What runs in the background: exports (progress 0..1, done, error) and the caption recognition (progress, status, then its result).",
            json!({}),
            &[],
            true,
        ),
        tool(
            "br_video_cancel",
            "Cancel video job",
            "Stop an export (`job` from br_video_jobs) or the caption recognition (`captions`: true). A stopped export leaves no file.",
            json!({"job": {"type": "integer"}, "captions": {"type": "boolean"}}),
            &[],
            false,
        ),
        tool("br_video_undo", "Undo (video)", "Undo the last change to the video.", json!({}), &[], false),
        tool("br_video_redo", "Redo (video)", "Redo the last undone change to the video.", json!({}), &[], false),
        tool(
            "film_commands",
            "FilmCraft commands",
            "Find FilmCraft commands (its whole editor: timeline edits, Lumetri, effects and keyframes, Essential Sound, captions, export settings, …): id, label, menu and parameters of those whose id or label contains `query`.",
            json!({"query": {"type": "string", "description": "e.g. razor, lumetri, keyframe, ducking (empty: all, a long list)"}}),
            &[],
            true,
        ),
        tool(
            "film_run",
            "Run a FilmCraft command",
            "Run any FilmCraft command on the video by its id (from film_commands), with its parameters. Times are in ticks (254016000000 per second). Changes are undoable with br_video_undo.",
            json!({"command": {"type": "string", "description": "e.g. sequence.inspect, timeline.razor, lumetri.set"}, "params": {"type": "object"}}),
            &["command"],
            false,
        ),
    ]
}

/// Whether `name` is one of BR's tools (the front handles those).
fn is_br_tool(name: &str) -> bool {
    name.starts_with(TOOL_PREFIX) || name.starts_with("film_")
}

/// The `br.*` command behind a `br_*` tool.
fn command_of(tool: &str) -> Option<&'static str> {
    if let Some((_, c)) = VIDEO_TOOLS.iter().find(|(t, _)| *t == tool) {
        return Some(c);
    }
    let name = tool.strip_prefix(TOOL_PREFIX)?;
    commands::COMMANDS.iter().map(|(id, _, _)| *id).find(|id| id.strip_prefix("br.") == Some(name))
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// Check `args` against a tool's input schema: known names, required ones, types, enums, ranges.
fn check_args(def: &Value, args: &Value) -> Result<(), String> {
    let name = def["name"].as_str().unwrap_or_default();
    let Some(given) = args.as_object() else { return Err("arguments must be an object".into()) };
    let props = def["inputSchema"]["properties"].as_object().cloned().unwrap_or_default();
    for (k, v) in given {
        let Some(schema) = props.get(k) else {
            let known: Vec<&String> = props.keys().collect();
            return Err(format!("unknown argument `{k}` for {name} (known: {known:?})"));
        };
        // an optional argument sent as null (strict tool-calling modes do) is not given
        if v.is_null() {
            continue;
        }
        let ok = match schema["type"].as_str() {
            Some("integer") => v.is_u64() || v.is_i64(),
            Some("number") => v.as_f64().is_some_and(f64::is_finite),
            Some("string") => v.is_string(),
            Some("boolean") => v.is_boolean(),
            Some("object") => v.is_object(),
            Some("array") => v.as_array().is_some_and(|a| match schema["items"]["type"].as_str() {
                Some("integer") => a.iter().all(|x| x.is_u64() || x.is_i64()),
                Some("string") => a.iter().all(Value::is_string),
                _ => true,
            }),
            _ => true,
        };
        if !ok {
            let want = match schema["type"].as_str() {
                Some("integer") => "a whole number",
                Some("number") => "a number",
                Some("boolean") => "true or false",
                Some("object") => "an object",
                Some("array") if schema["items"]["type"] == "integer" => "a list of whole numbers",
                Some("array") if schema["items"]["type"] == "string" => "a list of texts",
                Some("array") => "a list",
                _ => "text",
            };
            return Err(format!("{name}: `{k}` must be {want} (got {v})"));
        }
        if let Some(options) = schema["enum"].as_array()
            && !options.contains(v)
        {
            return Err(format!("{name}: `{k}` must be one of {}", Value::Array(options.clone())));
        }
        if let Some(x) = v.as_f64()
            && (schema["minimum"].as_f64().is_some_and(|m| x < m) || schema["maximum"].as_f64().is_some_and(|m| x > m))
        {
            return Err(format!("{name}: `{k}` must be from {} to {}", schema["minimum"], schema["maximum"]));
        }
    }
    for r in def["inputSchema"]["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if given.get(r).is_none_or(Value::is_null) {
            return Err(format!("{name} needs `{r}`"));
        }
    }
    Ok(())
}

/// What the front does with one message.
enum Front {
    /// Answer it here (a bad `br_*` call), without LightCraft.
    Reply(Value),
    /// Hand it (possibly rewritten) to LightCraft's server; patch the reply to `id` if asked.
    Forward(Value, Option<(String, Patch)>),
}

/// A change to one of LightCraft's replies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Patch {
    Initialize,
    ToolsList,
}

fn valid_id(id: Option<&Value>) -> Option<&Value> {
    id.filter(|i| i.is_string() || i.is_number())
}

fn front(tools: &[Value], msg: Value) -> Front {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let Some(id) = valid_id(msg.get("id")).cloned() else { return Front::Forward(msg, None) };
    match method {
        "initialize" => Front::Forward(msg, Some((id.to_string(), Patch::Initialize))),
        "tools/list" => Front::Forward(msg, Some((id.to_string(), Patch::ToolsList))),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default().to_string();
            if !is_br_tool(&name) || !msg["params"].is_object() {
                return Front::Forward(msg, None);
            }
            let Some(def) = tools.iter().find(|t| t["name"] == name.as_str()) else {
                return Front::Reply(rpc_error(id, -32602, format!("unknown tool `{name}`")));
            };
            let mut args = match msg["params"].get("arguments") {
                None | Some(Value::Null) => json!({}),
                Some(a) => a.clone(),
            };
            if let Err(e) = check_args(def, &args) {
                return Front::Reply(rpc_error(id, -32602, e));
            }
            if let Some(o) = args.as_object_mut() {
                o.retain(|_, v| !v.is_null());
            }
            let (call, call_args) = if name == "br_video_preview" {
                // through LightCraft's image path: an id from VIDEO_FRAME up is the video's frame
                let ms = args.get("seconds").and_then(Value::as_f64).map_or(0, |s| (s * 1000.0).round() as u64 + 1);
                let mut a = Map::new();
                a.insert("id".into(), json!(VIDEO_FRAME + ms));
                a.insert("max_side".into(), args.get("max_side").cloned().unwrap_or(json!(1024)));
                a.insert("format".into(), args.get("format").cloned().unwrap_or(json!("jpeg")));
                ("render_preview", Value::Object(a))
            } else if name == "film_run" {
                let command = args["command"].as_str().unwrap_or_default();
                let params = args.get("params").cloned().unwrap_or(json!({}));
                ("command_run", json!({"id": format!("film.{command}"), "params": params}))
            } else if name == "br_video_export" && args.get("wait").is_none() {
                // an agent wants the file: wait for it unless told otherwise
                let mut a = args.clone();
                a["wait"] = json!(true);
                ("command_run", json!({"id": "br.video.export", "params": a}))
            } else if name == "br_preview" {
                let mut a = Map::new();
                if let Some(v) = args.get("id") {
                    a.insert("id".into(), v.clone());
                }
                a.insert("max_side".into(), args.get("max_side").cloned().unwrap_or(json!(1024)));
                a.insert("format".into(), args.get("format").cloned().unwrap_or(json!("jpeg")));
                ("render_preview", Value::Object(a))
            } else {
                let Some(command) = command_of(&name) else { return Front::Reply(rpc_error(id, -32602, format!("unknown tool `{name}`"))) };
                ("command_run", json!({"id": command, "params": args}))
            };
            let mut msg = msg;
            msg["params"]["name"] = json!(call);
            msg["params"]["arguments"] = call_args;
            Front::Forward(msg, None)
        }
        _ => Front::Forward(msg, None),
    }
}

/// Apply `patch` to LightCraft's reply `r`.
fn patch(tools: &[Value], patch: Patch, r: &mut Value) {
    let Some(res) = r.get_mut("result").and_then(Value::as_object_mut) else { return };
    match patch {
        Patch::ToolsList => {
            if let Some(list) = res.get_mut("tools").and_then(Value::as_array_mut) {
                let lc = std::mem::take(list);
                list.extend(tools.iter().cloned());
                list.extend(lc);
            }
        }
        Patch::Initialize => {
            let lc = res.get("instructions").and_then(Value::as_str).unwrap_or_default().to_string();
            res.insert("serverInfo".into(), json!({"name": "br", "title": "BR", "version": env!("CARGO_PKG_VERSION")}));
            res.insert("instructions".into(), json!(format!("{INSTRUCTIONS}\n\nLightCraft: {lc}")));
        }
    }
}

/// Patch every reply in `v` (one reply or a batch) whose id has a pending patch.
fn patch_reply(tools: &[Value], pending: &Mutex<HashMap<String, Patch>>, v: &mut Value) {
    let one = |r: &mut Value| {
        let Some(id) = valid_id(r.get("id")).map(Value::to_string) else { return };
        let p = pending.lock().ok().and_then(|mut m| m.remove(&id));
        if let Some(p) = p {
            patch(tools, p, r);
        }
    };
    match v {
        Value::Array(a) => a.iter_mut().for_each(one),
        r => one(r),
    }
}

/// LightCraft's MCP server with BR in front, called message by message (tests, embedding). For
/// stdio use [`serve`], which also keeps progress notifications and cancellation.
pub struct BrServer {
    server: Server,
    tools: Vec<Value>,
}

impl BrServer {
    /// `full`: also list one `cmd_*` tool per LightCraft command (about 330 tools; they cost an
    /// agent a lot of context, and every command stays reachable through `command_run` anyway).
    pub fn new(backend: impl Backend + 'static, full: bool) -> Self {
        Self { server: Server::new(Box::new(backend)).with_command_tools(full), tools: all_tools() }
    }

    /// The backend, for callers that drive it directly.
    pub fn backend(&mut self) -> &mut dyn Backend {
        self.server.backend()
    }

    /// Handle one line; the reply line, or `None` for notifications and blank lines.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) if !batch.is_empty() => {
                let replies: Vec<Value> = batch.into_iter().filter_map(|m| self.handle(m)).collect();
                (!replies.is_empty()).then(|| Value::Array(replies).to_string())
            }
            Ok(msg @ Value::Object(_)) => self.handle(msg).map(|r| r.to_string()),
            // anything else (an empty batch, a bad message, bad JSON) LightCraft's server answers
            _ => self.server.handle_line(line),
        }
    }

    /// Handle one JSON-RPC message.
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        match front(&self.tools, msg) {
            Front::Reply(r) => Some(r),
            Front::Forward(msg, p) => {
                let mut r = self.server.handle(msg)?;
                if let Some((_, p)) = p {
                    patch(&self.tools, p, &mut r);
                }
                Some(r)
            }
        }
    }
}

/// Serve newline-delimited JSON-RPC from `input` to `output` until `input` closes.
///
/// LightCraft's own server loop runs on a thread of its own and does all the work; this thread
/// reads the client's lines, rewrites `br_*` calls (or answers bad ones), and a third thread
/// writes LightCraft's replies out, patching `initialize` and `tools/list`. Notifications such as
/// progress pass straight through, and a cancellation reaches LightCraft while it exports.
pub fn serve(backend: impl Backend + Send + 'static, full: bool, input: impl BufRead, output: impl Write + Send + 'static) -> std::io::Result<()> {
    let tools = Arc::new(all_tools());
    let out = Arc::new(Mutex::new(output));
    let pending: Arc<Mutex<HashMap<String, Patch>>> = Arc::default();
    // answers BR gave itself inside a batch, by the id of a request LightCraft answers in it
    let extras: Arc<Mutex<HashMap<String, Vec<Value>>>> = Arc::default();
    let (lc_in, mut to_lc) = std::io::pipe()?;
    let (from_lc, lc_out) = std::io::pipe()?;
    // LightCraft's server is not `Send`: it is built on the thread that runs it
    let server = std::thread::Builder::new()
        .name("br-mcp-lightcraft".into())
        .spawn(move || Server::new(Box::new(backend)).with_command_tools(full).serve(BufReader::new(lc_in), lc_out))?;
    let pump = {
        let (tools, out, pending, extras) = (tools.clone(), out.clone(), pending.clone(), extras.clone());
        std::thread::Builder::new().name("br-mcp-out".into()).spawn(move || -> std::io::Result<()> {
            for line in BufReader::new(from_lc).lines() {
                let line = line?;
                let text = match serde_json::from_str::<Value>(&line) {
                    Ok(mut v) => {
                        patch_reply(&tools, &pending, &mut v);
                        if let Value::Array(a) = &mut v {
                            let ids: Vec<String> = a.iter().filter_map(|r| valid_id(r.get("id")).map(Value::to_string)).collect();
                            if let Ok(mut map) = extras.lock() {
                                for id in ids {
                                    if let Some(more) = map.remove(&id) {
                                        a.extend(more);
                                    }
                                }
                            }
                        }
                        v.to_string()
                    }
                    Err(_) => line,
                };
                let mut o = out.lock().map_err(|_| std::io::Error::other("output lock poisoned"))?;
                writeln!(o, "{text}")?;
                o.flush()?;
            }
            Ok(())
        })?
    };
    let reply = |v: &Value| -> std::io::Result<()> {
        let mut o = out.lock().map_err(|_| std::io::Error::other("output lock poisoned"))?;
        writeln!(o, "{v}")?;
        o.flush()
    };
    let mut result = Ok(());
    for line in input.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                result = Err(e);
                break;
            }
        };
        let forward = match serde_json::from_str::<Value>(line.trim()) {
            Ok(Value::Array(batch)) if !batch.is_empty() => {
                let (mut keep, mut rejected) = (Vec::new(), Vec::new());
                for m in batch {
                    match front(&tools, m) {
                        Front::Reply(r) => rejected.push(r),
                        Front::Forward(m, p) => {
                            if let (Some((id, p)), Ok(mut map)) = (p, pending.lock()) {
                                map.insert(id, p);
                            }
                            keep.push(m);
                        }
                    }
                }
                // the batch is answered as one array: BR's own answers join LightCraft's
                match keep.iter().find_map(|m| valid_id(m.get("id")).filter(|_| m.get("method").is_some())) {
                    Some(id) if !rejected.is_empty() => {
                        if let Ok(mut map) = extras.lock() {
                            map.insert(id.to_string(), rejected);
                        }
                    }
                    _ if !rejected.is_empty() => reply(&Value::Array(rejected))?,
                    _ => {}
                }
                (!keep.is_empty()).then(|| Value::Array(keep).to_string())
            }
            Ok(msg @ Value::Object(_)) => match front(&tools, msg) {
                Front::Reply(r) => {
                    reply(&r)?;
                    None
                }
                Front::Forward(m, p) => {
                    if let (Some((id, p)), Ok(mut map)) = (p, pending.lock()) {
                        map.insert(id, p);
                    }
                    Some(m.to_string())
                }
            },
            // blank lines, bad JSON, empty batches: LightCraft answers them as it would
            _ => Some(line),
        };
        if let Some(f) = forward
            && (writeln!(to_lc, "{f}").is_err() || to_lc.flush().is_err())
        {
            break; // LightCraft's server has stopped
        }
    }
    drop(to_lc); // end of input for LightCraft's server
    let served = server.join().map_err(|_| std::io::Error::other("the MCP server thread panicked"))?;
    let pumped = pump.join().map_err(|_| std::io::Error::other("the MCP output thread panicked"))?;
    result.and(served).and(pumped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_command_and_a_valid_name() {
        for t in all_tools() {
            let name = t["name"].as_str().unwrap();
            assert!(name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{name}");
            assert!(is_br_tool(name), "{name}");
            if !matches!(name, "br_preview" | "br_video_preview" | "film_run") {
                assert!(command_of(name).is_some(), "{name}");
            }
        }
        // and every video command has a tool
        for c in br_video::COMMANDS {
            assert!(VIDEO_TOOLS.iter().any(|(_, id)| *id == c.id), "{} has no tool", c.id);
        }
        let names: Vec<String> = all_tools().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "tool names are unique");
        // and every command has a tool
        for (id, _, _) in commands::COMMANDS {
            let name = format!("br_{}", id.trim_start_matches("br."));
            assert!(tools().iter().any(|t| t["name"] == name.as_str()), "{id}");
        }
    }

    #[test]
    fn edits_are_destructive_reads_and_exports_are_not() {
        for t in all_tools() {
            let a = &t["annotations"];
            let name = t["name"].as_str().unwrap();
            let expect = a["readOnlyHint"] == false && !matches!(name, "br_export" | "br_video_export");
            assert_eq!(a["destructiveHint"], expect, "{name}");
        }
    }

    #[test]
    fn arguments_are_checked_against_the_schema() {
        let tools = all_tools();
        let def = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap().clone();
        assert!(check_args(&def("br_video_add"), &json!({"paths": ["/a.mp4"], "at": "end"})).is_ok());
        assert!(check_args(&def("film_run"), &json!({"command": "sequence.inspect", "params": {}})).is_ok());
        assert!(check_args(&def("br_adjust"), &json!({"light": 10, "id": 3})).is_ok());
        for (tool, args) in [
            ("br_adjust", json!({"brightness": 1})),
            ("br_adjust", json!({"light": 101})),
            ("br_adjust", json!({"light": "10"})),
            ("br_state", json!({"id": "14"})),
            ("br_state", json!({"id": 14.5})),
            ("br_preview", json!({"id": "3"})),
            ("br_crop", json!({"format": "3:2"})),
            ("br_crop", json!({})),
            ("br_look", json!({"look": "lc.moody", "amount": -1})),
            ("br_video_add", json!({"paths": "/a.mp4"})),
            ("br_video_add", json!({"paths": [1]})),
            ("br_video_delete", json!({"clips": ["3"]})),
            ("br_video_music", json!({"path": "/m.mp3", "duck": "yes"})),
            ("br_video_format", json!({"format": "3:2"})),
            ("br_video_speed", json!({"speed": 0})),
            ("film_run", json!({"params": {}})),
            ("film_run", json!({"command": "x", "params": [1]})),
        ] {
            assert!(check_args(&def(tool), &args).is_err(), "{tool} {args}");
        }
    }
}
