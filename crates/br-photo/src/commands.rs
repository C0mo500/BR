//! The `br.*` commands: BR Foto's simple editing, by id with JSON params.
//!
//! The CLI and `br mcp` dispatch them here; the app calls the same functions directly. They sit
//! next to LightCraft's own command registry (which cannot be extended at runtime), so callers
//! route ids starting with `br.` here and everything else to `Session::execute`.
//!
//! Command ids never contain `_`: LightCraft's MCP server maps `.` to `_` in tool names and back.

use std::path::PathBuf;

use lightcraft_engine::{EngineError, Result, Session};
use serde_json::{Value, json};

use crate::adjust;
use crate::crop::{self, Format};
use crate::export::{self, Preset, Target};
use crate::looks::{self, LookState};
use crate::photo::{bad, id_param, oriented_dims, target};

/// State the `br.*` commands keep between calls: the look whose amount can still be changed.
#[derive(Clone, Debug, Default)]
pub struct BrState {
    pub look: Option<LookState>,
}

/// `(id, label, params)` of every `br.*` command.
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("br.state", "Photo State (BR)", "{id?} — the simple view of a photo: slider values, format, look, undo/redo"),
    ("br.looks", "List Looks (BR)", "{} — [{id, name, group}]"),
    (
        "br.look",
        "Apply Look (BR)",
        "{look: id or name, amount?: 0..200 (percent, default 100; 0 takes it off), id?} — changing the amount of the latest look replaces it (one undo step)",
    ),
    (
        "br.adjust",
        "Adjust (BR)",
        "{light?, contrast?, color?, warmth?, sharpness?, vignette?: -100..100 (0 = unchanged), id?} — absolute slider values, one undo step",
    ),
    ("br.auto", "Auto (BR)", "{id?} — automatic exposure, contrast and tone"),
    ("br.crop", "Format (BR)", "{format: original|1:1|4:5|9:16|16:9, id?} — largest centred crop, exact ratio in whole pixels"),
    (
        "br.export",
        "Export (BR)",
        "{preset: instagram|story|square|wide|full, dir?: folder, path?: exact file, id?} — instagram 1080×1350, story 1080×1920, square 1080×1080, wide 1920×1080, full = full size; the photo's edit is not changed",
    ),
    ("br.reset", "Reset All (BR)", "{id?} — back to how the photo was imported (looks, sliders, format, Pro edits)"),
    ("br.undo", "Undo (BR)", "{}"),
    ("br.redo", "Redo (BR)", "{}"),
];

/// Is `id` one of ours?
pub fn is_br(id: &str) -> bool {
    id.starts_with("br.")
}

/// The `br.*` commands in the shape of LightCraft's `engine.commands` entries.
pub fn list(s: &Session) -> Vec<Value> {
    let photo = s.active().is_some();
    COMMANDS
        .iter()
        .map(|(id, label, params)| {
            let enabled = match *id {
                "br.looks" | "br.export" => true,
                "br.undo" => !s.undo.is_empty() || s.interaction.is_some(),
                "br.redo" => !s.redo.is_empty(),
                _ => photo,
            };
            let mut v = json!({"id": id, "label": label, "menu": ["BR"], "shortcut": null, "params": params, "enabled": enabled});
            if !enabled {
                v["disabled_reason"] = json!(if photo { "nothing to undo or redo" } else { "no photo selected (or pass `id`)" });
            }
            v
        })
        .collect()
}

fn str_param<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str)
}

/// The simple view of photo `id`.
pub fn state(s: &Session, st: &BrState, id: lightcraft_engine::catalog::PhotoId) -> Value {
    let p = s.catalog.photo(id);
    let (w, h) = oriented_dims(s, id).unwrap_or((0.0, 0.0));
    let look = looks::current(&st.look, s, id).map(|l| json!({"id": l.preset_id, "name": l.label.trim_start_matches("Look: "), "amount": l.amount}));
    let crop = s.develop_of(id).map(|d| d.crop.geometry.rect_px(w, h));
    json!({
        "id": id.0,
        "file": p.map(|p| p.file_name.clone()),
        "width": w,
        "height": h,
        "crop": crop.map(|r| json!({"width": r.width().round(), "height": r.height().round()})),
        "format": crop::current(s, id).map(Format::key),
        "look": look,
        "sliders": adjust::values_json(s, id),
        "edited": p.is_some_and(|p| p.edited.is_some()),
        "undo": s.undo.last().map(|e| e.label.clone()),
        "redo": s.redo.last().map(|e| e.label.clone()),
    })
}

