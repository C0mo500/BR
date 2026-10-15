//! BR's inspector on the Schnitt page: CapCut's right column with DaVinci's depth, in three steps.
//!
//! - *Basis*: the rows that matter for the selected clip's kind (video, picture, text, shape,
//!   sound, music, caption, transition), one tab per area as in CapCut, German labels.
//! - *Mehr*: every effect on the clip (and on its sound) with every parameter of its schema,
//!   switch on/off, reset, remove, and "+ Effekt" from all of FilmCraft's effects.
//! - *Voll*: FilmCraft's own panels (Effect Controls, Properties, Lumetri, Essential Sound,
//!   captions) in the same column.
//!
//! The inspector holds no values: it reads `br.video.inspect` each frame and changes go through
//! `br.video.set` / `keyframe` / `effect`, one undo step each; the frames of a drag and the keys
//! of one typed value are one step (`gesture`). Every control has a widget id (`insp.*`).

use std::collections::HashMap;

use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Ui, vec2};
use filmcraft_ui_egui::dock::PanelKind;
use filmcraft_ui_egui::icons::Icon;
use serde_json::{Value, json};

use crate::BrApp;
use crate::style::{ACCENT, ON_ACCENT, Theme};

/// The column's width when it first opens.
pub const WIDTH: f32 = 350.0;
/// The label column of a row.
const LABEL_W: f32 = 122.0;
const ROW_H: f32 = 22.0;

/// How much the inspector shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    Basis,
    Mehr,
    Voll,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Basis, Level::Mehr, Level::Voll];

    pub fn key(self) -> &'static str {
        match self {
            Level::Basis => "basis",
            Level::Mehr => "mehr",
            Level::Voll => "voll",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Basis => "Basis",
            Level::Mehr => "Mehr",
            Level::Voll => "Voll",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Level::Basis => "Das Wichtigste für diesen Clip",
            Level::Mehr => "Jeder Effekt des Clips mit jedem Parameter",
            Level::Voll => "FilmCrafts eigene Panels: Effekteinstellungen, Eigenschaften, Lumetri, Essential Sound, Untertitel",
        }
    }

    pub fn find(key: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.key() == key)
    }
}

/// FilmCraft's panels Voll shows: (panel, key, label).
const VOLL: [(PanelKind, &str, &str); 5] = [
    (PanelKind::EffectControls, "effects", "Effekte"),
    (PanelKind::Properties, "properties", "Eigenschaften"),
    (PanelKind::LumetriColor, "lumetri", "Lumetri"),
    (PanelKind::EssentialSound, "sound", "Essential Sound"),
    (PanelKind::Text, "captions", "Untertitel"),
];

/// The FilmCraft panel for a kind of clip.
fn panel_for(kind: &str) -> PanelKind {
    match kind {
        "text" | "shape" => PanelKind::Properties,
        "caption" => PanelKind::Text,
        _ => PanelKind::EffectControls,
    }
}

/// What the inspector remembers between frames.
#[derive(Default)]
pub struct InspectorState {
    pub level: Level,
    /// Voll: the panel shown (`None`: the one for the clip's kind).
    pub panel: Option<PanelKind>,
    /// Basis: the tab shown, per kind of clip.
    tab: HashMap<String, String>,
    /// The control being dragged or typed in and its gesture: its changes are one undo step.
    gesture: Option<(String, String)>,
    gestures: u64,
    /// Mehr: the effect picker is open, with its search text.
    picker: Option<String>,
    /// A control of the inspector has the keyboard this frame.
    typing: bool,
}

impl InspectorState {
    /// The gesture of a change to control `key`: the same while one drag or one typed value lasts.
    fn gesture_for(&mut self, key: &str) -> String {
        match &self.gesture {
            Some((k, g)) if k == key => g.clone(),
            _ => {
                self.gestures += 1;
                let g = format!("insp{}", self.gestures);
                self.gesture = Some((key.to_string(), g.clone()));
                g
            }
        }
    }
}

/// The inspector is shown with the timeline: on the Schnitt page, when there is a video.
pub fn visible(app: &BrApp) -> bool {
    crate::timeline::visible(app)
}

