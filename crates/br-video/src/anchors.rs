//! Anchors: CapCut's linkage. Titles, overlays, sound effects and captions belong to the main
//! clip they start on, and when that clip moves, is trimmed, sped up, cut in two or deleted, they
//! go with it. What starts after the end of the main track keeps its distance to that end (an end
//! card), what starts in a gap belongs to the clip before it.
//!
//! FilmCraft has no such relation, and BR stores none: an anchor is worked out from where things
//! are *before* an edit. Undo snapshots therefore carry the anchors exactly, and FilmCraft reusing
//! a clip id after an undo cannot confuse them. The one thing kept is the user's exception, as a
//! hidden effect on the child clip (`br.anchor`): detached from any clip, or fixed to one.
//!
//! [`plan`] and [`apply`] run inside the edit that changed the main track (BR's own commands, and
//! BR's catch-up after an edit made with FilmCraft's own tools, see [`crate::reconcile`]): every
//! child its edit left where it was moves to where its main clip (or its picture) went.

use std::collections::{HashMap, HashSet};

use filmcraft_edit::EditCtx;
use filmcraft_project::{AudioType, ClipId, ItemId, ParamValue, Sequence, TrackId, TrackItem, TrackKind};
use filmcraft_time::Tick;

use crate::main_track::{self, is_background, is_background_track};

/// Hidden clip effect holding the user's exception: `detached` (Bool) or `parent` (a main clip id).
pub const ANCHOR: &str = "br.anchor";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A video clip on another track than the main one: overlay, title, sticker, graphic.
    Overlay,
    /// Sound on its own (not a main clip's sound, not music): a sound effect, a voice-over.
    Sound,
    /// A caption. Captions follow the main clip's picture (they belong to its speech).
    Caption,
}

/// What a child is attached to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parent {
    /// A main clip.
    Clip(ClipId),
    /// The end of the main track (the child starts after it).
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub child: ClipId,
    pub kind: Kind,
    pub parent: Parent,
    /// The child's start and end before the edit.
    pub start: Tick,
    pub end: Tick,
    /// The parent's start and end before the edit (the main track's end for [`Parent::End`]).
    pub parent_start: Tick,
    pub parent_end: Tick,
    /// The parent's media time at the child's start and end, and the parent's media.
    pub media: Tick,
    pub media_end: Tick,
    pub item: ItemId,
    /// The child is in a gap after its parent, not on it.
    pub after: bool,
    /// The child's own media (sound taken off its picture keeps the picture's media).
    pub own_item: ItemId,
    /// A caption of the parent's own speech (it follows the parent's picture); else it keeps its
    /// distance to the parent's start, as a title does (captions of a voice-over over pictures).
    pub speech: bool,
}

impl Anchor {
    pub fn parent_clip(&self) -> Option<ClipId> {
        match self.parent {
            Parent::Clip(c) => Some(c),
            Parent::End => None,
        }
    }
}

/// What happens to children whose main clip is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orphans {
    /// They go too (a main clip deleted by BR, or by FilmCraft's Ripple Delete, Clear, Extract).
    Delete,
    /// They stay where they are (the main clip was replaced, nested, cut to the clipboard, …).
    Keep,
}

/// What [`apply`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Followed {
    pub moved: Vec<ClipId>,
    pub deleted: Vec<ClipId>,
}

impl Followed {
    pub fn is_empty(&self) -> bool {
        self.moved.is_empty() && self.deleted.is_empty()
    }
}

/// The user's exception for a child clip: `Some(None)` detached, `Some(Some(id))` fixed to a clip.
/// It holds the id of the clip it was set on: a copy, or the right piece of a split, carries the
/// effect along but is a clip of its own and follows the automatic anchor.
fn exception(it: &TrackItem) -> Option<Option<ClipId>> {
    let e = it.effect(ANCHOR)?;
    if !matches!(e.param("child").map(|p| &p.value), Some(ParamValue::Float(c)) if *c == it.id.0 as f64) {
        return None;
    }
    if matches!(e.param("detached").map(|p| &p.value), Some(ParamValue::Bool(true))) {
        return Some(None);
    }
    match e.param("parent").map(|p| &p.value) {
        Some(ParamValue::Float(id)) if *id >= 0.0 => Some(Some(ClipId(*id as u64))),
        _ => None,
    }
}

