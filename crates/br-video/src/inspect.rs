//! The inspector (CapCut's right column, DaVinci's Inspector): what a clip, caption or transition
//! is made of, as rows to read and to set.
//!
//! *Basis* is a table per kind of clip (video, picture, text, shape, sound, music, caption,
//! transition) with German labels; the type, range, default and whether a row can be animated come
//! from FilmCraft's effect schemas. *Mehr* lists every effect on a clip with every parameter of its
//! schema, nothing left out. Every change is one undo step, and the frames of one drag (`gesture`)
//! fold into one.
//!
//! Effects are addressed by their index, looked up afresh for every change (two Lumetris on a clip
//! are two effects, not "the Lumetri"). *Anpassen* is the clip's first Lumetri that is not an
//! unchanged look from `br.video.filter`: it is what the Farbe page's Lumetri panel edits, and a
//! new one goes in front of the looks, so a later filter replaces the look and keeps the grade.
//! Values are checked against the schema before they reach FilmCraft, which stores what it gets.

use std::sync::{Arc, OnceLock};

use filmcraft_engine::{EngineError, Session};
use filmcraft_project::{
    Caption, ClipId, EffectDef, EffectInstance, ItemKind, ParamDef, ParamKind, ParamValue, Sequence, TrackItem, TrackKind, TransitionId,
};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::{magnet, main_track};

type R<T> = filmcraft_engine::Result<T>;

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

/// What the inspector shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Video,
    Picture,
    Text,
    Shape,
    Adjustment,
    Nested,
    Sound,
    Music,
    Caption,
    Transition,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Video => "video",
            Kind::Picture => "picture",
            Kind::Text => "text",
            Kind::Shape => "shape",
            Kind::Adjustment => "adjustment",
            Kind::Nested => "nested",
            Kind::Sound => "sound",
            Kind::Music => "music",
            Kind::Caption => "caption",
            Kind::Transition => "transition",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Video => "Video",
            Kind::Picture => "Bild",
            Kind::Text => "Text",
            Kind::Shape => "Form",
            Kind::Adjustment => "Einstellungsebene",
            Kind::Nested => "Verschachtelte Sequenz",
            Kind::Sound => "Ton",
            Kind::Music => "Musik",
            Kind::Caption => "Untertitel",
            Kind::Transition => "Übergang",
        }
    }

    /// The Basis sections of this kind, in order.
    fn sections(self) -> &'static [&'static Section] {
        match self {
            Kind::Video => &[&TRANSFORM, &BACKGROUND, &CROP, &OPACITY, &ADJUST, &SPEED, &CLIP_AUDIO],
            Kind::Picture => &[&TRANSFORM, &BACKGROUND, &CROP, &OPACITY, &ADJUST],
            Kind::Nested => &[&TRANSFORM, &BACKGROUND, &OPACITY, &ADJUST, &SPEED, &CLIP_AUDIO],
            Kind::Adjustment => &[&OPACITY, &ADJUST],
            Kind::Text => &[&TEXT, &STYLE, &LAYER_TRANSFORM],
            Kind::Shape => &[&SHAPE, &STYLE, &LAYER_TRANSFORM],
            Kind::Sound => &[&SOUND, &SPEED],
            Kind::Music => &[&MUSIC, &SPEED],
            Kind::Caption => &[&CAPTION, &CAPTION_STYLE],
            Kind::Transition => &[&TRANSITION],
        }
    }
}

/// Which effect of a clip a row edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fx {
    /// The first effect with this id.
    Id(&'static str),
    /// The grade (see the module docs).
    Adjust,
    /// The graphic layer the inspector shows (the selected one, else the first).
    Layer,
    /// The effect at this index.
    Index(usize),
}

