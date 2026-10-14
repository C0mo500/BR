//! Looks: LightCraft's develop presets as one-tap filters with an Amount.
//!
//! LightCraft's own `preset.apply {amount}` blends from the photo's *current* settings, so moving an
//! Amount slider with it stacks the look on itself. BR remembers the settings from before the
//! look ([`LookState::base`]) and recomputes the blend from there each time, so changing the
//! amount replaces the look and the whole thing stays one undo step ("Look: <name>").
//!
//! [`blend_look`] also fixes two things LightCraft's linear blend gets wrong below 100 %: fields
//! that cannot be blended (white-balance mode, black & white) switch only at 50 % there, and colour
//! grading hues are interpolated from 0° through unrelated colours.

use std::sync::Arc;

use lightcraft_engine::catalog::PhotoId;
use lightcraft_engine::develop::{DevelopSettings, Preset};
use lightcraft_engine::{Result, Session};
use serde_json::{Value, json};

use crate::photo::bad;

/// Highest Amount in percent (LightCraft's presets allow up to 200 %).
pub const MAX_AMOUNT: f64 = 200.0;

/// A Look as listed to the app and to agents.
#[derive(Clone, Debug, PartialEq)]
pub struct LookInfo {
    pub id: String,
    pub name: String,
    pub group: String,
}

/// The built-in Looks, in LightCraft's order. User presets made in Pro mode are listed after them.
pub fn list(s: &Session) -> Vec<LookInfo> {
    let mut v: Vec<&Preset> = s.presets.iter().filter(|p| p.builtin).collect();
    v.extend(s.presets.iter().filter(|p| !p.builtin));
    v.into_iter()
        .map(|p| LookInfo {
            id: p.id.clone(),
            name: p.name.clone(),
            group: p.group.clone(),
        })
        .collect()
}

/// [`list`] as JSON.
pub fn list_json(s: &Session) -> Value {
    Value::Array(
        list(s)
            .into_iter()
            .map(|l| json!({"id": l.id, "name": l.name, "group": l.group}))
            .collect(),
    )
}

/// Find a look by id, or by name ignoring case.
pub fn find<'a>(s: &'a Session, look: &str) -> Option<&'a Preset> {
    s.presets.iter().find(|p| p.id == look).or_else(|| {
        s.presets
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(look.trim()))
    })
}

/// The look the Amount slider edits.
#[derive(Clone, Debug)]
pub struct LookState {
    pub photo: PhotoId,
    pub preset_id: String,
    /// Undo and history label, `Look: <preset name>`.
    pub label: String,
    /// Percent, 0..=200.
    pub amount: f64,
    /// The photo's settings before the look.
    pub base: Arc<DevelopSettings>,
    /// `Session::commits()` right after the look's step was committed.
    pub commits: u64,
}

impl LookState {
    /// The look is still the latest undo step and nothing was committed since: changing its amount
    /// replaces it. (The same rule as LightCraft's own preset Amount slider.)
    pub fn is_current(&self, s: &Session) -> bool {
        s.commits() == self.commits
            && s.redo.is_empty()
            && s.undo.last().is_some_and(|e| e.label == self.label)
    }
}

/// The look on `id` whose amount can still be changed, if any.
pub fn current<'a>(look: &'a Option<LookState>, s: &Session, id: PhotoId) -> Option<&'a LookState> {
    look.as_ref().filter(|l| {
        l.photo == id
            && (l.is_current(s)
                || s.interaction
                    .as_ref()
                    .is_some_and(|i| i.photo == id && i.label == l.label))
    })
}

