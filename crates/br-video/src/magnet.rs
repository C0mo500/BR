//! CapCut's magnetic main track on FilmCraft's sequence: the main clips sit back to back,
//! deleting closes the gap, moving reorders, inserting pushes the rest along, trimming ripples,
//! and the clips attached to them (see [`crate::anchors`]) go along. Every command here is one
//! undo step. With the magnet off the main track keeps its gaps, as in CapCut; with linkage off
//! nothing follows, which is FilmCraft's own behaviour.
//!
//! The main clips are moved here directly (with their linked sound and background), not through
//! FilmCraft's Premiere-style inserts and ripples: those cut titles and music that cross the edit
//! point on every sync-locked track, and leave a gap on the main track where other clips on the
//! sound track overlap.

use std::collections::HashSet;

use filmcraft_edit::{Edge, EditCtx, TrimMode};
use filmcraft_engine::{EngineError, Session};
use filmcraft_project::{ClipId, Sequence, TrackId, TrackKind};
use filmcraft_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::anchors::{self, Followed, Orphans};
use crate::main_track::{self, is_background, is_background_track};

type R<T> = filmcraft_engine::Result<T>;

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

/// The switches of the active sequence.
fn switches(s: &Session) -> main_track::Settings {
    s.active_sequence().map(main_track::settings).unwrap_or_default()
}

/// Ids as FilmCraft's commands take them.
fn raw(ids: &[ClipId]) -> Vec<u64> {
    ids.iter().map(|c| c.0).collect()
}

/// A main clip with its partners (sound, background).
fn with_partners(q: &Sequence, c: ClipId) -> Vec<ClipId> {
    std::iter::once(c).chain(main_track::partners(q, c)).collect()
}

/// Is `c` one of the main track's clips?
pub fn is_main(q: &Sequence, c: ClipId) -> bool {
    main_track::main_clips(q).iter().any(|it| it.id == c)
}

/// Where the transitions on each track sit: they keep to their cut when the clips around it moved
/// together, and go when the clips beside them are no longer next to each other.
fn keep_transitions(before: &Sequence, q: &mut Sequence) {
    for tr in q.all_tracks_mut() {
        let old = before.track(tr.id);
        let pos = |t: &filmcraft_project::Track, x: &filmcraft_project::Transition| match (x.from, x.to) {
            (_, Some(to)) => t.item(to).map(|i| i.start),
            (Some(from), None) => t.item(from).map(|i| i.end()),
            _ => None,
        };
        let shifts: Vec<Option<Tick>> = tr
            .transitions
            .iter()
            .map(|x| match (old.and_then(|o| pos(o, x)), pos(tr, x)) {
                (Some(a), Some(b)) => Some(b - a),
                _ => None,
            })
            .collect();
        let items = tr.items.clone();
        let find = |c: ClipId| items.iter().find(|i| i.id == c);
        let mut keep = Vec::new();
        for (x, d) in tr.transitions.drain(..).zip(shifts) {
            let adjacent = match (x.from, x.to) {
                (Some(a), Some(b)) => matches!((find(a), find(b)), (Some(a), Some(b)) if a.end() == b.start),
                (Some(a), None) => find(a).is_some(),
                (None, Some(b)) => find(b).is_some(),
                (None, None) => false,
            };
            if let (true, Some(d)) = (adjacent, d) {
                // a fade from or to nothing is no longer than its clip
                let len = match (x.from, x.to) {
                    (Some(a), None) | (None, Some(a)) => find(a).map_or(x.duration, |i| x.duration.min(i.duration)),
                    _ => x.duration,
                };
                let start = match (x.from, x.to) {
                    (Some(a), None) => find(a).map_or(x.start + d, |i| i.end() - len),
                    _ => x.start + d,
                };
                keep.push(filmcraft_project::Transition { start, duration: len, ..x });
            }
        }
        tr.transitions = keep;
        tr.sort();
    }
}

/// The next track of `kind` (after `from`; never the main track or the backgrounds) on which
/// `range` is free, else a new one.
fn free_track(q: &mut Sequence, kind: TrackKind, from: Option<TrackId>, range: TimeRange, ctx: &mut EditCtx) -> TrackId {
    let main = main_track::main_track_id(q);
    let main_audio = if kind == TrackKind::Audio { main_track::main_audio_track(q) } else { None };
    let tracks = q.tracks(kind);
    let at = from.and_then(|f| tracks.iter().position(|t| t.id == f)).map_or(0, |i| i + 1);
    let found = tracks
        .iter()
        .skip(at)
        .chain(tracks.iter().take(at))
        .filter(|t| Some(t.id) != main && Some(t.id) != main_audio && Some(t.id) != from && !is_background_track(t) && !t.locked)
        .find(|t| t.items.iter().all(|it| it.end() <= range.start || it.start >= range.end()))
        .map(|t| t.id);
    if let Some(t) = found {
        return t;
    }
    let id = TrackId(ctx.alloc());
    let n = q.tracks(kind).len() + 1;
    q.tracks_mut(kind).push(main_track::new_track(id, kind, n));
    id
}