/// What a Basis row reads and sets.
#[derive(Clone, Copy, Debug)]
enum T {
    /// A parameter of an effect.
    P(Fx, &'static str),
    /// Speed factor (BR's magnetic speed change).
    Speed,
    Reverse,
    /// Clip gain in dB (music: it never fights the ducking keyframes on Volume).
    Gain,
    FadeIn,
    FadeOut,
    /// An effect without parameters that is there or not (flips).
    Has(&'static str),
    Fit,
    Fill,
    /// A text layer's text.
    Text,
    Ducking,
    DuckDb,
    CaptionText,
    CaptionStart,
    CaptionEnd,
    /// A key of the caption track's style (`captions.setStyle`).
    CaptionStyle(&'static str),
    TransitionDuration,
    /// How the frame behind a main clip is filled (`br.video.background`): none, blur, color.
    BgMode,
    BgAmount,
    BgColor,
    /// The picture fills the frame and follows its subject (`br.video.reframe`).
    Reframe,
    /// A speed curve (`br.video.curve`).
    Curve,
}

struct Row {
    key: &'static str,
    label: &'static str,
    t: T,
    /// The slider's range, when it differs from the schema's.
    slider: Option<(f64, f64)>,
    /// The row edits the linked sound of a video clip.
    linked: bool,
}

struct Section {
    key: &'static str,
    label: &'static str,
    rows: &'static [Row],
}

const fn r(key: &'static str, label: &'static str, t: T) -> Row {
    Row { key, label, t, slider: None, linked: false }
}

const fn rs(key: &'static str, label: &'static str, t: T, lo: f64, hi: f64) -> Row {
    Row { key, label, t, slider: Some((lo, hi)), linked: false }
}

const fn linked(row: Row) -> Row {
    Row { linked: true, ..row }
}

use Fx::{Adjust, Id, Layer};
use T::P;

const TRANSFORM: Section = Section {
    key: "transform",
    label: "Transformieren",
    rows: &[
        rs("scale", "Skalieren", P(Id("motion"), "scale"), 0.0, 400.0),
        r("position", "Position", P(Id("motion"), "position")),
        rs("rotation", "Drehen", P(Id("motion"), "rotation"), -180.0, 180.0),
        r("flipH", "Horizontal spiegeln", T::Has("horizontal_flip")),
        r("flipV", "Vertikal spiegeln", T::Has("vertical_flip")),
        r("reframe", "Motiv folgen", T::Reframe),
        r("fit", "Einpassen", T::Fit),
        r("fill", "Füllen", T::Fill),
    ],
};

const BACKGROUND: Section = Section {
    key: "background",
    label: "Hintergrund",
    rows: &[r("mode", "Hintergrund", T::BgMode), r("amount", "Unschärfe", T::BgAmount), r("color", "Farbe", T::BgColor)],
};

const CROP: Section = Section {
    key: "crop",
    label: "Zuschneiden",
    rows: &[
        rs("left", "Links", P(Id("crop"), "left"), 0.0, 50.0),
        rs("top", "Oben", P(Id("crop"), "top"), 0.0, 50.0),
        rs("right", "Rechts", P(Id("crop"), "right"), 0.0, 50.0),
        rs("bottom", "Unten", P(Id("crop"), "bottom"), 0.0, 50.0),
    ],
};

const OPACITY: Section = Section {
    key: "opacity",
    label: "Deckkraft",
    rows: &[r("opacity", "Deckkraft", P(Id("opacity"), "opacity")), r("blend", "Mischmodus", P(Id("opacity"), "blend"))],
};

const ADJUST: Section = Section {
    key: "adjust",
    label: "Anpassen",
    rows: &[
        r("exposure", "Belichtung", P(Adjust, "exposure")),
        r("contrast", "Kontrast", P(Adjust, "contrast")),
        r("highlights", "Lichter", P(Adjust, "highlights")),
        r("shadows", "Tiefen", P(Adjust, "shadows")),
        r("whites", "Weiß", P(Adjust, "whites")),
        r("blacks", "Schwarz", P(Adjust, "blacks")),
        r("saturation", "Sättigung", P(Adjust, "saturation")),
        r("vibrance", "Dynamik", P(Adjust, "vibrance")),
        r("temperature", "Temperatur", P(Adjust, "temperature")),
        r("tint", "Tönung", P(Adjust, "tint")),
        r("sharpen", "Schärfe", P(Adjust, "sharpen")),
        r("fade", "Verblassen", P(Adjust, "faded_film")),
        r("vignette", "Vignette", P(Adjust, "vignette_amount")),
    ],
};

const SPEED: Section = Section {
    key: "speed",
    label: "Tempo",
    rows: &[rs("speed", "Tempo", T::Speed, 0.1, 10.0), r("curve", "Kurve", T::Curve), r("reverse", "Rückwärts", T::Reverse)],
};

const CLIP_AUDIO: Section = Section {
    key: "audio",
    label: "Ton",
    rows: &[
        linked(rs("volume", "Lautstärke", P(Id("volume"), "level"), -60.0, 15.0)),
        linked(r("balance", "Balance", P(Id("panner"), "balance"))),
        linked(rs("fadeIn", "Einblenden", T::FadeIn, 0.0, 5.0)),
        linked(rs("fadeOut", "Ausblenden", T::FadeOut, 0.0, 5.0)),
    ],
};

const SOUND: Section = Section {
    key: "audio",
    label: "Ton",
    rows: &[
        rs("volume", "Lautstärke", P(Id("volume"), "level"), -60.0, 15.0),
        r("balance", "Balance", P(Id("panner"), "balance")),
        rs("fadeIn", "Einblenden", T::FadeIn, 0.0, 5.0),
        rs("fadeOut", "Ausblenden", T::FadeOut, 0.0, 5.0),
    ],
};

const MUSIC: Section = Section {
    key: "audio",
    label: "Ton",
    rows: &[
        rs("volume", "Lautstärke", T::Gain, -60.0, 15.0),
        r("ducking", "Leiser bei Sprache", T::Ducking),
        rs("duck", "Absenkung", T::DuckDb, -30.0, 0.0),
        r("balance", "Balance", P(Id("panner"), "balance")),
        rs("fadeIn", "Einblenden", T::FadeIn, 0.0, 5.0),
        rs("fadeOut", "Ausblenden", T::FadeOut, 0.0, 5.0),
    ],
};

const TEXT: Section = Section {
    key: "text",
    label: "Text",
    rows: &[
        r("text", "Text", T::Text),
        r("font", "Schrift", P(Layer, "font")),
        r("style", "Schriftschnitt", P(Layer, "font_style")),
        r("size", "Größe", P(Layer, "size")),
        r("color", "Farbe", P(Layer, "fill_color")),
        r("align", "Ausrichtung", P(Layer, "align")),
        r("bold", "Fett", P(Layer, "faux_bold")),
        r("italic", "Kursiv", P(Layer, "faux_italic")),
        r("underline", "Unterstrichen", P(Layer, "underline")),
        r("tracking", "Laufweite", P(Layer, "tracking")),
    ],
};

const SHAPE: Section = Section {
    key: "shape",
    label: "Form",
    rows: &[
        r("shape", "Form", P(Layer, "shape")),
        r("size", "Größe", P(Layer, "size")),
        r("corner", "Eckenradius", P(Layer, "corner_radius")),
        r("fill", "Füllung", P(Layer, "fill")),
        r("color", "Farbe", P(Layer, "fill_color")),
    ],
};

const STYLE: Section = Section {
    key: "style",
    label: "Stil",
    rows: &[
        r("stroke", "Kontur", P(Layer, "stroke")),
        r("strokeColor", "Konturfarbe", P(Layer, "stroke_color")),
        r("strokeWidth", "Konturbreite", P(Layer, "stroke_width")),
        r("background", "Hintergrund", P(Layer, "background")),
        r("backgroundColor", "Hintergrundfarbe", P(Layer, "background_color")),
        r("backgroundOpacity", "Hintergrund-Deckkraft", P(Layer, "background_opacity")),
        r("backgroundRadius", "Hintergrund-Ecken", P(Layer, "background_radius")),
        r("shadow", "Schatten", P(Layer, "shadow")),
        r("shadowColor", "Schattenfarbe", P(Layer, "shadow_color")),
        r("shadowOpacity", "Schatten-Deckkraft", P(Layer, "shadow_opacity")),
        r("shadowDistance", "Schattenabstand", P(Layer, "shadow_distance")),
        r("shadowBlur", "Schattenweichheit", P(Layer, "shadow_blur")),
    ],
};

const LAYER_TRANSFORM: Section = Section {
    key: "transform",
    label: "Transformieren",
    rows: &[
        r("position", "Position", P(Layer, "position")),
        r("scale", "Skalieren", P(Layer, "scale")),
        rs("rotation", "Drehen", P(Layer, "rotation"), -180.0, 180.0),
        r("opacity", "Deckkraft", P(Layer, "opacity")),
    ],
};

const CAPTION: Section = Section {
    key: "caption",
    label: "Untertitel",
    rows: &[r("text", "Text", T::CaptionText), r("start", "Anfang", T::CaptionStart), r("end", "Ende", T::CaptionEnd)],
};

const CAPTION_STYLE: Section = Section {
    key: "captionStyle",
    label: "Stil (alle Untertitel der Spur)",
    rows: &[
        r("font", "Schrift", T::CaptionStyle("font")),
        rs("size", "Größe", T::CaptionStyle("size"), 12.0, 160.0),
        r("color", "Farbe", T::CaptionStyle("color")),
        r("background", "Hintergrund", T::CaptionStyle("background")),
        r("backgroundColor", "Hintergrundfarbe", T::CaptionStyle("backgroundColor")),
        r("align", "Ausrichtung", T::CaptionStyle("align")),
        r("anchor", "Position", T::CaptionStyle("anchor")),
        r("margin", "Abstand vom Rand", T::CaptionStyle("margin")),
        rs("outline", "Kontur", T::CaptionStyle("outline"), 0.0, 12.0),
        r("outlineColor", "Konturfarbe", T::CaptionStyle("outlineColor")),
    ],
};

const TRANSITION: Section = Section { key: "transition", label: "Übergang", rows: &[rs("duration", "Dauer", T::TransitionDuration, 0.1, 3.0)] };

/// What the inspector is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Clip(ClipId),
    Caption(ClipId),
    Transition(TransitionId),
}

/// The subject a command names (`clip`, `caption`, `transition`), else the selection: a selected
/// clip (a picture before its sound; a background stands for its main clip), else a caption, else
/// a transition.
pub fn subject(s: &Session, p: &Value) -> Option<Subject> {
    let q = s.active_sequence()?;
    if let Some(c) = p.get("clip").and_then(Value::as_u64).map(ClipId) {
        return if q.find_item(c).is_some() { Some(Subject::Clip(main_of_background(q, c))) } else { q.find_caption(c).map(|_| Subject::Caption(c)) };
    }
    if let Some(c) = p.get("caption").and_then(Value::as_u64).map(ClipId) {
        return q.find_caption(c).map(|_| Subject::Caption(c));
    }
    if let Some(t) = p.get("transition").and_then(Value::as_u64).map(TransitionId) {
        return transition(q, t).map(|_| Subject::Transition(t));
    }
    let sel = &s.state.selection;
    let on_video = |c: &&ClipId| q.video_tracks.iter().any(|t| t.items.iter().any(|it| it.id == **c));
    if let Some(c) = sel.iter().find(on_video).or_else(|| sel.iter().find(|c| q.find_item(**c).is_some())) {
        return Some(Subject::Clip(main_of_background(q, *c)));
    }
    if let Some(c) = s.state.caption_selection.iter().find(|c| q.find_caption(**c).is_some()) {
        return Some(Subject::Caption(*c));
    }
    s.state.transition_selection.iter().find(|t| transition(q, **t).is_some()).map(|t| Subject::Transition(*t))
}

fn main_of_background(q: &Sequence, c: ClipId) -> ClipId {
    match q.find_item(c) {
        Some((_, it)) if main_track::is_background(it) => main_track::main_of(q, c).unwrap_or(c),
        _ => c,
    }
}

fn transition(q: &Sequence, id: TransitionId) -> Option<&filmcraft_project::Transition> {
    q.video_tracks.iter().chain(&q.audio_tracks).flat_map(|t| &t.transitions).find(|x| x.id == id)
}

fn track_kind(q: &Sequence, c: ClipId) -> Option<TrackKind> {
    let (tid, _) = q.find_item(c)?;
    if q.video_tracks.iter().any(|t| t.id == tid) { Some(TrackKind::Video) } else { Some(TrackKind::Audio) }
}

/// The kind of a clip.
pub fn kind_of(s: &Session, q: &Sequence, c: ClipId) -> Option<Kind> {
    let (_, it) = q.find_item(c)?;
    if track_kind(q, c)? == TrackKind::Audio {
        let music = it.essential.as_ref().is_some_and(|e| e.kind == filmcraft_project::AudioType::Music);
        return Some(if music { Kind::Music } else { Kind::Sound });
    }
    if let Some(e) = layer(it, layer_choice(s)) {
        return Some(if e.1.effect == filmcraft_project::graphic::SHAPE_LAYER { Kind::Shape } else { Kind::Text });
    }
    Some(match s.project.item(it.item).map(|i| &i.kind) {
        Some(ItemKind::AdjustmentLayer { .. }) => Kind::Adjustment,
        Some(ItemKind::Sequence(_)) => Kind::Nested,
        _ if s.project.resolve_media(it.item).is_some_and(|(_, m, _)| m.info.kind == filmcraft_media::MediaKind::Still) => Kind::Picture,
        _ => Kind::Video,
    })
}

/// The graphic layer the inspector edits: the selected one (`graphic_layers`), else the first.
fn layer_choice(s: &Session) -> usize {
    s.state.graphic_layers.first().copied().unwrap_or(0)
}

/// The `n`-th graphic layer of a clip (the first when there are fewer): (effect index, layer).
fn layer(it: &TrackItem, n: usize) -> Option<(usize, &EffectInstance)> {
    let layers: Vec<(usize, &EffectInstance)> = it.effects.iter().enumerate().filter(|(_, e)| filmcraft_project::graphic::is_layer(e)).collect();
    layers.get(n).or(layers.first()).copied()
}

/// The ordinal (among the clip's layers) of the layer at effect index `i`.
fn layer_ordinal(it: &TrackItem, i: usize) -> usize {
    it.effects[..i].iter().filter(|e| filmcraft_project::graphic::is_layer(e)).count()
}

fn looks() -> &'static [EffectInstance] {
    static LOOKS: OnceLock<Vec<EffectInstance>> = OnceLock::new();
    LOOKS.get_or_init(|| filmcraft_render::lumetri_presets::presets().iter().map(|l| l.instance()).collect())
}

/// The clip's grade: its first Lumetri that is not a look applied unchanged.
pub fn adjust_index(it: &TrackItem) -> Option<usize> {
    it.effects.iter().position(|e| e.effect == "lumetri" && !looks().contains(e))
}

/// The linked sound of a video clip.
fn linked_audio(q: &Sequence, c: ClipId) -> Option<ClipId> {
    let (_, it) = q.find_item(c)?;
    let l = it.link?;
    q.audio_tracks.iter().flat_map(|t| &t.items).find(|a| a.link == Some(l) && a.id != c).map(|a| a.id)
}

/// Where in a clip the inspector looks: `t` held inside it (FilmCraft's rule for the playhead).
fn inside(it: &TrackItem, t: Tick) -> Tick {
    t.clamp(it.start, (it.end() - Tick(1)).max(it.start))
}

/// The timeline time at which a clip shows media time `mt`.
fn timeline_at(it: &TrackItem, mt: Tick) -> Tick {
    let speed = it.speed.abs().max(1e-9);
    let rel = if it.reverse {
        (it.source_in.0 + (it.duration.0 as f64 * speed).round() as i64 - 1 - mt.0) as f64 / speed
    } else {
        (mt.0 - it.source_in.0) as f64 / speed
    };
    it.start + Tick(rel.round() as i64)
}

fn secs(t: Tick) -> f64 {
    t.0 as f64 / filmcraft_time::TICKS_PER_SECOND as f64
}

fn ticks(sec: f64) -> Tick {
    Tick((sec * filmcraft_time::TICKS_PER_SECOND as f64).round() as i64)
}

fn num(x: f64) -> Value {
    if x.is_finite() { json!(x) } else { Value::Null }
}

fn hex(c: [f32; 4]) -> String {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    if c[3] >= 0.999 {
        format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]), b(c[3]))
    }
}

