//! Speed, CapCut's way: curves (Montage, Held, Bullet, Sprung, Blitz rein/raus), freeze frames
//! and playing backwards, each one undo step.
//!
//! FilmCraft stores Time Remapping keyframes but renders only a clip's constant speed (#510), so a
//! curve is *baked*: the clip is cut into pieces, each with the constant speed of its stretch of
//! the curve, frame-exact, the rest of the video moving up or on. The pieces share a
//! `TrackItem.group` and a hidden `shnyp.curve` effect (no definition, so FilmCraft skips it, as it
//! does its own Remix state), which says which curve they are and lets shnyp put the clip back
//! together. shnyp never writes Time Remapping keyframes: they would apply twice once FilmCraft
//! renders them.

use std::collections::HashSet;

use filmcraft_engine::{EngineError, Session};
use filmcraft_project::{ClipId, ParamValue, Sequence};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::anchors::Orphans;
use crate::{magnet, main_track};

type R<T> = filmcraft_engine::Result<T>;

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

/// A speed curve: speed (×) at points of the clip's own run (0…1).
pub struct Curve {
    pub id: &'static str,
    pub label: &'static str,
    pub points: &'static [(f64, f64)],
}

pub const CURVES: &[Curve] = &[
    Curve { id: "montage", label: "Montage", points: &[(0.0, 1.0), (0.18, 3.0), (0.36, 0.4), (0.54, 0.4), (0.72, 3.0), (0.86, 1.0), (1.0, 1.0)] },
    Curve { id: "held", label: "Held", points: &[(0.0, 2.0), (0.32, 2.0), (0.45, 0.25), (0.58, 0.25), (0.70, 2.0), (1.0, 2.0)] },
    Curve { id: "bullet", label: "Bullet", points: &[(0.0, 3.0), (0.22, 0.35), (0.78, 0.35), (1.0, 3.0)] },
    Curve { id: "sprung", label: "Sprung", points: &[(0.0, 1.0), (0.42, 1.0), (0.5, 8.0), (0.58, 1.0), (1.0, 1.0)] },
    Curve { id: "blitzRein", label: "Blitz rein", points: &[(0.0, 8.0), (0.35, 1.0), (1.0, 1.0)] },
    Curve { id: "blitzRaus", label: "Blitz raus", points: &[(0.0, 1.0), (0.65, 1.0), (1.0, 8.0)] },
];

pub fn find_curve(id: &str) -> Option<&'static Curve> {
    CURVES.iter().find(|c| c.id.eq_ignore_ascii_case(id))
}

/// The hidden effect on every piece of a curve.
pub const CURVE: &str = "shnyp.curve";

/// The speed at `x`: eased between the points (smoothstep on the logarithm, so a slow-down and a
/// speed-up of the same factor look alike).
fn speed_at(points: &[(f64, f64)], x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    let i = points.iter().position(|p| p.0 >= x).unwrap_or(points.len() - 1);
    if i == 0 {
        return points[0].1;
    }
    let (a, b) = (points[i - 1], points[i]);
    let u = if b.0 > a.0 { (x - a.0) / (b.0 - a.0) } else { 1.0 };
    let e = u * u * (3.0 - 2.0 * u);
    (a.1.log2() + (b.1.log2() - a.1.log2()) * e).exp2()
}

/// The pieces a curve bakes into: (end of the piece in the clip's run 0…1, output frames).
/// `run` is the media the clip plays, in frames at speed 1.
fn pieces(points: &[(f64, f64)], run_frames: f64, base: f64) -> Vec<(f64, i64)> {
    for threshold in [0.25, 0.35, 0.5, 0.75, 1.0, 2.0] {
        let mut out = Vec::new();
        let (mut x, mut frames, mut total) = (0.0, 0i64, 0i64);
        let mut piece_v = (base * speed_at(points, 0.0)).log2();
        while x < 1.0 && total < 1_000_000 {
            let v = base * speed_at(points, x);
            if frames >= 3 && (v.log2() - piece_v).abs() > threshold {
                out.push((x, frames));
                frames = 0;
                piece_v = v.log2();
            }
            x += v / run_frames.max(1.0);
            frames += 1;
            total += 1;
        }
        out.push((1.0, frames.max(1)));
        if out.len() <= 32 {
            return out;
        }
    }
    vec![(1.0, (run_frames / base).round().max(1.0) as i64)]
}

