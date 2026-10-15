//! Catching up after FilmCraft's own tools. An edit made in FilmCraft's timeline, menus or panels
//! (or through `film_run`) moves main clips without knowing about BR's anchors. BR compares the
//! project with the last one it reconciled: when the main track changed, the children follow
//! inside the undo step on top, so one undo takes back the edit and the following together, as
//! with BR's own commands.
//!
//! One [`Watch`] belongs to one session: the window keeps one for its video, and every command
//! (BR's or FilmCraft's) run through [`crate::execute_with`] advances it, so no step is followed
//! twice and BR's own steps are never taken for FilmCraft's.

use std::sync::Arc;

use filmcraft_engine::Session;
use filmcraft_project::{Project, Sequence};
use filmcraft_time::Tick;

use crate::anchors::{self, Orphans};
use crate::main_track;

/// FilmCraft's commands (and their undo labels) that remove clips: what hangs on a main clip
/// they remove goes too.
/// (Cut is not one: the clip goes to the clipboard alone, so what hangs on it stays.)
const REMOVING: &[&str] = &["edit.rippleDelete", "edit.clear", "sequence.extract", "sequence.lift"];
const REMOVING_LABELS: &[&str] = &["Ripple Delete", "Clear", "Extract", "Lift"];

/// The project BR last reconciled, and what redo and the command journal held then.
#[derive(Clone, Default)]
pub struct Watch {
    reconciled: Option<Arc<Project>>,
    redo_top: Option<Arc<Project>>,
    journal: usize,
}

impl Watch {
    pub fn new(s: &Session) -> Self {
        let mut w = Watch::default();
        w.look(s);
        w
    }

    /// Take the session as it is now as reconciled.
    pub fn look(&mut self, s: &Session) {
        self.reconciled = Some(s.project.clone());
        self.redo_top = s.history.redo.last().map(|r| r.1.clone());
        self.journal = s.journal.len();
    }
}

/// The main track as anchors see it: each main clip's place and media.
fn main_shape(q: &Sequence) -> Vec<(u64, i64, i64, i64, u64, bool, u64)> {
    main_track::main_clips(q)
        .iter()
        .map(|it| (it.id.0, it.start.0, it.duration.0, it.source_in.0, it.speed.to_bits(), it.reverse, it.item.0))
        .collect()
}

/// Has the project changed by edits on top of what `w` reconciled (not an undo, a redo, an opened
/// project)? Then let the children of the main clips those edits changed follow, inside the undo
/// step on top. Returns whether anything moved.
pub fn catch_up(s: &mut Session, w: &mut Watch) -> bool {
    // a dynamic trim (J/K/L in trim mode) changes the project without a step until it ends
    if s.trim_play.dynamic.is_some() {
        return false;
    }
    let Some(rec) = w.reconciled.clone() else {
        w.look(s);
        return false;
    };
    if Arc::ptr_eq(&rec, &s.project) {
        w.look(s);
        return false;
    }
    let redo = w.redo_top.as_ref().is_some_and(|r| Arc::ptr_eq(r, &s.project));
    let base = s.history.undo.iter().rposition(|u| Arc::ptr_eq(&u.1, &rec));
    // a gesture that merges into one step (a drag) changes the project without a new step
    let merged = base.is_none() && s.history.merge_key.is_some();
    if !s.history.redo.is_empty() || redo || (base.is_none() && !merged) {
        // undo, redo, a project opened, made or recovered: nothing to follow
        w.look(s);
        return false;
    }
    let labels: Vec<&str> = match base {
        Some(i) => s.history.undo[i..].iter().map(|u| u.0.as_str()).collect(),
        None => s.history.undo.last().map(|u| u.0.as_str()).into_iter().collect(),
    };
    let journal = s.journal.get(w.journal.min(s.journal.len())..).unwrap_or_default();
    let removing = labels.iter().any(|l| REMOVING_LABELS.contains(l)) || journal.iter().any(|j| REMOVING.contains(&j.0.as_str()));
    let orphans = if removing { Orphans::Delete } else { Orphans::Keep };
    let moved = follow(s, &rec, orphans);
    w.look(s);
    moved
}

