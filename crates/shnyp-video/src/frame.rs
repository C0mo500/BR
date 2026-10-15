//! The picture of a clip in the frame: where it is (`shnyp.video.transform`, `shnyp.video.fit`), what
//! fills the frame around it (`shnyp.video.background`, CapCut's "Hintergrund": blurred, a colour or
//! a picture) and how it follows its subject when the frame has another shape
//! (`shnyp.video.reframe`).
//!
//! A filled background is a clip of its own on a track below the main track (FilmCraft renders
//! effects in the clip's own pixels, before Motion places it, so one clip cannot draw outside its
//! rectangle). It shares the main clip's `link`, so FilmCraft's own trim, speed change, ripple
//! delete and move carry it along, and shnyp's commands treat it as the main clip's partner; a hidden
//! `shnyp.bg` effect marks it, so shnyp leaves it out of selections, filters, fitting and anchors.

use filmcraft_engine::{EngineError, Session};
use filmcraft_project::{ClipId, Sequence, TrackItem};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::{Format, inspect, magnet, main_track};

type R<T> = filmcraft_engine::Result<T>;

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

/// The clips a picture command works on: `clips` (or `clip`), else the selection; backgrounds
/// stand for their main clip, sound and captions are left out.
fn pictures(s: &Session, p: &Value) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let mut p = p.clone();
    if let Some(c) = p.get("clip").cloned().filter(Value::is_u64) {
        p["clips"] = json!([c]);
    }
    crate::chosen_or_selected(s, &p).into_iter().filter(|c| q.video_tracks.iter().any(|t| t.item(*c).is_some())).collect()
}

fn motion_index(q: &Sequence, c: ClipId) -> Option<usize> {
    q.find_item(c).and_then(|(_, it)| it.effects.iter().position(|e| e.effect == "motion"))
}