/// The clip with everything that moves with it (its sound, its background).
fn with_partners(q: &Sequence, c: ClipId) -> Vec<ClipId> {
    std::iter::once(c).chain(main_track::partners(q, c)).collect()
}

/// The pieces of the curve clip `c` belongs to, in time order (just `c` without a curve).
pub(crate) fn curve_pieces(q: &Sequence, c: ClipId) -> Vec<ClipId> {
    let Some((tid, it)) = q.find_item(c) else { return Vec::new() };
    let (Some(g), true) = (it.group, main_track::effect_of(it, CURVE).is_some()) else { return vec![c] };
    let mut v: Vec<(Tick, ClipId)> = q
        .track(tid)
        .map(|t| t.items.iter().filter(|x| x.group == Some(g) && main_track::effect_of(x, CURVE).is_some()).map(|x| (x.start, x.id)).collect())
        .unwrap_or_default();
    v.sort();
    v.into_iter().map(|x| x.1).collect()
}

/// The curve a clip has: (id, base speed).
pub fn curve_of(q: &Sequence, c: ClipId) -> Option<(String, f64)> {
    let (_, it) = q.find_item(c)?;
    let e = main_track::effect_of(it, CURVE)?;
    let id = match &e.param("preset")?.value {
        ParamValue::Text(t) => t.clone(),
        _ => return None,
    };
    Some((id, e.param("base").and_then(|p| p.value.as_f64()).unwrap_or(1.0)))
}

/// Starts that put `pieces` back to back from the first one's start, and move every main clip
/// after them by what the clip got longer or shorter.
fn starts_after(q: &Sequence, before: &Sequence, pieces: &[ClipId], old_end: Tick) -> Vec<(ClipId, Tick)> {
    let mut starts = Vec::new();
    let Some(first) = pieces.first().and_then(|c| q.find_item(*c)).map(|(_, it)| it.start) else { return starts };
    let mut at = first;
    for c in pieces {
        let Some((_, it)) = q.find_item(*c) else { continue };
        starts.push((*c, at));
        at += it.duration;
    }
    let delta = at - old_end;
    let on_main = pieces.first().is_some_and(|c| magnet::is_main(q, *c));
    if on_main && delta != Tick::ZERO {
        for m in main_track::main_clips(before).iter().filter(|m| m.start >= old_end) {
            if q.find_item(m.id).is_some() && !pieces.contains(&m.id) {
                starts.push((m.id, m.start + delta));
            }
        }
    }
    starts
}

