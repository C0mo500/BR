//! Where shnyp keeps its photos, and opening a session on them.
//!
//! shnyp uses LightCraft's library format in its own folder (`~/Pictures/shnyp Library`), so the two
//! apps never lock each other out; LightCraft can still open a shnyp library when shnyp is closed.

use std::path::{Path, PathBuf};

use lightcraft_engine::{Result, Session};

/// Folder name of the default library.
pub const DEFAULT_NAME: &str = "shnyp Library";
/// The default library's folder name while shnyp was called BR.
const OLD_NAME: &str = "BR Library";

/// shnyp's library folder: `$SHNYP_LIBRARY`, else `~/Pictures/shnyp Library` (or, while that is not
/// there, the `~/Pictures/BR Library` of the time shnyp was called BR: a library is not moved, its
/// previews and catalog stay where they are).
pub fn default_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("SHNYP_LIBRARY").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty())?;
    let pictures = PathBuf::from(home).join("Pictures");
    let (new, old) = (pictures.join(DEFAULT_NAME), pictures.join(OLD_NAME));
    Some(if !new.exists() && old.is_dir() { old } else { new })
}

/// A session that can read real files, with no library yet (in memory).
pub fn new_session() -> Session {
    Session::new().with_fs().with_default_denoise_models().with_system_clock().with_default_face_models().with_default_sam3_model()
}

/// An in-memory session with LightCraft's 24 generated demo photos.
pub fn demo_session() -> Session {
    Session::with_demo().with_fs().with_default_denoise_models().with_system_clock().with_default_face_models().with_default_sam3_model()
}

/// A session on the library in `dir` (created if missing). Fails with
/// [`lightcraft_engine::EngineError::LibraryInUse`] while another program has it open.
pub fn open(dir: &Path) -> Result<Session> {
    let mut s = new_session();
    s.open_library(dir, false)?;
    Ok(s)
}
