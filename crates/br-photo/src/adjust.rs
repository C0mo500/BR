//! The six simple sliders of BR Foto.
//!
//! Each slider runs from -100 to 100, with 0 meaning "unchanged", and writes a fixed mix of
//! LightCraft develop controls:
//!
//! | Slider    | LightCraft controls                                   |
//! |-----------|-------------------------------------------------------|
//! | Licht     | `light.exposure` = v / 50 (±2 EV)                     |
//! | Kontrast  | `light.contrast` = v                                  |
//! | Farbe     | `color.vibrance` = v, `color.saturation` = v / 4      |
//! | Wärme     | `wb.temp`, v mired steps of 0.8 around the as-shot white balance |
//! | Schärfe   | `effects.texture` = 0.6 v, `effects.clarity` = 0.3 v  |
//! | Vignette  | `vignette.amount` = -v (positive darkens the corners) |
//!
//! A slider shows the value read back from its first control, so edits made in Pro mode show up
//! here too. Changes are made through `develop.set` inside an engine interaction, which gives one
//! undo step named after the slider.

use lightcraft_engine::catalog::PhotoId;
use lightcraft_engine::develop::DevelopSettings;
use lightcraft_engine::pipeline::SourceInfo;
use lightcraft_engine::pipeline::local::effective_wb;
use lightcraft_engine::{Result, Session};
use serde_json::{Map, Value, json};

use crate::photo::{activate, bad};

/// Lowest and highest value of every simple slider.
pub const MIN: f64 = -100.0;
pub const MAX: f64 = 100.0;

/// Mired per slider step for [`Slider::Warmth`]: LightCraft's own relative white-balance scale.
const MIRED_STEP: f64 = 0.8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slider {
    Light,
    Contrast,
    Color,
    Warmth,
    Sharpness,
    Vignette,
}

impl Slider {
    pub const ALL: [Slider; 6] = [Slider::Light, Slider::Contrast, Slider::Color, Slider::Warmth, Slider::Sharpness, Slider::Vignette];

    /// Parameter name in `br.adjust` and the MCP tools.
    pub fn key(self) -> &'static str {
        match self {
            Slider::Light => "light",
            Slider::Contrast => "contrast",
            Slider::Color => "color",
            Slider::Warmth => "warmth",
            Slider::Sharpness => "sharpness",
            Slider::Vignette => "vignette",
        }
    }

    /// The key, or the German label in any case, with or without umlauts.
    pub fn parse(s: &str) -> Option<Slider> {
        let s = s.trim().to_lowercase();
        Slider::ALL.into_iter().find(|k| {
            s == k.key() || s == k.label().to_lowercase() || s == k.label().to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue")
        })
    }

    /// German label shown in the app.
    pub fn label(self) -> &'static str {
        match self {
            Slider::Light => "Licht",
            Slider::Contrast => "Kontrast",
            Slider::Color => "Farbe",
            Slider::Warmth => "Wärme",
            Slider::Sharpness => "Schärfe",
            Slider::Vignette => "Vignette",
        }
    }

    /// One line on what the slider does, for tooltips and tool descriptions.
    pub fn hint(self) -> &'static str {
        match self {
            Slider::Light => "Heller oder dunkler (Belichtung)",
            Slider::Contrast => "Mehr oder weniger Unterschied zwischen hell und dunkel",
            Slider::Color => "Kräftigere oder blassere Farben",
            Slider::Warmth => "Wärmer (gelber) oder kühler (blauer)",
            Slider::Sharpness => "Mehr Details und Struktur, negativ macht weicher",
            Slider::Vignette => "Dunklere Ecken, negativ hellt sie auf",
        }
    }

    /// LightCraft control values for slider value `v`. `as_shot_temp` is the photo's as-shot white
    /// balance on the scale its `wb.temp` uses ([`Session::source_info`]): 6500 for JPEGs.
    pub fn controls(self, v: f64, as_shot_temp: f64) -> Vec<(&'static str, f64)> {
        let v = clamp(v);
        match self {
            Slider::Light => vec![("light.exposure", v / 50.0)],
            Slider::Contrast => vec![("light.contrast", v)],
            Slider::Color => vec![("color.vibrance", v), ("color.saturation", v / 4.0)],
            Slider::Warmth => vec![("wb.temp", 1e6 / (1e6 / as_shot_temp.max(1000.0) - v * MIRED_STEP))],
            Slider::Sharpness => vec![("effects.texture", v * 0.6), ("effects.clarity", v * 0.3)],
            Slider::Vignette => vec![("vignette.amount", -v)],
        }
    }

    /// The slider's value for `d` on a photo with source info `info`, read back from its first
    /// control. Wärme reads the white balance in effect (as LightCraft's Edit panel does), so a
    /// look's relative white balance on a Kelvin raw shows what it really does.
    pub fn read(self, d: &DevelopSettings, info: &SourceInfo) -> f64 {
        let v = match self {
            Slider::Light => d.light.exposure * 50.0,
            Slider::Contrast => d.light.contrast,
            Slider::Color => d.color.vibrance,
            Slider::Warmth => {
                let (t, _) = effective_wb(info, d);
                (1e6 / info.as_shot_temp.max(1000.0) - 1e6 / t.max(1000.0)) / MIRED_STEP
            }
            Slider::Sharpness => d.effects.texture / 0.6,
            Slider::Vignette => -d.vignette.amount,
        };
        (clamp(v) * 10.0).round() / 10.0
    }
}