/// `base` with `preset` at `amount` (1.0 = 100 %, up to 2.0). Pure.
pub fn blend_look(base: &DevelopSettings, preset: &Preset, amount: f64) -> DevelopSettings {
    let t = if amount.is_finite() {
        amount.clamp(0.0, MAX_AMOUNT / 100.0)
    } else {
        1.0
    };
    if t == 0.0 {
        return base.clone();
    }
    let full = preset.apply(base, 1.0);
    if (t - 1.0).abs() < 1e-9 {
        return full;
    }
    let mut out = preset.apply(base, t);
    // Fields that cannot be blended (white-balance mode, treatment, profile ids, integers): take
    // the look's value at any amount above 0, instead of switching at 50 %.
    let mut o = out.to_json();
    copy_discrete(&mut o, &full.to_json(), &preset.settings);
    if let Ok(d) = DevelopSettings::from_json(&o) {
        out = d;
    }
    // Colour grading hues are angles: blend along the shorter arc, or use the look's hue outright
    // when the photo's wheel had no colour.
    let tt = t.min(1.0);
    for (o, b, f) in [
        (
            &mut out.grading.shadows,
            base.grading.shadows,
            full.grading.shadows,
        ),
        (
            &mut out.grading.midtones,
            base.grading.midtones,
            full.grading.midtones,
        ),
        (
            &mut out.grading.highlights,
            base.grading.highlights,
            full.grading.highlights,
        ),
        (
            &mut out.grading.global,
            base.grading.global,
            full.grading.global,
        ),
    ] {
        if f.hue == b.hue {
            continue;
        }
        o.hue = if b.sat <= 0.0 {
            f.hue
        } else {
            let d = ((f.hue - b.hue + 540.0) % 360.0) - 180.0;
            (b.hue + d * tt).rem_euclid(360.0)
        };
    }
    out
}

fn copy_discrete(out: &mut Value, full: &Value, patch: &Value) {
    match patch {
        Value::Object(p) => {
            for (k, pv) in p {
                if k == "masks" {
                    continue; // Preset::apply scales masks itself
                }
                if let (Some(ov), Some(fv)) = (out.get_mut(k), full.get(k)) {
                    copy_discrete(ov, fv, pv);
                }
            }
        }
        Value::Number(pn) => {
            let blended = pn.is_f64() || out.as_number().is_some_and(|n| n.is_f64());
            if !blended {
                *out = full.clone();
            }
        }
        _ => *out = full.clone(),
    }
}