/// `shnyp.video.transform`: position (sequence pixels), scale (%), rotation (degrees), anchor and
/// opacity of the clips, at the playhead when they animate. One undo step (a `gesture` one for
/// all its calls).
pub fn transform(s: &mut Session, p: &Value) -> Result<Value, String> {
    let clips = pictures(s, p);
    if clips.is_empty() {
        return Err("select a picture (or give `clips`)".into());
    }
    let mut sets: Vec<(&str, &str, Value)> = Vec::new();
    for (key, effect, param) in [
        ("position", "motion", "position"),
        ("scale", "motion", "scale"),
        ("rotation", "motion", "rotation"),
        ("anchor", "motion", "anchor"),
        ("opacity", "opacity", "opacity"),
    ] {
        if let Some(v) = p.get(key).filter(|v| !v.is_null()) {
            let def = filmcraft_project::find_effect(effect).and_then(|d| d.param(param)).ok_or("no schema")?;
            sets.push((effect, param, inspect::param_input(def, v)?));
        }
    }
    if sets.is_empty() {
        return Err("give position, scale, rotation, anchor or opacity".into());
    }
    let gesture = p.get("gesture").and_then(Value::as_str);
    let keyframe = p.get("keyframe").and_then(Value::as_bool).unwrap_or(false);
    let t = s.playhead();
    inspect::step(s, "Transformieren", gesture, |s| {
        for c in &clips {
            for (effect, param, v) in &sets {
                let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
                let (_, it) = q.find_item(*c).ok_or_else(|| other("no such clip"))?;
                let at = t.clamp(it.start, (it.end() - Tick(1)).max(it.start));
                let i = it.effects.iter().position(|e| e.effect == *effect).ok_or_else(|| other(format!("the clip has no {effect}")))?;
                s.execute("effects.setParam", json!({"clip": c.0, "effect": i, "param": param, "value": v, "time": at.0, "keyframe": keyframe}))?;
            }
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    Ok(json!({"clips": clips.iter().map(|c| c.0).collect::<Vec<_>>()}))
}

/// `shnyp.video.fit`: fit the clips into the frame, fill it with them, or put Motion back as it was.
pub fn fit(s: &mut Session, p: &Value) -> Result<Value, String> {
    let clips = pictures(s, p);
    if clips.is_empty() {
        return Err("select a picture (or give `clips`)".into());
    }
    let ids: Vec<u64> = clips.iter().map(|c| c.0).collect();
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("fit");
    let label = match mode {
        "fit" => "Einpassen",
        "fill" => "Füllen",
        "reset" => "Bewegung zurücksetzen",
        other => return Err(format!("unknown mode `{other}`: fit, fill or reset")),
    };
    inspect::step(s, label, None, |s| {
        match mode {
            "fit" => s.execute("clip.fitToFrame", json!({"clips": ids}))?,
            "fill" => s.execute("clip.fillFrame", json!({"clips": ids}))?,
            _ => {
                for c in &clips {
                    let i = s.active_sequence().and_then(|q| motion_index(q, *c)).ok_or_else(|| other("the clip has no Motion"))?;
                    s.execute("effects.reset", json!({"clip": c.0, "index": i}))?;
                }
                Value::Null
            }
        };
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    Ok(json!({"clips": ids, "mode": mode}))
}

/// How a background is made.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    None,
    /// The clip itself, blurred (amount 0…100).
    Blur(f64),
    Color(String),
    Image(String),
}

/// The background of a main clip, as the inspector shows it.
pub fn fill_of(q: &Sequence, c: ClipId) -> Fill {
    let Some(bg) = background_of(q, c) else { return Fill::None };
    if let Some(b) = bg.effect("gaussian_blur") {
        let x = b.params.get("blurriness").and_then(|p| p.value.as_f64()).unwrap_or(60.0);
        return Fill::Blur(((x - 10.0) / 1.4).clamp(0.0, 100.0));
    }
    let marker = main_track::effect_of(bg, main_track::BG_CLIP);
    match marker.and_then(|m| m.params.get("color")).map(|p| &p.value) {
        Some(filmcraft_project::ParamValue::Text(c)) => Fill::Color(c.clone()),
        _ => Fill::Image(String::new()),
    }
}

/// The background clip behind main clip `c`.
pub fn background_of(q: &Sequence, c: ClipId) -> Option<&TrackItem> {
    let (_, it) = q.find_item(c)?;
    let link = it.link?;
    q.video_tracks
        .iter()
        .filter(|t| main_track::is_background_track(t))
        .flat_map(|t| &t.items)
        .find(|b| b.link == Some(link) && main_track::is_background(b))
}

fn blurriness(amount: f64) -> f64 {
    10.0 + amount.clamp(0.0, 100.0) * 1.4
}

/// The track for backgrounds, right below the main track (made the first time).
fn background_track(s: &mut Session) -> R<filmcraft_project::TrackId> {
    if let Some(t) = s.active_sequence().and_then(|q| q.video_tracks.iter().find(|t| main_track::is_background_track(t))) {
        return Ok(t.id);
    }
    s.edit_sequence("Hintergrund", |q, ctx, _| {
        let main = main_track::ensure(q).ok_or(EngineError::NoSequence)?;
        let at = q.video_tracks.iter().position(|t| t.id == main).unwrap_or(0);
        let mut t = main_track::new_track(filmcraft_project::TrackId(ctx.alloc()), filmcraft_project::TrackKind::Video, 0);
        t.name = "Hintergrund".into();
        // it moves with the main track, as the main clips' own sound does
        t.sync_lock = true;
        t.effects.push(main_track::hidden(main_track::BACKGROUND, &[]));
        let id = t.id;
        q.video_tracks.insert(at, t);
        Ok(id)
    })
}

/// `shnyp.video.background`: fill the frame behind the main clips (the chosen ones, else the
/// selected, else all) with the clip blurred, a colour or a picture, or take that away.
pub fn background(s: &mut Session, p: &Value) -> Result<Value, String> {
    let q = s.active_sequence().ok_or("no video yet")?;
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("blur");
    let fill = match mode {
        "none" => Fill::None,
        "blur" => Fill::Blur(p.get("amount").and_then(Value::as_f64).unwrap_or(50.0)),
        "color" => {
            let c = p.get("color").and_then(Value::as_str).unwrap_or("#000000");
            let ok = c.strip_prefix('#').is_some_and(|h| h.len() == 6 && h.chars().all(|x| x.is_ascii_hexdigit()));
            if !ok {
                return Err(format!("`color` is #rrggbb, not `{c}`"));
            }
            Fill::Color(c.to_ascii_lowercase())
        }
        "image" => Fill::Image(p.get("path").and_then(Value::as_str).ok_or("`image` needs `path`")?.to_string()),
        other => return Err(format!("unknown mode `{other}`: blur, color, image or none")),
    };
    // the main clips meant: chosen, else selected, else all
    let chosen: Vec<ClipId> = pictures(s, p).into_iter().filter(|c| magnet::is_main(q, *c)).collect();
    let mains: Vec<ClipId> = if chosen.is_empty() && p.get("clips").is_none() && p.get("clip").is_none() {
        let sel: Vec<ClipId> = pictures(s, &json!({})).into_iter().filter(|c| magnet::is_main(q, *c)).collect();
        if sel.is_empty() { main_track::main_clips(q).iter().map(|it| it.id).collect() } else { sel }
    } else {
        chosen
    };
    if mains.is_empty() {
        return Err("no main clips to put a background behind".into());
    }
    let label = match &fill {
        Fill::None => "Hintergrund entfernen",
        _ => "Hintergrund füllen",
    };
    let n = s
        .edit_group(label, |s| {
            remove_backgrounds(s, &mains)?;
            if fill == Fill::None {
                return Ok(0);
            }
            let track = background_track(s)?;
            let made = match &fill {
                Fill::Blur(amount) => blurred(s, &mains, track, *amount)?,
                Fill::Color(c) => {
                    let longest = longest(s, &mains);
                    let item = s.execute("file.newColorMatte", json!({"color": c, "seconds": longest + 1.0}))?["item"]
                        .as_u64()
                        .ok_or_else(|| other("no colour matte"))?;
                    placed(s, &mains, track, item, Some(c))?
                }
                Fill::Image(path) => {
                    let imported = s.execute("file.import", json!({"paths": [path]}))?;
                    let item = crate::ids(&imported, "items").first().copied().ok_or_else(|| other(format!("{path} ließ sich nicht lesen")))?;
                    placed(s, &mains, track, item, None)?
                }
                Fill::None => Vec::new(),
            };
            let ids: Vec<u64> = made.iter().map(|c| c.0).collect();
            if !ids.is_empty() {
                s.execute("clip.fillFrame", json!({"clips": ids}))?;
            }
            Ok(made.len())
        })
        .map_err(|e| e.to_string())?;
    Ok(json!({"backgrounds": n, "mode": mode}))
}

fn longest(s: &Session, mains: &[ClipId]) -> f64 {
    let q = s.active_sequence();
    mains.iter().filter_map(|c| q.and_then(|q| q.find_item(*c)).map(|(_, it)| it.duration.seconds())).fold(0.0, f64::max)
}

/// Take away the backgrounds behind `mains` (unlinked first: FilmCraft deletes linked partners).
fn remove_backgrounds(s: &mut Session, mains: &[ClipId]) -> R<()> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let old: Vec<ClipId> = mains.iter().filter_map(|c| background_of(q, *c).map(|b| b.id)).collect();
    if old.is_empty() {
        return Ok(());
    }
    s.edit_sequence("Hintergrund entfernen", |q, _, _| {
        for t in q.video_tracks.iter_mut().filter(|t| main_track::is_background_track(t)) {
            t.items.retain(|it| !old.contains(&it.id));
        }
        Ok(())
    })
}

/// The link of a main clip, made if it has none (a picture without sound).
fn link_of(it: &mut TrackItem, ctx: &mut filmcraft_edit::EditCtx) -> u64 {
    *it.link.get_or_insert_with(|| ctx.alloc())
}

/// A blurred copy of each main clip below it.
fn blurred(s: &mut Session, mains: &[ClipId], track: filmcraft_project::TrackId, amount: f64) -> R<Vec<ClipId>> {
    let blur = filmcraft_project::find_effect("gaussian_blur").ok_or_else(|| other("no Gaussian Blur"))?;
    s.edit_sequence("Hintergrund füllen", |q, ctx, _| {
        let mut made = Vec::new();
        for c in mains {
            let Some((_, it)) = q.find_item_mut(*c) else { continue };
            let link = link_of(it, ctx);
            let mut bg = it.clone();
            bg.id = ClipId(ctx.alloc());
            bg.link = Some(link);
            bg.group = None;
            bg.markers.clear();
            let mut fx = blur.instance();
            if let Some(p) = fx.param_mut("blurriness") {
                p.value = filmcraft_project::ParamValue::Float(blurriness(amount));
            }
            if let Some(p) = fx.param_mut("repeat_edge") {
                p.value = filmcraft_project::ParamValue::Bool(true);
            }
            // the picture only: its own Motion, Opacity and time, a blur, the mark
            let mut effects = vec![fx];
            effects.extend(filmcraft_project::effect::intrinsic_video());
            effects.push(main_track::hidden(main_track::BG_CLIP, &[]));
            bg.effects = effects;
            let t = q.video_tracks.iter_mut().find(|t| t.id == track).ok_or(EngineError::NoSequence)?;
            let at = t.items.partition_point(|x| x.start < bg.start);
            t.items.insert(at, bg.clone());
            made.push(bg.id);
        }
        Ok(made)
    })
}

/// `item` (a colour matte or a picture) behind each main clip, as long as it.
fn placed(s: &mut Session, mains: &[ClipId], track: filmcraft_project::TrackId, item: u64, color: Option<&str>) -> R<Vec<ClipId>> {
    let mut made = Vec::new();
    for c in mains {
        let (start, len) = {
            let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
            let (_, it) = q.find_item(*c).ok_or_else(|| other("no such clip"))?;
            (it.start, it.duration)
        };
        let r = s.execute(
            "timeline.place",
            json!({"item": item, "track": track.0, "time": start.0, "duration": len.0, "insert": false, "audio": false}),
        )?;
        made.extend(crate::ids(&r, "clips").into_iter().map(ClipId));
    }
    let pairs: Vec<(ClipId, ClipId)> = mains.iter().copied().zip(made.iter().copied()).collect();
    let color = color.map(str::to_string);
    s.edit_sequence("Hintergrund füllen", |q, ctx, _| {
        for (main, bg) in &pairs {
            let Some((_, it)) = q.find_item_mut(*main) else { continue };
            let link = link_of(it, ctx);
            let Some((_, b)) = q.find_item_mut(*bg) else { continue };
            b.link = Some(link);
            let params: Vec<(&str, filmcraft_project::ParamValue)> =
                color.iter().map(|c| ("color", filmcraft_project::ParamValue::Text(c.clone()))).collect();
            b.effects.push(main_track::hidden(main_track::BG_CLIP, &params));
        }
        Ok(())
    })?;
    Ok(made)
}

/// The aspect `auto_reframe` frames for: the format's name in its list ("Sequence" is never used:
/// it frames wrongly after a change of shape).
fn reframe_aspect(f: Format) -> &'static str {
    match f {
        Format::Vertical => "Vertical 9:16",
        Format::Wide => "Horizontal 16:9",
        Format::Square => "Square 1:1",
        Format::Portrait => "Vertical 4:5",
    }
}

/// A clip's picture has another shape than the frame.
pub(crate) fn other_shape(s: &Session, q: &Sequence, it: &TrackItem) -> bool {
    let Some((w, h)) = s.project.conformed_source_size(it.item, q.settings.par) else { return false };
    let (fw, fh) = (q.settings.width as f64, q.settings.height as f64);
    ((w / h.max(1.0)) - (fw / fh.max(1.0))).abs() > 0.01
}

/// The clips reframing is about: media pictures (no titles, mattes or backgrounds).
fn media_pictures(s: &Session, q: &Sequence) -> Vec<ClipId> {
    q.video_tracks
        .iter()
        .flat_map(|t| &t.items)
        .filter(|it| !main_track::is_background(it))
        .filter(|it| {
            s.project
                .item(it.item)
                .is_some_and(|i| matches!(i.kind, filmcraft_project::ItemKind::Media(_) | filmcraft_project::ItemKind::Subclip { .. }))
        })
        .filter(|it| s.project.resolve_media(it.item).is_some_and(|(_, m, _)| m.info.kind != filmcraft_media::MediaKind::Synthetic))
        .map(|it| it.id)
        .collect()
}

/// Reframe (inside the caller's step): the clips of another shape fill the frame and follow
/// their subject (FilmCraft's Auto Reframe for this format); the others, and all with `on` off,
/// lose it.
pub(crate) fn reframe_clips(s: &mut Session, clips: &[ClipId], on: bool, preset: u32) -> R<usize> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let format = Format::of_size(q.settings.width, q.settings.height);
    let mut fill = Vec::new();
    let mut n = 0;
    for c in clips {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        let Some((_, it)) = q.find_item(*c) else { continue };
        let wanted = on && format.is_some() && other_shape(s, q, it);
        // what is there goes first (an index is looked up afresh each time)
        while let Some(i) =
            s.active_sequence().and_then(|q| q.find_item(*c)).and_then(|(_, it)| it.effects.iter().position(|e| e.effect == "auto_reframe"))
        {
            if wanted {
                break;
            }
            s.execute("effects.remove", json!({"clip": c.0, "index": i}))?;
        }
        if !wanted {
            continue;
        }
        let has = s.active_sequence().and_then(|q| q.find_item(*c)).is_some_and(|(_, it)| it.effect("auto_reframe").is_some());
        if !has {
            s.execute("effects.apply", json!({"clips": [c.0], "effect": "auto_reframe"}))?;
        }
        let i = s
            .active_sequence()
            .and_then(|q| q.find_item(*c))
            .and_then(|(_, it)| it.effects.iter().position(|e| e.effect == "auto_reframe"))
            .ok_or_else(|| other("Auto Reframe did not go on the clip"))?;
        let aspect = reframe_aspect(format.unwrap_or(Format::Vertical));
        s.execute("effects.setParam", json!({"clip": c.0, "effect": i, "param": "aspect", "value": aspect}))?;
        s.execute("effects.setParam", json!({"clip": c.0, "effect": i, "param": "preset", "value": preset}))?;
        fill.push(c.0);
        n += 1;
    }
    if !fill.is_empty() {
        s.execute("clip.fillFrame", json!({"clips": fill}))?;
    }
    Ok(n)
}

/// `shnyp.video.reframe`: the clips (chosen, else selected, else all pictures) of another shape
/// than the frame fill it and follow their subject; `on: false` takes that back (they fit).
pub fn reframe(s: &mut Session, p: &Value) -> Result<Value, String> {
    let q = s.active_sequence().ok_or("no video yet")?;
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
    let preset = match p.get("speed").and_then(Value::as_str).unwrap_or("default") {
        "slow" => 0,
        "default" => 1,
        "fast" => 2,
        other => return Err(format!("unknown speed `{other}`: slow, default or fast")),
    };
    let all = media_pictures(s, q);
    let chosen: Vec<ClipId> = pictures(s, p).into_iter().filter(|c| all.contains(c)).collect();
    let clips = if chosen.is_empty() { all } else { chosen };
    let label = if on { "Automatisch ausrichten" } else { "Automatisch ausrichten aus" };
    let n = s
        .edit_group(label, |s| {
            let n = reframe_clips(s, &clips, on, preset)?;
            if !on {
                let ids: Vec<u64> = clips.iter().map(|c| c.0).collect();
                if !ids.is_empty() {
                    s.execute("clip.fitToFrame", json!({"clips": ids}))?;
                }
            }
            Ok(n)
        })
        .map_err(|e| e.to_string())?;
    Ok(json!({"reframed": n, "on": on}))
}

/// After the frame changed shape (inside `shnyp.video.format`'s step): every background fills the
/// new frame, and each Auto Reframe frames for it.
pub(crate) fn after_format(s: &mut Session) -> R<()> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let bgs: Vec<u64> = q.video_tracks.iter().flat_map(|t| &t.items).filter(|it| main_track::is_background(it)).map(|it| it.id.0).collect();
    let reframed: Vec<ClipId> =
        q.video_tracks.iter().flat_map(|t| &t.items).filter(|it| it.effect("auto_reframe").is_some()).map(|it| it.id).collect();
    if !bgs.is_empty() {
        s.execute("clip.fillFrame", json!({"clips": bgs}))?;
    }
    if !reframed.is_empty() {
        let preset = s
            .active_sequence()
            .and_then(|q| q.find_item(reframed[0]))
            .and_then(|(_, it)| it.effect("auto_reframe")?.params.get("preset")?.value.as_f64())
            .unwrap_or(1.0) as u32;
        reframe_clips(s, &reframed, true, preset)?;
    }
    Ok(())
}

