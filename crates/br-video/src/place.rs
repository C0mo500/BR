//! Putting media where it is dropped, CapCut's way: every drag-and-drop path (BR's asset bar,
//! FilmCraft's Project panel and Source monitor, files from the file manager, the browser page)
//! ends in [`place`], so a drop and the same request from Claude give the same video.
//!
//! - On the main track: inserted at the cut nearest to the drop (nothing is cut in two), the
//!   rest moves on; the clip's sound goes with it.
//! - Above it: an overlay at the drop time on the overlay track asked for when it is free there,
//!   else the first free one (a new one only when none is), at `position` in the picture.
//! - Sound alone: at the drop time on a free sound track; a long piece is music, a short one a
//!   sound effect that hangs on the clip it starts on.
//! - Subtitle files become captions.
//!
//! Import, placing and what follows are one undo step.

use filmcraft_engine::{EngineError, Session};
use filmcraft_project::{ClipId, TrackId};
use filmcraft_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::{magnet, main_track};

type R<T> = filmcraft_engine::Result<T>;

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

/// Where a drop goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Pictures on the main track (onto the overlay track named by `track`), sound on its own.
    Auto,
    Main,
    Overlay,
    /// The sound of the items only.
    Audio,
}

impl Target {
    pub fn find(key: &str) -> Option<Target> {
        Some(match key {
            "auto" => Target::Auto,
            "main" => Target::Main,
            "overlay" => Target::Overlay,
            "audio" => Target::Audio,
            _ => return None,
        })
    }
}

/// A track named in a request: `V2`, `A3`, a track id, or `main`.
fn track_param(s: &Session, v: &Value) -> Result<Option<TrackId>, String> {
    let Some(q) = s.active_sequence() else { return Ok(None) };
    Ok(match v {
        Value::Null => None,
        Value::Number(n) => {
            let id = TrackId(n.as_u64().ok_or("`track` is a track id, V2, A3 or main")?);
            if q.track(id).is_none() {
                return Err(format!("no track {}", id.0));
            }
            Some(id)
        }
        Value::String(k) if k == "main" => main_track::main_track_id(q),
        Value::String(k) => {
            let (list, n) = match (k.get(..1), k.get(1..).and_then(|n| n.parse::<usize>().ok())) {
                (Some("V" | "v"), Some(n)) => (&q.video_tracks, n),
                (Some("A" | "a"), Some(n)) => (&q.audio_tracks, n),
                _ => return Err(format!("unknown track `{k}`: V2, A3, a track id or main")),
            };
            Some(list.get(n.wrapping_sub(1)).ok_or_else(|| format!("there is no track {k}"))?.id)
        }
        _ => return Err("`track` is a track id, V2, A3 or main".into()),
    })
}

/// A video track above the main track free from `t` for `len`: `wanted` if it is one and free,
/// else the first free one above the main track (a new one on top when none is). Its index.
fn free_overlay_track(s: &mut Session, t: Tick, len: Tick, wanted: Option<TrackId>) -> R<usize> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let main = main_track::main_track_id(q);
    let from = q.video_tracks.iter().position(|tr| Some(tr.id) == main).map_or(0, |i| i + 1);
    let range = TimeRange::new(t, len.max(Tick(1)));
    let free =
        |tr: &filmcraft_project::Track| !tr.locked && !main_track::is_background_track(tr) && tr.items.iter().all(|i| !i.range().overlaps(&range));
    let at_wanted = wanted.and_then(|w| q.video_tracks.iter().position(|tr| tr.id == w)).filter(|i| *i >= from && free(&q.video_tracks[*i]));
    if let Some(i) = at_wanted.or_else(|| q.video_tracks.iter().enumerate().skip(from).find(|(_, tr)| free(tr)).map(|(i, _)| i)) {
        return Ok(i);
    }
    s.execute("sequence.addTracks", json!({"video": 1, "audio": 0}))?;
    Ok(s.active_sequence().map(|q| q.video_tracks.len()).unwrap_or(1) - 1)
}

/// An overlay: `item` at `t` on a free track above the main track, its sound on a free sound
/// track, at `position` (0…1 of the picture) when given.
fn overlay(s: &mut Session, item: u64, t: Tick, wanted: Option<TrackId>, position: Option<(f64, f64)>, range: Option<(Tick, Tick)>) -> R<Vec<u64>> {
    let len = magnet::item_length(s, item, range);
    let i = free_overlay_track(s, t, len, wanted)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let track = q.video_tracks[i].id;
    let (_, has_audio) = s.project.source_streams(filmcraft_project::ItemId(item));
    let mut p = json!({"item": item, "track": track.0, "time": t.0, "insert": false});
    if has_audio {
        p["audioTrack"] = magnet::free_sound_track(q, t, len, None).map_or(json!("new"), |id| json!(id.0));
    }
    if let Some((from, len)) = range {
        p["sourceIn"] = json!(from.0);
        p["duration"] = json!(len.0);
    }
    let placed = s.execute("timeline.place", p)?;
    let clips: Vec<u64> = placed.get("clips").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
    if let Some((x, y)) = position {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        let (w, h) = (q.settings.width as f64, q.settings.height as f64);
        let pictures: Vec<(u64, usize)> = clips
            .iter()
            .filter_map(|c| {
                q.find_item(ClipId(*c))
                    .filter(|(tr, _)| *tr == track)
                    .and_then(|(_, it)| it.effects.iter().position(|e| e.effect == "motion").map(|m| (*c, m)))
            })
            .collect();
        for (c, m) in pictures {
            s.execute(
                "effects.setParam",
                json!({"clip": c, "effect": m, "param": "position", "value": [x.clamp(0.0, 1.0) * w, y.clamp(0.0, 1.0) * h]}),
            )?;
        }
    }
    Ok(clips)
}