fn hex8(c: [u8; 4]) -> String {
    if c[3] == 255 { format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]) } else { format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3]) }
}

fn parse_hex(s: &str) -> Option<[f32; 4]> {
    let h = s.strip_prefix('#').unwrap_or(s);
    if !(h.len() == 6 || h.len() == 8) || !h.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some([byte(0)?, byte(2)?, byte(4)?, if h.len() == 8 { byte(6)? } else { 1.0 }])
}

/// A colour given as `#rrggbb[aa]` or `[r, g, b, a?]` (0…1).
fn color_in(v: &Value) -> Option<[f32; 4]> {
    match v {
        Value::String(s) => parse_hex(s),
        Value::Array(a) if a.len() == 3 || a.len() == 4 => {
            let c = |i: usize| a.get(i).and_then(Value::as_f64).filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0) as f32);
            Some([c(0)?, c(1)?, c(2)?, if a.len() == 4 { c(3)? } else { 1.0 }])
        }
        _ => None,
    }
}

/// A parameter value as JSON: numbers, `[x, y]`, `#rrggbb[aa]`, a choice's option name.
fn value_json(v: &ParamValue, def: Option<&ParamDef>) -> Value {
    match v {
        ParamValue::Float(x) => num(*x),
        ParamValue::Vec2(p) => json!([num(p.x), num(p.y)]),
        ParamValue::Color(c) => json!(hex(*c)),
        ParamValue::Bool(b) => json!(b),
        ParamValue::Choice(i) => match def.map(|d| &d.kind) {
            Some(ParamKind::Choice(opts)) => opts.get(*i as usize).map_or(json!(i), |o| json!(o)),
            _ => json!(i),
        },
        ParamValue::Text(t) => json!(t),
        ParamValue::Curve(c) => json!(c),
        ParamValue::Path(p) => serde_json::to_value(p).unwrap_or(Value::Null),
    }
}

fn type_of(kind: &ParamKind) -> &'static str {
    match kind {
        ParamKind::Float { .. } => "float",
        ParamKind::Point => "point",
        ParamKind::Color => "color",
        ParamKind::Bool => "bool",
        ParamKind::Choice(_) => "choice",
        ParamKind::Angle => "angle",
        ParamKind::Text => "text",
        ParamKind::Curve { .. } => "curve",
        ParamKind::Wheel => "wheel",
        ParamKind::Path => "path",
    }
}

/// The schema part of a parameter row: type, range, slider, unit, options, default.
fn schema_json(def: &ParamDef, slider: Option<(f64, f64)>) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert("type".into(), json!(type_of(&def.kind)));
    m.insert("default".into(), value_json(&def.default, Some(def)));
    m.insert("animatable".into(), json!(def.animatable));
    match &def.kind {
        ParamKind::Float { min, max, soft_min, soft_max, unit, decimals } => {
            m.insert("min".into(), num(*min));
            m.insert("max".into(), num(*max));
            let (lo, hi) = slider.unwrap_or((*soft_min, *soft_max));
            m.insert("slider".into(), json!([lo, hi]));
            let unit = if unit.is_empty() && def.id.starts_with("scale") { "%" } else { unit };
            m.insert("unit".into(), json!(unit));
            m.insert("decimals".into(), json!(decimals));
        }
        ParamKind::Angle => {
            m.insert("slider".into(), json!(slider.map_or([-180.0, 180.0], |(a, b)| [a, b])));
            m.insert("unit".into(), json!("°"));
            m.insert("decimals".into(), json!(1));
        }
        ParamKind::Choice(opts) => {
            m.insert("options".into(), json!(opts));
        }
        ParamKind::Wheel => {
            m.insert("min".into(), json!(-1.0));
            m.insert("max".into(), json!(1.0));
        }
        _ => {}
    }
    if let Some(g) = def.group {
        m.insert("group".into(), json!(g));
    }
    m
}

/// A value checked against a parameter's schema, as `effects.setParam` takes it: floats held in
/// their range, choices by index or name, colours from hex or 0…1 components.
pub(crate) fn param_input(def: &ParamDef, v: &Value) -> Result<Value, String> {
    let bad = |what: &str| format!("`{}` {what}", def.id);
    Ok(match &def.kind {
        ParamKind::Float { min, max, .. } => {
            let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad("takes a number"))?;
            json!(x.clamp(*min, *max))
        }
        ParamKind::Angle => json!(v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad("takes a number (degrees)"))?),
        ParamKind::Point | ParamKind::Wheel => {
            let xy = match v {
                Value::Array(a) if a.len() == 2 => a.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>(),
                Value::Object(o) => [o.get("x"), o.get("y")].into_iter().map(|c| c.and_then(Value::as_f64)).collect(),
                _ => None,
            }
            .filter(|xy| xy.iter().all(|c| c.is_finite()))
            .ok_or_else(|| bad("takes [x, y]"))?;
            let lim = |c: f64| if matches!(def.kind, ParamKind::Wheel) { c.clamp(-1.0, 1.0) } else { c };
            json!([lim(xy[0]), lim(xy[1])])
        }
        ParamKind::Color => json!(color_in(v).ok_or_else(|| bad("takes #rrggbb[aa] or [r, g, b, a] in 0…1"))?),
        ParamKind::Bool => json!(v.as_bool().ok_or_else(|| bad("takes true or false"))?),
        ParamKind::Choice(opts) => {
            let i = match v {
                Value::String(name) => opts.iter().position(|o| o.eq_ignore_ascii_case(name)),
                _ => v.as_u64().map(|i| i as usize).filter(|i| *i < opts.len()),
            };
            json!(i.ok_or_else(|| bad(&format!("is one of {opts:?}")))?)
        }
        ParamKind::Text => json!(v.as_str().ok_or_else(|| bad("takes text"))?),
        ParamKind::Curve { .. } => {
            let pts = v
                .as_array()
                .and_then(|a| {
                    a.iter()
                        .map(|p| Some([p.get(0)?.as_f64()?.clamp(0.0, 1.0), p.get(1)?.as_f64()?.clamp(0.0, 1.0)]))
                        .collect::<Option<Vec<[f64; 2]>>>()
                })
                .ok_or_else(|| bad("takes [[x, y], …] in 0…1"))?;
            json!(pts)
        }
        ParamKind::Path => v.clone(),
    })
}

/// The effect a row means on a clip: its index, if it is there.
fn fx_index(it: &TrackItem, fx: Fx, layer_n: usize) -> Option<usize> {
    match fx {
        Fx::Id(id) => it.effects.iter().position(|e| e.effect == id),
        Fx::Adjust => adjust_index(it),
        Fx::Layer => layer(it, layer_n).map(|l| l.0),
        Fx::Index(i) => (i < it.effects.len()).then_some(i),
    }
}