/// Put a curve clip back together (inside an edit): its first piece plays the whole run at the
/// base speed again, the others go. The clip.
fn merge(q: &mut Sequence, c: ClipId) -> R<Option<(ClipId, Tick)>> {
    let pieces = curve_pieces(q, c);
    let Some((_, first)) = pieces.first().and_then(|p| q.find_item(*p)) else { return Ok(None) };
    if main_track::effect_of(first, CURVE).is_none() {
        return Ok(None);
    }
    let base = curve_of(q, c).map_or(1.0, |x| x.1);
    let fd = q.settings.frame_rate.frame_duration();
    let old_end = pieces.last().and_then(|p| q.find_item(*p)).map(|(_, it)| it.end()).unwrap_or(first.end());
    // the run: every piece's media together, from its earliest point
    let groups: Vec<Vec<ClipId>> = pieces.iter().map(|p| with_partners(q, *p)).collect();
    let keep = groups[0].clone();
    for id in &keep {
        // the matching partner of every piece (same track)
        let Some((tid, _)) = q.find_item(*id) else { continue };
        let same: Vec<ClipId> = groups.iter().filter_map(|g| g.iter().copied().find(|x| q.find_item(*x).is_some_and(|(t, _)| t == tid))).collect();
        let (from, span) = same
            .iter()
            .filter_map(|x| q.find_item(*x).map(|(_, it)| (it.source_in, it.source_out() - it.source_in)))
            .fold((Tick(i64::MAX), Tick::ZERO), |a, b| (a.0.min(b.0), a.1 + b.1));
        let len = Tick(((span.0 as f64 / base / fd.0 as f64).round() as i64).max(1) * fd.0);
        if let Some((_, it)) = q.find_item_mut(*id) {
            it.source_in = from;
            it.speed = span.0 as f64 / len.0 as f64;
            it.duration = len;
            it.group = None;
            it.effects.retain(|e| !main_track::is(&e.effect, CURVE));
        }
    }
    let gone: HashSet<ClipId> = groups[1..].iter().flatten().copied().collect();
    for t in q.all_tracks_mut() {
        t.items.retain(|it| !gone.contains(&it.id));
        filmcraft_edit::remove_orphan_transitions(t);
    }
    Ok(Some((pieces[0], old_end)))
}

/// Bake `curve` into clip `c` (inside an edit; the clip has no curve): the pieces.
fn bake(q: &mut Sequence, ctx: &mut filmcraft_edit::EditCtx, c: ClipId, curve: &Curve) -> R<Vec<ClipId>> {
    let fd = q.settings.frame_rate.frame_duration();
    let (start, dur, base) = q.find_item(c).map(|(_, it)| (it.start, it.duration, it.speed.abs())).ok_or_else(|| other("no such clip"))?;
    let run_frames = dur.0 as f64 * base / fd.0 as f64;
    let plan = pieces(curve.points, run_frames, base);
    // cut where the pieces meet (in the clip as it plays now), on frames; two cuts in one frame
    // are one, and each piece then plays the frames of every planned piece it covers
    let mut marks: Vec<(Tick, usize)> = Vec::new();
    for (i, (x, _)) in plan[..plan.len() - 1].iter().enumerate() {
        let t = q.settings.frame_rate.snap(start + Tick((dur.0 as f64 * x).round() as i64));
        if !(t > start && t < start + dur) {
            continue;
        }
        match marks.last_mut() {
            Some(last) if last.0 == t => last.1 = i,
            _ => marks.push((t, i)),
        }
    }
    let mut frames_of: Vec<i64> = Vec::new();
    let mut from = 0;
    for (_, i) in &marks {
        frames_of.push(plan[from..=*i].iter().map(|x| x.1).sum());
        from = i + 1;
    }
    frames_of.push(plan[from.min(plan.len())..].iter().map(|x| x.1).sum());
    let cuts: Vec<Tick> = marks.iter().map(|m| m.0).collect();
    let group = with_partners(q, c);
    let mut right: Vec<ClipId> = Vec::new();
    for t in cuts.iter().rev() {
        let new = filmcraft_edit::razor_items(q, &group, *t, ctx);
        // the new right part of the clip itself (on its track)
        let tid = q.find_item(c).map(|(t, _)| t);
        if let Some(r) = new.into_iter().find(|n| q.find_item(*n).map(|(t, _)| t) == tid) {
            right.push(r);
        }
    }
    right.reverse();
    let mut all = vec![c];
    all.extend(right);
    let gid = ctx.alloc();
    for (i, p) in all.iter().enumerate() {
        let frames = frames_of.get(i).copied().unwrap_or(1).max(1);
        let len = Tick(frames * fd.0);
        for id in with_partners(q, *p) {
            let Some((_, it)) = q.find_item_mut(id) else { continue };
            let span = it.source_out() - it.source_in;
            it.speed = span.0 as f64 / len.0 as f64;
            it.duration = len;
            it.group = Some(gid);
            it.effects.retain(|e| !main_track::is(&e.effect, CURVE));
            it.effects.push(main_track::hidden(
                CURVE,
                &[
                    ("preset", ParamValue::Text(curve.id.into())),
                    ("base", ParamValue::Float(base)),
                    ("i", ParamValue::Float(i as f64)),
                    ("n", ParamValue::Float(all.len() as f64)),
                ],
            ));
        }
    }
    Ok(all)
}