/// Set or clear the user's exception on `clip`: `Some(None)` detaches it, `Some(Some(p))` fixes it
/// to main clip `p`, `None` returns it to the automatic anchor.
pub fn set_exception(q: &mut Sequence, clip: ClipId, to: Option<Option<ClipId>>) -> bool {
    let Some((_, it)) = q.find_item_mut(clip) else { return false };
    it.effects.retain(|e| e.effect != ANCHOR);
    let child = ("child", ParamValue::Float(clip.0 as f64));
    match to {
        None => {}
        Some(None) => it.effects.push(main_track::hidden(ANCHOR, &[child, ("detached", ParamValue::Bool(true))])),
        Some(Some(p)) => it.effects.push(main_track::hidden(ANCHOR, &[child, ("parent", ParamValue::Float(p.0 as f64))])),
    }
    true
}

/// The timeline time at which `it` shows media time `m` (None for a frame hold).
fn time_of_media(it: &TrackItem, m: Tick) -> Option<Tick> {
    if it.frame_hold.is_some() {
        return None;
    }
    let speed = it.speed.abs();
    if speed <= 0.0 {
        return None;
    }
    let rel = if it.reverse {
        // moving_source_time_at: source_in + duration·speed − rel·speed − 1
        (it.source_out().0 - 1 - m.0) as f64 / speed
    } else {
        (m.0 - it.source_in.0) as f64 / speed
    };
    Some(it.start + Tick(rel.round() as i64))
}

/// Does BR keep anchors on this sequence? Not on multi-camera sources and merged clips (their
/// tracks are cameras and channels, not overlays), and not with linkage off.
pub fn active(q: &Sequence) -> bool {
    q.multicam.is_none() && q.merged.is_none() && main_track::settings(q).linkage
}

/// The anchors of `q`: every child and what it belongs to. Children on locked tracks keep their
/// place and have none.
pub fn derive(q: &Sequence) -> Vec<Anchor> {
    if !active(q) {
        return Vec::new();
    }
    let Some(main_id) = main_track::main_track_id(q) else { return Vec::new() };
    let main = main_track::main_clips(q);
    let Some(last) = main.last() else { return Vec::new() };
    let main_end = last.end();
    // the main clips' own sound and backgrounds are part of them, not children
    let main_links: HashSet<u64> = main.iter().filter_map(|m| m.link).collect();
    let mut out = Vec::new();
    let anchor = |kind: Kind, child: ClipId, start: Tick, end: Tick, own: ItemId, fixed: Option<Option<ClipId>>| -> Option<Anchor> {
        let parent = match fixed {
            Some(None) => return None,
            Some(Some(p)) => main.iter().find(|m| m.id == p).copied(),
            None => None,
        };
        let on = parent.or_else(|| main.iter().find(|m| m.start <= start && start < m.end()).copied());
        let (parent, after) = match on {
            Some(p) => (Some(p), false),
            // after the end: the end; in a gap: the clip before it
            None if start >= main_end => (None, true),
            None => (main.iter().rev().find(|m| m.end() <= start).or(main.first()).copied(), true),
        };
        Some(match parent {
            Some(p) => Anchor {
                child,
                kind,
                parent: Parent::Clip(p.id),
                start,
                end,
                parent_start: p.start,
                parent_end: p.end(),
                media: p.moving_source_time_at(start.min(p.end() - Tick(1))),
                media_end: p.moving_source_time_at(end.min(p.end() - Tick(1))),
                item: p.item,
                after,
                own_item: own,
                speech: false,
            },
            None => Anchor {
                child,
                kind,
                parent: Parent::End,
                start,
                end,
                parent_start: main_end,
                parent_end: main_end,
                media: Tick::ZERO,
                media_end: Tick::ZERO,
                item: ItemId(u64::MAX),
                after: true,
                own_item: own,
                speech: false,
            },
        })
    };
    let mut overlay_links: HashSet<u64> = HashSet::new();
    for t in q.video_tracks.iter().filter(|t| t.id != main_id && !is_background_track(t) && !t.locked) {
        for it in t.items.iter().filter(|it| !is_background(it) && !it.link.is_some_and(|l| main_links.contains(&l))) {
            // an overlay's own sound moves with it as its partner
            if let Some(l) = it.link {
                overlay_links.insert(l);
            }
            out.extend(anchor(Kind::Overlay, it.id, it.start, it.end(), it.item, exception(it)));
        }
    }
    let mut sound_links: HashSet<u64> = HashSet::new();
    for t in q.audio_tracks.iter().filter(|t| !t.locked) {
        for it in t.items.iter() {
            let linked_elsewhere = it.link.is_some_and(|l| main_links.contains(&l) || overlay_links.contains(&l));
            let music = it.essential.as_ref().is_some_and(|e| e.kind == AudioType::Music);
            // of linked sound without a picture (a stereo pair on two tracks) one stands for all
            let seen = it.link.is_some_and(|l| !sound_links.insert(l));
            if !linked_elsewhere && !music && !seen {
                out.extend(anchor(Kind::Sound, it.id, it.start, it.end(), it.item, exception(it)));
            }
        }
    }
    // sound of its own (a voice-over) under the pictures: captions over it are its speech
    let voices: Vec<(Tick, Tick)> = out.iter().filter(|a| a.kind == Kind::Sound).map(|a| (a.start, a.end)).collect();
    let speaks = |p: ClipId| {
        let Some(link) = main.iter().find(|m| m.id == p).and_then(|m| m.link) else { return false };
        q.audio_tracks.iter().filter(|t| !t.muted).flat_map(|t| t.items.iter()).any(|it| it.link == Some(link) && it.enabled)
    };
    for t in q.caption_tracks.iter().filter(|t| !t.locked) {
        for c in &t.captions {
            let Some(mut a) = anchor(Kind::Caption, c.id, c.start, c.end(), ItemId(u64::MAX), None) else { continue };
            let len = (c.end() - c.start).0.max(1);
            let over_voice = voices.iter().any(|(s, e)| (*e).min(c.end()).0 - (*s).max(c.start).0 > len / 2);
            a.speech = a.parent_clip().is_some_and(speaks) && !over_voice;
            out.push(a);
        }
    }
    out
}