/// The schema of the effect a row means (the layer's own, for a graphic layer).
fn fx_def(it: &TrackItem, fx: Fx, layer_n: usize) -> Option<&'static EffectDef> {
    match fx {
        Fx::Id(id) => filmcraft_project::find_effect(id),
        Fx::Adjust => filmcraft_project::find_effect("lumetri"),
        Fx::Layer | Fx::Index(_) => fx_index(it, fx, layer_n).and_then(|i| it.effects[i].def()),
    }
}

/// A parameter at `t` as a row: value, keyframes and the schema.
fn param_json(
    it: &TrackItem,
    idx: Option<usize>,
    def: &ParamDef,
    t: Tick,
    slider: Option<(f64, f64)>,
    keyframe_times: bool,
) -> serde_json::Map<String, Value> {
    let mut m = schema_json(def, slider);
    let mt = it.source_time_at(inside(it, t));
    let prm = idx.and_then(|i| it.effects[i].params.get(def.id));
    let value = prm.map_or_else(|| def.default.clone(), |p| p.value_at(mt));
    m.insert("value".into(), value_json(&value, Some(def)));
    m.insert("effect".into(), idx.map_or(Value::Null, |i| json!(i)));
    let kfs = prm.map(|p| p.keyframes.as_slice()).unwrap_or(&[]);
    m.insert("animated".into(), json!(!kfs.is_empty()));
    m.insert("atKeyframe".into(), json!(kfs.iter().any(|k| k.time == mt)));
    if keyframe_times {
        let list: Vec<Value> = kfs
            .iter()
            .map(|k| json!({"seconds": secs(timeline_at(it, k.time)), "mediaTime": k.time.0, "value": value_json(&k.value, Some(def)), "interpolation": format!("{:?}", k.interp)}))
            .collect();
        m.insert("keyframes".into(), json!(list));
    } else {
        m.insert("keyframes".into(), json!(kfs.len()));
    }
    m
}

/// Everything the inspector needs from the session for one subject.
struct Ctx<'a> {
    s: &'a Session,
    q: &'a Sequence,
    t: Tick,
    layer: usize,
}

fn fade(q: &Sequence, c: ClipId, incoming: bool) -> Option<&filmcraft_project::Transition> {
    let (tid, _) = q.find_item(c)?;
    let tr = q.video_tracks.iter().chain(&q.audio_tracks).find(|t| t.id == tid)?;
    tr.transitions.iter().find(|x| if incoming { x.to == Some(c) } else { x.from == Some(c) })
}

fn caption(q: &Sequence, c: ClipId) -> Option<(filmcraft_project::TrackId, &Caption)> {
    q.find_caption(c)
}

