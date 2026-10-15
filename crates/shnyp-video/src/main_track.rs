//! The main track: the video track CapCut builds a video on (clips back to back, everything else
//! attached to them). FilmCraft has no such notion, so shnyp marks the track with a hidden track
//! effect (an effect id without a definition, which render, mixer and panels skip, as FilmCraft's
//! own Remix state does). The mark lives in the project, so undo, save and FilmCraft's track
//! renumbering (a background track added below it) keep it, and its parameters hold the video's
//! CapCut switches: the magnetic main track and the linkage of attached clips.

use filmcraft_project::{ClipId, EffectInstance, Param, ParamValue, Sequence, Track, TrackId, TrackItem, TrackKind};

/// Hidden track effect on the main video track; `magnet` and `linkage` (Bool) are its params.
pub const MAIN: &str = "shnyp.main";
/// Hidden track effect on the track that holds the filled backgrounds (below the main track).
pub const BACKGROUND: &str = "shnyp.background";
/// Hidden clip effect on a background clip (the blurred copy, colour or picture behind a main clip).
pub const BG_CLIP: &str = "shnyp.bg";

/// A hidden effect: no definition, so nothing renders, mixes or lists it.
pub fn hidden(id: &str, params: &[(&str, ParamValue)]) -> EffectInstance {
    EffectInstance {
        effect: id.into(),
        enabled: true,
        params: params.iter().map(|(k, v)| ((*k).to_string(), Param::new(v.clone()))).collect(),
        masks: Vec::new(),
        post_fader: false,
        essential: false,
        layer: None,
    }
}

/// Hidden effect `effect` is `id`: also under the name it had while shnyp was called BR
/// (`br.main` for `shnyp.main`, …), so videos made then keep their main track, backgrounds,
/// anchors, curves and slots.
pub fn is(effect: &str, id: &str) -> bool {
    effect == id || id.strip_prefix("shnyp.").is_some_and(|name| effect.strip_prefix("br.") == Some(name))
}

/// The clip's hidden effect `id` (see [`is`]).
pub fn effect_of<'a>(it: &'a TrackItem, id: &str) -> Option<&'a EffectInstance> {
    it.effects.iter().find(|e| is(&e.effect, id))
}

fn marked(effects: &[EffectInstance], id: &str) -> bool {
    effects.iter().any(|e| is(&e.effect, id))
}

/// The track holds the backgrounds.
pub fn is_background_track(t: &Track) -> bool {
    marked(&t.effects, BACKGROUND)
}

/// The clip is a filled background.
pub fn is_background(it: &TrackItem) -> bool {
    marked(&it.effects, BG_CLIP)
}

/// The main track: the marked video track, else the first one that holds no backgrounds.
pub fn main_track(q: &Sequence) -> Option<&Track> {
    q.video_tracks.iter().find(|t| marked(&t.effects, MAIN)).or_else(|| q.video_tracks.iter().find(|t| !is_background_track(t)))
}

pub fn main_track_id(q: &Sequence) -> Option<TrackId> {
    main_track(q).map(|t| t.id)
}

/// Mark the main track if no track is marked yet. A sequence shnyp did not make (a FilmCraft project,
/// an imported one) keeps FilmCraft's behaviour: both switches off until the user turns them on.
/// Part of the caller's edit, so it costs no undo step of its own.
pub fn ensure(q: &mut Sequence) -> Option<TrackId> {
    if let Some(t) = q.video_tracks.iter().find(|t| marked(&t.effects, MAIN)) {
        return Some(t.id);
    }
    let t = q.video_tracks.iter_mut().find(|t| !is_background_track(t))?;
    t.effects.push(hidden(MAIN, &[("magnet", ParamValue::Bool(false)), ("linkage", ParamValue::Bool(false))]));
    Some(t.id)
}

/// Make `q` a CapCut-style video: the main track marked, magnet and linkage on (a video shnyp makes).
pub fn capcut(q: &mut Sequence) {
    ensure(q);
    set_flag(q, "magnet", true);
    set_flag(q, "linkage", true);
}

/// The video's CapCut switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// The main track stays without gaps: deleting closes them, moving reorders, inserting pushes.
    pub magnet: bool,
    /// Titles, overlays, sound effects and captions follow the main clip they start on.
    pub linkage: bool,
}

fn flag(e: &EffectInstance, key: &str) -> Option<bool> {
    match e.param(key)?.value {
        ParamValue::Bool(b) => Some(b),
        _ => None,
    }
}

fn marker(q: &Sequence) -> Option<&EffectInstance> {
    q.video_tracks.iter().flat_map(|t| t.effects.iter()).find(|e| is(&e.effect, MAIN))
}

/// The switches; both off on a sequence shnyp has not made its own.
pub fn settings(q: &Sequence) -> Settings {
    let Some(e) = marker(q) else { return Settings::default() };
    Settings { magnet: flag(e, "magnet").unwrap_or(false), linkage: flag(e, "linkage").unwrap_or(false) }
}