/// `br.ui.inspector {level?, panel?}`: switch the inspector (window only).
pub fn control(app: &mut BrApp, p: &Value) -> Result<Value, String> {
    if let Some(l) = p.get("level").and_then(Value::as_str) {
        app.inspector.level = Level::find(l).ok_or_else(|| format!("unknown level `{l}`: basis, mehr or voll"))?;
    }
    if let Some(k) = p.get("panel").and_then(Value::as_str) {
        let (panel, _, _) = VOLL.iter().find(|v| v.1 == k).ok_or_else(|| format!("unknown panel `{k}`: {}", VOLL.map(|v| v.1).join(", ")))?;
        app.inspector.panel = Some(*panel);
        app.inspector.level = Level::Voll;
    }
    Ok(state(app))
}

/// What the inspector shows, for `br.inspect`.
pub fn state(app: &BrApp) -> Value {
    let panel = app.inspector.panel.and_then(|p| VOLL.iter().find(|v| v.0 == p)).map(|v| v.1);
    json!({"level": app.inspector.level.key(), "panel": panel})
}

/// Draw the inspector as a column on the right (after the toolbar and the timeline, before
/// FilmCraft takes the rest).
pub fn show(app: &mut BrApp, ui: &mut Ui) {
    if !visible(app) {
        return;
    }
    let t = Theme::get(ui.ctx());
    egui::Panel::right("br_inspector")
        .resizable(true)
        .default_size(WIDTH)
        .size_range(260.0..=620.0)
        .frame(egui::Frame::NONE.fill(t.bar).stroke(Stroke::new(1.0, t.divider)).inner_margin(egui::Margin::symmetric(10, 8)))
        .show(ui, |ui| {
            app.inspector.typing = false;
            let level = app.inspector.level;
            let p = json!({"level": if level == Level::Mehr { "mehr" } else { "basis" }});
            let v = br_video::inspect::inspect(&app.fc.session, &p).unwrap_or_else(|_| json!({"kind": "none"}));
            header(app, ui, &t, &v);
            ui.add_space(6.0);
            match level {
                Level::Voll => voll(app, ui, &t, &v),
                _ if v["kind"] == "none" => nothing(ui, &t),
                Level::Basis => basis(app, ui, &t, &v),
                Level::Mehr => mehr(app, ui, &t, &v),
            }
            ui.allocate_rect(ui.available_rect_before_wrap(), Sense::hover());
        });
    // a drag or a typed value has ended: the next change is a new undo step
    if !ui.input(|i| i.pointer.any_down()) && !app.inspector.typing {
        app.inspector.gesture = None;
    }
}

/// The title (what is selected) and the Basis / Mehr / Voll switch.
fn header(app: &mut BrApp, ui: &mut Ui, t: &Theme, v: &Value) {
    ui.horizontal(|ui| {
        let title = v["label"].as_str().unwrap_or("Inspector");
        ui.label(RichText::new(title).size(14.0).strong().color(t.text));
        if let Some(name) = v["name"].as_str().filter(|n| !n.is_empty()) {
            let g = crate::style::elided(ui, name, 12.0, t.dim, ui.available_width());
            ui.label(g);
        }
    });
    ui.add_space(4.0);
    egui::Frame::NONE.fill(t.well).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let w = ((ui.available_width() - 4.0) / 3.0).max(40.0);
            for l in Level::ALL {
                let on = app.inspector.level == l;
                let b = egui::Button::new(RichText::new(l.label()).size(12.5).color(if on { ON_ACCENT } else { t.text }))
                    .fill(if on { ACCENT } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(6))
                    .min_size(vec2(w, 24.0));
                let r = ui.add(b).on_hover_text(l.hint());
                app.widgets.insert(format!("insp.level.{}", l.key()), r.rect);
                if r.clicked() {
                    app.inspector.level = l;
                }
            }
        });
    });
}

fn nothing(ui: &mut Ui, t: &Theme) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("Nichts ausgewählt").size(13.0).color(t.text));
        ui.add_space(4.0);
        ui.label(RichText::new("Einen Clip, Titel oder Untertitel in der Timeline anklicken.").size(12.0).color(t.dim));
    });
}

