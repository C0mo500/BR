//! Automatic captions, CapCut's "Auto captions": speech recognition (Whisper, FilmCraft's speech
//! crate) on a background thread, then FilmCraft's transcript and a caption track as one undo step.
//!
//! FilmCraft's own `transcript.generate` runs inside the command and holds the window until it
//! is done (minutes for a long video on the CPU). Here only the inputs are gathered on the UI
//! thread: each media item's sound source (thread-safe) and the recogniser. Decoding the sound,
//! downloading the model the first time, and recognising run on their own thread with progress
//! and a cancel; [`apply`] then puts the result in the project.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use filmcraft_engine::Session;
use filmcraft_media::SharedSource;
use filmcraft_project::{ItemId, ItemKind, Transcript};
use filmcraft_speech::{Options, Transcriber};
use serde_json::{Value, json};

/// What to recognise with.
#[derive(Clone, Debug, Default)]
pub struct CaptionOptions {
    /// The spoken language (`de`, `en`, …); `None` or `auto`: detected.
    pub language: Option<String>,
    /// A model of FilmCraft's catalogue (`whisper-tiny`, `whisper-base`, `whisper-small`);
    /// `None`: the one chosen in FilmCraft's settings.
    pub model: Option<String>,
}

/// One media item to recognise.
struct Input {
    item: ItemId,
    name: String,
    source: SharedSource,
    /// Its length in samples at 16 kHz.
    samples: usize,
}

/// Where the recogniser comes from.
enum Recogniser {
    /// One a host or test installed (`Session::transcriber`).
    Installed(Arc<dyn Transcriber>),
    /// A catalogue model in FilmCraft's model folder, downloaded first when it is missing.
    Model { dir: PathBuf, id: String },
}

/// The recognised words, per media item.
pub type Recognised = Vec<(ItemId, Transcript)>;

/// Recognition in progress.
pub struct CaptionJob {
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<(f32, String)>>,
    answer: Receiver<Result<Recognised, String>>,
    /// The sequence the captions are for.
    pub sequence: ItemId,
}

impl CaptionJob {
    /// How far it is (0..1) and what it is doing, in words.
    pub fn progress(&self) -> (f32, String) {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Stop as soon as possible; the answer is then an error.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The answer, once there is one (`None` while it runs).
    pub fn poll(&self) -> Option<Result<Recognised, String>> {
        match self.answer.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("die Spracherkennung wurde unerwartet beendet".into())),
        }
    }

    /// Wait for the answer.
    pub fn wait(self) -> Result<Recognised, String> {
        self.answer.recv().unwrap_or_else(|_| Err("die Spracherkennung wurde unerwartet beendet".into()))
    }
}

/// Whether this build can recognise speech (a model can be loaded or one is installed).
pub fn available(s: &Session) -> bool {
    s.transcriber.is_some() || filmcraft_speech::available()
}

/// The speech model FilmCraft's settings name is on this computer (or a recogniser is installed):
/// no download before the first recognition.
pub fn model_ready(s: &Session) -> bool {
    if s.transcriber.is_some() {
        return true;
    }
    let id = &s.prefs.media_analysis.whisper_model;
    match (filmcraft_engine::transcript::models_dir(), filmcraft_speech::models::find(id)) {
        (Some(dir), Some(m)) => filmcraft_speech::models::installed(&dir, m),
        _ => false,
    }
}

/// The media items behind the active sequence's sound: every enabled clip on an audio track,
/// subclips resolved to their media, each once.
fn targets(s: &Session) -> Vec<ItemId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let mut out = Vec::new();
    for it in q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|it| it.enabled) {
        if let Some((m, _, _)) = s.project.resolve_media(it.item)
            && !out.contains(&m)
        {
            out.push(m);
        }
    }
    out
}

