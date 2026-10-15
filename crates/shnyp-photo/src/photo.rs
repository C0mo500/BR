//! Picking the photo a command works on, and its size as the crop sees it.

use lightcraft_engine::catalog::PhotoId;
use lightcraft_engine::{EngineError, Result, Session};
use serde_json::{Value, json};

pub(crate) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The photo a command is about: `id` when given (it must exist), else the active photo.
pub fn target(s: &Session, cmd: &str, id: Option<u64>) -> Result<PhotoId> {
    match id {
        Some(id) => {
            let id = PhotoId(id);
            if s.catalog.photo(id).is_none() {
                return Err(bad(cmd, format!("no such photo {}", id.0)));
            }
            Ok(id)
        }
        None => s.active().ok_or_else(|| bad(cmd, "no photo selected: pass `id` or select one")),
    }
}

/// Make `id` the active (and only selected) photo, unless it already is. Commands such as
/// `develop.auto` and engine interactions only work on the active photo.
pub fn activate(s: &mut Session, id: PhotoId) -> Result<()> {
    if s.active() == Some(id) {
        return Ok(());
    }
    s.execute("library.select", &json!({"ids": [id.0], "active": id.0})).map(|_| ())
}

/// `id` param of a command: absent or null means the active photo; anything else must be a photo
/// id (a whole number), so a mistyped id never edits the wrong photo.
pub fn id_param(cmd: &str, p: &Value) -> Result<Option<u64>> {
    match p.get("id") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| bad(cmd, format!("`id` must be a photo id (a whole number), got {v}"))),
    }
}

/// A text param. Numbers and booleans are taken as their text (`dir=2024` on the command line);
/// anything else is an error.
pub fn text_param(cmd: &str, p: &Value, key: &str) -> Result<Option<String>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(v @ (Value::Number(_) | Value::Bool(_))) => Ok(Some(v.to_string())),
        Some(v) => Err(bad(cmd, format!("`{key}` must be text, got {v}"))),
    }
}

/// The photo's pixel size after its orientation (rotation) is applied: the frame its crop lives in.
pub fn oriented_dims(s: &Session, id: PhotoId) -> Option<(f64, f64)> {
    let p = s.catalog.photo(id)?;
    let (w, h) = (p.width.max(1) as f64, p.height.max(1) as f64);
    Some(if p.develop.orientation.swaps_axes() { (h, w) } else { (w, h) })
}