/// What a row changes: a Basis row of the inspected subject, or a parameter of an effect.
#[derive(Clone, Copy)]
enum Addr<'a> {
    Row(&'a str),
    Param { clip: u64, effect: u64, param: &'a str },
}

/// The subject of an inspect answer, as `br.video.*` takes it.
fn subject_of(v: &Value) -> Value {
    for k in ["clip", "caption", "transition"] {
        if let Some(id) = v[k].as_u64() {
            return json!({k: id});
        }
    }
    json!({})
}

fn params(subject: &Value, addr: Addr) -> Value {
    match addr {
        Addr::Row(key) => {
            let mut p = subject.clone();
            p["row"] = json!(key);
            p
        }
        Addr::Param { clip, effect, param } => json!({"clip": clip, "effect": effect, "param": param}),
    }
}

fn basis(app: &mut BrApp, ui: &mut Ui, t: &Theme, v: &Value) {
    let kind = v["kind"].as_str().unwrap_or("").to_string();
    let sections = v["sections"].as_array().cloned().unwrap_or_default();
    let Some(first) = sections.first().and_then(|s| s["key"].as_str()) else { return };
    let current =
        app.inspector.tab.get(&kind).filter(|k| sections.iter().any(|s| s["key"] == k.as_str())).cloned().unwrap_or_else(|| first.to_string());
    // CapCut's tabs: one area at a time
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        for s in &sections {
            let key = s["key"].as_str().unwrap_or("");
            let on = key == current;
            let b = egui::Button::new(RichText::new(s["label"].as_str().unwrap_or(key)).size(12.0).color(if on { t.text } else { t.dim }))
                .fill(if on { t.button } else { Color32::TRANSPARENT })
                .stroke(if on { Stroke::new(1.0, ACCENT) } else { Stroke::NONE })
                .corner_radius(CornerRadius::same(6))
                .min_size(vec2(0.0, 24.0));
            let r = ui.add(b);
            app.widgets.insert(format!("insp.tab.{key}"), r.rect);
            if r.clicked() {
                app.inspector.tab.insert(kind.clone(), key.to_string());
            }
        }
    });
    ui.add_space(2.0);
    ui.separator();
    let Some(sec) = sections.iter().find(|s| s["key"] == current.as_str()) else { return };
    let subject = subject_of(v);
    let rows: Vec<Value> = sec["rows"].as_array().cloned().unwrap_or_default();
    // the area's ◆: a keyframe for every row that animates, at once (CapCut's one button)
    let animated: Vec<&Value> = rows.iter().filter(|r| r["animatable"] == true).collect();
    if animated.len() > 1 {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Keyframe für alle").size(12.0).color(t.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let here = animated.iter().all(|r| r["atKeyframe"] == true);
                let any = animated.iter().any(|r| r["animated"] == true);
                let r = diamond(ui, t, here, any, "Ein Keyframe für jeden Wert dieses Bereichs am Abspielkopf (noch einmal: entfernen)");
                app.widgets.insert(format!("insp.section.{current}.key"), r.rect);
                if r.clicked() {
                    let mut p = subject.clone();
                    p["row"] = json!(animated.iter().filter_map(|r| r["key"].as_str()).collect::<Vec<_>>());
                    run(app, ui.ctx(), "br.video.keyframe", p);
                }
            });
        });
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut i = 0;
        while i < rows.len() {
            // buttons in a row of their own (Einpassen, Füllen)
            if rows[i]["type"] == "action" {
                let mut actions = Vec::new();
                while i < rows.len() && rows[i]["type"] == "action" {
                    actions.push(rows[i].clone());
                    i += 1;
                }
                ui.horizontal(|ui| {
                    for a in &actions {
                        let key = a["key"].as_str().unwrap_or("");
                        let r = crate::style::button(ui, t, a["label"].as_str().unwrap_or(key), false, true);
                        app.widgets.insert(format!("insp.row.{key}"), r.rect);
                        if r.clicked() {
                            let mut p = params(&subject, Addr::Row(key));
                            p["value"] = Value::Null;
                            run(app, ui.ctx(), "br.video.set", p);
                        }
                    }
                });
                continue;
            }
            let key = rows[i]["key"].as_str().unwrap_or("").to_string();
            row_ui(app, ui, t, &subject, &rows[i], Addr::Row(&key), &format!("insp.row.{key}"));
            i += 1;
        }
    });
}