/// Run `br.*` command `id` with params `p`.
pub fn execute(s: &mut Session, st: &mut BrState, id: &str, p: &Value) -> Result<Value> {
    let p = if p.is_null() { &json!({}) } else { p };
    match id {
        "br.state" => {
            let photo = target(s, id, id_param(p))?;
            Ok(state(s, st, photo))
        }
        "br.looks" => Ok(looks::list_json(s)),
        "br.look" => {
            let photo = target(s, id, id_param(p))?;
            let look = str_param(p, "look").ok_or_else(|| bad(id, "missing `look` (see br.looks)"))?;
            let amount = match p.get("amount") {
                None | Some(Value::Null) => looks::current(&st.look, s, photo)
                    .filter(|l| looks::find(s, look).is_some_and(|f| f.id == l.preset_id))
                    .map_or(100.0, |l| l.amount),
                Some(v) => v.as_f64().ok_or_else(|| bad(id, "`amount` must be a number"))?,
            };
            looks::set_look(s, &mut st.look, photo, look, amount, false)
        }
        "br.adjust" => {
            let photo = target(s, id, id_param(p))?;
            let changes = adjust::parse_changes(p)?;
            adjust::set(s, photo, &changes)
        }
        "br.auto" => {
            let photo = target(s, id, id_param(p))?;
            adjust::auto(s, photo)
        }
        "br.crop" => {
            let photo = target(s, id, id_param(p))?;
            let f = str_param(p, "format").ok_or_else(|| bad(id, "missing `format`: original, 1:1, 4:5, 9:16 or 16:9"))?;
            let f = Format::parse(f).ok_or_else(|| bad(id, format!("unknown format `{f}`: use original, 1:1, 4:5, 9:16 or 16:9")))?;
            crop::apply(s, photo, f)
        }
        "br.export" => {
            let photo = target(s, id, id_param(p))?;
            let name = str_param(p, "preset").unwrap_or("full");
            let preset =
                Preset::parse(name).ok_or_else(|| bad(id, format!("unknown preset `{name}`: use instagram, story, square, wide or full")))?;
            let to = match (str_param(p, "path"), str_param(p, "dir")) {
                (Some(_), Some(_)) => return Err(bad(id, "give `path` or `dir`, not both")),
                (Some(path), None) => Target::File(PathBuf::from(path)),
                (None, Some(dir)) => Target::Dir(PathBuf::from(dir)),
                (None, None) => Target::Dir(export::default_dir()),
            };
            let files = export::export(s, photo, preset, &to).map_err(|e| bad(id, e))?;
            Ok(json!({"id": photo.0, "preset": preset.key(), "files": files}))
        }
        "br.reset" => {
            let photo = target(s, id, id_param(p))?;
            st.look = None;
            s.execute_fn(id, |s| {
                s.execute("develop.reset", &json!({"ids": [photo.0]}))?;
                Ok(json!({"id": photo.0}))
            })
        }
        "br.undo" => s.execute("edit.undo", &json!({})),
        "br.redo" => s.execute("edit.redo", &json!({})),
        other => Err(EngineError::UnknownCommand(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_never_contain_underscores() {
        for (id, _, _) in COMMANDS {
            assert!(is_br(id) && !id.contains('_'), "{id}");
        }
    }

    #[test]
    fn a_simple_edit_session() {
        let mut s = Session::with_demo();
        let mut st = BrState::default();
        let id = s.active().unwrap().0;
        let r = execute(&mut s, &mut st, "br.adjust", &json!({"light": 30, "Wärme": -20})).unwrap();
        assert_eq!(r["sliders"]["light"], 30.0);
        assert_eq!(r["sliders"]["warmth"], -20.0);
        assert_eq!(s.undo.last().unwrap().label, "Anpassen");
        execute(&mut s, &mut st, "br.look", &json!({"look": "Golden Hour", "amount": 60})).unwrap();
        let state = execute(&mut s, &mut st, "br.state", &json!({})).unwrap();
        assert_eq!(state["look"]["id"], "lc.golden-hour");
        assert_eq!(state["look"]["amount"], 60.0);
        // a new amount replaces the look's step
        let undo = s.undo.len();
        execute(&mut s, &mut st, "br.look", &json!({"look": "lc.golden-hour", "amount": 20})).unwrap();
        assert_eq!(s.undo.len(), undo);
        assert_eq!(s.undo.last().unwrap().label, "Look: Golden Hour");
        let format0 = execute(&mut s, &mut st, "br.state", &json!({})).unwrap()["format"].clone();
        execute(&mut s, &mut st, "br.crop", &json!({"format": "4:5"})).unwrap();
        let state = execute(&mut s, &mut st, "br.state", &json!({"id": id})).unwrap();
        assert_eq!(state["format"], "4:5");
        assert_eq!(state["undo"], "Format 4:5");
        assert!(state["look"].is_null(), "the format edit made the look final");
        execute(&mut s, &mut st, "br.undo", &json!({})).unwrap();
        assert_eq!(execute(&mut s, &mut st, "br.state", &json!({})).unwrap()["format"], format0);
        execute(&mut s, &mut st, "br.reset", &json!({})).unwrap();
        assert_eq!(execute(&mut s, &mut st, "br.state", &json!({})).unwrap()["sliders"]["light"], 0.0);
    }

    #[test]
    fn bad_params_are_reported() {
        let mut s = Session::with_demo();
        let mut st = BrState::default();
        for (id, p) in [
            ("br.crop", json!({"format": "3:2"})),
            ("br.look", json!({"look": "nope"})),
            ("br.look", json!({"look": "lc.moody", "amount": 500})),
            ("br.adjust", json!({"light": "hell"})),
            ("br.adjust", json!({})),
            ("br.export", json!({"preset": "poster"})),
            ("br.state", json!({"id": 999999})),
        ] {
            assert!(execute(&mut s, &mut st, id, &p).is_err(), "{id} {p}");
        }
        assert!(matches!(execute(&mut s, &mut st, "br.nope", &json!({})), Err(EngineError::UnknownCommand(_))));
    }
}