/// Apply a look to `id`, or change the amount of the look it has.
///
/// - `live` (every frame of an Amount drag in the app): applied silently inside an engine
///   interaction on the active photo; no undo step yet, nothing saved.
/// - not live (a tap, the end of a drag, the CLI, MCP): committed. If `look` is still current on
///   this photo, the previous amount is replaced, so the look stays exactly one undo step.
///
/// An amount of 0 takes the look off again.
pub fn set_look(
    s: &mut Session,
    look: &mut Option<LookState>,
    id: PhotoId,
    preset_id: &str,
    amount_pct: f64,
    live: bool,
) -> Result<Value> {
    s.execute_fn("br.look", |s| {
        let preset = find(s, preset_id).cloned().ok_or_else(|| {
            bad(
                "br.look",
                format!("unknown look `{preset_id}`: see br.looks"),
            )
        })?;
        if s.develop_of(id).is_none() {
            return Err(bad("br.look", format!("no such photo {}", id.0)));
        }
        if !amount_pct.is_finite() || !(0.0..=MAX_AMOUNT).contains(&amount_pct) {
            return Err(bad("br.look", "`amount` must be from 0 to 200 (percent)"));
        }
        let label = format!("Look: {}", preset.name);
        if live {
            if s.active() != Some(id) {
                return Err(bad("br.look", "a live look needs the active photo"));
            }
            if s.interaction
                .as_ref()
                .is_none_or(|i| i.photo != id || i.label != label)
            {
                s.begin_interaction(&label)?;
            }
        } else if s
            .interaction
            .as_ref()
            .is_some_and(|i| i.photo == id && i.label == label)
        {
            s.cancel_interaction()?;
        }
        let prev = look
            .take()
            .filter(|l| l.photo == id && l.preset_id == preset.id && l.is_current(s));
        let base = match (&prev, &s.interaction) {
            (Some(l), _) => l.base.clone(),
            (None, Some(i)) if live => i.original.clone(),
            _ => s.develop_of(id).ok_or_else(|| bad("br.look", "no photo"))?,
        };
        let next = blend_look(&base, &preset, amount_pct / 100.0);
        if prev.is_some() && !live {
            s.undo_step()?;
        }
        s.set_develop(id, next, &label)?;
        *look = Some(LookState {
            photo: id,
            preset_id: preset.id.clone(),
            label,
            amount: amount_pct,
            base,
            commits: s.commits(),
        });
        Ok(json!({"id": id.0, "look": preset.id, "name": preset.name, "amount": amount_pct}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_engine::develop::{Treatment, WbMode};

    fn preset(s: &Session, id: &str) -> Preset {
        s.presets.iter().find(|p| p.id == id).cloned().unwrap()
    }

    #[test]
    fn discrete_fields_follow_the_look_at_any_amount() {
        let s = Session::new();
        let base = DevelopSettings::default();
        let warm = blend_look(&base, &preset(&s, "lc.warm-glow"), 0.3);
        assert_eq!(warm.wb.mode, WbMode::Custom, "warmth must show below 50 %");
        assert!(
            warm.wb.temp > 6500.0 && warm.wb.temp < 7200.0,
            "{}",
            warm.wb.temp
        );
        let bw = blend_look(&base, &preset(&s, "lc.bw-soft"), 0.4);
        assert_eq!(bw.treatment, Treatment::Bw);
    }

    #[test]
    fn grading_hue_is_the_looks_own_at_partial_amounts() {
        let s = Session::new();
        let base = DevelopSettings::default();
        let p = preset(&s, "lc.cool-morning");
        let full = blend_look(&base, &p, 1.0);
        let part = blend_look(&base, &p, 0.3);
        assert_eq!(part.grading.shadows.hue, full.grading.shadows.hue);
        assert!(part.grading.shadows.sat < full.grading.shadows.sat);
    }

    #[test]
    fn zero_is_the_base_and_full_is_lightcrafts() {
        let s = Session::new();
        let mut base = DevelopSettings::default();
        base.light.exposure = 0.7;
        let p = preset(&s, "lc.vivid-pop");
        assert_eq!(blend_look(&base, &p, 0.0), base);
        assert_eq!(blend_look(&base, &p, 1.0), p.apply(&base, 1.0));
    }

    #[test]
    fn changing_the_amount_stays_one_undo_step() {
        let mut s = Session::with_demo();
        let id = s.active().unwrap();
        let before = s.develop_of(id).unwrap();
        let undo0 = s.undo.len();
        let mut look = None;
        set_look(&mut s, &mut look, id, "lc.cinematic", 100.0, false).unwrap();
        set_look(&mut s, &mut look, id, "lc.cinematic", 40.0, false).unwrap();
        // a drag: live frames, then the release
        set_look(&mut s, &mut look, id, "lc.cinematic", 55.0, true).unwrap();
        set_look(&mut s, &mut look, id, "lc.cinematic", 70.0, true).unwrap();
        assert_eq!(s.undo.len(), undo0 + 1, "a live drag adds no step");
        set_look(&mut s, &mut look, id, "lc.cinematic", 70.0, false).unwrap();
        assert_eq!(s.undo.len(), undo0 + 1);
        assert_eq!(s.undo.last().unwrap().label, "Look: Cinematic");
        let p = preset(&s, "lc.cinematic");
        assert_eq!(*s.develop_of(id).unwrap(), blend_look(&before, &p, 0.7));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.develop_of(id).unwrap(), before);
    }

    #[test]
    fn another_edit_starts_a_new_look() {
        let mut s = Session::with_demo();
        let id = s.active().unwrap();
        let mut look = None;
        set_look(&mut s, &mut look, id, "lc.moody", 100.0, false).unwrap();
        s.execute(
            "develop.set",
            &json!({"control": "light.contrast", "value": 30}),
        )
        .unwrap();
        assert!(current(&look, &s, id).is_none());
        let undo0 = s.undo.len();
        set_look(&mut s, &mut look, id, "lc.moody", 50.0, false).unwrap();
        assert_eq!(s.undo.len(), undo0 + 1);
        assert_eq!(s.develop_of(id).unwrap().light.contrast > 0.0, true);
    }

    #[test]
    fn looks_are_found_by_id_or_name() {
        let s = Session::new();
        assert_eq!(find(&s, "lc.teal-orange").unwrap().name, "Teal & Orange");
        assert_eq!(find(&s, "teal & orange").unwrap().id, "lc.teal-orange");
        assert!(find(&s, "nope").is_none());
        assert!(list(&s).len() >= 18);
    }
}