fn mehr(app: &mut BrApp, ui: &mut Ui, t: &Theme, v: &Value) {
    let Some(clip) = v["clip"].as_u64() else {
        ui.label(RichText::new("Untertitel und Übergänge haben keine Effekte: alles steht unter Basis und Voll.").size(12.0).color(t.dim));
        return;
    };
    let kind = v["kind"].as_str().unwrap_or("").to_string();
    let linked: Vec<u64> = v["linked"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
    let effects = v["effects"].clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        effects_ui(app, ui, t, clip, &effects);
        for a in linked {
            let lv = br_video::inspect::inspect(&app.fc.session, &json!({"clip": a, "level": "mehr"})).unwrap_or(Value::Null);
            crate::style::heading(ui, t, "TON");
            effects_ui(app, ui, t, a, &lv["effects"]);
        }
        ui.add_space(8.0);
        picker(app, ui, t, clip, &kind);
    });
}

/// A clip's effects, each to fold open, with every parameter.
fn effects_ui(app: &mut BrApp, ui: &mut Ui, t: &Theme, clip: u64, effects: &Value) {
    for e in effects.as_array().into_iter().flatten() {
        let Some(i) = e["index"].as_u64() else { continue };
        let name = e["name"].as_str().unwrap_or("");
        let title = if e["adjust"] == true { "Anpassen (Lumetri)".to_string() } else { de(name).to_string() };
        let base = format!("insp.fx.{clip}.{i}");
        let id = egui::Id::new(("br_insp_fx", clip, i, e["effect"].as_str().unwrap_or("")));
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        let header = state.show_header(ui, |ui| {
            let on = e["enabled"] == true;
            let r = icon_button(ui, t, if on { Icon::Eye } else { Icon::EyeOff }, if on { "Ausschalten" } else { "Einschalten" }, on);
            app.widgets.insert(format!("{base}.enable"), r.rect);
            if r.clicked() {
                run(app, ui.ctx(), "br.video.effect", json!({"clip": clip, "on": clip, "op": if on { "disable" } else { "enable" }, "effect": i}));
            }
            ui.label(RichText::new(title).size(12.5).strong().color(if on { t.text } else { t.dim }));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if e["intrinsic"] == false {
                    let r = icon_button(ui, t, Icon::Trash, "Effekt entfernen", false);
                    app.widgets.insert(format!("{base}.remove"), r.rect);
                    if r.clicked() {
                        run(app, ui.ctx(), "br.video.effect", json!({"clip": clip, "on": clip, "op": "remove", "effect": i}));
                    }
                }
                let r = icon_button(ui, t, Icon::Reset, "Alle Werte zurücksetzen", false);
                app.widgets.insert(format!("{base}.reset"), r.rect);
                if r.clicked() {
                    run(app, ui.ctx(), "br.video.effect", json!({"clip": clip, "on": clip, "op": "reset", "effect": i}));
                }
            });
        });
        let (toggle, _, _) = header.body(|ui| {
            let mut group: Option<String> = None;
            for p in e["params"].as_array().into_iter().flatten() {
                let g = p["group"].as_str().map(str::to_string);
                if g != group {
                    if let Some(g) = &g {
                        crate::style::heading(ui, t, &de(g).to_uppercase());
                    }
                    group = g;
                }
                let pid = p["id"].as_str().unwrap_or("").to_string();
                row_ui(app, ui, t, &json!({"clip": clip}), p, Addr::Param { clip, effect: i, param: &pid }, &format!("{base}.{pid}"));
            }
        });
        app.widgets.insert(base, toggle.rect);
    }
}