/// Clear overlaps: on every track, what has no priority gives way to what has (the main clips and
/// their partners) and moves to a free track of its kind.
fn settle(q: &mut Sequence, fixed: &HashSet<ClipId>, ctx: &mut EditCtx) -> R<()> {
    let main = main_track::main_track_id(q);
    let ids: Vec<TrackId> = q.all_tracks().map(|t| t.id).collect();
    for tid in ids {
        while let Some(tr) = q.track_mut(tid) {
            tr.sort();
            let clash = tr.items.windows(2).find(|w| w[0].end() > w[1].start).map(|w| (w[0].id, w[1].id));
            let Some((a, b)) = clash else { break };
            // the later one moves, unless it has priority and the earlier one has not; nothing
            // ever leaves the main track
            let fa = fixed.contains(&a);
            let fb = fixed.contains(&b);
            let go = if Some(tid) == main {
                match (is_main(q, a), is_main(q, b)) {
                    (true, true) => return Err(other("zwei Clips der Hauptspur würden sich überdecken")),
                    (true, false) => b,
                    _ => a,
                }
            } else if fb && !fa {
                a
            } else {
                b
            };
            let Some(tr) = q.track_mut(tid) else { break };
            let Some(i) = tr.items.iter().position(|it| it.id == go) else { break };
            let it = tr.items.remove(i);
            filmcraft_edit::remove_orphan_transitions(tr);
            let kind = tr.kind;
            let dest = free_track(q, kind, Some(tid), it.range(), ctx);
            if let Some(t) = q.track_mut(dest) {
                t.items.push(it);
                t.sort();
            }
        }
    }
    Ok(())
}

/// Put main clips at new starts (their partners by the same amount), let the attached clips
/// follow and clear overlaps. `before` is the sequence before the whole edit.
pub(crate) fn relayout(before: &Sequence, q: &mut Sequence, starts: &[(ClipId, Tick)], ctx: &mut EditCtx, orphans: Orphans) -> R<Followed> {
    for (c, t) in starts {
        let Some((_, it)) = q.find_item(*c) else { continue };
        let d = *t - it.start;
        if d == Tick::ZERO {
            continue;
        }
        for id in with_partners(q, *c) {
            if let Some((_, it)) = q.find_item_mut(id) {
                it.start = (it.start + d).max(Tick::ZERO);
            }
        }
    }
    for tr in q.all_tracks_mut() {
        tr.sort();
    }
    keep_transitions(before, q);
    let plan = anchors::plan(before, q, orphans);
    let done = anchors::apply(q, ctx, plan)?;
    let fixed: HashSet<ClipId> = main_track::main_clips(q).iter().flat_map(|m| with_partners(q, m.id)).collect();
    settle(q, &fixed, ctx)?;
    Ok(done)
}

/// The cuts of the main track (its clips' starts and the end), in order.
fn cuts(q: &Sequence) -> Vec<Tick> {
    let main = main_track::main_clips(q);
    let mut v: Vec<Tick> = main.iter().map(|m| m.start).chain(main.last().map(|m| m.end())).collect();
    if v.is_empty() {
        v.push(Tick::ZERO);
    }
    v.dedup();
    v
}

/// The cut nearest to `t` (inserting there splits nothing).
pub fn nearest_cut(q: &Sequence, t: Tick) -> Tick {
    cuts(q).into_iter().min_by_key(|c| (c.0 - t.0).abs()).unwrap_or(Tick::ZERO)
}

/// Where the picture ends: the last clip on any video track. An export ends there (music that
/// runs on is not cut, it is just not played over black), as CapCut's video ends with its clips.
pub fn picture_end(q: &Sequence) -> Tick {
    q.video_tracks.iter().flat_map(|t| t.items.iter()).map(|it| it.end()).max().unwrap_or(Tick::ZERO)
}

fn followed_json(f: &Followed) -> Value {
    json!({"moved": f.moved.iter().map(|c| c.0).collect::<Vec<_>>(), "deleted": f.deleted.iter().map(|c| c.0).collect::<Vec<_>>()})
}

