//! BR's asset bar on the Schnitt page: CapCut's left column. Media, sound, text, effects,
//! transitions and looks as tiles: a click puts one in at the playhead (or onto the selected
//! clips), a drag puts it where it is dropped ([`crate::drop`]). "Profi" shows FilmCraft's own
//! Project panel, Effects panel, Essential Graphics and Media Browser in the same column.
//!
//! Drags use FilmCraft's own payloads wherever FilmCraft has one (a project item, an effect, a
//! look, a template), so FilmCraft's panels and BR's timeline understand the same drags; only a
//! plain title is BR's own ([`crate::drop::BrDrag`]).

use egui::{Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use filmcraft_project::{ItemId, ItemKind};
use filmcraft_ui_egui::dock::PanelKind;
use filmcraft_ui_egui::icons::Icon;
use filmcraft_ui_egui::panels;
use serde_json::{Value, json};

use crate::BrApp;
use crate::style::{ACCENT, Theme};

/// The column's width when it first opens.
pub const WIDTH: f32 = 300.0;
const TILE_W: f32 = 124.0;
const TILE_H: f32 = 70.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Media,
    Audio,
    Text,
    Effects,
    Transitions,
    Looks,
    Templates,
    Profi,
}

impl Tab {
    pub const ALL: [Tab; 8] = [Tab::Media, Tab::Audio, Tab::Text, Tab::Effects, Tab::Transitions, Tab::Looks, Tab::Templates, Tab::Profi];

    pub fn key(self) -> &'static str {
        match self {
            Tab::Media => "media",
            Tab::Audio => "audio",
            Tab::Text => "text",
            Tab::Effects => "effects",
            Tab::Transitions => "transitions",
            Tab::Looks => "looks",
            Tab::Templates => "templates",
            Tab::Profi => "profi",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Media => "Medien",
            Tab::Audio => "Audio",
            Tab::Text => "Text",
            Tab::Effects => "Effekte",
            Tab::Transitions => "Übergänge",
            Tab::Looks => "Filter",
            Tab::Templates => "Vorlagen",
            Tab::Profi => "Profi",
        }
    }

    pub fn find(key: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// FilmCraft's panels under "Profi": (panel, key, label).
const PROFI: [(PanelKind, &str, &str); 4] = [
    (PanelKind::Project, "project", "Projekt"),
    (PanelKind::Effects, "effects", "Effekte"),
    (PanelKind::EssentialGraphics, "graphics", "Grafiken"),
    (PanelKind::MediaBrowser, "browser", "Browser"),
];

/// What the asset bar remembers between frames.
pub struct AssetState {
    pub tab: Tab,
    /// "Profi": FilmCraft's panel shown.
    pub panel: PanelKind,
    /// The search over effects, transitions and looks.
    query: String,
    /// FilmCraft's graphics templates (asked for once).
    templates: Option<Vec<(String, String, String)>>,
}

impl Default for AssetState {
    fn default() -> Self {
        Self { tab: Tab::Media, panel: PanelKind::Project, query: String::new(), templates: None }
    }
}

/// The asset bar is shown with the timeline: on the Schnitt page, when there is a video.
pub fn visible(app: &BrApp) -> bool {
    crate::timeline::visible(app)
}

/// `br.ui.assets {tab?, panel?}`: switch the asset bar (window only).
pub fn control(app: &mut BrApp, p: &Value) -> Result<Value, String> {
    if let Some(k) = p.get("tab").and_then(Value::as_str) {
        app.assets.tab = Tab::find(k).ok_or_else(|| format!("unknown tab `{k}`: {}", Tab::ALL.map(Tab::key).join(", ")))?;
    }
    if let Some(k) = p.get("panel").and_then(Value::as_str) {
        let (panel, _, _) = PROFI.iter().find(|v| v.1 == k).ok_or_else(|| format!("unknown panel `{k}`: {}", PROFI.map(|v| v.1).join(", ")))?;
        app.assets.panel = *panel;
        app.assets.tab = Tab::Profi;
    }
    Ok(state(app))
}

pub fn state(app: &BrApp) -> Value {
    let panel = PROFI.iter().find(|v| v.0 == app.assets.panel).map(|v| v.1);
    json!({"tab": app.assets.tab.key(), "panel": panel})
}

/// Draw the asset bar as a column on the left (after the toolbar and the timeline).
pub fn show(app: &mut BrApp, ui: &mut Ui) {
    if !visible(app) {
        return;
    }
    let t = Theme::get(ui.ctx());
    egui::Panel::left("br_assets")
        .resizable(true)
        .default_size(WIDTH)
        .size_range(220.0..=560.0)
        .frame(egui::Frame::NONE.fill(t.bar).stroke(Stroke::new(1.0, t.divider)).inner_margin(egui::Margin::symmetric(8, 8)))
        .show(ui, |ui| {
            tabs(app, ui, &t);
            ui.add_space(4.0);
            match app.assets.tab {
                Tab::Profi => profi(app, ui, &t),
                tab => {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match tab {
                        Tab::Media => media(app, ui, &t, false),
                        Tab::Audio => media(app, ui, &t, true),
                        Tab::Text => text(app, ui, &t),
                        Tab::Effects => effects(app, ui, &t),
                        Tab::Transitions => transitions(app, ui, &t),
                        Tab::Templates => templates(app, ui, &t),
                        _ => looks(app, ui, &t),
                    });
                }
            }
            ui.allocate_rect(ui.available_rect_before_wrap(), Sense::hover());
        });
}

