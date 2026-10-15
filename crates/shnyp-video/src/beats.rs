//! Beat markers, CapCut's "Beats": the beats of the music (or of any sound clip) as markers on
//! the video's timeline, which shnyp's and FilmCraft's timelines snap to. FilmCraft's Remix analysis
//! finds the beat grid but reports only the tempo, so shnyp runs it itself and writes sequence
//! markers (comment `shnyp:beat`); they do not move when the main track ripples, they stay with the
//! music.

use filmcraft_engine::{EngineError, Session};
use filmcraft_project::{AudioType, ClipId, Label, Marker, MarkerId, MarkerKind, Sequence, TrackItem};
use filmcraft_time::Tick;
use serde_json::{Value, json};

type R<T> = filmcraft_engine::Result<T>;

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

/// What shnyp's beat markers say.
pub const BEAT: &str = "shnyp:beat";

/// The sound clip beats are found in: `clip`, else a selected sound clip, else the music, else
/// the first sound clip of its own.
fn sound_clip(s: &Session, p: &Value) -> Option<ClipId> {
    let q = s.active_sequence()?;
    let on_audio = |c: ClipId| q.audio_tracks.iter().any(|t| t.item(c).is_some());
    if let Some(c) = p.get("clip").and_then(Value::as_u64).map(ClipId) {
        // a picture's own sound
        if on_audio(c) {
            return Some(c);
        }
        let link = q.find_item(c).and_then(|(_, it)| it.link)?;
        return q.audio_tracks.iter().flat_map(|t| &t.items).find(|it| it.link == Some(link)).map(|it| it.id);
    }
    if let Some(c) = s.state.selection.iter().copied().find(|c| on_audio(*c)) {
        return Some(c);
    }
    let all: Vec<&TrackItem> = q.audio_tracks.iter().flat_map(|t| &t.items).collect();
    all.iter()
        .find(|it| it.essential.as_ref().is_some_and(|e| e.kind == AudioType::Music))
        .or_else(|| all.iter().find(|it| it.link.is_none()))
        .map(|it| it.id)
}

/// The beats of a sound clip, on the timeline: (tempo, times). `mode`: beats, bars (every 4th),
/// half (and between), onsets (every hit; also when there is no steady beat).
pub fn detect(s: &Session, clip: ClipId, mode: &str) -> R<(f64, Vec<Tick>)> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(clip).ok_or_else(|| other("no such clip"))?;
    let sr = q.settings.sample_rate.max(1);
    let m0 = it.source_in.to_units_floor(sr as i64);
    let len = it.source_out().to_units_floor(sr as i64) - m0;
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let src = filmcraft_render::SourceProvider::source(&provider, it.item).ok_or_else(|| other("the clip's media is offline"))?;
    let buf = src.audio_stream(it.audio_stream, m0, len.max(0) as usize, sr).map_err(|e| other(format!("reading the sound: {e}")))?;
    let refs: Vec<&[f32]> = buf.channels.iter().map(Vec::as_slice).collect();
    let lead = filmcraft_audio_dsp::remix::beat_lead(sr);
    // media samples of the beats (or hits)
    let (bpm, mut hits): (f64, Vec<i64>) = match (mode, filmcraft_audio_dsp::remix::analyze(&refs, sr)) {
        ("onsets", _) | (_, Err(_)) => (0.0, onsets(&refs, sr)),
        (_, Ok(a)) => {
            let b: Vec<i64> = a.beats.iter().map(|b| b + lead).collect();
            let picked = match mode {
                "bars" => b.iter().copied().step_by(4).collect(),
                "half" => {
                    let mut v = Vec::new();
                    for w in b.windows(2) {
                        v.push(w[0]);
                        v.push((w[0] + w[1]) / 2);
                    }
                    v.extend(b.last());
                    v
                }
                _ => b,
            };
            (a.bpm, picked)
        }
    };
    hits.retain(|h| *h >= 0 && *h < len);
    // media time → the timeline (forwards or backwards, at the clip's speed)
    let speed = it.speed.abs().max(1e-9);
    let rate = q.settings.frame_rate;
    let mut out: Vec<Tick> = hits
        .into_iter()
        .map(|h| {
            let media = Tick::from_units(h, sr as i64);
            let rel = if it.reverse { (it.source_out() - it.source_in - media).0 as f64 / speed } else { media.0 as f64 / speed };
            rate.snap_nearest(it.start + Tick(rel.round() as i64))
        })
        .filter(|t| *t >= it.start && *t < it.end())
        .collect();
    out.sort();
    out.dedup();
    Ok((bpm, out))
}