/// Delete clips. Main clips go with their sound and background, the gap closes (magnet on) and
/// what was attached to them goes too; other clips just go.
pub fn delete(s: &mut Session, chosen: &[ClipId]) -> R<Value> {
    let sw = switches(s);
    if !sw.linkage {
        // FilmCraft's own: Ripple Delete with the magnet on, else Clear (sync locks decide what
        // else moves)
        let all: Vec<ClipId> = s.active_sequence().map(|q| chosen.iter().flat_map(|c| with_partners(q, *c)).collect()).unwrap_or_default();
        let cmd = if sw.magnet { "edit.rippleDelete" } else { "edit.clear" };
        return s.edit_group("Löschen", |s| {
            s.execute(cmd, json!({"clips": raw(&all)}))?;
            Ok(json!({"deleted": raw(&all), "followed": {"moved": [], "deleted": []}}))
        });
    }
    s.edit_group("Löschen", |s| {
        s.edit_sequence("Löschen", |q, ctx, _| {
            main_track::ensure(q);
            let before = q.clone();
            let settings = main_track::settings(q);
            let mains: Vec<ClipId> = chosen.iter().copied().filter(|c| is_main(q, *c)).collect();
            let mut gone: Vec<ClipId> = Vec::new();
            for c in chosen {
                if is_main(q, *c) || q.find_item(*c).is_some_and(|(_, it)| is_background(it)) {
                    // a background is a main clip's: it goes alone, the main clip keeps its sound
                    if !is_main(q, *c) {
                        if let Some((_, it)) = q.find_item_mut(*c) {
                            it.link = None;
                        }
                        gone.push(*c);
                    }
                    continue;
                }
                gone.extend(with_partners(q, *c));
            }
            let spans: Vec<TimeRange> = mains.iter().filter_map(|c| q.find_item(*c).map(|(_, it)| it.range())).collect();
            for c in &mains {
                gone.extend(with_partners(q, *c));
            }
            gone.sort_by_key(|c| c.0);
            gone.dedup();
            if gone.is_empty() {
                return Err(other("nichts zu löschen"));
            }
            filmcraft_edit::delete_items(q, &gone);
            // the gap of each deleted main clip closes: what came after moves up by the length of
            // the deleted clips before it
            let starts: Vec<(ClipId, Tick)> = if settings.magnet {
                main_track::main_clips(q)
                    .iter()
                    .map(|m| (m.id, m.start - spans.iter().filter(|r| r.end() <= m.start).fold(Tick::ZERO, |a, r| a + r.duration)))
                    .collect()
            } else {
                Vec::new()
            };
            let followed = relayout(&before, q, &starts, ctx, Orphans::Delete)?;
            Ok(json!({"deleted": gone.iter().map(|c| c.0).collect::<Vec<_>>(), "followed": followed_json(&followed)}))
        })
    })
}

/// Close every gap of the main track (the attached clips go along).
pub fn pack(s: &mut Session) -> R<Value> {
    s.edit_group("Lücken schließen", |s| {
        s.edit_sequence("Lücken schließen", |q, ctx, _| {
            main_track::ensure(q);
            let before = q.clone();
            let mut at = Tick::ZERO;
            let mut starts = Vec::new();
            for m in main_track::main_clips(q) {
                starts.push((m.id, at));
                at += m.duration;
            }
            let followed = relayout(&before, q, &starts, ctx, Orphans::Keep)?;
            Ok(followed_json(&followed))
        })
    })
}

/// Lay the main clips out in `order` from where the first of them that changes place was: back to
/// back with the magnet on, else each in the slot of the clip it replaces.
fn order_starts(q: &Sequence, order: &[ClipId]) -> Vec<(ClipId, Tick)> {
    let main = main_track::main_clips(q);
    let first = main.iter().zip(order).position(|(m, o)| m.id != *o).unwrap_or(main.len());
    let mut at = main.get(first).map(|m| m.start).unwrap_or(Tick::ZERO);
    let mut out = Vec::new();
    for c in order.iter().skip(first) {
        if let Some(m) = main.iter().find(|m| m.id == *c) {
            out.push((m.id, at));
            at += m.duration;
        }
    }
    out
}

/// Where `seconds` falls in the main track's order: the index of the cut nearest to it.
fn index_at(q: &Sequence, t: Tick) -> usize {
    let cut = nearest_cut(q, t);
    main_track::main_clips(q).iter().position(|m| m.start >= cut).unwrap_or(main_track::main_clips(q).len())
}

/// Where a clip should go.
pub enum To {
    /// On the main track, at this place in its order.
    Index(usize),
    /// On the main track, at the cut nearest to this time.
    Main(Tick),
    /// Off the main track: onto this track (a free one when `None`), starting at this time.
    Track(Option<TrackId>, Tick),
}