/// Where a child belongs after the edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// At this time (captions: with this length).
    At(Tick, Option<Tick>),
    /// Its content is gone (a caption whose speech was trimmed away).
    Gone,
    /// Its main clip is gone.
    Orphan,
}

/// The time span in which `it` shows media from `m0` to `m1` (either order), cut to the clip.
fn span_of(it: &TrackItem, m0: Tick, m1: Tick) -> Option<(Tick, Tick)> {
    let (a, b) = (time_of_media(it, m0)?, time_of_media(it, m1)?);
    let (lo, hi) = (a.min(b).max(it.start), a.max(b).min(it.end()));
    (hi > lo).then_some((lo, hi))
}

/// Did the edit change where `p` is or what it shows (start, end, media, speed, direction)?
fn reshaped(a: &Anchor, p: &TrackItem) -> bool {
    p.start != a.parent_start
        || p.end() != a.parent_end
        || p.item != a.item
        || (!a.after && p.moving_source_time_at(a.start.min(p.end() - Tick(1))) != a.media)
}

/// Where the child of anchor `a` belongs in `q`. Overlays and sounds keep their distance to the
/// start of their main clip; captions stay with its picture. When the main clip was cut in two
/// (FilmCraft's split keeps the id on the left piece) or something was inserted into it, the
/// child goes with the piece that still shows its picture.
fn place(a: &Anchor, q: &Sequence, old_ids: &HashSet<ClipId>, old_end: Tick) -> Place {
    let Some(pid) = a.parent_clip() else {
        // after the end: the same distance to the end
        let end = main_track::main_clips(q).last().map(|m| m.end()).unwrap_or(Tick::ZERO);
        return Place::At(end + (a.start - old_end), None);
    };
    let same = q.find_item(pid).map(|(_, it)| it);
    let content = |p: &TrackItem| p.item == a.item && p.frame_hold.is_none() && !a.after;
    if let Some(p) = same {
        if !reshaped(a, p) {
            return Place::At(a.start, None);
        }
        match a.kind {
            // a caption of the clip's speech follows its picture while the clip still shows it
            Kind::Caption if content(p) && a.speech => {
                if let Some((lo, hi)) = span_of(p, a.media, a.media_end) {
                    return Place::At(lo, Some(hi - lo));
                }
            }
            _ => {
                let t = p.start + (a.start - a.parent_start);
                if a.after || (t >= p.start && t < p.end()) || !content(p) {
                    return Place::At(t, None);
                }
            }
        }
    }
    // another piece of the same media that the edit made (not a clip that was there before)
    if !a.after {
        for it in main_track::main_clips(q).into_iter().filter(|it| it.item == a.item && it.id != pid && !old_ids.contains(&it.id)) {
            match a.kind {
                Kind::Caption if a.speech => {
                    if let Some((lo, hi)) = span_of(it, a.media, a.media_end) {
                        return Place::At(lo, Some(hi - lo));
                    }
                }
                _ => {
                    if let Some(t) = time_of_media(it, a.media).filter(|t| *t >= it.start && *t < it.end()) {
                        return Place::At(t, None);
                    }
                }
            }
        }
    }
    match (same, a.kind) {
        (Some(_), Kind::Caption) if a.speech && content_kept(a, same) => Place::Gone,
        // an overlay past the end of a shortened main clip keeps its distance to its start
        (Some(p), _) => Place::At(p.start + (a.start - a.parent_start), None),
        (None, _) => Place::Orphan,
    }
}

