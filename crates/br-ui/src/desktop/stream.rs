//! Sound for video playback when BR runs on a server and is used in the browser: instead of a
//! sound card, FilmCraft's playback mixer plays into a Unix socket (`BR_AUDIO_STREAM`), from
//! which br-dateien passes it to the page (`/api/ton`), where an AudioWorklet plays it.
//!
//! The stream is raw PCM: signed 16-bit little-endian, 48 000 Hz, two channels interleaved, in
//! blocks of 10 ms. A clock thread asks the mixer for the samples that are due, as a sound card's
//! callback would, so playback keeps its pace whether or not a page listens. A listener that falls
//! behind misses blocks (the socket is not waited for): the page's latency stays short.

use std::io::{ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use filmcraft_ui_egui::AudioOut;

/// The rate the stream has.
pub const RATE: u32 = 48_000;
/// Its channels.
pub const CHANNELS: usize = 2;
/// One block.
const BLOCK: Duration = Duration::from_millis(10);
/// At most this much is made up at once after the thread was held up (more is skipped).
const MAX_CATCH_UP: u64 = RATE as u64 / 10;

type Fill = Box<dyn FnMut(&mut [f32], usize) + Send>;

/// FilmCraft's audio output into a Unix socket.
pub struct StreamOut {
    path: PathBuf,
    listeners: Arc<Mutex<Vec<UnixStream>>>,
    played: Arc<AtomicU64>,
    running: Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>,
    note: Option<String>,
    /// The socket is this one's (to remove when it ends).
    bound: bool,
}

impl StreamOut {
    /// Listen on `path` (a socket left there by an earlier BR is replaced). Pages connect any
    /// time; without one the sound goes nowhere and playback runs on.
    pub fn new(path: &Path) -> Self {
        let listeners: Arc<Mutex<Vec<UnixStream>>> = Arc::default();
        let note = match bind(path) {
            Ok(l) => {
                let list = listeners.clone();
                std::thread::Builder::new()
                    .name("br-ton".into())
                    .spawn(move || {
                        for c in l.incoming().flatten() {
                            if c.set_nonblocking(true).is_ok() {
                                list.lock().unwrap_or_else(PoisonError::into_inner).push(c);
                            }
                        }
                    })
                    .ok();
                log::info!("Ton für den Browser: {}", path.display());
                None
            }
            Err(e) => {
                log::warn!("Ton für den Browser: {}: {e}", path.display());
                Some(format!("Ton für den Browser nicht verfügbar ({e})"))
            }
        };
        let bound = note.is_none();
        StreamOut { path: path.to_path_buf(), listeners, played: Arc::default(), running: None, note, bound }
    }

    /// Where it listens.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // a socket from a BR that ended (one still running answers: then this one gives way)
    if UnixStream::connect(path).is_ok() {
        return Err(std::io::Error::new(ErrorKind::AddrInUse, "ein anderes BR spielt schon dorthin"));
    }
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(l)
}

/// Samples to the stream's format.
pub fn encode(samples: &[f32], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(samples.len() * 2);
    for s in samples {
        let v = if s.is_finite() { (s.clamp(-1.0, 1.0) * 32767.0).round() as i16 } else { 0 };
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Send a block to every listener; one that is gone is dropped, one that is behind misses it.
fn send(listeners: &Mutex<Vec<UnixStream>>, bytes: &[u8]) {
    let mut list = listeners.lock().unwrap_or_else(PoisonError::into_inner);
    list.retain_mut(|c| {
        let mut rest = bytes;
        while !rest.is_empty() {
            match c.write(rest) {
                Ok(0) => return false,
                Ok(n) => rest = &rest[n..],
                // behind: the rest of this block is missed (a whole sample frame is kept whole by
                // the page, which counts bytes)
                Err(e) if e.kind() == ErrorKind::WouldBlock => return rest.len().is_multiple_of(4) || finish(c, rest),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
        true
    });
}

/// The bytes of a sample frame begun before a listener fell behind, waited for (a few bytes), so
/// the stream stays aligned.
fn finish(c: &mut UnixStream, rest: &[u8]) -> bool {
    let tail = rest.len() % 4;
    let _ = c.set_nonblocking(false);
    let _ = c.set_write_timeout(Some(Duration::from_millis(50)));
    let ok = c.write_all(&rest[..tail]).is_ok();
    let _ = c.set_nonblocking(true);
    ok
}

impl AudioOut for StreamOut {
    fn start(&mut self, mut fill: Fill) -> Result<u32, String> {
        self.stop();
        self.played.store(0, Ordering::SeqCst);
        let run = Arc::new(AtomicBool::new(true));
        let (r, played, listeners) = (run.clone(), self.played.clone(), self.listeners.clone());
        let thread = std::thread::Builder::new()
            .name("br-ton-takt".into())
            .spawn(move || {
                let start = Instant::now();
                let mut made: u64 = 0;
                let mut buf: Vec<f32> = Vec::new();
                let mut bytes: Vec<u8> = Vec::new();
                while r.load(Ordering::SeqCst) {
                    std::thread::sleep(BLOCK);
                    let due = (start.elapsed().as_secs_f64() * RATE as f64) as u64;
                    // held up (a suspended machine): skip what is too late to play
                    if due > made + MAX_CATCH_UP {
                        made = due - MAX_CATCH_UP;
                    }
                    let n = (due - made.min(due)) as usize;
                    if n == 0 {
                        continue;
                    }
                    buf.clear();
                    buf.resize(n * CHANNELS, 0.0);
                    fill(&mut buf, CHANNELS);
                    made += n as u64;
                    played.fetch_add(n as u64, Ordering::SeqCst);
                    encode(&buf, &mut bytes);
                    send(&listeners, &bytes);
                }
            })
            .map_err(|e| e.to_string())?;
        self.running = Some((run, thread));
        Ok(RATE)
    }
    fn stop(&mut self) {
        if let Some((run, thread)) = self.running.take() {
            run.store(false, Ordering::SeqCst);
            let _ = thread.join();
        }
    }
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn channels(&self) -> usize {
        CHANNELS
    }
    fn played_frames(&self) -> Option<u64> {
        self.running.as_ref().map(|_| self.played.load(Ordering::SeqCst))
    }
    fn note(&self) -> Option<String> {
        self.note.clone()
    }
}

impl Drop for StreamOut {
    fn drop(&mut self) {
        self.stop();
        if self.bound {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn the_mixer_plays_into_the_socket_at_its_pace() {
        let dir = std::env::temp_dir().join(format!("br-ton-{}", std::process::id()));
        let path = dir.join("ton.sock");
        let mut out = StreamOut::new(&path);
        assert!(out.note().is_none(), "{:?}", out.note());
        let mut c = UnixStream::connect(out.path()).unwrap();
        // the listener is picked up by its thread
        std::thread::sleep(Duration::from_millis(50));
        // a 1 kHz tone at half volume
        let mut t = 0u64;
        out.start(Box::new(move |buf: &mut [f32], ch: usize| {
            for f in buf.chunks_mut(ch) {
                let v = 0.5 * (t as f32 * 1000.0 * std::f32::consts::TAU / RATE as f32).sin();
                f.fill(v);
                t += 1;
            }
        }))
        .unwrap();
        c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut got = vec![0u8; RATE as usize / 5 * 4];
        c.read_exact(&mut got).unwrap();
        let played = out.played_frames().unwrap();
        assert!(played >= RATE as u64 / 5, "{played}");
        let samples: Vec<i16> = got.chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((16000..=16400).contains(&peak), "half volume: {peak}");
        assert!(samples.chunks(2).all(|f| f[0] == f[1]), "both channels");
        out.stop();
        assert!(out.played_frames().is_none());
        // a second BR does not take the socket of a running one
        let other = StreamOut::new(&path);
        assert!(other.note().is_some());
        drop(other);
        assert!(path.exists(), "the running one keeps its socket");
        drop(out);
        assert!(!path.exists(), "cleaned up");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