fn tabs(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        for tab in Tab::ALL {
            let on = app.assets.tab == tab;
            let b = egui::Button::new(RichText::new(tab.label()).size(12.0).color(if on { t.text } else { t.dim }))
                .fill(if on { t.button } else { Color32::TRANSPARENT })
                .stroke(if on { Stroke::new(1.0, ACCENT) } else { Stroke::NONE })
                .corner_radius(CornerRadius::same(6))
                .min_size(vec2(0.0, 24.0));
            let r = ui.add(b);
            app.widgets.insert(format!("assets.tab.{}", tab.key()), r.rect);
            if r.clicked() {
                app.assets.tab = tab;
                app.assets.query.clear();
            }
        }
    });
    ui.separator();
}

fn run(app: &mut BrApp, ctx: &egui::Context, id: &str, p: Value) {
    crate::cut::run(app, ctx, id, p);
}

/// The selected clips, or a message why nothing happens.
fn selection_or_say(app: &mut BrApp, ctx: &egui::Context) -> Option<Vec<u64>> {
    let sel: Vec<u64> = app.fc.session.state.selection.iter().map(|c| c.0).collect();
    if sel.is_empty() {
        crate::cut::toast(app, ctx, "Erst einen Clip in der Timeline anklicken (oder es auf einen Clip ziehen)");
        return None;
    }
    Some(sel)
}

/// The project's media for the Medien (pictures) or Audio (sound alone) tab.
fn media_items(app: &BrApp, sound: bool) -> Vec<(ItemId, String, f64)> {
    let p = &app.fc.session.project;
    let active = app.fc.session.state.active_sequence;
    let mut out: Vec<(ItemId, String, f64)> = p
        .items
        .iter()
        .filter(|(id, it)| {
            let (v, a) = p.source_streams(**id);
            match &it.kind {
                ItemKind::Media(_) | ItemKind::Subclip { .. } => {
                    if sound {
                        !v && a
                    } else {
                        v
                    }
                }
                ItemKind::Sequence(_) => !sound && Some(**id) != active,
                _ => false,
            }
        })
        .map(|(id, it)| {
            let secs = filmcraft_engine::media_duration(p, &app.fc.session.media, *id).map_or(0.0, |d| d.seconds());
            (*id, it.name.clone(), secs)
        })
        .collect();
    out.sort_by_key(|x| x.0.0);
    out
}