/// Set one switch (`magnet` or `linkage`); marks the main track if needed. Linkage on takes the
/// tracks other than the main track (with its sound and backgrounds) off Premiere's sync lock, so
/// that shnyp's anchors move what hangs on the main clips, and remembers how they were; linkage off
/// gives them back their sync locks, which is FilmCraft's own behaviour.
pub fn set_flag(q: &mut Sequence, key: &str, on: bool) {
    let Some(id) = ensure(q) else { return };
    if key == "linkage" && settings(q).linkage != on {
        if on {
            let main_audio = main_audio_track(q);
            let mut kept = Vec::new();
            for t in q.all_tracks_mut().filter(|t| t.id != id && Some(t.id) != main_audio && !is_background_track(t)) {
                kept.push(format!("{}={}", t.id.0, u8::from(t.sync_lock)));
                t.sync_lock = false;
            }
            for t in q.caption_tracks.iter_mut() {
                kept.push(format!("{}={}", t.id.0, u8::from(t.sync_lock)));
                t.sync_lock = false;
            }
            set_param(q, id, "locks", ParamValue::Text(kept.join(",")));
        } else {
            let locks = marker(q).and_then(|e| match &e.param("locks")?.value {
                ParamValue::Text(t) => Some(t.clone()),
                _ => None,
            });
            let was: std::collections::HashMap<u64, bool> = locks
                .unwrap_or_default()
                .split(',')
                .filter_map(|kv| kv.split_once('='))
                .filter_map(|(k, v)| Some((k.parse().ok()?, v == "1")))
                .collect();
            for t in q.all_tracks_mut() {
                if let Some(l) = was.get(&t.id.0) {
                    t.sync_lock = *l;
                }
            }
            for t in q.caption_tracks.iter_mut() {
                if let Some(l) = was.get(&t.id.0) {
                    t.sync_lock = *l;
                }
            }
            set_param(q, id, "locks", ParamValue::Text(String::new()));
        }
    }
    set_param(q, id, key, ParamValue::Bool(on));
}

fn set_param(q: &mut Sequence, track: TrackId, key: &str, v: ParamValue) {
    if let Some(e) = q.track_mut(track).and_then(|t| t.effects.iter_mut().find(|e| is(&e.effect, MAIN))) {
        e.params.insert(key.to_string(), Param::new(v));
    }
}

/// Note tracks shnyp has just added and taken off the sync lock: they had no state of their own, and
/// stay off it when linkage goes off (a music bed on a sync-locked track would block every ripple).
pub fn note_unlocked(q: &mut Sequence, tracks: &[TrackId]) {
    let Some(id) = main_track_id(q) else { return };
    let mut locks = marker(q)
        .and_then(|e| match &e.param("locks")?.value {
            ParamValue::Text(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap_or_default();
    for t in tracks {
        if !locks.is_empty() {
            locks.push(',');
        }
        locks.push_str(&format!("{}=0", t.0));
    }
    set_param(q, id, "locks", ParamValue::Text(locks));
}

/// The clips that belong to `clip` (its sound, its background): linked with it by the clips' own
/// link (not by FilmCraft's Linked Selection switch, which the user may have turned off), never
/// on the main track, sound or backgrounds only, at most one per track. (FilmCraft's Link can
/// also tie two pictures together; those stay clips of their own.)
pub fn partners(q: &Sequence, clip: ClipId) -> Vec<ClipId> {
    let Some((track, link)) = q.find_item(clip).and_then(|(t, it)| Some((t, it.link?))) else { return Vec::new() };
    let main = main_track_id(q);
    // a background belongs to its main clip only (not to that clip's sound)
    let on_main = Some(track) == main;
    let mut out = Vec::new();
    for t in q.all_tracks().filter(|t| Some(t.id) != main) {
        if let Some(it) =
            t.items.iter().find(|it| it.link == Some(link) && it.id != clip && (t.kind == TrackKind::Audio || (on_main && is_background(it))))
        {
            out.push(it.id);
        }
    }
    out
}

/// The main clip a background belongs to (the clip on the main track it shares its link with).
pub fn main_of(q: &Sequence, background: ClipId) -> Option<ClipId> {
    let link = q.find_item(background).and_then(|(_, it)| it.link)?;
    main_track(q)?.items.iter().find(|it| it.link == Some(link) && !is_background(it)).map(|it| it.id)
}

/// Is the main track marked (a video shnyp has worked on)?
pub fn is_marked(q: &Sequence) -> bool {
    q.video_tracks.iter().any(|t| marked(&t.effects, MAIN))
}

/// The main track's clips (without backgrounds), in time order.
pub fn main_clips(q: &Sequence) -> Vec<&TrackItem> {
    let mut v: Vec<&TrackItem> = main_track(q).into_iter().flat_map(|t| t.items.iter()).filter(|it| !is_background(it)).collect();
    v.sort_by_key(|it| it.start);
    v
}

/// The audio track the main track's sound is on: where most of its linked partners are, else
/// the first audio track.
pub fn main_audio_track(q: &Sequence) -> Option<TrackId> {
    let main = main_track(q)?;
    let links: Vec<u64> = main.items.iter().filter_map(|it| it.link).collect();
    let best = q
        .audio_tracks
        .iter()
        .map(|t| (t.items.iter().filter(|it| it.link.is_some_and(|l| links.contains(&l))).count(), t.id))
        .filter(|(n, _)| *n > 0)
        .max_by_key(|(n, _)| *n)
        .map(|(_, id)| id);
    best.or_else(|| q.audio_tracks.first().map(|t| t.id))
}

/// A track of `kind` named like FilmCraft names new ones.
pub fn new_track(id: TrackId, kind: TrackKind, n: usize) -> Track {
    let name = match kind {
        TrackKind::Video => format!("Video {n}"),
        _ => format!("Audio {n}"),
    };
    let mut t = Track::new(id, kind, name);
    // what shnyp adds follows the main clips through shnyp's anchors, not through Premiere's sync lock
    t.sync_lock = false;
    t
}
