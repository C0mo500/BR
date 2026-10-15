//! Looks: LightCraft's develop presets as one-tap filters with an Amount.
//!
//! LightCraft's own `preset.apply {amount}` blends from the photo's *current* settings, so moving an
//! Amount slider with it stacks the look on itself. shnyp remembers the settings from before the
//! look ([`LookState::base`]) and recomputes the blend from there each time, so changing the
//! amount replaces the look and the whole thing stays one undo step ("Look: <name>").
//!
//! Tapping another look replaces the current one (like a filter in CapCut) as long as nothing else
//! was edited since; after another edit, a new look goes on top of the edited photo.
//!
//! [`blend_look`] also fixes what LightCraft's linear blend gets wrong below 100 %: fields that
//! cannot be blended (white-balance mode, black & white) switch only at 50 % there, colour grading
//! hues are interpolated from 0° through unrelated colours, and ([`blend_look_for`]) a look's
//! white balance, written on LightCraft's relative scale, is mixed with a raw photo's Kelvin values.

use std::sync::Arc;

use lightcraft_engine::catalog::PhotoId;
use lightcraft_engine::develop::{DevelopSettings, Preset, WbMode, WbScale};
use lightcraft_engine::pipeline::SourceInfo;
use lightcraft_engine::pipeline::local::effective_wb;
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

/// The built-in Looks, then user presets made in Pro mode, grouped (groups in the order they first
/// appear in LightCraft's list, looks in LightCraft's order within a group).
pub fn list(s: &Session) -> Vec<LookInfo> {
    let mut v: Vec<&Preset> = s.presets.iter().filter(|p| p.builtin).collect();
    v.extend(s.presets.iter().filter(|p| !p.builtin));
    let mut groups: Vec<&str> = Vec::new();
    for p in &v {
        if !groups.contains(&p.group.as_str()) {
            groups.push(&p.group);
        }
    }
    // a stable sort keeps LightCraft's order inside each group
    v.sort_by_key(|p| groups.iter().position(|g| *g == p.group).unwrap_or(usize::MAX));
    v.into_iter().map(|p| LookInfo { id: p.id.clone(), name: p.name.clone(), group: p.group.clone() }).collect()
}

/// [`list`] as JSON.
pub fn list_json(s: &Session) -> Value {
    Value::Array(list(s).into_iter().map(|l| json!({"id": l.id, "name": l.name, "group": l.group})).collect())
}

/// Find a look by id, or by name ignoring case.
pub fn find<'a>(s: &'a Session, look: &str) -> Option<&'a Preset> {
    s.presets.iter().find(|p| p.id == look).or_else(|| s.presets.iter().find(|p| p.name.eq_ignore_ascii_case(look.trim())))
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
        s.commits() == self.commits && s.redo.is_empty() && s.undo.last().is_some_and(|e| e.label == self.label)
    }
}

/// The look on `id` whose amount can still be changed, if any.
pub fn current<'a>(look: &'a Option<LookState>, s: &Session, id: PhotoId) -> Option<&'a LookState> {
    look.as_ref().filter(|l| l.photo == id && (l.is_current(s) || s.interaction.as_ref().is_some_and(|i| i.photo == id && i.label == l.label)))
}

