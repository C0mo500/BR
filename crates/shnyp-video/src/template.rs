//! Video templates, CapCut's "Vorlagen": a recipe of slots (each a length in seconds or in beats
//! of the music, maybe a speed curve or a transition after it), titles, a look and a background.
//! `shnyp.video.template.new` makes a video of grey placeholders from it, `…fill` puts the media in
//! (in order, each filling its slot; a clip too short plays slower), `…save` keeps the video's
//! shape as a template of the user's. FilmCraft's own project templates (`file.saveAsTemplate`)
//! stay as they are; these are shnyp's.
//!
//! Slots are main clips with a hidden `shnyp.slot` effect, so undo, save and every edit keep them.

use filmcraft_engine::{EngineError, Session};
use filmcraft_project::{ClipId, ItemId, ParamValue, Sequence};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::anchors::Orphans;
use crate::{magnet, main_track};

type R<T> = filmcraft_engine::Result<T>;

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

/// The hidden effect on a placeholder; `index` is its place in the template.
pub const SLOT: &str = "shnyp.slot";
/// The placeholders' grey.
const GREY: &str = "#2b2f33";

/// shnyp's own templates.
fn builtin() -> Vec<Value> {
    vec![
        json!({
            "id": "story", "name": "Schnelle Story", "description": "Sechs kurze Szenen, ein Titel am Anfang",
            "format": "9:16", "fit": "fill",
            "slots": [{"seconds": 1.4}, {"seconds": 1.2}, {"seconds": 1.2}, {"seconds": 1.2}, {"seconds": 1.4}, {"seconds": 2.0}],
            "texts": [{"text": "Dein Titel", "at": 0.2, "seconds": 2.0}],
        }),
        json!({
            "id": "reise", "name": "Reise-Montage", "description": "Fünf Szenen mit Tempo-Kurven und weichen Übergängen, warmer Look",
            "format": "9:16", "fit": "fill", "filter": "Golden Dusk",
            "slots": [{"seconds": 2.0, "transition": "cross_dissolve"}, {"seconds": 1.6, "curve": "montage"}, {"seconds": 1.6, "transition": "cross_dissolve"}, {"seconds": 2.0, "curve": "blitzRein"}, {"seconds": 3.0}],
            "texts": [{"text": "Unterwegs", "at": 0.3, "seconds": 2.2}],
        }),
        json!({
            "id": "takt", "name": "Im Takt", "description": "Acht Szenen, jede zwei Schläge der Musik lang",
            "format": "9:16", "fit": "fill",
            "slots": [{"beats": 2}, {"beats": 2}, {"beats": 2}, {"beats": 2}, {"beats": 2}, {"beats": 2}, {"beats": 2}, {"beats": 4}],
            "texts": [],
        }),
        json!({
            "id": "intro", "name": "YouTube-Intro", "description": "Drei Szenen im Querformat mit Titel, unscharfer Hintergrund",
            "format": "16:9", "fit": "fit", "background": "blur",
            "slots": [{"seconds": 3.0, "transition": "cross_dissolve"}, {"seconds": 3.0, "transition": "cross_dissolve"}, {"seconds": 4.0}],
            "texts": [{"text": "Willkommen", "at": 0.5, "seconds": 2.5}],
        }),
    ]
}

/// A template file's extension.
pub const EXT: &str = "shnypvt";
/// The extension a template saved while shnyp was called BR has (still read).
const OLD_EXT: &str = "brvt";

/// Where the user's templates are (`SHNYP_TEMPLATES`, else shnyp's data folder).
pub fn template_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("SHNYP_TEMPLATES").filter(|d| !d.is_empty()) {
        return d.into();
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).or_else(|| {
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| std::path::PathBuf::from(h).join(".local").join("share"))
    });
    data.unwrap_or_else(std::env::temp_dir).join("shnyp").join("Vorlagen")
}

fn user_templates(dir: &std::path::Path) -> Vec<Value> {
    let Ok(dir) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<Value> = dir
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == EXT || x == OLD_EXT))
        .filter_map(|e| {
            let mut t: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            t["source"] = json!("user");
            t["path"] = json!(e.path().to_string_lossy());
            Some(t)
        })
        .collect();
    v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    v
}

fn all(dir: &std::path::Path) -> Vec<Value> {
    builtin()
        .into_iter()
        .map(|mut t| {
            t["source"] = json!("builtin");
            t
        })
        .chain(user_templates(dir))
        .collect()
}