/// "+ Effekt": all of FilmCraft's effects for the clip, to search.
fn picker(app: &mut BrApp, ui: &mut Ui, t: &Theme, clip: u64, kind: &str) {
    let open = app.inspector.picker.is_some();
    let r = crate::style::button(ui, t, "+ Effekt hinzufügen", open, true);
    app.widgets.insert("insp.add".into(), r.rect);
    if r.clicked() {
        app.inspector.picker = if open { None } else { Some(String::new()) };
    }
    let Some(mut query) = app.inspector.picker.clone() else { return };
    let r = ui.add(egui::TextEdit::singleline(&mut query).hint_text("Effekt suchen").desired_width(f32::INFINITY));
    app.widgets.insert("insp.add.search".into(), r.rect);
    if r.has_focus() {
        app.inspector.typing = true;
    }
    app.inspector.picker = Some(query.clone());
    // a sound takes audio effects; a picture both (its sound gets the audio ones)
    let sound = matches!(kind, "sound" | "music");
    let all = br_video::inspect::schema(&json!({})).unwrap_or(Value::Null);
    let q = query.to_lowercase();
    let mut shown = 0;
    for d in all.as_array().into_iter().flatten() {
        let (id, name, k) = (d["id"].as_str().unwrap_or(""), d["name"].as_str().unwrap_or(""), d["kind"].as_str().unwrap_or(""));
        if d["intrinsic"] == true || !(k == "Audio" || (k == "Video" && !sound)) {
            continue;
        }
        if !q.is_empty() && !name.to_lowercase().contains(&q) && !id.contains(&q) && !de(name).to_lowercase().contains(&q) {
            continue;
        }
        let folder = d["category"].as_array().and_then(|c| c.last()).and_then(Value::as_str).unwrap_or("");
        let r = ui.horizontal(|ui| {
            let r = ui.selectable_label(false, RichText::new(de(name)).size(12.5).color(t.text));
            ui.label(RichText::new(folder).size(11.0).color(t.dim));
            r
        });
        app.widgets.insert(format!("insp.add.{id}"), r.inner.rect);
        if r.inner.clicked() {
            run(app, ui.ctx(), "br.video.effect", json!({"clip": clip, "op": "add", "effect": id}));
            app.inspector.picker = None;
            return;
        }
        shown += 1;
        if shown >= 80 {
            ui.label(RichText::new("… weitere: genauer suchen").size(11.0).color(t.dim));
            break;
        }
    }
}

/// Voll: FilmCraft's own panel for the clip, in the column.
fn voll(app: &mut BrApp, ui: &mut Ui, t: &Theme, v: &Value) {
    let kind = v["kind"].as_str().unwrap_or("none");
    let current = app.inspector.panel.unwrap_or(panel_for(kind));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        for (panel, key, label) in VOLL {
            let on = panel == current;
            let b = egui::Button::new(RichText::new(label).size(12.0).color(if on { t.text } else { t.dim }))
                .fill(if on { t.button } else { Color32::TRANSPARENT })
                .stroke(if on { Stroke::new(1.0, ACCENT) } else { Stroke::NONE })
                .corner_radius(CornerRadius::same(6))
                .min_size(vec2(0.0, 24.0));
            let r = ui.add(b);
            app.widgets.insert(format!("insp.voll.{key}"), r.rect);
            if r.clicked() {
                app.inspector.panel = Some(panel);
            }
        }
    });
    ui.add_space(4.0);
    if app.fc.ui.dock.contains(current) {
        // FilmCraft's panels draw with their own ids: never twice in one frame
        ui.label(RichText::new("Dieses Panel ist im Fenster angedockt und wird dort gezeigt.").size(12.0).color(t.dim));
        return;
    }
    let rect = ui.available_rect_before_wrap();
    if ui.rect_contains_pointer(rect) && ui.input(|i| i.pointer.any_pressed()) {
        app.fc.ui.focused = current;
    }
    filmcraft_ui_egui::panels::show(&mut app.fc, ui, current, rect);
}