fn duration(secs: f64) -> String {
    if secs <= 0.0 {
        return String::new();
    }
    let s = secs.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Medien / Audio: the project's media as tiles; "+" puts one in at the playhead, a drag where
/// it is dropped, a double click opens it in the Source monitor.
fn media(app: &mut BrApp, ui: &mut Ui, t: &Theme, sound: bool) {
    let ctx = ui.ctx().clone();
    let (label, what) =
        if sound { ("+ Musik oder Ton importieren", crate::cut::PickFor::ImportSound) } else { ("+ Importieren", crate::cut::PickFor::Import) };
    let r = crate::style::button(ui, t, label, true, app.cut.picking.is_none());
    app.widgets.insert(format!("assets.{}.import", if sound { "audio" } else { "media" }), r.rect);
    if r.clicked() {
        crate::cut::pick(app, &ctx, what);
    }
    ui.add_space(6.0);
    let items = media_items(app, sound);
    if items.is_empty() {
        ui.label(
            RichText::new(if sound { "Noch keine Musik oder Töne im Projekt." } else { "Noch keine Medien im Projekt." }).size(12.0).color(t.dim),
        );
        return;
    }
    let prefix = if sound { "assets.audio" } else { "assets.media" };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (item, name, secs) in items {
            let (rect, r) = ui.allocate_exact_size(vec2(TILE_W, TILE_H + 16.0), Sense::click_and_drag());
            app.widgets.insert(format!("{prefix}.{}", item.0), rect);
            let pic = egui::Rect::from_min_size(rect.min, vec2(TILE_W, TILE_H));
            let p = ui.painter();
            p.rect_filled(pic, CornerRadius::same(6), t.well);
            if !sound && let Some((tex, size)) = app.fc.thumbnail(&ctx, item, filmcraft_time::Tick::ZERO, 160) {
                let fit = (pic.width() / size.x).min(pic.height() / size.y);
                let r = egui::Rect::from_center_size(pic.center(), size * fit);
                p.image(tex, r, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                filmcraft_ui_egui::icons::paint(
                    p,
                    egui::Rect::from_center_size(pic.center(), vec2(22.0, 22.0)),
                    if sound { Icon::Audio } else { Icon::Film },
                    t.dim,
                );
            }
            if !duration(secs).is_empty() {
                p.text(
                    pic.right_bottom() + vec2(-5.0, -4.0),
                    egui::Align2::RIGHT_BOTTOM,
                    duration(secs),
                    egui::FontId::proportional(10.5),
                    Color32::WHITE,
                );
            }
            let g = crate::style::elided(ui, &name, 11.5, t.text, TILE_W - 4.0);
            ui.painter().galley(pos2(rect.min.x + 2.0, pic.max.y + 2.0), g, t.text);
            if r.hovered() {
                ui.painter().rect_stroke(pic, CornerRadius::same(6), Stroke::new(1.5, ACCENT), StrokeKind::Inside);
            }
            // "+": in at the playhead
            let plus = egui::Rect::from_min_size(pos2(pic.max.x - 26.0, pic.min.y + 4.0), vec2(22.0, 22.0));
            let pr = ui.interact(plus, ui.id().with(("assets_add", item.0)), Sense::click());
            app.widgets.insert(format!("{prefix}.{}.add", item.0), plus);
            if r.hovered() || pr.hovered() {
                ui.painter().circle_filled(plus.center(), 11.0, ACCENT);
                filmcraft_ui_egui::icons::paint(ui.painter(), plus.shrink(5.0), Icon::Plus, crate::style::ON_ACCENT);
            }
            let pr = pr.on_hover_text(if sound { "Am Abspielkopf unter das Video" } else { "Am Abspielkopf ins Video (am nächsten Schnitt)" });
            if pr.clicked() {
                run(app, &ctx, "br.video.place", json!({"items": [item.0], "target": if sound { "audio" } else { "main" }}));
            } else if r.double_clicked() {
                run(app, &ctx, "source.open", json!({"item": item.0}));
            }
            if r.drag_started() {
                panels::start_drag_item(ui, item);
            }
        }
    });
}