/// Move a clip: reorder the main track, take a clip off it or onto it, or move any other clip.
pub fn move_clip(s: &mut Session, clip: ClipId, to: To) -> R<Value> {
    s.edit_group("Verschieben", |s| {
        s.edit_sequence("Verschieben", |q, ctx, _| {
            main_track::ensure(q);
            let before = q.clone();
            let main_id = main_track::main_track_id(q).ok_or(EngineError::NoSequence)?;
            let magnet = main_track::settings(q).magnet;
            let from_main = is_main(q, clip);
            let (_, it) = q.find_item(clip).ok_or_else(|| other(format!("kein Clip {}", clip.0)))?;
            let it = it.clone();
            let index_of = |q: &Sequence, to: &To| match to {
                To::Index(i) => Some(*i),
                To::Main(t) => Some(index_at(q, *t)),
                To::Track(..) => None,
            };
            let followed = match (from_main, index_of(q, &to)) {
                // reorder
                (true, Some(i)) => {
                    let mut order: Vec<ClipId> = main_track::main_clips(q).iter().map(|m| m.id).filter(|c| *c != clip).collect();
                    order.insert(i.min(order.len()), clip);
                    let starts = order_starts(&before, &order);
                    relayout(&before, q, &starts, ctx, Orphans::Keep)?
                }
                // off the main track: the clip (with its sound) becomes an overlay; its background
                // goes; the gap closes
                (true, None) => {
                    let To::Track(track, t) = to else { unreachable!() };
                    let range = TimeRange::new(t.max(Tick::ZERO), it.duration);
                    let dest = match track {
                        Some(d) if q.track(d).is_some_and(|tr| tr.kind == TrackKind::Video && tr.id != main_id && !is_background_track(tr)) => d,
                        Some(_) => return Err(other("das Ziel ist keine Overlay-Spur")),
                        None => free_track(q, TrackKind::Video, Some(main_id), range, ctx),
                    };
                    let d = range.start - it.start;
                    let mut moves = vec![(clip, dest, range.start)];
                    let mut gone = Vec::new();
                    for p in main_track::partners(q, clip) {
                        let Some((ptid, pit)) = q.find_item(p) else { continue };
                        if is_background(pit) {
                            gone.push(p);
                        } else if q.track(ptid).is_some_and(|t| t.kind == TrackKind::Audio) {
                            let r = TimeRange::new((pit.start + d).max(Tick::ZERO), pit.duration);
                            let a = free_track(q, TrackKind::Audio, None, r, ctx);
                            moves.push((p, a, r.start));
                        }
                    }
                    filmcraft_edit::delete_items(q, &gone);
                    let starts: Vec<(ClipId, Tick)> = if magnet {
                        main_track::main_clips(q)
                            .iter()
                            .filter(|m| m.id != clip && m.start >= it.end())
                            .map(|m| (m.id, m.start - it.duration))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    filmcraft_edit::move_items(q, &moves, false, ctx)?;
                    relayout(&before, q, &starts, ctx, Orphans::Keep)?
                }
                // onto the main track at a cut; what follows moves on by its length
                (false, Some(i)) => {
                    let i = i.min(main_track::main_clips(q).len());
                    let cut = main_track::main_clips(q)
                        .get(i)
                        .map(|m| m.start)
                        .or_else(|| main_track::main_clips(q).last().map(|m| m.end()))
                        .unwrap_or(Tick::ZERO);
                    let starts: Vec<(ClipId, Tick)> =
                        main_track::main_clips(q).iter().filter(|m| m.start >= cut).map(|m| (m.id, m.start + it.duration)).collect();
                    // the clip (and its sound) goes onto the main track (and its sound track) at
                    // the cut, what follows makes room; nothing is written over
                    let d = cut - it.start;
                    let main_audio = main_track::main_audio_track(q);
                    let mut lands = vec![(clip, main_id, cut)];
                    for p in main_track::partners(q, clip) {
                        if let (Some((_, pit)), Some(a)) = (q.find_item(p), main_audio) {
                            lands.push((p, a, (pit.start + d).max(Tick::ZERO)));
                        }
                    }
                    for (c, dest, at) in lands {
                        let Some((src, _)) = q.find_item(c) else { continue };
                        let Some(tr) = q.track_mut(src) else { continue };
                        let Some(i) = tr.items.iter().position(|x| x.id == c) else { continue };
                        let mut item = tr.items.remove(i);
                        filmcraft_edit::remove_orphan_transitions(tr);
                        item.start = at;
                        if let Some(t) = q.track_mut(dest) {
                            t.items.push(item);
                            t.sort();
                        }
                    }
                    relayout(&before, q, &starts, ctx, Orphans::Keep)?
                }
                // any other clip: where it is asked to go (overwriting nothing: onto a free track)
                (false, None) => {
                    let To::Track(track, t) = to else { unreachable!() };
                    let (src, _) = q.find_item(clip).ok_or_else(|| other("kein Clip"))?;
                    let kind = q.track(src).map(|t| t.kind).unwrap_or(TrackKind::Video);
                    let range = TimeRange::new(t.max(Tick::ZERO), it.duration);
                    let d = range.start - it.start;
                    let partners = main_track::partners(q, clip);
                    let moving: HashSet<ClipId> = std::iter::once(clip).chain(partners.iter().copied()).collect();
                    let fits = |q: &Sequence, tid: TrackId, r: TimeRange| {
                        q.track(tid).is_some_and(|tr| {
                            !tr.locked && tr.items.iter().filter(|i| !moving.contains(&i.id)).all(|i| i.end() <= r.start || i.start >= r.end())
                        })
                    };
                    let wanted = track.unwrap_or(src);
                    if q.track(wanted).is_none_or(|t| t.kind != kind || t.id == main_id) {
                        return Err(other("auf diese Spur passt der Clip nicht"));
                    }
                    let dest = if fits(q, wanted, range) { wanted } else { free_track(q, kind, Some(wanted), range, ctx) };
                    let mut moves = vec![(clip, dest, range.start)];
                    for p in partners {
                        if let Some((ptid, pit)) = q.find_item(p) {
                            let r = TimeRange::new((pit.start + d).max(Tick::ZERO), pit.duration);
                            let pk = q.track(ptid).map(|t| t.kind).unwrap_or(TrackKind::Audio);
                            let pd = if fits(q, ptid, r) { ptid } else { free_track(q, pk, Some(ptid), r, ctx) };
                            moves.push((p, pd, r.start));
                        }
                    }
                    filmcraft_edit::move_items(q, &moves, false, ctx)?;
                    Followed::default()
                }
            };
            Ok(followed_json(&followed))
        })
    })
}

/// The main clips after `clip` that move when its end moves by `d`, with their new starts.
fn later_starts(before: &Sequence, clip: ClipId, d: Tick) -> Vec<(ClipId, Tick)> {
    let Some((_, it)) = before.find_item(clip) else { return Vec::new() };
    let end = it.end();
    main_track::main_clips(before).iter().filter(|m| m.id != clip && m.start >= end).map(|m| (m.id, m.start + d)).collect()
}

/// The clips of `clip`'s group that are on unlocked tracks (all of them, or an error).
fn unlocked(q: &Sequence, clips: &[ClipId]) -> R<()> {
    for c in clips {
        let (t, _) = q.find_item(*c).ok_or_else(|| other(format!("kein Clip {}", c.0)))?;
        if q.track(t).is_some_and(|t| t.locked) {
            return Err(other("die Spur ist gesperrt"));
        }
    }
    Ok(())
}

/// Trim an edge of a clip by `delta`. A main clip ripples with the magnet on: its start stays,
/// what follows moves up or on (with its sound and background, whether or not this clip has
/// any), and the attached clips follow. Everything else trims in place.
pub fn trim(s: &mut Session, clip: ClipId, edge: Edge, delta: Tick) -> R<Value> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let main = is_main(q, clip);
    let ripple = main && main_track::settings(q).magnet;
    if !main_track::settings(q).linkage {
        let mode = if ripple { "ripple" } else { "regular" };
        let e = if edge == Edge::In { "in" } else { "out" };
        let d = filmcraft_engine::commands::trim_delta(s, clip, edge, if ripple { TrimMode::Ripple } else { TrimMode::Regular }, delta)?;
        if d == Tick::ZERO {
            return Err(other("dieser Rand lässt sich nicht weiter ziehen"));
        }
        s.execute("timeline.trim", json!({"clip": clip.0, "edge": e, "mode": mode, "delta": d.0}))?;
        return Ok(json!({"seconds": crate::seconds(d.0), "followed": {"moved": [], "deleted": []}}));
    }
    let label = if ripple { "Trimmen (Ripple)" } else { "Trimmen" };
    s.edit_group(label, |s| {
        s.edit_sequence(label, |q, ctx, _| {
            main_track::ensure(q);
            let before = q.clone();
            let clips = with_partners(q, clip);
            unlocked(q, &clips)?;
            let mode = if ripple { TrimMode::Ripple } else { TrimMode::Regular };
            // the whole group moves its edge by the same amount: the least any member allows
            let mut d = delta;
            for c in &clips {
                let x = filmcraft_edit::clamp_trim(q, *c, edge, mode, d, ctx)?;
                if x.abs() < d.abs() {
                    d = x;
                }
            }
            if d == Tick::ZERO {
                return Err(other("dieser Rand lässt sich nicht weiter ziehen"));
            }
            if !ripple {
                for c in &clips {
                    filmcraft_edit::trim(q, *c, edge, mode, d, ctx)?;
                }
                let followed = if main { anchors::follow(&before, q, ctx, Orphans::Keep)? } else { Followed::default() };
                return Ok(json!({"seconds": crate::seconds(d.0), "followed": followed_json(&followed)}));
            }
            // the clip keeps its start; its end moves by d (out) or −d (in)
            for c in &clips {
                if let Some((_, it)) = q.find_item_mut(*c) {
                    filmcraft_edit::move_edge(it, edge, d, false);
                }
            }
            let end_moves = if edge == Edge::Out { d } else { Tick::ZERO - d };
            let starts = later_starts(&before, clip, end_moves);
            let followed = relayout(&before, q, &starts, ctx, Orphans::Keep)?;
            Ok(json!({"seconds": crate::seconds(d.0), "followed": followed_json(&followed)}))
        })
    })
}