/// One row: its label, the control for its type, ◆ and reset.
fn row_ui(app: &mut BrApp, ui: &mut Ui, t: &Theme, subject: &Value, row: &Value, addr: Addr, wid: &str) {
    let label = de(row["label"].as_str().unwrap_or("")).to_string();
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_H + 4.0);
        let tip = row["key"].as_str().or(row["id"].as_str()).unwrap_or("").to_string();
        ui.allocate_ui_with_layout(vec2(LABEL_W, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(LABEL_W);
            ui.add(egui::Label::new(RichText::new(&label).size(12.5).color(t.text)).truncate()).on_hover_text(tip);
        });
        let w = (ui.available_width() - 46.0).max(60.0);
        let inner = ui.allocate_ui(vec2(w, ROW_H), |ui| widget(ui, t, row, wid)).inner;
        let (changed, r) = inner;
        app.widgets.insert(wid.to_string(), r.rect);
        if r.has_focus() {
            app.inspector.typing = true;
        }
        if let Some(value) = changed {
            let mut p = params(subject, addr);
            p["value"] = value;
            p["gesture"] = json!(app.inspector.gesture_for(wid));
            run(app, &ctx, "br.video.set", p);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if row["animatable"] == true {
                let r = diamond(ui, t, row["atKeyframe"] == true, row["animated"] == true, "Keyframe am Abspielkopf setzen oder entfernen");
                app.widgets.insert(format!("{wid}.key"), r.rect);
                if r.clicked() {
                    let p = match addr {
                        Addr::Row(_) => params(subject, addr),
                        Addr::Param { clip, effect, param } => json!({"clip": clip, "effect": effect, "param": param}),
                    };
                    run(app, &ctx, "br.video.keyframe", p);
                }
            } else {
                ui.add_space(20.0);
            }
            let changed_from_default = !row["default"].is_null() && row["value"] != row["default"] && row["type"] != "text";
            if changed_from_default {
                let r = icon_button(ui, t, Icon::Reset, "Zurücksetzen", false);
                app.widgets.insert(format!("{wid}.reset"), r.rect);
                if r.clicked() {
                    match addr {
                        Addr::Row(_) => {
                            let mut p = params(subject, addr);
                            p["value"] = row["default"].clone();
                            run(app, &ctx, "br.video.set", p);
                        }
                        Addr::Param { clip, effect, param } => {
                            run(app, &ctx, "br.video.effect", json!({"clip": clip, "on": clip, "op": "reset", "effect": effect, "param": param}));
                        }
                    }
                }
            }
        });
    });
}