/// `base` with `preset` at `amount` (1.0 = 100 %, up to 2.0). Pure.
pub fn blend_look(base: &DevelopSettings, preset: &Preset, amount: f64) -> DevelopSettings {
    let t = if amount.is_finite() { amount.clamp(0.0, MAX_AMOUNT / 100.0) } else { 1.0 };
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
        (&mut out.grading.shadows, base.grading.shadows, full.grading.shadows),
        (&mut out.grading.midtones, base.grading.midtones, full.grading.midtones),
        (&mut out.grading.highlights, base.grading.highlights, full.grading.highlights),
        (&mut out.grading.global, base.grading.global, full.grading.global),
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

/// [`blend_look`] for a photo with source info `info`. On raws whose white balance is edited in
/// Kelvin, a look's white balance (stored relative to as shot) is blended in Kelvin, between what
/// the photo shows now and what the look shows at 100 %, so a warm look gets steadily warmer.
pub fn blend_look_for(info: &SourceInfo, base: &DevelopSettings, preset: &Preset, amount: f64) -> DevelopSettings {
    let mut out = blend_look(base, preset, amount);
    let t = if amount.is_finite() { amount.clamp(0.0, MAX_AMOUNT / 100.0) } else { 1.0 };
    let touches_wb = preset.settings.get("wb").is_some_and(|w| w.get("temp").is_some() || w.get("tint").is_some());
    if t == 0.0 || (t - 1.0).abs() < 1e-9 || !touches_wb || !(info.raw && !info.relative_wb) {
        return out;
    }
    let full = preset.apply(base, 1.0);
    let (bt, bn) = effective_wb(info, base);
    let (ft, fnt) = effective_wb(info, &full);
    // in mireds, which is how even steps of warmth look
    let (bm, fm) = (1e6 / bt.max(1000.0), 1e6 / ft.max(1000.0));
    let m = (bm + (fm - bm) * t).clamp(1e6 / 50_000.0, 1e6 / 2000.0);
    out.wb.mode = WbMode::Custom;
    out.wb.scale = WbScale::Kelvin;
    out.wb.temp = 1e6 / m;
    out.wb.tint = (bn + (fnt - bn) * t).clamp(-150.0, 150.0);
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
///   this photo, it is replaced (another look, or the same look at a new amount), so looks stay
///   exactly one undo step.
///
/// An amount of 0 takes the look off again. The selection is not changed.
pub fn set_look(s: &mut Session, look: &mut Option<LookState>, id: PhotoId, preset_id: &str, amount_pct: f64, live: bool) -> Result<Value> {
    s.execute_fn("shnyp.look", |s| {
        let preset = find(s, preset_id).cloned().ok_or_else(|| bad("shnyp.look", format!("unknown look `{preset_id}`: see shnyp.looks")))?;
        if s.develop_of(id).is_none() {
            return Err(bad("shnyp.look", format!("no such photo {}", id.0)));
        }
        if !amount_pct.is_finite() || !(0.0..=MAX_AMOUNT).contains(&amount_pct) {
            return Err(bad("shnyp.look", "`amount` must be from 0 to 200 (percent)"));
        }
        let label = format!("Look: {}", preset.name);
        if live {
            if s.active() != Some(id) {
                return Err(bad("shnyp.look", "a live look needs the active photo"));
            }
            if s.interaction.as_ref().is_none_or(|i| i.photo != id || i.label != label) {
                s.begin_interaction(&label)?;
            }
        } else if s.interaction.as_ref().is_some_and(|i| i.photo == id && i.label == label) {
            s.cancel_interaction()?;
        }
        let prev = look.take().filter(|l| l.photo == id && l.is_current(s));
        let base = match (&prev, &s.interaction) {
            (Some(l), _) => l.base.clone(),
            (None, Some(i)) if live => i.original.clone(),
            _ => s.develop_of(id).ok_or_else(|| bad("shnyp.look", "no photo"))?,
        };
        let next = blend_look_for(&s.source_info(id), &base, &preset, amount_pct / 100.0);
        if prev.is_some() && !live {
            // undoing shows the undone photo: keep the selection as it was
            let view = (s.selection.clone(), s.active_mask, s.active_spot);
            s.undo_step()?;
            (s.selection, s.active_mask, s.active_spot) = view;
        }
        s.set_develop(id, next, &label)?;
        *look = Some(LookState { photo: id, preset_id: preset.id.clone(), label, amount: amount_pct, base, commits: s.commits() });
        Ok(json!({"id": id.0, "look": preset.id, "name": preset.name, "amount": amount_pct}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_engine::develop::Treatment;

    fn preset(s: &Session, id: &str) -> Preset {
        s.presets.iter().find(|p| p.id == id).cloned().unwrap()
    }

    #[test]
    fn discrete_fields_follow_the_look_at_any_amount() {
        let s = Session::new();
        let base = DevelopSettings::default();
        let warm = blend_look(&base, &preset(&s, "lc.warm-glow"), 0.3);
        assert_eq!(warm.wb.mode, WbMode::Custom, "warmth must show below 50 %");
        assert!(warm.wb.temp > 6500.0 && warm.wb.temp < 7200.0, "{}", warm.wb.temp);
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
        assert_eq!(*s.develop_of(id).unwrap(), blend_look_for(&s.source_info(id), &before, &p, 0.7));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.develop_of(id).unwrap(), before);
    }

    #[test]
    fn another_edit_starts_a_new_look() {
        let mut s = Session::with_demo();
        let id = s.active().unwrap();
        let mut look = None;
        set_look(&mut s, &mut look, id, "lc.moody", 100.0, false).unwrap();
        s.execute("develop.set", &json!({"control": "light.contrast", "value": 30})).unwrap();
        assert!(current(&look, &s, id).is_none());
        let undo0 = s.undo.len();
        set_look(&mut s, &mut look, id, "lc.moody", 50.0, false).unwrap();
        assert_eq!(s.undo.len(), undo0 + 1);
        assert!(s.develop_of(id).unwrap().light.contrast > 0.0);
    }

    #[test]
    fn tapping_another_look_replaces_the_first() {
        let mut s = Session::with_demo();
        let id = s.active().unwrap();
        let before = s.develop_of(id).unwrap();
        let undo0 = s.undo.len();
        let mut look = None;
        set_look(&mut s, &mut look, id, "lc.warm-glow", 100.0, false).unwrap();
        set_look(&mut s, &mut look, id, "lc.bw-soft", 100.0, false).unwrap();
        set_look(&mut s, &mut look, id, "lc.cool-morning", 60.0, false).unwrap();
        assert_eq!(s.undo.len(), undo0 + 1, "one look, one step");
        assert_eq!(*s.develop_of(id).unwrap(), blend_look_for(&s.source_info(id), &before, &preset(&s, "lc.cool-morning"), 0.6));
        assert_eq!(*current(&look, &s, id).unwrap().base, *before);
        set_look(&mut s, &mut look, id, "lc.cool-morning", 0.0, false).unwrap();
        assert_eq!(s.develop_of(id).unwrap(), before, "amount 0 is the photo before any look");
    }

    #[test]
    fn a_look_on_another_photo_leaves_the_selection_alone() {
        let mut s = Session::with_demo();
        let ids = s.visible_cloned();
        let (a, b) = (ids[1], ids[0]);
        assert_eq!(s.active(), Some(b));
        let mut look = None;
        set_look(&mut s, &mut look, a, "lc.moody", 100.0, false).unwrap();
        let undo = s.undo.len();
        set_look(&mut s, &mut look, a, "lc.moody", 50.0, false).unwrap();
        assert_eq!(s.active(), Some(b));
        assert_eq!(s.selection.ids, vec![b]);
        assert_eq!(s.undo.len(), undo);
    }

    #[test]
    fn a_warm_look_gets_steadily_warmer_on_a_kelvin_raw() {
        let s = Session::new();
        // a raw edited in Kelvin (a camera with measured matrices), as shot 5200 K / +2
        let info =
            SourceInfo { raw: true, relative_wb: false, legacy_relative_wb: true, as_shot_temp: 5200.0, as_shot_tint: 2.0, ..Default::default() };
        let mut base = DevelopSettings::default();
        base.wb.mode = WbMode::AsShot;
        base.wb.temp = 5200.0;
        base.wb.tint = 2.0;
        base.wb.scale = WbScale::Kelvin;
        for (look, warmer) in [("lc.warm-glow", true), ("lc.golden-hour", true), ("lc.cool-morning", false)] {
            let p = preset(&s, look);
            let mut last = effective_wb(&info, &base).0;
            for a in [0.1, 0.25, 0.5, 0.75, 1.0, 1.5] {
                let t = effective_wb(&info, &blend_look_for(&info, &base, &p, a)).0;
                assert!(if warmer { t > last } else { t < last }, "{look} at {a}: {t} after {last}");
                last = t;
            }
        }
        // JPEGs keep LightCraft's relative scale
        let jpeg = SourceInfo::default();
        let d = blend_look_for(&jpeg, &DevelopSettings::default(), &preset(&s, "lc.warm-glow"), 0.5);
        assert_eq!((d.wb.scale, d.wb.temp), (WbScale::Legacy, 6850.0));
    }

    #[test]
    fn looks_are_listed_by_group() {
        let s = Session::new();
        let l = list(&s);
        let mut seen: Vec<&str> = Vec::new();
        for x in &l {
            if seen.last() != Some(&x.group.as_str()) {
                assert!(!seen.contains(&x.group.as_str()), "group {} listed twice", x.group);
                seen.push(&x.group);
            }
        }
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