fn follow(s: &mut Session, rec: &Project, orphans: Orphans) -> bool {
    let Some(active) = s.state.active_sequence else { return false };
    let Some(before) = rec.sequence(active).cloned() else { return false };
    let Some(now) = s.active_sequence() else { return false };
    // only videos BR works on: a sequence of FilmCraft's own keeps FilmCraft's behaviour
    if !main_track::is_marked(&before) || !main_track::is_marked(now) {
        return false;
    }
    if main_shape(&before) == main_shape(now) {
        return false;
    }
    // with the magnet on, a removal made in FilmCraft (Clear, Lift, Extract) closes its gap as BR's
    // delete does; FilmCraft's Ripple Delete has closed it already
    let close_up = orphans == Orphans::Delete && main_track::settings(now).magnet;
    let mut starts = Vec::new();
    if close_up {
        let mut at = main_track::main_clips(now).first().map(|m| m.start.min(Tick::ZERO)).unwrap_or(Tick::ZERO);
        for m in main_track::main_clips(now) {
            if m.start != at {
                starts.push((m.id, at));
            }
            at += m.duration;
        }
    }
    let plan = anchors::plan(&before, now, orphans);
    if plan.is_empty() && starts.is_empty() {
        return false;
    }
    let r = if starts.is_empty() {
        amend_last(s, "Anker", |q, ctx| anchors::apply(q, ctx, plan))
    } else {
        // the gap cannot close (a locked track): what hung on the removed clip still follows
        amend_last(s, "Anker", |q, ctx| crate::magnet::relayout(&before, q, &starts, ctx, orphans)).or_else(|e| {
            log::info!("the gap stays: {e}");
            let plan = s.active_sequence().map(|now| anchors::plan(&before, now, orphans)).unwrap_or_default();
            amend_last(s, "Anker", |q, ctx| anchors::apply(q, ctx, plan))
        })
    };
    match r {
        Ok(_) => true,
        Err(e) => {
            log::warn!("anchors could not follow: {e}");
            false
        }
    }
}

/// Change the active sequence as part of the undo step on top, so the change costs no step of
/// its own: edit (which pushes a step holding the state before this change), then drop that step
/// again, so undo returns to the state before the step on top in one go. Selections of clips the
/// change removed are dropped; playback and autosave get the result.
pub fn amend_last<R>(
    s: &mut Session,
    label: &str,
    f: impl FnOnce(&mut Sequence, &mut filmcraft_edit::EditCtx) -> filmcraft_engine::Result<R>,
) -> filmcraft_engine::Result<R> {
    if s.history.undo.is_empty() {
        return Err(filmcraft_engine::EngineError::Other("no step to add to".into()));
    }
    let (limit, merge) = (s.history.limit, s.history.merge_key.clone());
    s.history.limit = usize::MAX;
    let depth = s.history.undo.len();
    let pre = s.project.clone();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.edit_sequence(label, |q, ctx, _| f(q, ctx))));
    if matches!(r, Ok(Ok(_))) && s.history.undo.len() == depth + 1 && s.history.undo.last().is_some_and(|u| Arc::ptr_eq(&u.1, &pre)) {
        s.history.undo.pop();
    }
    s.history.limit = limit;
    s.history.merge_key = merge;
    let r = match r {
        Ok(r) => r,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    if r.is_ok() {
        s.fix_state();
        let live: std::collections::HashSet<filmcraft_project::ClipId> =
            s.active_sequence().map(|q| q.all_tracks().flat_map(|t| t.items.iter()).map(|it| it.id).collect()).unwrap_or_default();
        let captions: std::collections::HashSet<filmcraft_project::ClipId> =
            s.active_sequence().map(|q| q.caption_tracks.iter().flat_map(|t| t.captions.iter()).map(|c| c.id).collect()).unwrap_or_default();
        s.state.selection.retain(|c| live.contains(c));
        s.state.caption_selection.retain(|c| captions.contains(c));
        s.sync_persistence();
        s.previews.live.publish_project(s.project.clone());
    }
    r
}

/// The tracks of the active sequence (video, sound, captions).
pub fn track_ids(s: &Session) -> Vec<filmcraft_project::TrackId> {
    s.active_sequence().map(|q| q.all_tracks().map(|t| t.id).chain(q.caption_tracks.iter().map(|t| t.id)).collect()).unwrap_or_default()
}

/// Tracks a BR command added follow BR's anchors, not Premiere's sync lock (so FilmCraft's own
/// ripples are not refused because of a music bed, and inserts do not cut it).
pub fn unlock_new_tracks(s: &mut Session, before: &[filmcraft_project::TrackId]) {
    let locked_new = s.active_sequence().is_some_and(|q| {
        q.all_tracks().any(|t| t.sync_lock && !before.contains(&t.id)) || q.caption_tracks.iter().any(|t| t.sync_lock && !before.contains(&t.id))
    });
    // with linkage off the tracks keep FilmCraft's sync lock; a new video's main track, its sound
    // and the backgrounds keep theirs (a command that made a new video and added music or titles
    // to it, as a template does, unlocks those)
    let linkage = s.active_sequence().is_some_and(|q| main_track::settings(q).linkage);
    if !locked_new || !linkage {
        return;
    }
    let main = s.active_sequence().and_then(main_track::main_track_id);
    let main_audio = s.active_sequence().and_then(main_track::main_audio_track);
    let r = amend_last(s, "BR", |q, _| {
        let mut unlocked = Vec::new();
        for t in q.all_tracks_mut().filter(|t| {
            t.sync_lock && !before.contains(&t.id) && Some(t.id) != main && Some(t.id) != main_audio && !main_track::is_background_track(t)
        }) {
            t.sync_lock = false;
            unlocked.push(t.id);
        }
        for t in q.caption_tracks.iter_mut().filter(|t| t.sync_lock && !before.contains(&t.id)) {
            t.sync_lock = false;
            unlocked.push(t.id);
        }
        main_track::note_unlocked(q, &unlocked);
        Ok(())
    });
    if let Err(e) = r {
        log::warn!("new tracks keep their sync lock: {e}");
    }
}