/// The control for a row's type: a changed value, and the control's response.
fn widget(ui: &mut Ui, t: &Theme, row: &Value, id: &str) -> (Option<Value>, egui::Response) {
    let v = &row["value"];
    match row["type"].as_str().unwrap_or("") {
        "float" | "angle" => {
            let mut x = v.as_f64().unwrap_or(0.0);
            let before = x;
            let lo = row["slider"][0].as_f64().unwrap_or(-100.0);
            let hi = row["slider"][1].as_f64().filter(|h| *h > lo).unwrap_or(lo + 1.0);
            let decimals = row["decimals"].as_u64().unwrap_or(1) as usize;
            let unit = row["unit"].as_str().unwrap_or("");
            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(40.0);
            let mut s = egui::Slider::new(&mut x, lo..=hi).clamping(egui::SliderClamping::Never).max_decimals(decimals);
            if !unit.is_empty() {
                s = s.suffix(if matches!(unit, "%" | "°" | "×") { unit.to_string() } else { format!(" {unit}") });
            }
            if row["key"] == "speed.speed" {
                s = s.logarithmic(true);
            }
            let r = ui.add(s);
            ((r.changed() && x != before).then(|| json!(x)), r)
        }
        "point" | "wheel" => {
            let (mut x, mut y) = (v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0));
            let wheel = row["type"] == "wheel";
            let w = ((ui.available_width() - 6.0) / 2.0).max(40.0);
            let r1 = ui.add_sized([w, ROW_H], drag_value(&mut x, "X ", wheel));
            let r2 = ui.add_sized([w, ROW_H], drag_value(&mut y, "Y ", wheel));
            let changed = r1.changed() || r2.changed();
            (changed.then(|| json!([x, y])), r1.union(r2))
        }
        "bool" => {
            let mut b = v.as_bool().unwrap_or(false);
            let r = ui.add(egui::Checkbox::without_text(&mut b));
            (r.changed().then(|| json!(b)), r)
        }
        "choice" => {
            let cur = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            let opts: Vec<String> = row["options"].as_array().into_iter().flatten().filter_map(|o| o.as_str().map(str::to_string)).collect();
            let mut pick = None;
            let r = egui::ComboBox::from_id_salt(id)
                .selected_text(de(&cur))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for o in &opts {
                        if ui.selectable_label(*o == cur, de(o)).clicked() && *o != cur {
                            pick = Some(o.clone());
                        }
                    }
                })
                .response;
            (pick.map(|o| json!(o)), r)
        }
        "color" => {
            let mut c = color32(v.as_str().unwrap_or("#000000"));
            let before = c;
            let r = egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend);
            ((c != before).then(|| json!(hex(c))), r)
        }
        "text" => {
            let mut s = v.as_str().unwrap_or("").to_string();
            let long = matches!(row["key"].as_str(), Some("text.text" | "caption.text"));
            let edit = if long { egui::TextEdit::multiline(&mut s).desired_rows(2) } else { egui::TextEdit::singleline(&mut s) };
            let r = ui.add(edit.id_salt(id).desired_width(f32::INFINITY));
            (r.changed().then(|| json!(s)), r)
        }
        _ => {
            // curves, masks: drawn and edited in FilmCraft's panels
            let r = ui.label(RichText::new("unter Voll / Farbe").size(11.5).color(t.dim));
            (None, r)
        }
    }
}

/// A coordinate: pixels, or −1…1 on a colour wheel.
fn drag_value<'a>(n: &'a mut f64, prefix: &'static str, wheel: bool) -> egui::DragValue<'a> {
    let d = egui::DragValue::new(n).prefix(prefix).max_decimals(if wheel { 3 } else { 1 }).speed(if wheel { 0.005 } else { 1.0 });
    if wheel { d.range(-1.0..=1.0) } else { d }
}

/// The keyframe diamond: filled on a keyframe, outlined while the value animates.
fn diamond(ui: &mut Ui, t: &Theme, here: bool, animated: bool, tip: &str) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(20.0, ROW_H), Sense::click());
    let c = rect.center();
    let s = 5.5;
    let pts = vec![egui::pos2(c.x, c.y - s), egui::pos2(c.x + s, c.y), egui::pos2(c.x, c.y + s), egui::pos2(c.x - s, c.y)];
    let col = if here || r.hovered() {
        ACCENT
    } else if animated {
        t.text
    } else {
        t.dim
    };
    let fill = if here { ACCENT } else { Color32::TRANSPARENT };
    ui.painter().add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.3, col)));
    r.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn icon_button(ui: &mut Ui, t: &Theme, icon: Icon, tip: &str, on: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(20.0, ROW_H), Sense::click());
    let col = if r.hovered() || on { ACCENT } else { t.dim };
    filmcraft_ui_egui::icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), icon, col);
    r.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn run(app: &mut BrApp, ctx: &egui::Context, id: &str, p: Value) {
    crate::cut::run(app, ctx, id, p);
}

/// `#rrggbb[aa]` as a colour (straight alpha).
fn color32(hex: &str) -> Color32 {
    let h = hex.trim_start_matches('#');
    let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
    match (b(0), b(2), b(4)) {
        (Some(r), Some(g), Some(bl)) => Color32::from_rgba_unmultiplied(r, g, bl, b(6).unwrap_or(255)),
        _ => Color32::BLACK,
    }
}