/// Peaks of the onset envelope: every hit, at least 0.15 s apart.
fn onsets(channels: &[&[f32]], sr: u32) -> Vec<i64> {
    let len = channels.iter().map(|c| c.len()).max().unwrap_or(0);
    let k = 1.0 / channels.len().max(1) as f32;
    let mono: Vec<f32> = (0..len).map(|i| channels.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() * k).collect();
    let (env, _, hop) = filmcraft_audio_dsp::remix::onset_envelope(&mono, sr);
    if env.len() < 3 {
        return Vec::new();
    }
    let mean = env.iter().sum::<f32>() / env.len() as f32;
    let sd = (env.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / env.len() as f32).sqrt();
    let gap = (0.15 * sr as f64 / hop as f64).ceil() as usize;
    let mut out: Vec<i64> = Vec::new();
    let mut last: Option<usize> = None;
    for i in 1..env.len() - 1 {
        if env[i] > mean + sd && env[i] > env[i - 1] && env[i] >= env[i + 1] && last.is_none_or(|l| i - l >= gap) {
            out.push((i * hop) as i64);
            last = Some(i);
        }
    }
    out
}

fn is_beat(m: &Marker) -> bool {
    // `br:beat`: set while shnyp was called BR
    m.comment == BEAT || m.comment == "br:beat"
}

/// The beat markers of the video, in seconds.
pub fn markers(q: &Sequence) -> Vec<f64> {
    q.markers.iter().filter(|m| is_beat(m)).map(|m| m.start.seconds()).collect()
}

/// `shnyp.video.beats`: beat markers for a sound clip (replacing the ones in its range), or none
/// (`mode: none` takes them all away).
pub fn beats(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("beats");
    if !matches!(mode, "beats" | "bars" | "half" | "onsets" | "none") {
        return Err(format!("unknown mode `{mode}`: beats, bars, half, onsets or none"));
    }
    if s.active_sequence().is_none() {
        return Err("no video yet".into());
    }
    if mode == "none" {
        let n = s
            .edit_sequence("Beats entfernen", |q, _, _| {
                let before = q.markers.len();
                q.markers.retain(|m| !is_beat(m));
                Ok(before - q.markers.len())
            })
            .map_err(|e| e.to_string())?;
        return Ok(json!({"removed": n}));
    }
    let clip = sound_clip(s, p).ok_or("no music or sound clip to find beats in")?;
    let (bpm, times) = detect(s, clip, mode).map_err(|e| e.to_string())?;
    let (start, end) = s.active_sequence().and_then(|q| q.find_item(clip)).map(|(_, it)| (it.start, it.end())).ok_or("no such clip")?;
    let n = times.len();
    s.edit_sequence("Beats", |q, ctx, _| {
        q.markers.retain(|m| !(is_beat(m) && m.start >= start && m.start < end));
        for t in &times {
            q.markers.push(Marker {
                id: MarkerId(ctx.alloc()),
                start: *t,
                duration: Tick::ZERO,
                name: String::new(),
                comment: BEAT.into(),
                kind: MarkerKind::Comment,
                color: Label::Yellow,
            });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    Ok(json!({"clip": clip.0, "bpm": (bpm * 10.0).round() / 10.0, "count": n, "seconds": times.iter().map(|t| t.seconds()).collect::<Vec<_>>()}))
}