/// The caption's clip still shows its media (only then is speech trimmed away "gone"; a clip that
/// now shows other media, or holds a frame, keeps its captions by their distance to its start).
fn content_kept(a: &Anchor, p: Option<&TrackItem>) -> bool {
    p.is_some_and(|p| p.item == a.item && p.frame_hold.is_none())
}

/// What the children of a changed main track need: where they go, which go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    clip_moves: Vec<(ClipId, Tick)>,
    caption_moves: Vec<(ClipId, Tick, Option<Tick>)>,
    gone: Vec<ClipId>,
    gone_captions: Vec<ClipId>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.clip_moves.is_empty() && self.caption_moves.is_empty() && self.gone.is_empty() && self.gone_captions.is_empty()
    }
}

/// What the edit from `before` to `q` means for the children of the main clips it moved,
/// trimmed, sped up, cut or deleted. Children the edit moved itself (FilmCraft's sync lock took
/// them along, or the user dragged one) stay where they are.
pub fn plan(before: &Sequence, q: &Sequence, orphans: Orphans) -> Plan {
    let mut plan = Plan::default();
    if !active(q) {
        return plan;
    }
    let old_ids: HashSet<ClipId> = before.all_tracks().flat_map(|t| t.items.iter()).map(|it| it.id).collect();
    let old_end = main_track::main_clips(before).last().map(|m| m.end()).unwrap_or(Tick::ZERO);
    for a in derive(before) {
        let now = match a.kind {
            Kind::Caption => q.find_caption(a.child).map(|(_, c)| (c.start, c.end())),
            _ => q.find_item(a.child).map(|(_, it)| (it.start, it.end())),
        };
        let Some((now, now_end)) = now else { continue };
        let caption = a.kind == Kind::Caption;
        match place(&a, q, &old_ids, old_end) {
            Place::Orphan => {
                // what reaches well past the deleted clip (a voice-over, a logo over the whole
                // video) is not its own and stays, as does sound taken off this very clip
                let slack = before.settings.frame_rate.frame_duration().max(Tick((a.parent_end - a.parent_start).0 / 10));
                let keeps = a.end > a.parent_end + slack || (a.kind == Kind::Sound && a.own_item == a.item);
                if orphans == Orphans::Delete && !keeps && !a.after {
                    if caption { plan.gone_captions.push(a.child) } else { plan.gone.push(a.child) }
                }
            }
            // a child the edit moved itself is where it should be
            _ if now != a.start || now_end != a.end => {}
            Place::Gone => plan.gone_captions.push(a.child),
            Place::At(t, len) => {
                let len = len.filter(|l| *l != now_end - now);
                if t != now || len.is_some() {
                    if caption { plan.caption_moves.push((a.child, t, len)) } else { plan.clip_moves.push((a.child, t)) }
                }
            }
        }
    }
    plan
}

/// Move the children of the main clips that the edit from `before` to `q` changed. Runs inside
/// that edit (one undo step with it).
pub fn follow(before: &Sequence, q: &mut Sequence, ctx: &mut EditCtx, orphans: Orphans) -> filmcraft_engine::Result<Followed> {
    let p = plan(before, q, orphans);
    apply(q, ctx, p)
}

/// Is `range` free on track `t`, apart from the clips in `moving`?
fn free(q: &Sequence, t: TrackId, start: Tick, end: Tick, moving: &HashSet<ClipId>, taken: &[(TrackId, Tick, Tick)]) -> bool {
    let Some(tr) = q.track(t) else { return false };
    !tr.locked
        && tr.items.iter().filter(|it| !moving.contains(&it.id)).all(|it| it.end() <= start || it.start >= end)
        && taken.iter().filter(|(tt, _, _)| *tt == t).all(|(_, s, e)| *e <= start || *s >= end)
}