/// Text: a plain title and FilmCraft's graphics templates.
fn text(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    let r = tile_row(ui, t, "Standardtext", "Ein Titel am Abspielkopf");
    app.widgets.insert("assets.text.default".into(), r.rect);
    if r.clicked() {
        run(app, &ctx, "br.video.text", json!({"text": "Text"}));
    }
    if r.drag_started() {
        egui::DragAndDrop::set_payload(&ctx, crate::drop::BrDrag::Text);
    }
    crate::style::heading(ui, t, "VORLAGEN");
    if app.assets.templates.is_none() {
        let list = app.fc.session.execute("graphics.template.list", json!({})).unwrap_or(Value::Null);
        let v: Vec<(String, String, String)> = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| Some((x["id"].as_str()?.to_string(), x["name"].as_str()?.to_string(), x["category"].as_str().unwrap_or("").to_string())))
            .collect();
        app.assets.templates = Some(v);
    }
    for (id, name, category) in app.assets.templates.clone().unwrap_or_default() {
        let r = tile_row(ui, t, &name, &category);
        app.widgets.insert(format!("assets.text.{id}"), r.rect);
        if r.clicked() {
            run(app, &ctx, "graphics.template.apply", json!({"template": id}));
        }
        if r.drag_started() {
            panels::start_drag_template(ui, &id, &name);
        }
    }
}

/// A row to click or drag: a name and a grey detail.
fn tile_row(ui: &mut Ui, t: &Theme, name: &str, detail: &str) -> egui::Response {
    let w = ui.available_width();
    let (rect, r) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click_and_drag());
    if r.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
    }
    let name_g = crate::style::elided(ui, crate::inspector::de(name), 12.5, t.text, w * 0.62);
    let name_w = name_g.size().x;
    ui.painter().galley(pos2(rect.min.x + 8.0, rect.center().y - name_g.size().y / 2.0), name_g, t.text);
    let detail_g = crate::style::elided(ui, detail, 11.0, t.dim, (w - name_w - 24.0).max(10.0));
    ui.painter().galley(pos2(rect.min.x + 16.0 + name_w, rect.center().y - detail_g.size().y / 2.0), detail_g, t.dim);
    r.on_hover_cursor(egui::CursorIcon::Grab)
}

fn search(app: &mut BrApp, ui: &mut Ui, key: &str) {
    let mut q = app.assets.query.clone();
    let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Suchen").desired_width(f32::INFINITY));
    app.widgets.insert(format!("assets.{key}.search"), r.rect);
    app.assets.query = q;
    ui.add_space(4.0);
}

fn matches(q: &str, name: &str, id: &str) -> bool {
    let q = q.to_lowercase();
    q.is_empty() || name.to_lowercase().contains(&q) || id.contains(&q) || crate::inspector::de(name).to_lowercase().contains(&q)
}