fn clamp(v: f64) -> f64 {
    if v.is_finite() { v.clamp(MIN, MAX) } else { 0.0 }
}

/// The as-shot white balance of `id` on its own `wb.temp` scale.
pub fn as_shot_temp(s: &Session, id: PhotoId) -> f64 {
    s.source_info(id).as_shot_temp
}

/// All six slider values of `id`.
pub fn values(s: &Session, id: PhotoId) -> Vec<(Slider, f64)> {
    let Some(d) = s.develop_of(id) else {
        return Vec::new();
    };
    let info = s.source_info(id);
    Slider::ALL.into_iter().map(|k| (k, k.read(&d, &info))).collect()
}

/// [`values`] as a JSON object keyed by [`Slider::key`].
pub fn values_json(s: &Session, id: PhotoId) -> Value {
    Value::Object(values(s, id).into_iter().map(|(k, v)| (k.key().to_string(), json!(v))).collect())
}

/// The `values` object of a `develop.set` call for these slider changes.
pub fn develop_values(s: &Session, id: PhotoId, changes: &[(Slider, f64)]) -> Map<String, Value> {
    let t = as_shot_temp(s, id);
    let mut out = Map::new();
    for (k, v) in changes {
        for (c, x) in k.controls(*v, t) {
            out.insert(c.to_string(), json!(x));
        }
    }
    out
}

/// Undo label for a set of changes.
fn label(changes: &[(Slider, f64)]) -> &'static str {
    match changes {
        [(k, _)] => k.label(),
        _ => "Anpassen",
    }
}

/// Set slider values on `id` as one undo step (CLI, MCP, keyboard, reset). A slider drag in the app
/// uses [`begin`], [`drag`] and [`end`] instead.
pub fn set(s: &mut Session, id: PhotoId, changes: &[(Slider, f64)]) -> Result<Value> {
    if changes.is_empty() {
        return Err(bad("br.adjust", "give at least one of light, contrast, color, warmth, sharpness, vignette"));
    }
    s.execute_fn("br.adjust", |s| {
        activate(s, id)?;
        let values = develop_values(s, id, changes);
        s.begin_interaction(label(changes))?;
        let r = s.execute("develop.set", &json!({ "values": values }));
        match r {
            Ok(_) => s.end_interaction()?,
            Err(e) => {
                let _ = s.cancel_interaction();
                return Err(e);
            }
        }
        Ok(json!({ "id": id.0, "sliders": values_json(s, id) }))
    })
}

/// Start dragging `slider` on the active photo: the drag becomes one undo step when it [`end`]s.
pub fn begin(s: &mut Session, slider: Slider) -> Result<()> {
    s.begin_interaction(slider.label())
}

/// Apply a value while dragging (silent: no undo step, nothing saved until [`end`]).
pub fn drag(s: &mut Session, id: PhotoId, slider: Slider, v: f64) -> Result<()> {
    let values = develop_values(s, id, &[(slider, v)]);
    s.execute("develop.set", &json!({ "values": values })).map(|_| ())
}

/// Finish a drag: commits it as one undo step, saved (and its XMP sidecar written, if the library
/// writes them) before this returns.
pub fn end(s: &mut Session) -> Result<()> {
    if s.interaction.is_some() { s.execute("develop.endInteraction", &json!({})).map(|_| ()) } else { Ok(()) }
}

/// LightCraft's Auto (exposure, contrast and tone from the photo's statistics) on `id`.
pub fn auto(s: &mut Session, id: PhotoId) -> Result<Value> {
    s.execute_fn("br.auto", |s| {
        activate(s, id)?;
        s.execute("develop.auto", &json!({}))?;
        Ok(json!({ "id": id.0, "sliders": values_json(s, id) }))
    })
}