/// Change the speed of clips (a reversed clip stays reversed). A main clip keeps its start and
/// its length follows the speed; with the magnet on what follows moves up or on (with its sound
/// and background) and the attached clips follow. Other clips change length in place, up to the
/// next clip on their track.
pub fn speed(s: &mut Session, clips: &[ClipId], speed: f64) -> R<Value> {
    if !(speed.is_finite() && speed > 0.0) {
        return Err(other("das Tempo muss größer als 0 sein"));
    }
    let label = format!("Tempo {speed}×");
    let sw = switches(s);
    if !sw.linkage {
        // FilmCraft's own Speed/Duration, a reversed clip staying reversed
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        let (backward, forward): (Vec<ClipId>, Vec<ClipId>) = clips.iter().partition(|c| q.find_item(**c).is_some_and(|(_, it)| it.reverse));
        return s.edit_group(&label, |s| {
            for (ids, reverse) in [(forward, false), (backward, true)] {
                if !ids.is_empty() {
                    s.execute("clip.speedDuration", json!({"clips": raw(&ids), "speed": speed * 100.0, "ripple": sw.magnet, "reverse": reverse}))?;
                }
            }
            Ok(json!({"moved": [], "deleted": []}))
        });
    }
    s.edit_group(&label, |s| {
        s.edit_sequence(&label, |q, ctx, _| {
            main_track::ensure(q);
            let before = q.clone();
            let magnet = main_track::settings(q).magnet;
            let mut taken: HashSet<ClipId> = HashSet::new();
            let mut starts: Vec<(ClipId, Tick)> = Vec::new();
            let mut changed = 0;
            // main clips from the last to the first, so each later clip moves by the sum of the
            // changes before it
            // with the magnet off a main clip changes in place, up to the next one (as others do)
            let mut mains: Vec<ClipId> = if magnet { clips.iter().copied().filter(|c| is_main(q, *c)).collect() } else { Vec::new() };
            mains.sort_by_key(|c| before.find_item(*c).map(|(_, it)| it.start));
            let mut shift: Vec<(Tick, Tick)> = Vec::new();
            for c in &mains {
                let g = with_partners(q, *c);
                unlocked(q, &g)?;
                for id in &g {
                    taken.insert(*id);
                    let Some((_, it)) = q.find_item_mut(*id) else { continue };
                    let src = it.source_out() - it.source_in;
                    let dur = Tick((src.0 as f64 / speed).round() as i64).max(ctx.min_duration);
                    if *id == *c {
                        shift.push((it.end(), dur - it.duration));
                    }
                    it.speed = speed;
                    it.duration = dur;
                }
                changed += 1;
            }
            if magnet {
                for m in main_track::main_clips(&before) {
                    let d = shift.iter().filter(|(end, _)| *end <= m.start).fold(Tick::ZERO, |a, (_, d)| a + *d);
                    if d != Tick::ZERO && !mains.contains(&m.id) {
                        starts.push((m.id, m.start + d));
                    }
                }
            }
            // other clips: FilmCraft's own speed change, in place
            let others: Vec<ClipId> = clips.iter().copied().filter(|c| !mains.contains(c)).collect();
            for c in &others {
                if taken.contains(c) {
                    continue;
                }
                let g = with_partners(q, *c);
                unlocked(q, &g)?;
                let reverse = q.find_item(*c).is_some_and(|(_, it)| it.reverse);
                filmcraft_edit::set_speed_group(q, &g, speed, reverse, false, ctx)?;
                taken.extend(g);
                changed += 1;
            }
            if changed == 0 {
                return Err(other("keine Clips, deren Tempo sich ändern lässt"));
            }
            let followed = relayout(&before, q, &starts, ctx, Orphans::Keep)?;
            Ok(followed_json(&followed))
        })
    })
}