fn inputs(s: &Session) -> Vec<Input> {
    let rate = filmcraft_speech::SAMPLE_RATE as i64;
    targets(s)
        .into_iter()
        .filter_map(|item| {
            let pi = s.project.item(item)?;
            let ItemKind::Media(m) = &pi.kind else { return None };
            let source = s.source(item)?;
            if !source.info().has_audio() {
                return None;
            }
            let samples = m.duration().to_units_floor(rate).max(0) as usize;
            Some(Input { item, name: pi.name.clone(), source, samples })
        })
        .collect()
}

fn recogniser(s: &Session, opts: &CaptionOptions) -> Result<Recogniser, String> {
    if let Some(t) = &s.transcriber {
        return Ok(Recogniser::Installed(t.clone()));
    }
    if !filmcraft_speech::available() {
        return Err("Spracherkennung ist in diesem BR nicht enthalten (ohne die Funktion `whisper` gebaut)".into());
    }
    let id = opts.model.clone().unwrap_or_else(|| s.prefs.media_analysis.whisper_model.clone());
    if filmcraft_speech::models::find(&id).is_none() {
        let known: Vec<&str> = filmcraft_speech::models::catalogue().iter().map(|m| m.id).collect();
        return Err(format!("unbekanntes Sprachmodell `{id}`: {}", known.join(", ")));
    }
    let dir = filmcraft_engine::transcript::models_dir().ok_or("kein Datenordner für Sprachmodelle")?;
    Ok(Recogniser::Model { dir, id })
}

/// Start recognising the speech of the active sequence.
pub fn start(s: &Session, opts: &CaptionOptions) -> Result<CaptionJob, String> {
    let sequence = s.state.active_sequence.ok_or("noch kein Video: erst Medien hinzufügen")?;
    let inputs = inputs(s);
    if inputs.is_empty() {
        return Err("das Video hat keinen Ton, aus dem sich Untertitel erkennen ließen".into());
    }
    let recogniser = recogniser(s, opts)?;
    let language = opts.language.clone().filter(|l| !l.is_empty() && l != "auto");
    let speech_opts = Options { language, diarize: false, ..Options::default() };
    let cancel = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new((0.0, "Spracherkennung startet …".to_string())));
    let (tx, answer) = std::sync::mpsc::channel();
    let (c, st) = (cancel.clone(), status.clone());
    std::thread::Builder::new()
        .name("br-captions".into())
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| recognise(inputs, recogniser, &speech_opts, &c, &st)))
                .unwrap_or_else(|_| Err("die Spracherkennung ist abgestürzt (Details im Protokoll)".into()));
            let _ = tx.send(r);
        })
        .map_err(|e| format!("Spracherkennung: {e}"))?;
    Ok(CaptionJob { cancel, status, answer, sequence })
}

fn set(status: &Mutex<(f32, String)>, fraction: f32, text: String) {
    *status.lock().unwrap_or_else(|e| e.into_inner()) = (fraction.clamp(0.0, 1.0), text);
}

#[cfg(feature = "whisper")]
fn mb(bytes: u64) -> u64 {
    bytes.div_ceil(1_000_000)
}

/// Download a catalogue model into FilmCraft's model folder (the first tenth of the bar).
#[cfg(feature = "whisper")]
fn download(
    dir: &std::path::Path,
    m: &filmcraft_speech::models::ModelInfo,
    cancel: &AtomicBool,
    status: &Mutex<(f32, String)>,
) -> Result<(), String> {
    set(status, 0.0, format!("Sprachmodell {} wird geladen …", m.name));
    let mut progress = |done: u64, total: u64, _: &str| {
        let f = if total > 0 { done as f32 / total as f32 } else { 0.0 };
        set(status, 0.1 * f, format!("Sprachmodell {} wird geladen ({} / {} MB) …", m.name, mb(done), mb(total)));
        !cancel.load(Ordering::Relaxed)
    };
    filmcraft_speech::models::download(dir, m, &mut progress)
        .map_err(|e| if cancel.load(Ordering::Relaxed) { "abgebrochen".to_string() } else { format!("Sprachmodell laden: {e}") })
}