/// Back to 0 on all six sliders, as one undo step. Looks, format and Pro-only edits stay.
pub fn reset(s: &mut Session, id: PhotoId) -> Result<Value> {
    let zero: Vec<(Slider, f64)> = Slider::ALL.into_iter().map(|k| (k, 0.0)).collect();
    set(s, id, &zero)
}

/// Parse `{light?, contrast?, ...}` (keys or German labels) into slider changes.
pub fn parse_changes(p: &Value) -> Result<Vec<(Slider, f64)>> {
    let mut out = Vec::new();
    let Some(o) = p.as_object() else {
        return Ok(out);
    };
    for (k, v) in o {
        if k == "id" {
            continue;
        }
        let slider = Slider::parse(k)
            .ok_or_else(|| bad("br.adjust", format!("unknown slider `{k}`: use light, contrast, color, warmth, sharpness or vignette")))?;
        let v = v.as_f64().ok_or_else(|| bad("br.adjust", format!("`{k}` must be a number from -100 to 100")))?;
        if !v.is_finite() || !(MIN..=MAX).contains(&v) {
            return Err(bad("br.adjust", format!("`{k}` must be from -100 to 100")));
        }
        out.push((slider, v));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG (relative white balance), and a raw edited in Kelvin.
    fn infos() -> [SourceInfo; 2] {
        [SourceInfo::default(), SourceInfo { raw: true, relative_wb: false, legacy_relative_wb: true, as_shot_temp: 5200.0, ..Default::default() }]
    }

    #[test]
    fn every_slider_reads_back_what_it_writes() {
        for info in infos() {
            for k in Slider::ALL {
                for v in [-100.0, -37.0, 0.0, 12.5, 100.0] {
                    let mut d = DevelopSettings::default();
                    // what develop.set does: the photo's own scale first, then the value, Custom
                    lightcraft_engine::pipeline::local::normalize_wb(&mut d, &info);
                    for (c, x) in k.controls(v, info.as_shot_temp) {
                        assert!(lightcraft_engine::develop::controls::set(&mut d, c, x), "{c}");
                    }
                    if k == Slider::Warmth {
                        d.wb.mode = lightcraft_engine::develop::WbMode::Custom;
                    }
                    let r = k.read(&d, &info);
                    assert!((r - v).abs() < 0.051, "{k:?} {v} @ {}: {r}", info.as_shot_temp);
                }
            }
        }
    }

    #[test]
    fn warmth_reads_what_a_looks_white_balance_does_on_a_kelvin_raw() {
        let info = &infos()[1];
        // a look writes 7200 K on LightCraft's relative scale (6500 = as shot)
        let mut d = DevelopSettings::default();
        d.wb.mode = lightcraft_engine::develop::WbMode::Custom;
        d.wb.temp = 7200.0;
        d.wb.scale = lightcraft_engine::develop::WbScale::Legacy;
        let shows = Slider::Warmth.read(&d, info);
        let (eff, _) = effective_wb(info, &d);
        assert!(eff > 5200.0 && eff < 6500.0, "{eff}");
        assert!((shows - (1e6 / 5200.0 - 1e6 / eff) / MIRED_STEP).abs() < 0.06, "{shows}");
        // setting the slider to what it shows keeps the colour
        let (c, x) = Slider::Warmth.controls(shows, info.as_shot_temp)[0];
        assert_eq!(c, "wb.temp");
        assert!((x - eff).abs() < 15.0, "{x} vs {eff}");
    }

    #[test]
    fn zero_is_neutral() {
        let d = DevelopSettings::default();
        for k in Slider::ALL {
            assert_eq!(k.read(&d, &SourceInfo::default()), 0.0, "{k:?}");
            for (c, x) in k.controls(0.0, 6500.0) {
                let neutral = if c == "wb.temp" { 6500.0 } else { 0.0 };
                assert!((x - neutral).abs() < 1e-9, "{c} = {x}");
            }
        }
    }

    #[test]
    fn labels_and_keys_parse() {
        assert_eq!(Slider::parse("Wärme"), Some(Slider::Warmth));
        assert_eq!(Slider::parse("waerme"), Some(Slider::Warmth));
        assert_eq!(Slider::parse("SCHÄRFE"), Some(Slider::Sharpness));
        assert_eq!(Slider::parse("light"), Some(Slider::Light));
        assert_eq!(Slider::parse("exposure"), None);
        assert!(parse_changes(&json!({"light": 120})).is_err());
        assert!(parse_changes(&json!({"glow": 1})).is_err());
        assert_eq!(parse_changes(&json!({"id": 3, "Licht": 20})).unwrap(), vec![(Slider::Light, 20.0)]);
    }
}