/// The media pictures a format change fits or fills (not backgrounds, not ones that reframe).
pub(crate) fn to_fit(s: &Session) -> Vec<u64> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    media_pictures(s, q).into_iter().filter(|c| q.find_item(*c).is_some_and(|(_, it)| it.effect("auto_reframe").is_none())).map(|c| c.0).collect()
}

/// Titles and shapes after the frame changed shape (inside `shnyp.video.format`'s step, after
/// FilmCraft scaled every clip's Motion to fit the old frame into the new one): a graphic's canvas
/// is the frame, so it becomes the new frame; its layers keep their place relative to the frame
/// and their size relative to its short side (a title made for 9:16 reads the same in 16:9), and
/// the clip's own Motion is what it was, moved with the frame.
pub(crate) fn refit_graphics(s: &mut Session, before: &Sequence) -> R<()> {
    let Some(q) = s.active_sequence() else { return Ok(()) };
    let (w0, h0) = (before.settings.width as f64, before.settings.height as f64);
    let (w1, h1) = (q.settings.width as f64, q.settings.height as f64);
    if (w0, h0) == (w1, h1) || w0 <= 0.0 || h0 <= 0.0 {
        return Ok(());
    }
    let graphics: Vec<(ClipId, filmcraft_project::ItemId)> = q
        .video_tracks
        .iter()
        .flat_map(|t| &t.items)
        .filter(|it| matches!(s.project.item(it.item).map(|i| &i.kind), Some(filmcraft_project::ItemKind::Graphic { .. })))
        .map(|it| (it.id, it.item))
        .collect();
    if graphics.is_empty() {
        return Ok(());
    }
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let (rx, ry, k) = (w1 / w0, h1 / h0, w1.min(h1) / w0.min(h0));
    let before = before.clone();
    s.edit("Titel anpassen", |p, _| {
        for (_, item) in &graphics {
            if let Some(filmcraft_project::ItemKind::Graphic { width, height, .. }) = p.items.get_mut(item).map(|i| &mut i.kind) {
                *width = w1 as u32;
                *height = h1 as u32;
            }
        }
        let q = p.sequence_mut(seq).ok_or(EngineError::NoSequence)?;
        let map_point = |v: filmcraft_project::ParamValue| match v {
            filmcraft_project::ParamValue::Vec2(mut v) if v.x.is_finite() && v.y.is_finite() => {
                v.x *= rx;
                v.y *= ry;
                filmcraft_project::ParamValue::Vec2(v)
            }
            other => other,
        };
        for (c, _) in &graphics {
            let old_motion = before.find_item(*c).and_then(|(_, it)| it.effect("motion").cloned());
            let Some((_, it)) = q.find_item_mut(*c) else { continue };
            // the clip's Motion as it was, its position moved with the frame
            if let (Some(old), Some(m)) = (old_motion, it.effect_mut("motion")) {
                *m = old;
                if let Some(p) = m.param_mut("position") {
                    p.map_values(map_point);
                }
            }
            for e in it.effects.iter_mut().filter(|e| filmcraft_project::graphic::is_layer(e)) {
                if let Some(p) = e.param_mut("position") {
                    p.map_values(map_point);
                }
                for id in ["size", "box_width", "box_height", "stroke_width", "shadow_distance", "background_size", "corner_radius"] {
                    if let Some(p) = e.param_mut(id) {
                        p.map_values(|v| match v {
                            filmcraft_project::ParamValue::Float(x) => filmcraft_project::ParamValue::Float(x * k),
                            filmcraft_project::ParamValue::Vec2(mut v) => {
                                v.x *= k;
                                v.y *= k;
                                filmcraft_project::ParamValue::Vec2(v)
                            }
                            other => other,
                        });
                    }
                }
            }
        }
        Ok(())
    })
}