/// The clips a speed command works on: `clips` (or `clip`), else the selection, else the main
/// clip at the playhead; pictures only (their sound goes along), a curve once.
fn chosen(s: &Session, p: &Value) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let mut p = p.clone();
    if let Some(c) = p.get("clip").cloned().filter(Value::is_u64) {
        p["clips"] = json!([c]);
    }
    let mut v: Vec<ClipId> = crate::chosen_or_selected(s, &p).into_iter().filter(|c| q.video_tracks.iter().any(|t| t.item(*c).is_some())).collect();
    if v.is_empty() && p.get("clips").is_none() {
        v.extend(magnet::main_clip_at(q, s.playhead()));
    }
    let mut seen = HashSet::new();
    v.into_iter().filter(|c| curve_pieces(q, *c).first().is_some_and(|f| seen.insert(*f))).map(|c| curve_pieces(q, c)[0]).collect()
}

/// `shnyp.video.curve`: bake a speed curve into the clips (`none` puts them back together).
pub fn curve(s: &mut Session, p: &Value) -> Result<Value, String> {
    let id = p.get("preset").and_then(Value::as_str).unwrap_or("montage");
    let curve = if id == "none" {
        None
    } else {
        Some(find_curve(id).ok_or_else(|| format!("unknown curve `{id}`: {} or none", CURVES.iter().map(|c| c.id).collect::<Vec<_>>().join(", ")))?)
    };
    let clips = chosen(s, p);
    if clips.is_empty() {
        return Err("select a clip (or give `clips`)".into());
    }
    let label = curve.map_or("Kurve entfernen".to_string(), |c| format!("Kurve {}", c.label));
    let pieces = s
        .edit_sequence(&label, |q, ctx, _| {
            main_track::ensure(q);
            let mut made = Vec::new();
            for c in &clips {
                let before = q.clone();
                let old_end = curve_pieces(q, *c).last().and_then(|x| q.find_item(*x)).map(|(_, it)| it.end()).unwrap_or_default();
                let merged = merge(q, *c)?;
                let pieces = match curve {
                    Some(curve) => bake(q, ctx, *c, curve)?,
                    None => merged.map(|m| vec![m.0]).unwrap_or_default(),
                };
                let starts = starts_after(q, &before, &pieces, old_end);
                magnet::relayout(&before, q, &starts, ctx, Orphans::Keep)?;
                made.extend(pieces.iter().map(|c| c.0));
            }
            Ok(made)
        })
        .map_err(|e| e.to_string())?;
    let mut out = crate::state(s)?;
    out["pieces"] = json!(pieces);
    Ok(out)
}