/// `br.video.place`.
pub fn place(s: &mut Session, p: &Value) -> Result<Value, String> {
    let target = match p.get("target").and_then(Value::as_str) {
        None => Target::Auto,
        Some(k) => Target::find(k).ok_or_else(|| format!("unknown target `{k}`: auto, main, overlay or audio"))?,
    };
    let paths: Vec<String> = match p.get("paths") {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        Some(Value::String(one)) => vec![one.clone()],
        _ => Vec::new(),
    };
    let paths = crate::expand_folders(&paths);
    let mut items: Vec<u64> = p.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
    if paths.is_empty() && items.is_empty() {
        return Err("needs `paths` or `items`".into());
    }
    let at = match p.get("seconds") {
        None | Some(Value::Null) => None,
        Some(v) => Some(Tick::from_seconds_f64(v.as_f64().filter(|x| x.is_finite() && *x >= 0.0).ok_or("`seconds` must be a time ≥ 0")?)),
    };
    let wanted = track_param(s, p.get("track").unwrap_or(&Value::Null))?;
    let position = match p.get("position") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let xy = v.as_array().filter(|a| a.len() == 2).and_then(|a| Some((a[0].as_f64()?, a[1].as_f64()?)));
            Some(xy.filter(|(x, y)| x.is_finite() && y.is_finite()).ok_or("`position` is [x, y] in 0…1 of the picture")?)
        }
    };
    let range = match (p.get("sourceIn").and_then(Value::as_f64), p.get("duration").and_then(Value::as_f64)) {
        (Some(a), Some(d)) if a >= 0.0 && d > 0.0 => Some((Tick::from_seconds_f64(a), Tick::from_seconds_f64(d))),
        (None, None) => None,
        _ => return Err("`sourceIn` and `duration` are seconds (both, duration > 0)".into()),
    };
    let kind = |p: &String| crate::file_kind(p).unwrap_or("");
    let captions: Vec<String> = paths.iter().filter(|p| kind(p) == "caption").cloned().collect();
    let media: Vec<String> = paths.iter().filter(|p| matches!(kind(p), "video" | "audio" | "image")).cloned().collect();
    if let Some(project) = paths.iter().find(|p| kind(p) == "project") {
        return Err(format!("{project} is a project: file.open opens it"));
    }
    let mut errors: Vec<Value> = Vec::new();
    let placed = s
        .edit_group("Einfügen", |s| {
            if !media.is_empty() {
                let imported = s.execute("file.import", json!({"paths": media}))?;
                errors = imported["errors"].as_array().cloned().unwrap_or_default();
                items.extend(crate::ids(&imported, "items"));
            }
            let mut clips: Vec<u64> = Vec::new();
            if !items.is_empty() {
                clips = place_items(s, &items, target, at, wanted, position, range)?;
            }
            for c in &captions {
                s.execute("captions.import", json!({"path": c}))?;
            }
            if clips.is_empty() && captions.is_empty() {
                return Err(other("nichts davon ließ sich einfügen"));
            }
            Ok(clips)
        })
        .map_err(|e| e.to_string())?;
    let mut out = crate::state(s)?;
    out["clips"] = json!(placed);
    if !errors.is_empty() {
        out["errors"] = json!(errors);
    }
    Ok(out)
}

/// Place imported items by the rules above. The new clips.
fn place_items(
    s: &mut Session,
    items: &[u64],
    target: Target,
    at: Option<Tick>,
    wanted: Option<TrackId>,
    position: Option<(f64, f64)>,
    range: Option<(Tick, Tick)>,
) -> R<Vec<u64>> {
    if s.active_sequence().is_none() {
        // the first media make the video
        crate::place_media(s, items, None)?;
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        return Ok(q.video_tracks.iter().chain(&q.audio_tracks).flat_map(|t| &t.items).map(|it| it.id.0).collect());
    }
    let t = at.unwrap_or_else(|| s.playhead());
    let (main, wanted_video, wanted_audio) = {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        let main = main_track::main_track_id(q);
        let video = wanted.filter(|w| q.video_tracks.iter().any(|tr| tr.id == *w));
        let audio = wanted.filter(|w| q.audio_tracks.iter().any(|tr| tr.id == *w));
        (main, video, audio)
    };
    let mut clips = Vec::new();
    let mut to_main = Vec::new();
    for item in items {
        let (picture, _) = s.project.source_streams(filmcraft_project::ItemId(*item));
        let go = match target {
            Target::Audio => Target::Audio,
            _ if !picture => Target::Audio,
            Target::Auto if wanted_video.is_some() && wanted_video != main => Target::Overlay,
            Target::Auto if wanted_audio.is_some() => Target::Audio,
            Target::Auto => Target::Main,
            other => other,
        };
        match go {
            Target::Main => to_main.push(*item),
            Target::Overlay => clips.extend(overlay(s, *item, t, wanted_video, position, range)?),
            _ => clips.extend(magnet::place_sound(s, *item, t, wanted_audio, range)?),
        }
    }
    if !to_main.is_empty() {
        clips.extend(magnet::insert_range(s, &to_main, Some(t), range)?);
    }
    Ok(clips)
}