/// Effekte: FilmCraft's effects by folder; a click puts one on the selected clips, a drag onto
/// the clip it is dropped on.
fn effects(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    search(app, ui, "effects");
    let q = app.assets.query.clone();
    let all = br_video::inspect::schema(&json!({})).unwrap_or(Value::Null);
    for (kind, heading) in [("Video", "VIDEO"), ("Audio", "TON")] {
        let mut folder = String::new();
        let mut list: Vec<&Value> = all
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["kind"] == kind && d["intrinsic"] == false && matches(&q, d["name"].as_str().unwrap_or(""), d["id"].as_str().unwrap_or("")))
            .collect();
        list.sort_by_key(|d| {
            (
                d["category"].as_array().and_then(|c| c.last()).and_then(Value::as_str).unwrap_or("").to_string(),
                d["name"].as_str().unwrap_or("").to_string(),
            )
        });
        if list.is_empty() {
            continue;
        }
        crate::style::heading(ui, t, heading);
        for d in list {
            let id = d["id"].as_str().unwrap_or("");
            let name = d["name"].as_str().unwrap_or(id);
            let f = d["category"].as_array().and_then(|c| c.last()).and_then(Value::as_str).unwrap_or("").to_string();
            let detail = if f != folder { f.clone() } else { String::new() };
            folder = f;
            let r = tile_row(ui, t, name, &detail);
            app.widgets.insert(format!("assets.effect.{id}"), r.rect);
            if r.clicked()
                && let Some(sel) = selection_or_say(app, &ctx)
            {
                // onto every selected clip it fits
                run(app, &ctx, "br.video.effect", json!({"clips": sel, "op": "add", "effect": id}));
            }
            if r.drag_started() {
                panels::start_drag_effect(ui, id);
            }
        }
    }
}

/// Übergänge: a click puts one at the cuts of the selected clips (every cut without a
/// selection), a drag onto the edge of the clip it is dropped on.
fn transitions(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    search(app, ui, "transitions");
    let q = app.assets.query.clone();
    let all = br_video::inspect::schema(&json!({"kind": "videoTransition"})).unwrap_or(Value::Null);
    let mut list: Vec<&Value> =
        all.as_array().into_iter().flatten().filter(|d| matches(&q, d["name"].as_str().unwrap_or(""), d["id"].as_str().unwrap_or(""))).collect();
    list.sort_by_key(|d| {
        (
            d["category"].as_array().and_then(|c| c.last()).and_then(Value::as_str).unwrap_or("").to_string(),
            d["name"].as_str().unwrap_or("").to_string(),
        )
    });
    for d in list {
        let id = d["id"].as_str().unwrap_or("");
        let folder = d["category"].as_array().and_then(|c| c.last()).and_then(Value::as_str).unwrap_or("");
        let r = tile_row(ui, t, d["name"].as_str().unwrap_or(id), folder);
        app.widgets.insert(format!("assets.transition.{id}"), r.rect);
        if r.clicked() {
            run(app, &ctx, "br.video.transition", json!({"effect": id}));
        }
        if r.drag_started() {
            panels::start_drag_effect(ui, id);
        }
    }
}

/// Filter: the looks; a click on the selected clips (all without a selection), a drag onto one.
fn looks(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    search(app, ui, "looks");
    let q = app.assets.query.clone();
    let r = tile_row(ui, t, "Kein Filter", "den Look entfernen");
    app.widgets.insert("assets.look.none".into(), r.rect);
    if r.clicked() {
        run(app, &ctx, "br.video.filter", json!({"preset": "none"}));
    }
    let mut folder = String::new();
    for l in filmcraft_render::lumetri_presets::presets() {
        if !matches(&q, l.name, "") && !l.folder.to_lowercase().contains(&q.to_lowercase()) {
            continue;
        }
        if l.folder != folder {
            folder = l.folder.to_string();
            crate::style::heading(ui, t, &folder.to_uppercase());
        }
        let r = tile_row(ui, t, l.name, "").on_hover_text(l.description);
        app.widgets.insert(format!("assets.look.{}", l.name), r.rect);
        if r.clicked() {
            run(app, &ctx, "br.video.filter", json!({"preset": l.name}));
        }
        if r.drag_started() {
            panels::start_drag_lumetri_preset(ui, l.name);
        }
    }
}