/// One Basis row of `subject` as JSON (`None`: the row does not apply, e.g. no linked sound).
fn row_json(cx: &Ctx, subject: Subject, section: &Section, row: &Row) -> Option<Value> {
    let q = cx.q;
    let mut m = serde_json::Map::new();
    m.insert("key".into(), json!(format!("{}.{}", section.key, row.key)));
    m.insert("label".into(), json!(row.label));
    let float = |m: &mut serde_json::Map<String, Value>, value: f64, default: f64, (min, max): (f64, f64), unit: &str, decimals: u8| {
        m.insert("type".into(), json!("float"));
        m.insert("value".into(), num(value));
        m.insert("default".into(), json!(default));
        m.insert("min".into(), num(min));
        m.insert("max".into(), num(max));
        m.insert("slider".into(), json!(row.slider.map_or([min, max], |(a, b)| [a, b])));
        m.insert("unit".into(), json!(unit));
        m.insert("decimals".into(), json!(decimals));
        m.insert("animatable".into(), json!(false));
    };
    let plain = |m: &mut serde_json::Map<String, Value>, ty: &str, value: Value, default: Value| {
        m.insert("type".into(), json!(ty));
        m.insert("value".into(), value);
        m.insert("default".into(), default);
        m.insert("animatable".into(), json!(false));
    };
    match subject {
        Subject::Clip(c0) => {
            let c = if row.linked { linked_audio(q, c0)? } else { c0 };
            let (_, it) = q.find_item(c)?;
            m.insert("clip".into(), json!(c.0));
            match row.t {
                T::P(fx, pid) => {
                    let def = fx_def(it, fx, cx.layer)?.param(pid)?;
                    m.extend(param_json(it, fx_index(it, fx, cx.layer), def, cx.t, row.slider, false));
                }
                T::Speed => float(&mut m, it.speed.abs(), 1.0, (0.01, 100.0), "×", 2),
                T::Reverse => plain(&mut m, "bool", json!(it.reverse), json!(false)),
                T::Gain => float(&mut m, it.gain_db, 0.0, (-96.0, 96.0), "dB", 1),
                T::FadeIn | T::FadeOut => {
                    let f = fade(q, c, matches!(row.t, T::FadeIn)).map_or(0.0, |x| secs(x.duration));
                    float(&mut m, f, 0.0, (0.0, secs(it.duration)), "s", 2);
                }
                T::Has(id) => plain(&mut m, "bool", json!(it.effect(id).is_some()), json!(false)),
                T::Fit | T::Fill => plain(&mut m, "action", Value::Null, Value::Null),
                T::Text => {
                    let (i, e) = layer(it, cx.layer)?;
                    let mt = it.source_time_at(inside(it, cx.t));
                    let text = e.params.get("text").map(|p| p.value_at(mt));
                    plain(&mut m, "text", text.map_or(json!(""), |v| value_json(&v, None)), json!(""));
                    m.insert("effect".into(), json!(i));
                }
                T::Ducking => plain(&mut m, "bool", json!(it.essential.as_ref().is_some_and(|e| e.ducking.enabled)), json!(false)),
                T::BgMode | T::BgAmount | T::BgColor => {
                    // backgrounds are behind main clips only
                    if !magnet::is_main(q, c) {
                        return None;
                    }
                    let fill = crate::frame::fill_of(q, c);
                    match row.t {
                        T::BgMode => {
                            let mode = match &fill {
                                crate::frame::Fill::None => "none",
                                crate::frame::Fill::Blur(_) => "blur",
                                crate::frame::Fill::Color(_) => "color",
                                crate::frame::Fill::Image(_) => "image",
                            };
                            plain(&mut m, "choice", json!(mode), json!("none"));
                            m.insert("options".into(), json!(["none", "blur", "color"]));
                        }
                        T::BgAmount => {
                            let a = if let crate::frame::Fill::Blur(a) = fill { a } else { 50.0 };
                            float(&mut m, a, 50.0, (0.0, 100.0), "", 0);
                        }
                        _ => {
                            let c = if let crate::frame::Fill::Color(c) = fill { c } else { "#000000".into() };
                            plain(&mut m, "color", json!(c), json!("#000000"));
                        }
                    }
                }
                // only a picture of another shape than the frame has something to reframe
                T::Reframe if crate::frame::other_shape(cx.s, q, it) => {
                    plain(&mut m, "bool", json!(it.effect("auto_reframe").is_some()), json!(false))
                }
                T::Reframe => return None,
                // curves are for pictures (their sound goes along)
                T::Curve if track_kind(q, c) == Some(TrackKind::Video) => {
                    let id = crate::speed::curve_of(q, c).map_or("none".to_string(), |x| x.0);
                    plain(&mut m, "choice", json!(id), json!("none"));
                    let opts: Vec<&str> = std::iter::once("none").chain(crate::speed::CURVES.iter().map(|c| c.id)).collect();
                    m.insert("options".into(), json!(opts));
                }
                T::Curve => return None,
                T::DuckDb => {
                    let db = it.essential.as_ref().map_or(-15.0, |e| e.ducking.reduce_db);
                    float(&mut m, db, -15.0, (-40.0, 0.0), "dB", 1);
                }
                _ => return None,
            }
        }
        Subject::Caption(c) => {
            let (tid, cap) = caption(q, c)?;
            let style = &q.caption_tracks.iter().find(|t| t.id == tid)?.style;
            m.insert("caption".into(), json!(c.0));
            match row.t {
                T::CaptionText => plain(&mut m, "text", json!(cap.text), json!("")),
                T::CaptionStart => float(&mut m, secs(cap.start), 0.0, (0.0, secs(q.duration()).max(secs(cap.end()))), "s", 2),
                T::CaptionEnd => float(&mut m, secs(cap.end()), 0.0, (0.0, secs(q.duration()).max(secs(cap.end()))), "s", 2),
                T::CaptionStyle(k) => {
                    let d = filmcraft_project::CaptionStyle::default();
                    match k {
                        "font" => plain(&mut m, "text", json!(style.font), json!(d.font)),
                        "size" => float(&mut m, style.size as f64, d.size as f64, (4.0, 400.0), "px", 0),
                        "color" => plain(&mut m, "color", json!(hex8(style.color)), json!(hex8(d.color))),
                        "background" => plain(&mut m, "bool", json!(style.background), json!(d.background)),
                        "backgroundColor" => plain(&mut m, "color", json!(hex8(style.background_color)), json!(hex8(d.background_color))),
                        "outlineColor" => plain(&mut m, "color", json!(hex8(style.outline_color)), json!(hex8(d.outline_color))),
                        "margin" => float(&mut m, style.margin as f64, d.margin as f64, (0.0, 0.45), "", 2),
                        "outline" => float(&mut m, style.outline as f64, d.outline as f64, (0.0, 40.0), "px", 1),
                        "align" | "anchor" => {
                            let (opts, value) = if k == "align" {
                                (["left", "center", "right"], format!("{:?}", style.align).to_lowercase())
                            } else {
                                (["top", "middle", "bottom"], format!("{:?}", style.anchor).to_lowercase())
                            };
                            plain(&mut m, "choice", json!(value), json!(opts[if k == "align" { 1 } else { 2 }]));
                            m.insert("options".into(), json!(opts));
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
        Subject::Transition(id) => {
            let x = transition(q, id)?;
            m.insert("transition".into(), json!(id.0));
            match row.t {
                T::TransitionDuration => {
                    let frame = secs(q.settings.frame_rate.frame_duration());
                    float(&mut m, secs(x.duration), 1.0, (frame, 60.0), "s", 2);
                }
                _ => return None,
            }
        }
    }
    Some(Value::Object(m))
}

fn basis_json(cx: &Ctx, subject: Subject, kind: Kind) -> Value {
    let sections: Vec<Value> = kind
        .sections()
        .iter()
        .filter_map(|sec| {
            let rows: Vec<Value> = sec.rows.iter().filter_map(|row| row_json(cx, subject, sec, row)).collect();
            (!rows.is_empty()).then(|| json!({"key": sec.key, "label": sec.label, "rows": rows}))
        })
        .collect();
    json!(sections)
}

/// Every effect of a clip with every parameter (Mehr), intrinsic ones first as FilmCraft shows
/// them. Hidden effects (BR's markers) are left out.
fn effects_json(it: &TrackItem, t: Tick) -> Value {
    let mut order: Vec<usize> = (0..it.effects.len()).collect();
    order.sort_by_key(|i| !it.effects[*i].def().is_some_and(|d| d.intrinsic));
    let list: Vec<Value> = order
        .into_iter()
        .filter_map(|i| {
            let e = &it.effects[i];
            let d = e.def()?;
            let params: Vec<Value> = d
                .params
                .iter()
                .map(|pd| {
                    let mut m = param_json(it, Some(i), pd, t, None, true);
                    m.insert("id".into(), json!(pd.id));
                    m.insert("label".into(), json!(pd.label));
                    Value::Object(m)
                })
                .collect();
            Some(json!({
                "index": i,
                "effect": e.effect,
                "name": d.name,
                "enabled": e.enabled,
                "intrinsic": d.intrinsic,
                "layer": filmcraft_project::graphic::is_layer(e),
                "adjust": adjust_index(it) == Some(i),
                "masks": e.masks.len(),
                "params": params,
            }))
        })
        .collect();
    json!(list)
}

/// `br.video.inspect`.
pub fn inspect(s: &Session, p: &Value) -> Result<Value, String> {
    let Some(q) = s.active_sequence() else { return Ok(json!({"kind": "none"})) };
    let Some(subject) = subject(s, p) else { return Ok(json!({"kind": "none"})) };
    let t = match p.get("seconds").and_then(Value::as_f64) {
        Some(x) if x.is_finite() => ticks(x.max(0.0)),
        _ => s.playhead(),
    };
    let mehr = matches!(p.get("level").and_then(Value::as_str), Some("mehr" | "all" | "full"));
    let cx = Ctx { s, q, t, layer: layer_choice(s) };
    let mut out = match subject {
        Subject::Clip(c) => {
            let (_, it) = q.find_item(c).ok_or("no such clip")?;
            let kind = kind_of(s, q, c).ok_or("no such clip")?;
            let mut o = json!({
                "kind": kind.key(),
                "label": kind.label(),
                "clip": c.0,
                "name": it.name,
                "start": secs(it.start),
                "end": secs(it.end()),
                "sections": basis_json(&cx, subject, kind),
            });
            if let Some((i, _)) = layer(it, cx.layer) {
                o["layer"] = json!(layer_ordinal(it, i));
                o["layers"] = json!(it.graphic_layers().count());
            }
            let linked: Vec<u64> = linked_audio(q, c).into_iter().map(|a| a.0).collect();
            o["linked"] = json!(linked);
            if mehr {
                o["effects"] = effects_json(it, t);
            }
            o
        }
        Subject::Caption(c) => {
            let (tid, cap) = caption(q, c).ok_or("no such caption")?;
            json!({"kind": "caption", "label": Kind::Caption.label(), "caption": c.0, "track": tid.0, "start": secs(cap.start), "end": secs(cap.start + cap.duration), "sections": basis_json(&cx, subject, Kind::Caption)})
        }
        Subject::Transition(id) => {
            let x = transition(q, id).ok_or("no such transition")?;
            json!({"kind": "transition", "label": Kind::Transition.label(), "transition": id.0, "effect": x.effect.effect, "name": x.effect.def().map_or("", |d| d.name), "start": secs(x.start), "end": secs(x.end()), "sections": basis_json(&cx, subject, Kind::Transition)})
        }
    };
    out["seconds"] = json!(secs(t));
    Ok(out)
}

/// The Basis row `key` (`section.row`) of a kind.
fn find_row(kind: Kind, key: &str) -> Option<(&'static Section, &'static Row)> {
    let (sec, row) = key.split_once('.')?;
    kind.sections().iter().find(|s| s.key == sec).and_then(|s| s.rows.iter().find(|r| r.key == row).map(|r| (*s, r)))
}

fn row_keys(kind: Kind) -> Vec<String> {
    kind.sections().iter().flat_map(|s| s.rows.iter().map(move |r| format!("{}.{}", s.key, r.key))).collect()
}

/// Run `f` as one undo step `label`. With a `gesture`, the steps of one drag fold into its first
/// (as FilmCraft's merge does, but on the history, so any command can take part).
pub(crate) fn step<X>(s: &mut Session, label: &str, gesture: Option<&str>, f: impl FnOnce(&mut Session) -> R<X>) -> R<X> {
    let key = gesture.map(|g| format!("br.gesture:{g}"));
    let merge = key.is_some() && s.history.merge_key == key && s.history.undo.last().is_some_and(|u| u.0 == label);
    let top = s.history.undo.last().map(|u| Arc::as_ptr(&u.1));
    let r = s.edit_group(label, f)?;
    let n = s.history.undo.len();
    if merge && n >= 2 && Some(Arc::as_ptr(&s.history.undo[n - 2].1)) == top && s.history.undo[n - 1].0 == label {
        // the gesture's first step keeps the project from before the drag
        s.history.undo.pop();
    }
    if key.is_some() {
        s.history.merge_key = key;
    }
    Ok(r)
}

/// The effect a `br.video.set` / `keyframe` / `effect` names: `adjust`, an index or an effect id
/// (its first instance).
fn fx_param(p: &Value) -> Result<Option<Fx>, String> {
    Ok(match p.get("effect") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s == "adjust" => Some(Fx::Adjust),
        Some(Value::String(s)) => Some(Fx::Id(filmcraft_project::effect::find_effect_by_name(s).ok_or_else(|| format!("unknown effect `{s}`"))?.id)),
        Some(v) => Some(Fx::Index(v.as_u64().ok_or("`effect` is an index, an effect id or \"adjust\"")? as usize)),
    })
}

/// Put the effect `fx` on clip `c` if it is missing (a crop, the grade): its index. Part of the
/// caller's step.
fn ensure(s: &mut Session, c: ClipId, fx: Fx, layer_n: usize) -> R<usize> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(c).ok_or_else(|| other("no such clip"))?;
    if let Some(i) = fx_index(it, fx, layer_n) {
        return Ok(i);
    }
    match fx {
        Fx::Id(id) => {
            s.execute("effects.apply", json!({"clips": [c.0], "effect": id}))?;
        }
        Fx::Adjust => {
            let def = filmcraft_project::find_effect("lumetri").ok_or_else(|| other("no Lumetri"))?;
            s.edit_sequence("Anpassen", |q, _, _| {
                let (_, it) = q.find_item_mut(c).ok_or_else(|| other("no such clip"))?;
                it.effects.insert(0, def.instance());
                Ok(())
            })?;
        }
        Fx::Layer | Fx::Index(_) => return Err(other("the clip has no such effect")),
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(c).ok_or_else(|| other("no such clip"))?;
    fx_index(it, fx, layer_n).ok_or_else(|| other("the effect could not be added to this clip"))
}

/// What `br.video.set` and `br.video.keyframe` change: a Basis row or an effect's parameter.
struct Target {
    subject: Subject,
    /// The clip the change lands on (a linked row's sound).
    clip: Option<ClipId>,
    label: String,
    t: T,
    slider: Option<(f64, f64)>,
}

fn target(s: &Session, p: &Value, row: Option<&str>) -> Result<Target, String> {
    let q = s.active_sequence().ok_or("no video yet")?;
    let subject = subject(s, p).ok_or("nothing selected: give `clip`, `caption` or `transition`")?;
    if let Some(key) = row {
        let kind = match subject {
            Subject::Clip(c) => kind_of(s, q, c).ok_or("no such clip")?,
            Subject::Caption(_) => Kind::Caption,
            Subject::Transition(_) => Kind::Transition,
        };
        let (_, r) = find_row(kind, key).ok_or_else(|| format!("{} has no row `{key}`: {}", kind.label(), row_keys(kind).join(", ")))?;
        let clip = match subject {
            Subject::Clip(c) if r.linked => Some(linked_audio(q, c).ok_or("the clip has no sound")?),
            Subject::Clip(c) => Some(c),
            _ => None,
        };
        return Ok(Target { subject, clip, label: r.label.to_string(), t: r.t, slider: r.slider });
    }
    let Subject::Clip(c) = subject else { return Err("give `row` (see br.video.inspect)".into()) };
    let fx = fx_param(p)?.ok_or("give `row`, or `effect` and `param`")?;
    let pid = p.get("param").and_then(Value::as_str).ok_or("give `param`")?;
    let (_, it) = q.find_item(c).ok_or("no such clip")?;
    let def = fx_def(it, fx, layer_choice(s)).ok_or("the clip has no such effect")?;
    let pd = def
        .param(pid)
        .ok_or_else(|| format!("{} has no parameter `{pid}`: {}", def.name, def.params.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")))?;
    Ok(Target { subject, clip: Some(c), label: pd.label.to_string(), t: T::P(fx, pd.id), slider: None })
}

/// The time a command means: `seconds`, else the playhead.
fn time(s: &Session, p: &Value) -> Tick {
    match p.get("seconds").and_then(Value::as_f64) {
        Some(x) if x.is_finite() => ticks(x.max(0.0)),
        _ => s.playhead(),
    }
}

fn want_f64(v: &Value, what: &str) -> R<f64> {
    v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| other(format!("{what} takes a number")))
}

fn want_bool(v: &Value, what: &str) -> R<bool> {
    v.as_bool().ok_or_else(|| other(format!("{what} takes true or false")))
}

/// `br.video.set`.
pub fn set(s: &mut Session, p: &Value) -> Result<Value, String> {
    let row = p.get("row").and_then(Value::as_str);
    let tg = target(s, p, row)?;
    let value = p.get("value").cloned().unwrap_or(Value::Null);
    let keyframe = p.get("keyframe").and_then(Value::as_bool).unwrap_or(false);
    let gesture = p.get("gesture").and_then(Value::as_str);
    let t = time(s, p);
    let layer_n = layer_choice(s);
    let label = tg.label.clone();
    // checked before anything changes
    let input = match tg.t {
        T::P(fx, pid) => {
            let q = s.active_sequence().ok_or("no video yet")?;
            let c = tg.clip.ok_or("no clip")?;
            let (_, it) = q.find_item(c).ok_or("no such clip")?;
            let pd = fx_def(it, fx, layer_n).and_then(|d| d.param(pid)).ok_or("the clip has no such parameter")?;
            if keyframe && !pd.animatable {
                return Err(format!("{} cannot be animated", tg.label));
            }
            Some(param_input(pd, &value)?)
        }
        _ if keyframe => return Err(format!("{} cannot be animated", tg.label)),
        _ => None,
    };
    let r = step(s, &label, gesture, |s| apply(s, &tg, value, input, keyframe, t, layer_n)).map_err(|e| e.to_string())?;
    let mut out = match row {
        Some(key) => {
            let q = s.active_sequence().ok_or("no video yet")?;
            let kind = match tg.subject {
                Subject::Clip(c) => kind_of(s, q, c),
                Subject::Caption(_) => Some(Kind::Caption),
                Subject::Transition(_) => Some(Kind::Transition),
            };
            let cx = Ctx { s, q, t, layer: layer_n };
            kind.and_then(|k| find_row(k, key)).and_then(|(sec, row)| row_json(&cx, tg.subject, sec, row)).unwrap_or(Value::Null)
        }
        None => param_after(s, &tg, t, layer_n).unwrap_or(Value::Null),
    };
    if let (Some(o), Some(k)) = (out.as_object_mut(), r) {
        o.insert("keyframesWritten".into(), json!(k));
    }
    Ok(out)
}

/// An effect parameter as it is after a change.
fn param_after(s: &Session, tg: &Target, t: Tick, layer_n: usize) -> Option<Value> {
    let T::P(fx, pid) = tg.t else { return None };
    let q = s.active_sequence()?;
    let (_, it) = q.find_item(tg.clip?)?;
    let pd = fx_def(it, fx, layer_n)?.param(pid)?;
    let mut m = param_json(it, fx_index(it, fx, layer_n), pd, t, tg.slider, true);
    m.insert("id".into(), json!(pd.id));
    m.insert("label".into(), json!(pd.label));
    Some(Value::Object(m))
}

/// Make the change of `br.video.set` (inside its step): `Some(n)` keyframes for a parameter.
fn apply(s: &mut Session, tg: &Target, value: Value, input: Option<Value>, keyframe: bool, t: Tick, layer_n: usize) -> R<Option<usize>> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let fps_frames = |sec: f64| {
        let fd = q.settings.frame_rate.frame_duration().0.max(1) as f64;
        ((ticks(sec).0 as f64 / fd).round() as i64).max(1)
    };
    match (tg.subject, tg.t) {
        (_, T::P(fx, pid)) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            let at = {
                let (_, it) = q.find_item(c).ok_or_else(|| other("no such clip"))?;
                inside(it, t)
            };
            let idx = ensure(s, c, fx, layer_n)?;
            let r =
                s.execute("effects.setParam", json!({"clip": c.0, "effect": idx, "param": pid, "value": input, "time": at.0, "keyframe": keyframe}))?;
            Ok(r.get("keyframes").and_then(Value::as_u64).map(|n| n as usize))
        }
        (Subject::Clip(_), T::Speed) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            let x = want_f64(&value, "Tempo")?.clamp(0.01, 100.0);
            magnet::speed(s, &[c], x)?;
            Ok(None)
        }
        (Subject::Clip(_), T::Reverse) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            let on = want_bool(&value, "Rückwärts")?;
            let (_, it) = q.find_item(c).ok_or_else(|| other("no such clip"))?;
            if it.reverse != on {
                if track_kind(q, c) == Some(TrackKind::Video) {
                    // as Tempo › Rückwärts does it: with its sound, a speed curve as a whole
                    crate::speed::reverse(s, &json!({"clips": [c.0]})).map_err(other)?;
                } else {
                    s.execute("clip.speedDuration", json!({"clips": [c.0], "speed": it.speed.abs() * 100.0, "reverse": on, "ripple": false}))?;
                }
            }
            Ok(None)
        }
        (Subject::Clip(_), T::Gain) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            let db = want_f64(&value, "Lautstärke")?.clamp(-96.0, 96.0);
            s.execute("clip.audioGain", json!({"clips": [c.0], "mode": "set", "db": db}))?;
            Ok(None)
        }
        (Subject::Clip(_), T::FadeIn | T::FadeOut) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            let incoming = matches!(tg.t, T::FadeIn);
            let (_, it) = q.find_item(c).ok_or_else(|| other("no such clip"))?;
            let sec = want_f64(&value, &tg.label)?.clamp(0.0, secs(it.duration));
            let kind = track_kind(q, c).unwrap_or(TrackKind::Audio);
            match fade(q, c, incoming).map(|x| x.id) {
                Some(id) if sec <= 0.0 => {
                    s.execute("sequence.removeTransition", json!({"transitions": [id.0]}))?;
                }
                Some(id) => {
                    s.execute("sequence.setTransition", json!({"transition": id.0, "frames": fps_frames(sec)}))?;
                }
                None if sec > 0.0 => {
                    let cmd = if kind == TrackKind::Video { "sequence.applyVideoTransition" } else { "sequence.applyAudioTransition" };
                    s.execute(cmd, json!({"clip": c.0, "edge": if incoming { "in" } else { "out" }, "frames": fps_frames(sec)}))?;
                }
                None => {}
            }
            Ok(None)
        }
        (Subject::Clip(_), T::Has(id)) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            let on = want_bool(&value, &tg.label)?;
            let (_, it) = q.find_item(c).ok_or_else(|| other("no such clip"))?;
            let present: Vec<usize> = it.effects.iter().enumerate().filter(|(_, e)| e.effect == id).map(|(i, _)| i).collect();
            if on && present.is_empty() {
                s.execute("effects.apply", json!({"clips": [c.0], "effect": id}))?;
            } else if !on {
                for i in present.into_iter().rev() {
                    s.execute("effects.remove", json!({"clip": c.0, "index": i}))?;
                }
            }
            Ok(None)
        }
        (Subject::Clip(_), T::Fit | T::Fill) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            s.execute(if matches!(tg.t, T::Fit) { "clip.fitToFrame" } else { "clip.fillFrame" }, json!({"clips": [c.0]}))?;
            Ok(None)
        }
        (Subject::Clip(_), T::Text) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            let text = value.as_str().ok_or_else(|| other("Text takes text"))?;
            let (_, it) = q.find_item(c).ok_or_else(|| other("no such clip"))?;
            let (i, _) = layer(it, layer_n).ok_or_else(|| other("the clip has no text"))?;
            s.execute("graphics.setText", json!({"clip": c.0, "layer": layer_ordinal(it, i), "text": text}))?;
            Ok(None)
        }
        (Subject::Clip(_), T::BgMode | T::BgAmount | T::BgColor) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            let p = match tg.t {
                T::BgMode => json!({"clip": c.0, "mode": value.as_str().ok_or_else(|| other("Hintergrund: none, blur or color"))?}),
                T::BgAmount => json!({"clip": c.0, "mode": "blur", "amount": want_f64(&value, &tg.label)?}),
                _ => {
                    let col = color_in(&value).ok_or_else(|| other("Farbe takes #rrggbb"))?;
                    json!({"clip": c.0, "mode": "color", "color": hex([col[0], col[1], col[2], 1.0])})
                }
            };
            crate::frame::background(s, &p).map_err(other)?;
            Ok(None)
        }
        (Subject::Clip(_), T::Curve) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            let id = value.as_str().ok_or_else(|| other("Kurve: none, montage, held, bullet, sprung, blitzRein, blitzRaus"))?;
            crate::speed::curve(s, &json!({"clips": [c.0], "preset": id})).map_err(other)?;
            Ok(None)
        }
        (Subject::Clip(_), T::Reframe) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            crate::frame::reframe(s, &json!({"clip": c.0, "on": want_bool(&value, &tg.label)?})).map_err(other)?;
            Ok(None)
        }
        (Subject::Clip(_), T::Ducking) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            s.execute("essentialSound.set", json!({"clips": [c.0], "key": "ducking.enabled", "value": want_bool(&value, &tg.label)?}))?;
            Ok(None)
        }
        (Subject::Clip(_), T::DuckDb) => {
            let c = tg.clip.ok_or_else(|| other("no clip"))?;
            s.execute(
                "essentialSound.set",
                json!({"clips": [c.0], "key": "ducking.reduceDb", "value": want_f64(&value, &tg.label)?.clamp(-40.0, 0.0)}),
            )?;
            Ok(None)
        }
        (Subject::Caption(c), T::CaptionText) => {
            s.execute("captions.setText", json!({"caption": c.0, "text": value.as_str().ok_or_else(|| other("Text takes text"))?}))?;
            Ok(None)
        }
        (Subject::Caption(c), T::CaptionStart | T::CaptionEnd) => {
            let (_, cap) = caption(q, c).ok_or_else(|| other("no such caption"))?;
            let (mut a, mut b) = (secs(cap.start), secs(cap.start + cap.duration));
            let x = want_f64(&value, &tg.label)?.max(0.0);
            if matches!(tg.t, T::CaptionStart) {
                a = x
            } else {
                b = x
            }
            s.execute("captions.setTimes", json!({"caption": c.0, "startSeconds": a, "endSeconds": b}))?;
            Ok(None)
        }
        (Subject::Caption(c), T::CaptionStyle(k)) => {
            let (tid, _) = caption(q, c).ok_or_else(|| other("no such caption"))?;
            let v = match k {
                "font" => json!(value.as_str().ok_or_else(|| other("Schrift takes text"))?),
                "size" | "margin" | "outline" => json!(want_f64(&value, &tg.label)?),
                "background" => json!(want_bool(&value, &tg.label)?),
                "color" | "backgroundColor" | "outlineColor" => {
                    let c = color_in(&value).ok_or_else(|| other(format!("{} takes #rrggbb[aa]", tg.label)))?;
                    json!(hex(c))
                }
                _ => {
                    let v = value.as_str().ok_or_else(|| other(format!("{} takes a name", tg.label)))?.to_ascii_lowercase();
                    json!(v)
                }
            };
            s.execute("captions.setStyle", json!({"track": tid.0, k: v}))?;
            Ok(None)
        }
        (Subject::Transition(id), T::TransitionDuration) => {
            let sec = want_f64(&value, &tg.label)?;
            s.execute("sequence.setTransition", json!({"transition": id.0, "frames": fps_frames(sec)}))?;
            Ok(None)
        }
        _ => Err(other(format!("{} cannot be set here", tg.label))),
    }
}