/// The folder a request names (`dir`), else [`template_dir`].
fn dir_of(p: &Value) -> std::path::PathBuf {
    p.get("dir").and_then(Value::as_str).filter(|d| !d.is_empty()).map_or_else(template_dir, std::path::PathBuf::from)
}

/// A template by id, name or file (the user's from `dir`).
pub fn find(key: &str, dir: &std::path::Path) -> Result<Value, String> {
    if [EXT, OLD_EXT].iter().any(|x| key.ends_with(&format!(".{x}"))) && std::path::Path::new(key).is_file() {
        let text = std::fs::read_to_string(key).map_err(|e| format!("{key}: {e}"))?;
        return serde_json::from_str(&text).map_err(|e| format!("{key}: {e}"));
    }
    all(dir)
        .into_iter()
        .find(|t| t["id"].as_str() == Some(key) || t["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(key)))
        .ok_or_else(|| format!("no template `{key}` (shnyp.video.template.list)"))
}

/// A template's length in seconds (slots in beats count half a second each at 120 bpm).
fn length(t: &Value) -> f64 {
    slots(t).iter().map(|s| s["seconds"].as_f64().unwrap_or_else(|| s["beats"].as_f64().unwrap_or(2.0) * 0.5)).sum()
}

fn slots(t: &Value) -> Vec<Value> {
    t["slots"].as_array().cloned().unwrap_or_default()
}

/// `shnyp.video.template.list`.
pub fn list(p: &Value) -> Value {
    json!(
        all(&dir_of(p))
            .iter()
            .map(|t| json!({
                "id": t["id"], "name": t["name"], "description": t["description"], "format": t["format"],
                "slots": slots(t).len(), "seconds": (length(t) * 10.0).round() / 10.0,
                "beats": slots(t).iter().any(|s| s.get("beats").is_some()), "source": t["source"], "path": t.get("path"),
            }))
            .collect::<Vec<_>>()
    )
}

/// The placeholders of the video, in order: (clip, index in the template). A slot baked into the
/// pieces of a speed curve is its first piece (a split or frozen slot stays two to fill).
pub fn placeholders(q: &Sequence) -> Vec<(ClipId, usize)> {
    main_track::main_clips(q)
        .iter()
        .filter(|it| crate::speed::curve_of(q, it.id).is_none() || crate::speed::curve_pieces(q, it.id).first() == Some(&it.id))
        .filter_map(|it| {
            let e = main_track::effect_of(it, SLOT)?;
            Some((it.id, e.param("index").and_then(|p| p.value.as_f64()).unwrap_or(0.0) as usize))
        })
        .collect()
}

/// Placeholders not filled yet (still the grey).
pub fn empty_placeholders(s: &Session) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    placeholders(q)
        .into_iter()
        .filter(|(c, _)| {
            q.find_item(*c)
                .and_then(|(_, it)| s.project.resolve_media(it.item))
                .is_none_or(|(_, m, _)| m.info.kind == filmcraft_media::MediaKind::Synthetic)
        })
        .map(|x| x.0)
        .collect()
}

fn frames(q: &Sequence, secs: f64) -> Tick {
    let fd = q.settings.frame_rate.frame_duration();
    Tick(((Tick::from_seconds_f64(secs).0 as f64 / fd.0 as f64).round() as i64).max(1) * fd.0)
}

/// `shnyp.video.template.new`: a new video of placeholders from a template (with `music`: under it,
/// slots in beats cut on its beats). One undo step.
pub fn new(s: &mut Session, p: &Value) -> Result<Value, String> {
    let key = p.get("template").and_then(Value::as_str).ok_or("give `template` (shnyp.video.template.list)")?;
    let t = find(key, &dir_of(p))?;
    let slots = slots(&t);
    if slots.is_empty() {
        return Err(format!("the template `{key}` has no slots"));
    }
    let music = p.get("music").and_then(Value::as_str).map(str::to_string);
    let name = t["name"].as_str().unwrap_or("Vorlage").to_string();
    s.edit_group(&format!("Vorlage {name}"), |s| {
        crate::execute_shnyp(s, "shnyp.video.new", &json!({"format": t["format"].as_str().unwrap_or("9:16"), "name": name})).map_err(other)?;
        let total = length(&t);
        let matte = s.execute("file.newColorMatte", json!({"color": GREY, "seconds": total + 60.0}))?["item"]
            .as_u64()
            .ok_or_else(|| other("no placeholder"))?;
        let main = s.active_sequence().and_then(main_track::main_track_id).ok_or(EngineError::NoSequence)?;
        let mut at = Tick::ZERO;
        let mut placed = Vec::new();
        for slot in &slots {
            let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
            let len = frames(q, slot["seconds"].as_f64().unwrap_or_else(|| slot["beats"].as_f64().unwrap_or(2.0) * 0.5));
            let r = s.execute(
                "timeline.place",
                json!({"item": matte, "track": main.0, "time": at.0, "duration": len.0, "insert": false, "audio": false}),
            )?;
            placed.extend(crate::ids(&r, "clips").into_iter().map(ClipId));
            at += len;
        }
        // the recipe goes with the video: filling it needs no file
        let tid = t.to_string();
        s.edit_sequence("Platzhalter", |q, _, _| {
            mark(q, &tid);
            for (i, c) in placed.iter().enumerate() {
                if let Some((_, it)) = q.find_item_mut(*c) {
                    it.name = format!("Platzhalter {}", i + 1);
                    it.effects.push(main_track::hidden(SLOT, &[("index", ParamValue::Float(i as f64))]));
                }
            }
            Ok(())
        })?;
        if let Some(path) = &music {
            crate::execute_shnyp(s, "shnyp.video.music", &json!({"path": path})).map_err(other)?;
            if slots.iter().any(|x| x.get("beats").is_some()) {
                on_beats(s, &placed, &slots)?;
            }
        }
        // transitions after slots, titles, the look
        for (slot, c) in slots.iter().zip(&placed) {
            if let Some(e) = slot["transition"].as_str() {
                let next = placed.iter().skip_while(|x| *x != c).nth(1);
                if next.is_some() {
                    s.execute("sequence.applyVideoTransition", json!({"clip": c.0, "edge": "out", "effect": e}))?;
                }
            }
        }
        for text in t["texts"].as_array().into_iter().flatten() {
            crate::execute_shnyp(s, "shnyp.video.text", &json!({"text": text["text"], "at": text["at"], "seconds": text["seconds"]}))
                .map_err(other)?;
        }
        if let Some(f) = t["filter"].as_str() {
            let ids: Vec<u64> = placed.iter().map(|c| c.0).collect();
            crate::execute_shnyp(s, "shnyp.video.filter", &json!({"preset": f, "clips": ids})).map_err(other)?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    let mut out = crate::state(s)?;
    out["template"] = json!(t["id"]);
    out["placeholders"] = json!(empty_placeholders(s).iter().map(|c| c.0).collect::<Vec<_>>());
    Ok(out)
}

/// Slots in beats: each placeholder as long as its beats of the music, then the music as long as
/// the video.
fn on_beats(s: &mut Session, placed: &[ClipId], slots: &[Value]) -> R<()> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let music = q
        .audio_tracks
        .iter()
        .flat_map(|t| &t.items)
        .find(|it| it.essential.as_ref().is_some_and(|e| e.kind == filmcraft_project::AudioType::Music))
        .map(|it| (it.id, it.item, it.source_in))
        .ok_or_else(|| other("no music"))?;
    // the beats of the whole piece, not only of the part under the placeholders
    let media = filmcraft_engine::media_duration(&s.project, &s.media, music.1).unwrap_or_default();
    s.edit_sequence("Musik", |q, _, _| {
        if let Some((_, it)) = q.find_item_mut(music.0) {
            it.duration = (media - it.source_in).max(it.duration);
        }
        Ok(())
    })?;
    let (_, beats) = crate::beats::detect(s, music.0, "beats")?;
    if beats.len() < 2 {
        return Ok(());
    }
    let period = (beats[beats.len() - 1] - beats[0]).0 / (beats.len() as i64 - 1);
    let beat = |n: usize| beats.get(n).copied().unwrap_or(beats[beats.len() - 1] + Tick(period * (n as i64 + 1 - beats.len() as i64)));
    s.edit_sequence("Im Takt", |q, ctx, _| {
        let before = q.clone();
        // the first cut on the first beat, then every slot its beats long
        let mut n = 0usize;
        let mut lens = Vec::new();
        for slot in slots {
            let k = slot["beats"].as_f64().map_or(4, |b| b.max(1.0) as usize);
            let from = if n == 0 { Tick::ZERO } else { beat(n) };
            lens.push((beat(n + k) - from).max(q.settings.frame_rate.frame_duration()));
            n += k;
        }
        let mut at = Tick::ZERO;
        let mut starts = Vec::new();
        for (c, len) in placed.iter().zip(lens) {
            let len = q.settings.frame_rate.snap(len);
            if let Some((_, it)) = q.find_item_mut(*c) {
                it.duration = len;
            }
            starts.push((*c, at));
            at += len;
        }
        magnet::relayout(&before, q, &starts, ctx, Orphans::Keep)?;
        // the music as long as the video
        if let Some((_, it)) = q.find_item_mut(music.0) {
            it.duration = (at - it.start).min(media - it.source_in).max(Tick(1));
        }
        Ok(())
    })?;
    Ok(())
}

/// `shnyp.video.template.fill`: media into the placeholders, in order (the empty ones; with `all`,
/// every one again). One undo step.
pub fn fill(s: &mut Session, p: &Value) -> Result<Value, String> {
    let paths: Vec<String> = p.get("paths").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
    let mut items: Vec<u64> = p.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
    if paths.is_empty() && items.is_empty() {
        return Err("give `paths` or `items`".into());
    }
    let all = p.get("all").and_then(Value::as_bool).unwrap_or(false);
    let targets_of = |s: &Session| -> Vec<ClipId> {
        match s.active_sequence() {
            Some(q) if all => placeholders(q).into_iter().map(|x| x.0).collect(),
            Some(_) => empty_placeholders(s),
            None => Vec::new(),
        }
    };
    s.active_sequence().ok_or("no video yet: shnyp.video.template.new first")?;
    if targets_of(s).is_empty() {
        return Err("no empty placeholders (all: true fills them again)".into());
    }
    let fit = p.get("fit").and_then(Value::as_str).unwrap_or("fill").to_string();
    let template = template_of(s);
    let (filled, short) = s
        .edit_group("Vorlage füllen", |s| {
            // a slot baked into curve pieces is one clip again before it gets new media
            let curved: Vec<u64> = {
                let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
                targets_of(s).into_iter().filter(|c| crate::speed::curve_of(q, *c).is_some()).map(|c| c.0).collect()
            };
            if !curved.is_empty() {
                crate::speed::curve(s, &json!({"clips": curved, "preset": "none"})).map_err(other)?;
            }
            let targets = targets_of(s);
            if !paths.is_empty() {
                let imported = s.execute("file.import", json!({"paths": crate::expand_folders(&paths)}))?;
                items.extend(crate::ids(&imported, "items"));
            }
            // pictures only
            items.retain(|i| s.project.source_streams(ItemId(*i)).0);
            let pairs: Vec<(ClipId, u64)> = targets.iter().copied().zip(items.iter().copied()).collect();
            let lengths: Vec<Option<Tick>> = pairs.iter().map(|(_, i)| filmcraft_engine::media_duration(&s.project, &s.media, ItemId(*i))).collect();
            let names: Vec<String> = pairs.iter().map(|(_, i)| s.project.item(ItemId(*i)).map(|x| x.name.clone()).unwrap_or_default()).collect();
            let short = s.edit_sequence("Vorlage füllen", |q, _, _| {
                let mut short = Vec::new();
                for (((c, item), len), name) in pairs.iter().zip(&lengths).zip(&names) {
                    let Some((_, it)) = q.find_item_mut(*c) else { continue };
                    it.item = ItemId(*item);
                    it.name = name.clone();
                    it.source_in = Tick::ZERO;
                    it.frame_hold = None;
                    it.reverse = false;
                    it.speed = 1.0;
                    // too short for its slot: it plays slower
                    if let Some(len) = len.filter(|l| *l < it.duration) {
                        it.speed = (len.0 as f64 / it.duration.0 as f64).max(0.01);
                        short.push(c.0);
                    }
                }
                Ok(short)
            })?;
            let ids: Vec<u64> = pairs.iter().map(|x| x.0.0).collect();
            if !ids.is_empty() {
                s.execute(if fit == "fit" { "clip.fitToFrame" } else { "clip.fillFrame" }, json!({"clips": ids}))?;
            }
            // what the template asks of its slots once they hold media
            if let Some(t) = &template {
                let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
                let by_index: Vec<(ClipId, usize)> = placeholders(q).into_iter().filter(|(c, _)| ids.contains(&c.0)).collect();
                if let Some(bg) = t["background"].as_str() {
                    crate::frame::background(s, &json!({"mode": bg, "clips": ids})).map_err(other)?;
                }
                for (c, i) in by_index {
                    if let Some(curve) = slots(t).get(i).and_then(|x| x["curve"].as_str()) {
                        crate::speed::curve(s, &json!({"clips": [c.0], "preset": curve})).map_err(other)?;
                    }
                }
            }
            Ok((ids, short))
        })
        .map_err(|e| e.to_string())?;
    let mut out = crate::state(s)?;
    out["filled"] = json!(filled);
    out["short"] = json!(short);
    out["empty"] = json!(empty_placeholders(s).iter().map(|c| c.0).collect::<Vec<_>>());
    Ok(out)
}

/// The template the video was made from (its recipe on the main track's mark).
fn template_of(s: &Session) -> Option<Value> {
    let q = s.active_sequence()?;
    let e = main_track::main_track(q)?.effects.iter().find(|e| main_track::is(&e.effect, main_track::MAIN))?;
    match &e.param("template")?.value {
        ParamValue::Text(t) => serde_json::from_str(t).ok(),
        _ => None,
    }
}

/// `shnyp.video.template.save`: the video's shape as a template of the user's: its format, its
/// main clips' lengths (with their curves and transitions), its titles, look and background.
pub fn save(s: &mut Session, p: &Value) -> Result<Value, String> {
    let name = p.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).ok_or("give `name`")?.trim().to_string();
    let q = s.active_sequence().ok_or("no video yet")?;
    let format = crate::Format::of_size(q.settings.width, q.settings.height).map_or("16:9", |f| f.key());
    let mut slots: Vec<Value> = Vec::new();
    let mains = main_track::main_clips(q);
    let main_t = main_track::main_track(q);
    let mut i = 0;
    while i < mains.len() {
        let it = mains[i];
        // a curve's pieces are one slot, as long as the clip played before the curve
        let (curve, n) = match crate::speed::curve_of(q, it.id) {
            Some((id, base)) => {
                let g = it.group;
                let n = mains[i..].iter().take_while(|x| x.group == g && main_track::effect_of(x, crate::speed::CURVE).is_some()).count().max(1);
                let span: Tick = mains[i..i + n].iter().map(|x| x.source_out() - x.source_in).fold(Tick::ZERO, |a, b| a + b);
                (Some((id, span.seconds() / base)), n)
            }
            None => (None, 1),
        };
        let last = mains[i + n - 1];
        let mut slot = json!({"seconds": ((curve.as_ref().map_or(it.duration.seconds(), |c| c.1)) * 100.0).round() / 100.0});
        if let Some((id, _)) = &curve {
            slot["curve"] = json!(id);
        }
        if let Some(x) = main_t.and_then(|t| t.transitions.iter().find(|x| x.from == Some(last.id))) {
            slot["transition"] = json!(x.effect.effect);
        }
        slots.push(slot);
        i += n;
    }
    if slots.is_empty() {
        return Err("the video has no clips on its main track".into());
    }
    let texts: Vec<Value> = q
        .video_tracks
        .iter()
        .flat_map(|t| &t.items)
        .filter_map(|it| {
            let text = it.graphic_layers().find_map(|l| match &l.param("text")?.value {
                ParamValue::Text(t) => Some(t.clone()),
                _ => None,
            })?;
            Some(
                json!({"text": text, "at": (it.start.seconds() * 100.0).round() / 100.0, "seconds": (it.duration.seconds() * 100.0).round() / 100.0}),
            )
        })
        .collect();
    let background = mains.first().map(|m| crate::frame::fill_of(q, m.id)).and_then(|f| match f {
        crate::frame::Fill::Blur(_) => Some("blur"),
        _ => None,
    });
    let id: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let t = json!({
        "id": format!("user-{id}"), "name": name, "description": p.get("description").and_then(Value::as_str).unwrap_or("Eigene Vorlage"),
        "format": format, "fit": "fill", "slots": slots, "texts": texts, "background": background,
    });
    let dir = dir_of(p);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{id}.{EXT}"));
    std::fs::write(&path, serde_json::to_string_pretty(&t).unwrap_or_default()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json!({"template": t["id"], "path": path.to_string_lossy(), "slots": t["slots"].as_array().map_or(0, |a| a.len())}))
}

/// Remember the template a video comes from (its recipe, on the main track's mark; part of
/// `new`'s step).
pub(crate) fn mark(q: &mut Sequence, recipe: &str) {
    let Some(t) = main_track::ensure(q) else { return };
    if let Some(e) = q.track_mut(t).and_then(|t| t.effects.iter_mut().find(|e| main_track::is(&e.effect, main_track::MAIN))) {
        e.params.insert("template".into(), filmcraft_project::Param::new(ParamValue::Text(recipe.into())));
    }
}