/// Sound alone at least this long is music.
const MUSIC_SECONDS: f64 = 20.0;

/// Insert imported items on the main track at the cut nearest to `at` (at its end without): what
/// follows moves on, nothing is cut in two (not the main clip, not a title or the music over it).
/// Pictures go on the main track with their sound on its sound track; sound alone goes on a free
/// sound track at `at` (at the playhead without). Returns the new clips.
pub fn insert(s: &mut Session, items: &[u64], at: Option<Tick>) -> R<Vec<u64>> {
    let sw = switches(s);
    if let (false, Some(at)) = (sw.linkage, at) {
        let (main_id, main_audio) = {
            let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
            (main_track::main_track_id(q).ok_or(EngineError::NoSequence)?, main_track::main_audio_track(q))
        };
        let mut p = json!({"items": items, "track": main_id.0, "time": at.0, "insert": sw.magnet});
        if let Some(a) = main_audio {
            p["audioTrack"] = json!(a.0);
        }
        let placed = s.execute("timeline.place", p)?;
        return Ok(placed.get("clips").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect());
    }
    let mut new_clips = Vec::new();
    let (main_id, main_audio) = {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        (main_track::main_track_id(q).ok_or(EngineError::NoSequence)?, main_track::main_audio_track(q))
    };
    // first at the end of the main track (and its sound track), where they cut nothing
    let mut mains: Vec<ClipId> = Vec::new();
    for item in items {
        let (has_video, has_audio) = s.project.source_streams(filmcraft_project::ItemId(*item));
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        let end = |t: Option<TrackId>| t.and_then(|t| q.track(t)).map(|t| t.end()).unwrap_or(Tick::ZERO);
        let placed = if has_video {
            let time = end(Some(main_id)).max(end(main_audio));
            let mut p = json!({"item": item, "track": main_id.0, "time": time.0, "insert": false});
            if let Some(a) = main_audio {
                p["audioTrack"] = json!(a.0);
            }
            s.execute("timeline.place", p)?
        } else {
            // sound alone: where asked (else at the playhead), on a sound track free there; it
            // then hangs on the main clip it starts on
            let time = at.unwrap_or_else(|| s.playhead());
            let placed = s.execute("timeline.place", json!({"item": item, "audioTrack": "new", "time": time.0, "video": false, "insert": false}))?;
            // a long piece is music (a bed under the video, not attached to a clip), a short one
            // a sound effect on the clip it starts on
            let clips: Vec<u64> = placed.get("clips").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
            let long = clips.iter().any(|c| {
                s.active_sequence().and_then(|q| q.find_item(ClipId(*c))).is_some_and(|(_, it)| it.duration >= Tick::from_seconds_f64(MUSIC_SECONDS))
            });
            if has_audio && !clips.is_empty() {
                s.execute("essentialSound.setType", json!({"clips": clips, "type": if long { "music" } else { "sfx" }}))?;
            }
            placed
        };
        let clips: Vec<u64> = placed.get("clips").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
        if has_video {
            let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
            mains.extend(clips.iter().map(|c| ClipId(*c)).filter(|c| q.find_item(*c).is_some_and(|(t, _)| t == main_id)));
        }
        new_clips.extend(clips);
    }
    // a sound placed on a new track: that track follows the anchors, not the sync lock
    s.edit_sequence("Einfügen", |q, _, _| {
        main_track::ensure(q);
        let main_audio = main_track::main_audio_track(q);
        for t in q.audio_tracks.iter_mut().filter(|t| Some(t.id) != main_audio && t.items.iter().all(|it| new_clips.contains(&it.id.0))) {
            t.sync_lock = false;
        }
        Ok(())
    })?;
    // then into their place in the order
    if let (Some(at), false) = (at, mains.is_empty()) {
        s.edit_sequence("Einfügen", |q, ctx, _| {
            let before = q.clone();
            let old: Vec<ClipId> = main_track::main_clips(q).iter().map(|m| m.id).filter(|c| !mains.contains(c)).collect();
            let without: Vec<(Tick, Tick)> =
                main_track::main_clips(q).iter().filter(|m| !mains.contains(&m.id)).map(|m| (m.start, m.end())).collect();
            let cut = {
                let mut c: Vec<Tick> = without.iter().map(|w| w.0).chain(without.last().map(|w| w.1)).collect();
                if c.is_empty() {
                    c.push(Tick::ZERO);
                }
                c.into_iter().min_by_key(|c| (c.0 - at.0).abs()).unwrap_or(Tick::ZERO)
            };
            let i = old.iter().position(|c| q.find_item(*c).is_some_and(|(_, it)| it.start >= cut)).unwrap_or(old.len());
            let mut order = old.clone();
            for (k, c) in mains.iter().enumerate() {
                order.insert(i + k, *c);
            }
            let starts = order_starts(q, &order);
            relayout(&before, q, &starts, ctx, Orphans::Keep)
        })?;
    }
    Ok(new_clips)
}