/// The parameters a `br.video.keyframe` works on: (clip, effect, parameter id).
fn keyframe_targets(s: &Session, p: &Value) -> Result<Vec<(ClipId, Fx, &'static str, String)>, String> {
    let rows: Vec<String> = match p.get("row") {
        Some(Value::String(r)) => vec![r.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    if rows.is_empty() {
        let tg = target(s, p, None)?;
        if let (T::P(fx, pid), Some(c)) = (tg.t, tg.clip) {
            out.push((c, fx, pid, tg.label));
        }
    }
    for key in rows {
        let tg = target(s, p, Some(&key))?;
        match (tg.t, tg.clip) {
            (T::P(fx, pid), Some(c)) => out.push((c, fx, pid, tg.label)),
            _ => return Err(format!("{} has no keyframes", tg.label)),
        }
    }
    Ok(out)
}

fn interpolation(name: &str) -> Option<filmcraft_project::Interpolation> {
    use filmcraft_project::Interpolation as I;
    Some(match name.to_ascii_lowercase().replace(['_', '-', ' '], "").as_str() {
        "linear" => I::Linear,
        "hold" => I::Hold,
        "bezier" => I::Bezier,
        "autobezier" => I::AutoBezier,
        "continuousbezier" => I::ContinuousBezier,
        "easein" => I::EaseIn,
        "easeout" => I::EaseOut,
        _ => return None,
    })
}

/// `br.video.keyframe`.
pub fn keyframe(s: &mut Session, p: &Value) -> Result<Value, String> {
    let op = p.get("op").and_then(Value::as_str).unwrap_or("toggle");
    let targets = keyframe_targets(s, p)?;
    if targets.is_empty() {
        return Err("nothing to animate".into());
    }
    let t = time(s, p);
    let layer_n = layer_choice(s);
    let q = s.active_sequence().ok_or("no video yet")?;
    for (c, fx, pid, label) in &targets {
        let (_, it) = q.find_item(*c).ok_or("no such clip")?;
        if fx_def(it, *fx, layer_n).and_then(|d| d.param(pid)).is_none_or(|d| !d.animatable) {
            return Err(format!("{label} cannot be animated"));
        }
    }
    // the media time of `t` in each clip, and whether a keyframe sits there
    let state = |s: &Session| -> Vec<(Tick, bool, Option<Tick>, Option<Tick>)> {
        let q = s.active_sequence();
        targets
            .iter()
            .map(|(c, fx, pid, _)| {
                let Some((_, it)) = q.and_then(|q| q.find_item(*c)) else { return (Tick::ZERO, false, None, None) };
                let mt = it.source_time_at(inside(it, t));
                let prm = fx_index(it, *fx, layer_n).and_then(|i| it.effects[i].params.get(*pid));
                let here = prm.is_some_and(|p| p.keyframes.iter().any(|k| k.time == mt));
                let prev = prm.and_then(|p| p.prev_keyframe(mt)).map(|k| timeline_at(it, k));
                let next = prm.and_then(|p| p.next_keyframe(mt)).map(|k| timeline_at(it, k));
                (mt, here, prev, next)
            })
            .collect()
    };
    let now = state(s);
    if op == "prev" || op == "next" {
        let to = if op == "prev" { now.iter().filter_map(|x| x.2).max() } else { now.iter().filter_map(|x| x.3).min() };
        return match to {
            Some(to) => {
                s.set_playhead(to);
                Ok(json!({"seconds": secs(to)}))
            }
            None => Ok(json!({"seconds": Value::Null})),
        };
    }
    let all_here = now.iter().all(|x| x.1);
    let (label, add) = match op {
        "toggle" if all_here => ("Keyframe entfernen", false),
        "toggle" | "add" => ("Keyframe setzen", true),
        "remove" => ("Keyframe entfernen", false),
        "clear" => ("Animation entfernen", false),
        "ease" => ("Keyframe-Interpolation", false),
        other => return Err(format!("unknown op `{other}`: toggle, add, remove, clear, ease, prev or next")),
    };
    let interp = match op {
        "ease" => {
            let name = p.get("interpolation").and_then(Value::as_str).unwrap_or("bezier");
            Some(
                interpolation(name)
                    .ok_or_else(|| format!("unknown interpolation `{name}`: linear, hold, bezier, autoBezier, continuousBezier, easeIn, easeOut"))?,
            )
        }
        _ => None,
    };
    let every = p.get("all").and_then(Value::as_bool).unwrap_or(false);
    let defs: Vec<&'static ParamDef> = targets
        .iter()
        .filter_map(|(c, fx, pid, _)| s.active_sequence()?.find_item(*c).and_then(|(_, it)| fx_def(it, *fx, layer_n)?.param(pid)))
        .collect();
    step(s, label, None, |s| {
        let mut idx = Vec::new();
        for (c, fx, _, _) in &targets {
            idx.push(if add {
                Some(ensure(s, *c, *fx, layer_n)?)
            } else {
                s.active_sequence().and_then(|q| q.find_item(*c)).and_then(|(_, it)| fx_index(it, *fx, layer_n))
            });
        }
        s.edit_sequence(label, |q, _, _| {
            for (((c, _, pid, _), i), (mt, _, _, _)) in targets.iter().zip(&idx).zip(&now) {
                let Some(i) = *i else { continue };
                let Some((_, it)) = q.find_item_mut(*c) else { continue };
                let Some(e) = it.effects.get_mut(i) else { continue };
                if !e.params.contains_key(*pid)
                    && let Some(d) = defs.iter().find(|d| d.id == *pid)
                {
                    e.params.insert(pid.to_string(), filmcraft_project::Param::new(d.default.clone()));
                }
                let Some(prm) = e.params.get_mut(*pid) else { continue };
                match op {
                    "toggle" | "add" if add => {
                        if !prm.keyframes.iter().any(|k| k.time == *mt) {
                            prm.add_keyframe(*mt);
                        }
                    }
                    "toggle" | "remove" => {
                        prm.remove_keyframe_at(*mt);
                    }
                    "clear" => {
                        if prm.is_animated() {
                            prm.value = prm.value_at(*mt);
                            prm.keyframes.clear();
                        }
                    }
                    _ => {
                        let Some(kind) = interp else { continue };
                        for k in prm.keyframes.iter_mut().filter(|k| every || k.time == *mt) {
                            k.interp = kind;
                        }
                    }
                }
            }
            Ok(())
        })
    })
    .map_err(|e| e.to_string())?;
    let after = state(s);
    Ok(json!({"keyframeHere": after.iter().all(|x| x.1), "seconds": secs(t)}))
}

/// `br.video.effect`.
pub fn effect(s: &mut Session, p: &Value) -> Result<Value, String> {
    let op = p.get("op").and_then(Value::as_str).unwrap_or("add");
    if op == "add" && p.get("clips").is_some_and(Value::is_array) {
        return add_to_clips(s, p);
    }
    let Some(Subject::Clip(c)) = subject(s, p) else { return Err("select a clip (or give `clip`)".into()) };
    let q = s.active_sequence().ok_or("no video yet")?;
    if op == "add" {
        let name = p.get("effect").and_then(Value::as_str).ok_or("give `effect` (an id or name, see br.video.schema)")?;
        let def = filmcraft_project::effect::find_effect_by_name(name).ok_or_else(|| format!("unknown effect `{name}`"))?;
        let on_video = track_kind(q, c) == Some(TrackKind::Video);
        let target = match def.kind {
            filmcraft_project::EffectKind::Audio if on_video => linked_audio(q, c).ok_or("an audio effect needs a clip with sound")?,
            filmcraft_project::EffectKind::Video if !on_video => return Err("a video effect needs a video clip".into()),
            filmcraft_project::EffectKind::Video | filmcraft_project::EffectKind::Audio => c,
            _ => return Err(format!("{} is a transition: br.video.transition", def.name)),
        };
        let before: Vec<EffectInstance> = q.find_item(target).map(|(_, it)| it.effects.clone()).unwrap_or_default();
        step(s, &format!("Effekt {}", def.name), None, |s| s.execute("effects.apply", json!({"clips": [target.0], "effect": def.id})))
            .map_err(|e| e.to_string())?;
        let after: Vec<EffectInstance> = s.active_sequence().and_then(|q| q.find_item(target)).map(|(_, it)| it.effects.clone()).unwrap_or_default();
        let index = (0..after.len()).find(|i| before.get(*i) != after.get(*i)).unwrap_or(after.len().saturating_sub(1));
        return Ok(json!({"clip": target.0, "index": index, "effect": def.id}));
    }
    // the clip the effect is on: the clip itself, else its sound (Mehr shows both)
    let on = p.get("on").and_then(Value::as_u64).map(ClipId).unwrap_or(c);
    let (_, it) = q.find_item(on).ok_or("no such clip")?;
    let index = match fx_param(p)? {
        Some(fx) => fx_index(it, fx, layer_choice(s)).ok_or("the clip has no such effect")?,
        None => return Err("give `effect` (an index or an effect id)".into()),
    };
    let e = &it.effects[index];
    let name = e.def().map_or(e.effect.as_str(), |d| d.name).to_string();
    let r = match op {
        "remove" => {
            if e.def().is_some_and(|d| d.intrinsic) {
                return Err(format!("{name} belongs to every clip and cannot be removed"));
            }
            step(s, &format!("{name} entfernen"), None, |s| s.execute("effects.remove", json!({"clip": on.0, "index": index})))
        }
        "enable" | "disable" => {
            if e.enabled == (op == "enable") {
                return Ok(json!({"clip": on.0, "index": index, "enabled": e.enabled}));
            }
            step(s, &format!("{name} {}", if op == "enable" { "an" } else { "aus" }), None, |s| {
                s.execute("effects.toggleEnabled", json!({"clip": on.0, "index": index}))
            })
        }
        "reset" => match p.get("param").and_then(Value::as_str) {
            Some(param) => step(s, &format!("{name} zurücksetzen"), None, |s| {
                s.execute("effects.resetParam", json!({"clip": on.0, "effect": index, "param": param}))
            }),
            None => step(s, &format!("{name} zurücksetzen"), None, |s| s.execute("effects.reset", json!({"clip": on.0, "index": index}))),
        },
        other => return Err(format!("unknown op `{other}`: add, remove, enable, disable or reset")),
    };
    r.map_err(|e| e.to_string())?;
    let enabled = s.active_sequence().and_then(|q| q.find_item(on)).and_then(|(_, it)| it.effects.get(index)).map(|e| e.enabled);
    Ok(json!({"clip": on.0, "index": index, "enabled": enabled}))
}

/// `br.video.effect` add with `clips`: the effect on each of them that it fits (a video effect on
/// the pictures, an audio effect on the sound, a picture's own sound included), in one step.
fn add_to_clips(s: &mut Session, p: &Value) -> Result<Value, String> {
    let name = p.get("effect").and_then(Value::as_str).ok_or("give `effect` (an id or name, see br.video.schema)")?;
    let def = filmcraft_project::effect::find_effect_by_name(name).ok_or_else(|| format!("unknown effect `{name}`"))?;
    let q = s.active_sequence().ok_or("no video yet")?;
    let clips: Vec<ClipId> = p["clips"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(ClipId).collect();
    let mut targets: Vec<ClipId> = Vec::new();
    for c in clips {
        let on_video = track_kind(q, c) == Some(TrackKind::Video);
        let target = match def.kind {
            filmcraft_project::EffectKind::Audio if on_video => linked_audio(q, c),
            filmcraft_project::EffectKind::Audio => Some(c).filter(|c| track_kind(q, *c) == Some(TrackKind::Audio)),
            filmcraft_project::EffectKind::Video => on_video.then_some(c),
            _ => return Err(format!("{} is a transition: br.video.transition", def.name)),
        };
        if let Some(t) = target.filter(|t| !targets.contains(t) && q.find_item(*t).is_some()) {
            targets.push(t);
        }
    }
    if targets.is_empty() {
        return Err(match def.kind {
            filmcraft_project::EffectKind::Audio => "an audio effect needs a clip with sound".into(),
            _ => "a video effect needs a video clip".into(),
        });
    }
    let ids: Vec<u64> = targets.iter().map(|c| c.0).collect();
    step(s, &format!("Effekt {}", def.name), None, |s| s.execute("effects.apply", json!({"clips": ids, "effect": def.id})))
        .map_err(|e| e.to_string())?;
    Ok(json!({"clips": ids, "clip": ids[0], "effect": def.id}))
}

/// `br.video.schema`: FilmCraft's effect definitions (all parameters with type, range, slider
/// range, unit, decimals, default, group and whether they animate).
pub fn schema(p: &Value) -> Result<Value, String> {
    if let Some(name) = p.get("effect").and_then(Value::as_str) {
        let def = filmcraft_project::effect::find_effect_by_name(name).ok_or_else(|| format!("unknown effect `{name}`"))?;
        return serde_json::to_value(def).map_err(|e| e.to_string());
    }
    let kind = p.get("kind").and_then(Value::as_str).map(|k| k.to_ascii_lowercase());
    let list: Vec<Value> = filmcraft_project::effect_defs()
        .iter()
        .filter(|d| !filmcraft_project::graphic::is_layer_id(d.id))
        .filter(|d| {
            kind.as_deref().is_none_or(|k| match d.kind {
                filmcraft_project::EffectKind::Video => k == "video",
                filmcraft_project::EffectKind::Audio => k == "audio",
                filmcraft_project::EffectKind::VideoTransition => k == "videotransition",
                filmcraft_project::EffectKind::AudioTransition => k == "audiotransition",
            })
        })
        .map(|d| {
            if p.get("detail").and_then(Value::as_bool).unwrap_or(false) {
                serde_json::to_value(d).unwrap_or(Value::Null)
            } else {
                json!({"id": d.id, "name": d.name, "kind": format!("{:?}", d.kind), "category": d.category, "intrinsic": d.intrinsic})
            }
        })
        .collect();
    Ok(json!(list))
}