fn hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// German for FilmCraft's effect, group, parameter and option names (FilmCraft has none); what
/// is not here stays as FilmCraft names it.
pub fn de(s: &str) -> &str {
    match s {
        // effects
        "Motion" => "Bewegung",
        "Opacity" => "Deckkraft",
        "Time Remapping" => "Zeitverzerrung",
        "Volume" => "Lautstärke",
        "Channel Volume" => "Kanallautstärke",
        "Panner" => "Panorama",
        "Lumetri Color" => "Lumetri-Farbe",
        "Crop" => "Zuschneiden",
        "Gaussian Blur" => "Gaußscher Weichzeichner",
        "Horizontal Flip" => "Horizontal spiegeln",
        "Vertical Flip" => "Vertikal spiegeln",
        "Brightness & Contrast" => "Helligkeit und Kontrast",
        "Tint" => "Tönung",
        "Shape" => "Form",
        "Parametric Equalizer" => "Parametrischer Equalizer",
        "Reverb" => "Hall",
        "Noise" => "Rauschen",
        // groups
        "Basic Correction" => "Grundkorrektur",
        "Creative" => "Kreativ",
        "Curves" => "Kurven",
        "Color Wheels & Match" => "Farbräder",
        "HSL Secondary" => "HSL-Sekundär",
        "Appearance" => "Aussehen",
        "Transform" => "Transformieren",
        // parameters
        "Position" => "Position",
        "Scale" => "Skalierung",
        "Scale Width" => "Breite skalieren",
        "Uniform Scale" => "Einheitlich skalieren",
        "Rotation" => "Drehung",
        "Anchor Point" => "Ankerpunkt",
        "Anti-flicker Filter" => "Flimmerfilter",
        "Blend Mode" => "Mischmodus",
        "Speed" => "Geschwindigkeit",
        "Bypass" => "Umgehen",
        "Level" => "Pegel",
        "Left" => "Links",
        "Right" => "Rechts",
        "Top" => "Oben",
        "Bottom" => "Unten",
        "Temperature" => "Temperatur",
        "Exposure" => "Belichtung",
        "Contrast" => "Kontrast",
        "Highlights" => "Lichter",
        "Shadows" => "Tiefen",
        "Whites" => "Weiß",
        "Blacks" => "Schwarz",
        "Saturation" => "Sättigung",
        "Vibrance" => "Dynamik",
        "Sharpen" => "Schärfen",
        "Faded Film" => "Verblasster Film",
        "Intensity" => "Intensität",
        "Amount" => "Stärke",
        "Midpoint" => "Mittelpunkt",
        "Roundness" => "Rundheit",
        "Feather" => "Weiche Kante",
        "Edge Feather" => "Weiche Kante",
        "Zoom" => "Zoomen",
        "Blurriness" => "Weichzeichnung",
        "Source Text" => "Text",
        "Font" => "Schrift",
        "Font Style" => "Schriftschnitt",
        "Font Size" => "Schriftgröße",
        "Alignment" => "Ausrichtung",
        "Tracking" => "Laufweite",
        "Leading" => "Zeilenabstand",
        "Fill" => "Füllung",
        "Fill Color" => "Füllfarbe",
        "Stroke" => "Kontur",
        "Stroke Color" => "Konturfarbe",
        "Stroke Width" => "Konturbreite",
        "Background" => "Hintergrund",
        "Background Color" => "Hintergrundfarbe",
        "Background Opacity" => "Hintergrund-Deckkraft",
        "Shadow" => "Schatten",
        "Shadow Color" => "Schattenfarbe",
        "Shadow Opacity" => "Schatten-Deckkraft",
        "Shadow Distance" => "Schattenabstand",
        "Shadow Blur" => "Schattenweichheit",
        "Layer Name" => "Ebenenname",
        // options
        "Center" | "center" | "middle" => "Mitte",
        "left" => "Links",
        "right" => "Rechts",
        "top" => "Oben",
        "bottom" => "Unten",
        "Justify" => "Blocksatz",
        "Normal" => "Normal",
        "Multiply" => "Multiplizieren",
        "Screen" => "Negativ multiplizieren",
        "Overlay" => "Ineinanderkopieren",
        "Darken" => "Abdunkeln",
        "Lighten" => "Aufhellen",
        "Rectangle" => "Rechteck",
        "Ellipse" => "Ellipse",
        "Polygon" => "Vieleck",
        "Path" => "Pfad",
        "None" => "Keiner",
        other => other,
    }
}