/// A track for a child that does not fit on its own: the next free one of its kind (never the
/// main track, its sound track or the backgrounds), else a new one.
fn other_track(
    q: &mut Sequence,
    kind: TrackKind,
    from: TrackId,
    start: Tick,
    end: Tick,
    moving: &HashSet<ClipId>,
    taken: &[(TrackId, Tick, Tick)],
    ctx: &mut EditCtx,
) -> TrackId {
    let main = main_track::main_track_id(q);
    let main_audio = main_track::main_audio_track(q);
    let tracks = q.tracks(kind);
    let at = tracks.iter().position(|t| t.id == from).map_or(0, |i| i + 1);
    let candidates: Vec<TrackId> = tracks
        .iter()
        .skip(at)
        .chain(tracks.iter().take(at))
        .filter(|t| t.id != from && Some(t.id) != main && Some(t.id) != main_audio && !is_background_track(t))
        .map(|t| t.id)
        .collect();
    if let Some(t) = candidates.into_iter().find(|t| free(q, *t, start, end, moving, taken)) {
        return t;
    }
    let id = TrackId(ctx.alloc());
    let n = q.tracks(kind).len() + 1;
    q.tracks_mut(kind).push(main_track::new_track(id, kind, n));
    id
}

/// Carry out a [`plan`]: children move with their own partners (an overlay's sound) onto free
/// room, never over something else; captions keep their tracks in order.
pub fn apply(q: &mut Sequence, ctx: &mut EditCtx, plan: Plan) -> filmcraft_engine::Result<Followed> {
    let Plan { clip_moves, caption_moves, gone, gone_captions } = plan;
    let mut done = Followed::default();
    if !gone.is_empty() {
        let mut all: Vec<ClipId> = gone.clone();
        for c in &gone {
            all.extend(main_track::partners(q, *c));
        }
        all.sort_by_key(|c| c.0);
        all.dedup();
        filmcraft_edit::delete_items(q, &all);
        done.deleted.extend(all);
    }
    if !clip_moves.is_empty() {
        let mut moving: HashSet<ClipId> = HashSet::new();
        let mut groups: Vec<Vec<(ClipId, TrackId, Tick, Tick)>> = Vec::new();
        for (c, t) in &clip_moves {
            // a clip named twice (two children of one link group) moves once
            if moving.contains(c) {
                continue;
            }
            let Some((_, it)) = q.find_item(*c) else { continue };
            let delta = *t - it.start;
            let mut g = Vec::new();
            for id in std::iter::once(*c).chain(main_track::partners(q, *c)) {
                if moving.contains(&id) {
                    continue;
                }
                if let Some((tid, it)) = q.find_item(id) {
                    moving.insert(id);
                    g.push((id, tid, (it.start + delta).max(Tick::ZERO), it.duration));
                }
            }
            groups.push(g);
        }
        let mut taken: Vec<(TrackId, Tick, Tick)> = Vec::new();
        let mut moves: Vec<(ClipId, TrackId, Tick)> = Vec::new();
        for g in groups {
            for (id, tid, start, dur) in g {
                let end = start + dur;
                let dest = if free(q, tid, start, end, &moving, &taken) {
                    tid
                } else {
                    let kind = q.track(tid).map(|t| t.kind).unwrap_or(TrackKind::Video);
                    other_track(q, kind, tid, start, end, &moving, &taken, ctx)
                };
                taken.push((dest, start, end));
                moves.push((id, dest, start));
                done.moved.push(id);
            }
        }
        filmcraft_edit::move_items(q, &moves, false, ctx)?;
    }

    // captions: set, then keep each track in order without overlaps (an earlier caption is cut
    // where a moved one now starts)
    if !gone_captions.is_empty() {
        filmcraft_edit::captions::delete_captions(q, &gone_captions, false)?;
        done.deleted.extend(gone_captions.iter().copied());
    }
    if !caption_moves.is_empty() {
        let to: HashMap<ClipId, (Tick, Option<Tick>)> = caption_moves.iter().map(|(c, t, l)| (*c, (*t, *l))).collect();
        for tr in q.caption_tracks.iter_mut().filter(|t| !t.locked) {
            let mut changed = false;
            for c in tr.captions.iter_mut() {
                if let Some((t, len)) = to.get(&c.id) {
                    c.start = (*t).max(Tick::ZERO);
                    if let Some(l) = len {
                        c.duration = *l;
                    }
                    changed = true;
                    done.moved.push(c.id);
                }
            }
            if !changed {
                continue;
            }
            tr.sort();
            let mut i = 0;
            while i + 1 < tr.captions.len() {
                let next = tr.captions[i + 1].start;
                let c = &mut tr.captions[i];
                if c.end() > next {
                    c.duration = next - c.start;
                }
                if c.duration.0 <= 0 {
                    done.deleted.push(c.id);
                    tr.captions.remove(i);
                } else {
                    i += 1;
                }
            }
        }
    }
    Ok(done)
}