/// `shnyp.video.freeze`: the picture at `at` (the playhead) stands still for `seconds`; the rest of
/// the clip and the video move on. The sound pauses.
pub fn freeze(s: &mut Session, p: &Value) -> Result<Value, String> {
    let q = s.active_sequence().ok_or("no video yet")?;
    let t = match p.get("at").and_then(Value::as_f64) {
        Some(x) if x.is_finite() && x >= 0.0 => Tick::from_seconds_f64(x),
        _ => s.playhead(),
    };
    let t = q.settings.frame_rate.snap(t);
    let secs = p.get("seconds").and_then(Value::as_f64).unwrap_or(3.0);
    if !(0.04..=600.0).contains(&secs) {
        return Err("`seconds` must be between 0.04 and 600".into());
    }
    let clip = match p.get("clip").and_then(Value::as_u64) {
        Some(c) => ClipId(c),
        None => chosen(s, &json!({}))
            .into_iter()
            .find(|c| q.find_item(*c).is_some_and(|(_, it)| it.start <= t && t < it.end()))
            .or_else(|| magnet::main_clip_at(q, t))
            .ok_or("no clip at the playhead")?,
    };
    let (_, it) = q.find_item(clip).ok_or("no such clip")?;
    if !(it.start <= t && t < it.end()) {
        return Err("the playhead (or `at`) is not on that clip".into());
    }
    if main_track::effect_of(it, CURVE).is_some() {
        return Err("a clip with a speed curve: remove the curve first (shnyp.video.curve none)".into());
    }
    let fd = q.settings.frame_rate.frame_duration();
    let len = Tick(((Tick::from_seconds_f64(secs).0 as f64 / fd.0 as f64).round() as i64).max(1) * fd.0);
    s.edit_sequence("Einfrieren", |q, ctx, _| {
        main_track::ensure(q);
        let before = q.clone();
        let group = with_partners(q, clip);
        // the part from `t` on: the clip itself or what a cut there makes
        let right = if q.find_item(clip).is_some_and(|(_, it)| it.start < t) {
            let tid = q.find_item(clip).map(|(t, _)| t);
            filmcraft_edit::razor_items(q, &group, t, ctx)
                .into_iter()
                .find(|n| q.find_item(*n).map(|(x, _)| x) == tid)
                .ok_or_else(|| other("could not cut there"))?
        } else {
            clip
        };
        let old_end = q.find_item(right).map(|(_, it)| it.end()).unwrap_or(t);
        // the still: the picture (and its background) held at `t`, the sound left out
        let link = Some(ctx.alloc());
        let mut holds = Vec::new();
        for id in with_partners(q, right) {
            let Some((tid, it)) = q.find_item(id) else { continue };
            if q.audio_tracks.iter().any(|tr| tr.id == tid) {
                continue;
            }
            let mut h = it.clone();
            h.id = ClipId(ctx.alloc());
            h.frame_hold = Some(it.source_time_at(t));
            h.start = t;
            h.duration = len;
            h.link = link;
            h.markers.clear();
            holds.push((tid, h));
        }
        let held: Vec<ClipId> = holds.iter().map(|h| h.1.id).collect();
        // what comes after moves on by the still's length, then the still goes in
        let mut starts: Vec<(ClipId, Tick)> = vec![(right, t + len)];
        let on_main = magnet::is_main(q, right);
        if on_main {
            starts.extend(main_track::main_clips(&before).iter().filter(|m| m.start >= old_end && m.id != right).map(|m| (m.id, m.start + len)));
        }
        let followed = magnet::relayout(&before, q, &starts, ctx, Orphans::Keep)?;
        for (tid, h) in holds {
            if let Some(tr) = q.track_mut(tid) {
                tr.items.push(h);
                tr.sort();
            }
        }
        if let Some(s) = q.check().err() {
            return Err(other(s));
        }
        Ok((held, followed))
    })
    .map_err(|e| e.to_string())?;
    crate::state(s)
}

/// `shnyp.video.reverse`: the clips play backwards (again forwards); a curve plays backwards with
/// them.
pub fn reverse(s: &mut Session, p: &Value) -> Result<Value, String> {
    let clips = chosen(s, p);
    if clips.is_empty() {
        return Err("select a clip (or give `clips`)".into());
    }
    s.edit_sequence("Rückwärts", |q, ctx, _| {
        for c in &clips {
            let before = q.clone();
            let curve = curve_of(q, *c).and_then(|(id, _)| find_curve(&id));
            let old_end = curve_pieces(q, *c).last().and_then(|x| q.find_item(*x)).map(|(_, it)| it.end()).unwrap_or_default();
            merge(q, *c)?;
            for id in with_partners(q, *c) {
                if let Some((_, it)) = q.find_item_mut(id) {
                    it.reverse = !it.reverse;
                }
            }
            if let Some(curve) = curve {
                let pieces = bake(q, ctx, *c, curve)?;
                let starts = starts_after(q, &before, &pieces, old_end);
                magnet::relayout(&before, q, &starts, ctx, Orphans::Keep)?;
            }
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    crate::state(s)
}