/// Cut a clip in two at `t`: the clip and its sound and background (whatever FilmCraft's Linked
/// Selection switch says), nothing else.
pub fn split(s: &mut Session, clip: ClipId, t: Tick) -> R<Value> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(clip).ok_or_else(|| other(format!("kein Clip {}", clip.0)))?;
    let t = q.settings.frame_rate.snap(t);
    if !(it.start < t && t < it.end()) {
        return Err(other("hier liegt der Clip nicht, nichts zu teilen"));
    }
    let group: Vec<ClipId> =
        with_partners(q, clip).into_iter().filter(|c| q.find_item(*c).is_some_and(|(_, it)| it.start < t && t < it.end())).collect();
    s.edit_group("Teilen", |s| {
        s.edit_sequence("Teilen", |q, ctx, _| {
            main_track::ensure(q);
            unlocked(q, &group)?;
            let new = filmcraft_edit::razor_items(q, &group, t, ctx);
            Ok(json!({"cuts": new.len(), "clips": new.iter().map(|c| c.0).collect::<Vec<_>>()}))
        })
    })
}

/// The main clip under `t`.
pub fn main_clip_at(q: &Sequence, t: Tick) -> Option<ClipId> {
    main_track::main_clips(q).iter().find(|m| m.start <= t && t < m.end()).map(|m| m.id)
}