#[cfg(not(feature = "whisper"))]
fn download(dir: &std::path::Path, m: &filmcraft_speech::models::ModelInfo, _: &AtomicBool, _: &Mutex<(f32, String)>) -> Result<(), String> {
    Err(format!("das Sprachmodell {} fehlt in {}", m.name, dir.display()))
}

/// The background half: model, sound, words.
fn recognise(
    inputs: Vec<Input>,
    recogniser: Recogniser,
    opts: &Options,
    cancel: &AtomicBool,
    status: &Mutex<(f32, String)>,
) -> Result<Recognised, String> {
    let cancelled = || Err::<Recognised, String>("abgebrochen".into());
    // the first tenth of the bar is the model (a download the first time takes longer, and says so)
    let t: Arc<dyn Transcriber> = match recogniser {
        Recogniser::Installed(t) => t,
        Recogniser::Model { dir, id } => {
            let m = filmcraft_speech::models::find(&id).ok_or_else(|| format!("unbekanntes Sprachmodell `{id}`"))?;
            if !filmcraft_speech::models::installed(&dir, m) {
                download(&dir, m, cancel, status)?;
            }
            if cancel.load(Ordering::Relaxed) {
                return cancelled();
            }
            set(status, 0.1, format!("Sprachmodell {} wird geöffnet …", m.name));
            filmcraft_speech::load(&dir, &id).map_err(|e| format!("Sprachmodell öffnen: {e}"))?
        }
    };
    let n = inputs.len() as f32;
    let mut out = Vec::new();
    for (i, input) in inputs.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return cancelled();
        }
        let base = 0.1 + 0.9 * i as f32 / n;
        let which = if n > 1.0 { format!(" ({} von {})", i + 1, n as usize) } else { String::new() };
        set(status, base, format!("Ton von „{}“ wird gelesen{which} …", input.name));
        let buf = match input.source.audio(0, input.samples, filmcraft_speech::SAMPLE_RATE) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("captions: no sound from {}: {e}", input.name);
                continue;
            }
        };
        let audio = filmcraft_speech::downmix(&buf.channels);
        drop(buf);
        let mut progress = |f: f32, _: &str| {
            set(status, base + 0.9 / n * f, format!("Sprache wird erkannt{which}: {} %", (f * 100.0).round()));
            !cancel.load(Ordering::Relaxed)
        };
        let mut tr = t
            .transcribe(&audio, opts, &mut progress)
            .map_err(|e| if cancel.load(Ordering::Relaxed) { "abgebrochen".to_string() } else { format!("Spracherkennung: {e}") })?;
        tr.normalize();
        out.push((input.item, tr));
    }
    if cancel.load(Ordering::Relaxed) {
        return cancelled();
    }
    if out.iter().all(|(_, t)| t.words.is_empty()) {
        return Err("im Ton wurde keine Sprache erkannt".into());
    }
    set(status, 1.0, "Untertitel werden angelegt …".into());
    Ok(out)
}

/// Put recognised words into the project: each media item's transcript and, from the sequence's
/// words, a caption track (FilmCraft's caption rules), one undo step "Auto-Untertitel". Runs on
/// the sequence the recognition was started for, if it is still the active one.
pub fn apply(s: &mut Session, sequence: ItemId, done: Recognised) -> Result<Value, String> {
    if s.state.active_sequence != Some(sequence) {
        return Err("das Video wurde während der Erkennung gewechselt: bitte noch einmal".into());
    }
    s.edit_group("Auto-Untertitel", |s| {
        for (item, t) in done {
            let transcript = serde_json::to_value(t).map_err(|e| filmcraft_engine::EngineError::Other(e.to_string()))?;
            s.execute("transcript.set", json!({"item": item.0, "transcript": transcript}))?;
        }
        s.execute("transcript.createCaptions", json!({"name": "Untertitel"}))
    })
    .map_err(|e| e.to_string())
}
