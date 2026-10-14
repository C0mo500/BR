//! Where BR keeps its photos, and opening a session on them.
//!
//! BR uses LightCraft's library format in its own folder (`~/Pictures/BR Library`), so the two
//! apps never lock each other out; LightCraft can still open a BR library when BR is closed.

use std::path::{Path, PathBuf};

use lightcraft_engine::{Result, Session};

/// Folder name of the default library.
pub const DEFAULT_NAME: &str = "BR Library";

/// BR's library folder: `$BR_LIBRARY`, else `~/Pictures/BR Library`.
pub fn default_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("BR_LIBRARY").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join("Pictures").join(DEFAULT_NAME))
}

/// A session that can read real files, with no library yet (in memory).
pub fn new_session() -> Session {
    Session::new()
        .with_fs()
        .with_default_denoise_models()
        .with_system_clock()
        .with_default_face_models()
        .with_default_sam3_model()
}

/// An in-memory session with LightCraft's 24 generated demo photos.
pub fn demo_session() -> Session {
    Session::with_demo()
        .with_fs()
        .with_default_denoise_models()
        .with_system_clock()
        .with_default_face_models()
        .with_default_sam3_model()
}

/// A session on the library in `dir` (created if missing). Fails with
/// [`lightcraft_engine::EngineError::LibraryInUse`] while another program has it open.
pub fn open(dir: &Path) -> Result<Session> {
    let mut s = new_session();
    s.open_library(dir, false)?;
    Ok(s)
}