/// A new video from a template; one that cuts on beats asks for its music first.
pub fn new_from_template(app: &mut BrApp, ctx: &egui::Context, id: &str, beats: bool) {
    if beats {
        app.cut.template = Some(id.to_string());
        crate::cut::pick(app, ctx, crate::cut::PickFor::TemplateMusic);
        return;
    }
    if crate::cut::run(app, ctx, "br.video.template.new", json!({"template": id})).is_some() {
        app.assets.tab = Tab::Templates;
        crate::cut::toast(app, ctx, "Vorlage angelegt: jetzt „Medien einsetzen“");
    }
}

/// Vorlagen: media into the placeholders of this video, a new video from a template, this video
/// kept as a template.
fn templates(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    let empty = br_video::template::empty_placeholders(&app.fc.session).len();
    if empty > 0 {
        let r = crate::style::button(ui, t, &format!("Medien einsetzen ({empty} frei)"), true, app.cut.picking.is_none());
        app.widgets.insert("assets.template.fill".into(), r.rect);
        if r.clicked() {
            crate::cut::pick(app, &ctx, crate::cut::PickFor::Template);
        }
        ui.label(RichText::new("Die Medien füllen die grauen Platzhalter der Reihe nach.").size(11.5).color(t.dim));
        ui.add_space(6.0);
    }
    let r = crate::style::button(ui, t, "Dieses Video als Vorlage speichern", false, true);
    app.widgets.insert("assets.template.save".into(), r.rect);
    if r.clicked() {
        let s = &app.fc.session;
        let name = s.state.active_sequence.and_then(|id| s.project.item(id)).map(|i| i.name.clone()).unwrap_or_else(|| "Meine Vorlage".into());
        if let Some(v) = crate::cut::run(app, &ctx, "br.video.template.save", json!({"name": name})) {
            crate::cut::toast(app, &ctx, format!("Als Vorlage gespeichert: {}", v["path"].as_str().unwrap_or("")));
        }
    }
    crate::style::heading(ui, t, "NEUES VIDEO AUS VORLAGE");
    let list = br_video::template::list(&json!({}));
    for tpl in list.as_array().into_iter().flatten() {
        let id = tpl["id"].as_str().unwrap_or("").to_string();
        let detail =
            format!("{} · {} Szenen{}", tpl["format"].as_str().unwrap_or(""), tpl["slots"], if tpl["beats"] == true { " · im Takt" } else { "" });
        let r = tile_row(ui, t, tpl["name"].as_str().unwrap_or(&id), &detail).on_hover_text(tpl["description"].as_str().unwrap_or(""));
        app.widgets.insert(format!("assets.template.{id}"), r.rect);
        if r.clicked() {
            new_from_template(app, &ctx, &id, tpl["beats"] == true);
        }
    }
}

/// Profi: FilmCraft's own panels in the column.
fn profi(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        for (panel, key, label) in PROFI {
            let on = app.assets.panel == panel;
            let b = egui::Button::new(RichText::new(label).size(12.0).color(if on { t.text } else { t.dim }))
                .fill(if on { t.button } else { Color32::TRANSPARENT })
                .stroke(if on { Stroke::new(1.0, ACCENT) } else { Stroke::NONE })
                .corner_radius(CornerRadius::same(6))
                .min_size(vec2(0.0, 22.0));
            let r = ui.add(b);
            app.widgets.insert(format!("assets.profi.{key}"), r.rect);
            if r.clicked() {
                app.assets.panel = panel;
            }
        }
    });
    ui.add_space(4.0);
    let panel = app.assets.panel;
    if crate::video::fc_shows(&app.fc, panel) {
        ui.label(RichText::new("Dieses Panel ist im Fenster angedockt oder schwebt und wird dort gezeigt.").size(12.0).color(t.dim));
        return;
    }
    let rect = ui.available_rect_before_wrap();
    if ui.rect_contains_pointer(rect) && ui.input(|i| i.pointer.any_pressed()) {
        app.fc.ui.focused = panel;
    }
    filmcraft_ui_egui::panels::show(&mut app.fc, ui, panel, rect);
}