/// Duplicate clips, CapCut's way: a main clip's copy (with its sound and background) comes right
/// after it and what follows moves on; any other clip's copy right after it on its track, or on
/// a free one. Nothing is written over. Returns the copies.
pub fn duplicate(s: &mut Session, clips: &[ClipId]) -> R<Value> {
    s.edit_group("Duplizieren", |s| {
        s.edit_sequence("Duplizieren", |q, ctx, _| {
            main_track::ensure(q);
            let mut copies = Vec::new();
            for c in clips {
                let before = q.clone();
                let Some((_, it)) = q.find_item(*c) else { continue };
                let (start, len, main) = (it.start, it.duration, is_main(q, *c));
                let group = with_partners(q, *c);
                unlocked(q, &group)?;
                if main && main_track::settings(q).magnet {
                    // make room right after it
                    let starts = later_starts(&before, *c, len);
                    relayout(&before, q, &starts, ctx, Orphans::Keep)?;
                }
                let link = (group.len() > 1).then(|| ctx.alloc());
                for id in &group {
                    let Some((tid, it)) = q.find_item(*id) else { continue };
                    let mut copy = it.clone();
                    copy.id = ClipId(ctx.alloc());
                    copy.start = it.start + len;
                    copy.link = link;
                    copy.group = None;
                    copy.effects.retain(|e| e.effect != anchors::ANCHOR);
                    let range = TimeRange::new(copy.start, copy.duration);
                    let fits = q.track(tid).is_some_and(|t| t.items.iter().all(|x| x.end() <= range.start || x.start >= range.end()));
                    let kind = q.track(tid).map(|t| t.kind).unwrap_or(TrackKind::Video);
                    let dest = if fits || (main && *id == *c) { tid } else { free_track(q, kind, Some(tid), range, ctx) };
                    if *id == *c {
                        copies.push(copy.id.0);
                    }
                    if let Some(t) = q.track_mut(dest) {
                        t.items.push(copy);
                        t.sort();
                    }
                }
                let _ = start;
                let fixed: HashSet<ClipId> = main_track::main_clips(q).iter().flat_map(|m| with_partners(q, m.id)).collect();
                settle(q, &fixed, ctx)?;
            }
            if copies.is_empty() {
                return Err(other("keine Clips zum Duplizieren"));
            }
            Ok(json!({"copies": copies}))
        })
    })
}
