//! The `shnyp.*` commands: shnyp Foto's simple editing, by id with JSON params.
//!
//! The CLI and `shnyp mcp` dispatch them here; the app calls the same functions directly. They sit
//! next to LightCraft's own command registry (which cannot be extended at runtime), so callers
//! route ids starting with `shnyp.` here and everything else to `Session::execute`.
//!
//! Command ids never contain `_`: LightCraft's MCP server maps `.` to `_` in tool names and back.

use std::path::PathBuf;

use lightcraft_engine::{EngineError, Result, Session};
use serde_json::{Value, json};

use crate::adjust;
use crate::crop::{self, Format};
use crate::export::{self, Preset, Target};
use crate::looks::{self, LookState};
use crate::photo::{activate, bad, id_param, oriented_dims, target, text_param};

/// State the `shnyp.*` commands keep between calls: the look whose amount can still be changed.
#[derive(Clone, Debug, Default)]
pub struct ShnypState {
    pub look: Option<LookState>,
}

/// `(id, label, params)` of every `shnyp.*` command.
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("shnyp.state", "Photo State (shnyp)", "{id?} — the simple view of a photo: slider values, format, look, undo/redo"),
    ("shnyp.looks", "List Looks (shnyp)", "{} — [{id, name, group}]"),
    (
        "shnyp.look",
        "Apply Look (shnyp)",
        "{look: id or name, amount?: 0..200 (percent, default 100; 0 takes it off), id?} — replaces the photo's latest look if nothing was edited since (one undo step); after another edit it goes on top",
    ),
    (
        "shnyp.adjust",
        "Adjust (shnyp)",
        "{light?, contrast?, color?, warmth?, sharpness?, vignette?: -100..100 (0 = unchanged), id?} — absolute slider values, one undo step",
    ),
    ("shnyp.auto", "Auto (shnyp)", "{id?} — automatic exposure, contrast and tone"),
    ("shnyp.crop", "Format (shnyp)", "{format: original|1:1|4:5|9:16|16:9, id?} — largest centred crop, exact ratio in whole pixels"),
    (
        "shnyp.export",
        "Export (shnyp)",
        "{preset: instagram|story|square|wide|full, dir?: folder, path?: exact .jpg file, id?} — instagram 1080×1350, story 1080×1920, square 1080×1080, wide 1920×1080, full = full size; the photo's edit is not changed",
    ),
    ("shnyp.reset", "Reset All (shnyp)", "{id?} — back to how the photo was imported (looks, sliders, format, Pro edits)"),
    ("shnyp.undo", "Undo (shnyp)", "{}"),
    ("shnyp.redo", "Redo (shnyp)", "{}"),
];

/// Is `id` one of ours?
pub fn is_shnyp(id: &str) -> bool {
    id.starts_with("shnyp.")
}

/// The `shnyp.*` commands in the shape of LightCraft's `engine.commands` entries.
pub fn list(s: &Session) -> Vec<Value> {
    let photo = s.active().is_some();
    COMMANDS
        .iter()
        .map(|(id, label, params)| {
            let enabled = match *id {
                "shnyp.looks" | "shnyp.export" => true,
                "shnyp.undo" => !s.undo.is_empty() || s.interaction.is_some(),
                "shnyp.redo" => !s.redo.is_empty(),
                _ => photo,
            };
            let mut v = json!({"id": id, "label": label, "menu": ["shnyp"], "shortcut": null, "params": params, "enabled": enabled});
            if !enabled {
                v["disabled_reason"] = json!(if photo { "nothing to undo or redo" } else { "no photo selected (or pass `id`)" });
            }
            v
        })
        .collect()
}

/// The simple view of photo `id`.
pub fn state(s: &Session, st: &ShnypState, id: lightcraft_engine::catalog::PhotoId) -> Value {
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

/// Run `shnyp.*` command `id` with params `p`.
pub fn execute(s: &mut Session, st: &mut ShnypState, id: &str, p: &Value) -> Result<Value> {
    let p = if p.is_null() { &json!({}) } else { p };
    match id {
        "shnyp.state" => {
            let photo = target(s, id, id_param(id, p)?)?;
            Ok(state(s, st, photo))
        }
        "shnyp.looks" => Ok(looks::list_json(s)),
        "shnyp.look" => {
            let photo = target(s, id, id_param(id, p)?)?;
            let look = text_param(id, p, "look")?.ok_or_else(|| bad(id, "missing `look` (see shnyp.looks)"))?;
            let look = look.as_str();
            let amount = match p.get("amount") {
                None | Some(Value::Null) => looks::current(&st.look, s, photo)
                    .filter(|l| looks::find(s, look).is_some_and(|f| f.id == l.preset_id))
                    .map_or(100.0, |l| l.amount),
                Some(v) => v.as_f64().ok_or_else(|| bad(id, "`amount` must be a number"))?,
            };
            looks::set_look(s, &mut st.look, photo, look, amount, false)
        }
        "shnyp.adjust" => {
            let photo = target(s, id, id_param(id, p)?)?;
            let changes = adjust::parse_changes(p)?;
            adjust::set(s, photo, &changes)
        }
        "shnyp.auto" => {
            let photo = target(s, id, id_param(id, p)?)?;
            adjust::auto(s, photo)
        }
        "shnyp.crop" => {
            let photo = target(s, id, id_param(id, p)?)?;
            let f = text_param(id, p, "format")?.ok_or_else(|| bad(id, "missing `format`: original, 1:1, 4:5, 9:16 or 16:9"))?;
            let f = f.as_str();
            let f = Format::parse(f).ok_or_else(|| bad(id, format!("unknown format `{f}`: use original, 1:1, 4:5, 9:16 or 16:9")))?;
            crop::apply(s, photo, f)
        }
        "shnyp.export" => {
            let photo = target(s, id, id_param(id, p)?)?;
            let name = text_param(id, p, "preset")?.unwrap_or_else(|| "full".into());
            let name = name.as_str();
            let preset =
                Preset::parse(name).ok_or_else(|| bad(id, format!("unknown preset `{name}`: use instagram, story, square, wide or full")))?;
            let to = match (text_param(id, p, "path")?, text_param(id, p, "dir")?) {
                (Some(_), Some(_)) => return Err(bad(id, "give `path` or `dir`, not both")),
                (Some(path), None) => Target::File(PathBuf::from(path)),
                (None, Some(dir)) => Target::Dir(PathBuf::from(dir)),
                (None, None) => Target::Dir(export::default_dir()),
            };
            let files = export::export(s, photo, preset, &to).map_err(|e| bad(id, e))?;
            Ok(json!({"id": photo.0, "preset": preset.key(), "files": files}))
        }
        "shnyp.reset" => {
            let photo = target(s, id, id_param(id, p)?)?;
            let r = s.execute_fn(id, |s| {
                // develop.reset needs a selection even when it is given the photo
                if s.selection.ids.is_empty() && s.selection.active.is_none() {
                    activate(s, photo)?;
                }
                s.execute("develop.reset", &json!({"ids": [photo.0]}))?;
                Ok(json!({"id": photo.0}))
            })?;
            st.look = None;
            Ok(r)
        }
        "shnyp.undo" => s.execute("edit.undo", &json!({})),
        "shnyp.redo" => s.execute("edit.redo", &json!({})),
        other => Err(EngineError::UnknownCommand(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_never_contain_underscores() {
        for (id, _, _) in COMMANDS {
            assert!(is_shnyp(id) && !id.contains('_'), "{id}");
        }
    }

    #[test]
    fn a_simple_edit_session() {
        let mut s = Session::with_demo();
        let mut st = ShnypState::default();
        let id = s.active().unwrap().0;
        let r = execute(&mut s, &mut st, "shnyp.adjust", &json!({"light": 30, "Wärme": -20})).unwrap();
        assert_eq!(r["sliders"]["light"], 30.0);
        assert_eq!(r["sliders"]["warmth"], -20.0);
        assert_eq!(s.undo.last().unwrap().label, "Anpassen");
        execute(&mut s, &mut st, "shnyp.look", &json!({"look": "Golden Hour", "amount": 60})).unwrap();
        let state = execute(&mut s, &mut st, "shnyp.state", &json!({})).unwrap();
        assert_eq!(state["look"]["id"], "lc.golden-hour");
        assert_eq!(state["look"]["amount"], 60.0);
        // a new amount replaces the look's step
        let undo = s.undo.len();
        execute(&mut s, &mut st, "shnyp.look", &json!({"look": "lc.golden-hour", "amount": 20})).unwrap();
        assert_eq!(s.undo.len(), undo);
        assert_eq!(s.undo.last().unwrap().label, "Look: Golden Hour");
        let format0 = execute(&mut s, &mut st, "shnyp.state", &json!({})).unwrap()["format"].clone();
        execute(&mut s, &mut st, "shnyp.crop", &json!({"format": "4:5"})).unwrap();
        let state = execute(&mut s, &mut st, "shnyp.state", &json!({"id": id})).unwrap();
        assert_eq!(state["format"], "4:5");
        assert_eq!(state["undo"], "Format 4:5");
        assert!(state["look"].is_null(), "the format edit made the look final");
        execute(&mut s, &mut st, "shnyp.undo", &json!({})).unwrap();
        assert_eq!(execute(&mut s, &mut st, "shnyp.state", &json!({})).unwrap()["format"], format0);
        execute(&mut s, &mut st, "shnyp.reset", &json!({})).unwrap();
        assert_eq!(execute(&mut s, &mut st, "shnyp.state", &json!({})).unwrap()["sliders"]["light"], 0.0);
    }

    #[test]
    fn bad_params_are_reported() {
        let mut s = Session::with_demo();
        let mut st = ShnypState::default();
        for (id, p) in [
            ("shnyp.crop", json!({"format": "3:2"})),
            ("shnyp.look", json!({"look": "nope"})),
            ("shnyp.look", json!({"look": "lc.moody", "amount": 500})),
            ("shnyp.adjust", json!({"light": "hell"})),
            ("shnyp.adjust", json!({})),
            ("shnyp.export", json!({"preset": "poster"})),
            ("shnyp.state", json!({"id": 999999})),
            ("shnyp.state", json!({"id": "14"})),
            ("shnyp.adjust", json!({"id": 3.5, "light": 10})),
            ("shnyp.look", json!({"look": ["lc.moody"]})),
        ] {
            assert!(execute(&mut s, &mut st, id, &p).is_err(), "{id} {p}");
        }
        assert!(matches!(execute(&mut s, &mut st, "shnyp.nope", &json!({})), Err(EngineError::UnknownCommand(_))));
    }

    #[test]
    fn reset_works_on_a_named_photo_with_nothing_selected() {
        let mut s = Session::with_demo();
        let mut st = ShnypState::default();
        let id = s.active().unwrap();
        execute(&mut s, &mut st, "shnyp.adjust", &json!({"light": 40})).unwrap();
        s.selection = Default::default();
        execute(&mut s, &mut st, "shnyp.reset", &json!({"id": id.0})).unwrap();
        assert_eq!(adjust::values(&s, id)[0].1, 0.0);
    }

    #[test]
    fn a_numeric_folder_name_is_a_folder_name() {
        let mut s = Session::with_demo();
        let mut st = ShnypState::default();
        let e = execute(&mut s, &mut st, "shnyp.export", &json!({"preset": "square", "dir": {"x": 1}})).unwrap_err();
        assert!(e.to_string().contains("`dir` must be text"), "{e}");
        assert_eq!(text_param("t", &json!({"dir": 2024}), "dir").unwrap().as_deref(), Some("2024"));
    }
}
