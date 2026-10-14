//! Simple mode: BR's CapCut-style photo editor.
//!
//! ```text
//! ┌ BR Foto [Einfach|Pro]  Importieren  Rückgängig Wiederholen ............ Exportieren ┐
//! │                                                    │ Looks Anpassen Format Export  │
//! │                    preview                         │                               │
//! │                                                    │   (the selected tab)          │
//! ├────────────────────────────────────────────────────┤                               │
//! │ filmstrip                                          │                               │
//! └────────────────────────────────────────────────────┴───────────────────────────────┘
//! ```
//!
//! Edits go through [`br_photo`], the same code the CLI and `br mcp` use. Slider and Amount drags
//! are engine interactions: applied live, committed as one undo step when the drag ends.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use br_photo::BrState;
use br_photo::adjust::{self, Slider};
use br_photo::crop::{self, Format};
use br_photo::export::{self, Preset, Target};
use br_photo::looks;
use egui::{Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use lightcraft_engine::catalog::PhotoId;
use lightcraft_engine::develop::DevelopSettings;
use lightcraft_ui_egui::render::Slot;
use serde_json::{Value, json};

use crate::style::{self, ACCENT, Theme};
use crate::{BrApp, Mode};

const TOP_H: f32 = 48.0;
const SIDE_W: f32 = 340.0;
const STRIP_H: f32 = 104.0;
const THUMB: Vec2 = Vec2::new(92.0, 70.0);
const LOOK_TILE_H: f32 = 72.0;
/// Long edge of Look thumbnails.
const LOOK_EDGE: usize = 192;

/// The side panel's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Looks,
    Adjust,
    Format,
    Export,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Looks, Tab::Adjust, Tab::Format, Tab::Export];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Looks => "Looks",
            Tab::Adjust => "Anpassen",
            Tab::Format => "Format",
            Tab::Export => "Export",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tab::Looks => "looks",
            Tab::Adjust => "adjust",
            Tab::Format => "format",
            Tab::Export => "export",
        }
    }
}

struct ExportJob {
    handle: JoinHandle<Result<Vec<Value>, String>>,
    preset: Preset,
}

/// Looks rendered for one photo's settings.
struct LooksCache {
    photo: PhotoId,
    base: Arc<DevelopSettings>,
    /// (look id, shown name, group, settings at 100 %)
    looks: Vec<(String, String, String, DevelopSettings)>,
}

/// Simple mode's own state.
pub struct SimpleState {
    pub tab: Tab,
    /// The look whose Amount can still be changed (shared with the `br.*` commands).
    pub br: BrState,
    /// The slider being dragged.
    drag: Option<Slider>,
    /// An Amount drag in progress.
    look_drag: bool,
    pub export_preset: Preset,
    pub export_dir: PathBuf,
    export: Option<ExportJob>,
    /// The files of the last finished export.
    pub last_export: Option<Vec<Value>>,
    /// A message under the preview: (text, is error, time shown).
    pub message: Option<(String, bool, f64)>,
    looks: Option<LooksCache>,
    /// The active photo last frame (to scroll the filmstrip to a new one).
    shown: Option<PhotoId>,
}

impl Default for SimpleState {
    fn default() -> Self {
        Self {
            tab: Tab::Looks,
            br: BrState::default(),
            drag: None,
            look_drag: false,
            export_preset: Preset::Instagram,
            export_dir: export::default_dir(),
            export: None,
            last_export: None,
            message: None,
            looks: None,
            shown: None,
        }
    }
}

impl SimpleState {
    /// An export is running.
    pub fn exporting(&self) -> bool {
        self.export.is_some()
    }
}

/// End a slider or Amount drag that is in progress: it becomes its undo step.
pub fn finish_drags(app: &mut BrApp) {
    let st = &mut app.simple;
    let s = &mut app.lc.session;
    if st.drag.take().is_some()
        && let Err(e) = adjust::end(s)
    {
        log::warn!("ending a slider drag: {e}");
    }
    if std::mem::take(&mut st.look_drag)
        && let Some(l) = st.br.look.clone()
        && let Err(e) = looks::set_look(s, &mut st.br.look, l.photo, &l.preset_id, l.amount, false)
    {
        log::warn!("ending an amount drag: {e}");
    }
}

fn say(app: &mut BrApp, ctx: &egui::Context, text: impl Into<String>, error: bool) {
    let now = ctx.input(|i| i.time);
    app.simple.message = Some((text.into(), error, now));
}

/// Run `f` on the session and report an error under the preview.
fn run(app: &mut BrApp, ctx: &egui::Context, what: &str, f: impl FnOnce(&mut BrApp) -> Result<Value, String>) {
    if let Err(e) = f(app) {
        say(app, ctx, format!("{what}: {e}"), true);
    }
}

/// Draw Simple mode.
pub fn show(app: &mut BrApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let t = Theme::get(&ctx);

    // a drag whose release we never saw (the pointer left the window) ends here
    if (app.simple.drag.is_some() || app.simple.look_drag) && !ctx.input(|i| i.pointer.any_down()) {
        finish_drags(app);
    }
    poll_export(app, &ctx);

    let photos = app.lc.session.visible_cloned();
    let mut active = app.lc.session.active().filter(|id| photos.contains(id));
    if active.is_none()
        && let Some(first) = photos.first()
        && app.lc.session.execute("library.select", &json!({"ids": [first.0], "active": first.0})).is_ok()
    {
        active = Some(*first);
    }

    shortcuts(app, &ctx, active);
    top_bar(app, ui, &t, active);

    let Some(id) = active else {
        empty(app, ui, &t);
        return;
    };

    egui::Panel::right("br_side").exact_size(SIDE_W).resizable(false).frame(egui::Frame::NONE.fill(t.bar)).show(ui, |ui| side(app, ui, &t, id));
    egui::Panel::bottom("br_strip")
        .exact_size(STRIP_H)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.well))
        .show(ui, |ui| strip(app, ui, &t, &photos, id));
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| preview(app, ui, &t, id));
    app.simple.shown = Some(id);
}

fn shortcuts(app: &mut BrApp, ctx: &egui::Context, active: Option<PhotoId>) {
    if ctx.memory(|m| m.focused().is_some()) {
        return;
    }
    use egui::{Key, KeyboardShortcut, Modifiers};
    let redo = ctx.input_mut(|i| {
        i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
            || i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Y))
    });
    let undo = ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)));
    if undo {
        undo_redo(app, ctx, "edit.undo");
    } else if redo {
        undo_redo(app, ctx, "edit.redo");
    }
    if active.is_some() {
        let step = ctx.input(|i| {
            if i.key_pressed(Key::ArrowRight) {
                Some("library.next")
            } else if i.key_pressed(Key::ArrowLeft) {
                Some("library.previous")
            } else {
                None
            }
        });
        if let Some(cmd) = step {
            finish_drags(app);
            let _ = app.lc.session.execute(cmd, &json!({}));
        }
    }
}

fn undo_redo(app: &mut BrApp, ctx: &egui::Context, cmd: &str) {
    finish_drags(app);
    match app.lc.session.execute(cmd, &json!({})) {
        Ok(v) => {
            let label = v.get("undone").or(v.get("redone")).and_then(Value::as_str).unwrap_or_default().to_string();
            let what = if cmd == "edit.undo" { "Rückgängig" } else { "Wiederholt" };
            if !label.is_empty() {
                say(app, ctx, format!("{what}: {label}"), false);
            }
        }
        Err(e) if e.kind() == lightcraft_engine::ErrorKind::Disabled => {}
        Err(e) => say(app, ctx, e.to_string(), true),
    }
}

fn top_bar(app: &mut BrApp, ui: &mut Ui, t: &Theme, active: Option<PhotoId>) {
    let ctx = ui.ctx().clone();
    egui::Panel::top("br_top").exact_size(TOP_H).frame(egui::Frame::NONE.fill(t.bar).inner_margin(egui::Margin::symmetric(14, 0))).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            style::brand(ui, t);
            ui.add_space(10.0);
            if style::mode_switch(ui, t, Mode::Simple, &mut app.widgets) == Some(Mode::Pro) {
                app.open_pro();
            }
            ui.add_space(14.0);
            let r = style::button(ui, t, "+ Importieren", false, app.lc.import.is_none());
            app.widgets.insert("import".into(), r.rect);
            if r.on_hover_text("Fotos hinzufügen (oder ins Fenster ziehen)").clicked() {
                import_dialog(app, &ctx);
            }
            let s = &app.lc.session;
            let (can_undo, can_redo) = (!s.undo.is_empty() || s.interaction.is_some(), !s.redo.is_empty());
            let undo_tip = s.undo.last().map(|e| format!("Rückgängig: {}", e.label)).unwrap_or_else(|| "Nichts rückgängig zu machen".into());
            let redo_tip = s.redo.last().map(|e| format!("Wiederholen: {}", e.label)).unwrap_or_else(|| "Nichts zu wiederholen".into());
            let r = style::button(ui, t, "Rückgängig", false, can_undo).on_hover_text(undo_tip);
            app.widgets.insert("undo".into(), r.rect);
            if r.clicked() {
                undo_redo(app, &ctx, "edit.undo");
            }
            let r = style::button(ui, t, "Wiederholen", false, can_redo).on_hover_text(redo_tip);
            app.widgets.insert("redo".into(), r.rect);
            if r.clicked() {
                undo_redo(app, &ctx, "edit.redo");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let r = style::button(ui, t, "  Exportieren  ", true, active.is_some() && !app.simple.exporting());
                app.widgets.insert("export".into(), r.rect);
                if r.clicked() {
                    if app.simple.tab == Tab::Export {
                        start_export(app, &ctx);
                    } else {
                        app.simple.tab = Tab::Export;
                    }
                }
                if let Some(lib) = &app.library {
                    ui.label(RichText::new(lib).size(11.5).color(t.dim)).on_hover_text("Bibliothek");
                }
            });
        });
    });
}

fn empty(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| {
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_space((ui.available_height() / 2.0 - 90.0).max(20.0));
            ui.label(RichText::new("Noch keine Fotos").size(24.0).color(t.text));
            ui.add_space(6.0);
            ui.label(RichText::new("Füge Fotos hinzu, um sie zu bearbeiten.").size(14.0).color(t.dim));
            ui.add_space(18.0);
            let busy = app.lc.import.is_some();
            let r = style::button(ui, t, if busy { "Wird importiert…" } else { "  Fotos importieren  " }, true, !busy);
            app.widgets.insert("import.empty".into(), r.rect);
            if r.clicked() {
                import_dialog(app, &ctx);
            }
            ui.add_space(8.0);
            ui.label(RichText::new("oder Dateien ins Fenster ziehen").size(12.5).color(t.dim));
        });
    });
}

fn import_dialog(app: &mut BrApp, ctx: &egui::Context) {
    let Some(files) = rfd::FileDialog::new().set_title("Fotos importieren").add_filter("Fotos", lightcraft_engine::import::EXTENSIONS).pick_files()
    else {
        return;
    };
    let paths: Vec<String> = files.into_iter().map(|p| p.to_string_lossy().into_owned()).collect();
    import(app, ctx, paths);
}

/// Import files or folders in the background (LightCraft's import; it reports when done).
pub fn import(app: &mut BrApp, ctx: &egui::Context, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    match lightcraft_ui_egui::import::start_paths(&mut app.lc, paths) {
        Ok(_) => say(app, ctx, "Fotos werden importiert…", false),
        Err(e) => say(app, ctx, format!("Import: {e}"), true),
    }
}

/// Draw `tex` fitted into `area`, centred; returns where it was drawn.
fn fit_image(ui: &Ui, tex: egui::TextureId, size: [usize; 2], area: Rect) -> Rect {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let k = (area.width() / w).min(area.height() / h);
    let r = Rect::from_center_size(area.center(), vec2(w * k, h * k));
    ui.painter().image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    r
}

fn preview(app: &mut BrApp, ui: &mut Ui, t: &Theme, id: PhotoId) {
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let area = full.shrink2(vec2(24.0, 20.0)).with_max_y(full.bottom() - 30.0);
    app.lc.canvas_rect = Some(area);
    let ppp = ui.ctx().pixels_per_point();
    let (w, h) = ((area.width() * ppp).max(64.0) as usize, (area.height() * ppp).max(64.0) as usize);
    let dragging = app.simple.drag.is_some() || app.simple.look_drag;
    if let Some(job) = app.lc.session.loupe_job(id, w, h, true) {
        let job = if dragging { job.draft() } else { job };
        app.lc.renderer.request(Slot::Main, job, 100);
    }
    let main = app.lc.renderer.textures.get(&Slot::Main).filter(|x| x.photo == id).map(|x| (x.tex.id(), x.size));
    let shown = main.or_else(|| {
        lightcraft_ui_egui::panels::grid::request_thumb(&mut app.lc, id, 512, 50);
        app.lc.renderer.thumb(id).map(|x| (x.tex.id(), x.size))
    });
    match shown {
        Some((tex, size)) => {
            fit_image(ui, tex, size, area);
        }
        None => {
            ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, "Wird geladen…", egui::FontId::proportional(14.0), t.dim);
        }
    }
    // caption: file, size, format
    let s = &app.lc.session;
    if let Some(p) = s.catalog.photo(id) {
        let crop = s.develop_of(id).zip(br_photo::photo::oriented_dims(s, id)).map(|(d, (w, h))| d.crop.geometry.rect_px(w, h));
        let size = crop.map(|r| format!("{} × {}", r.width().round(), r.height().round())).unwrap_or_default();
        let format = crop::current(s, id).filter(|f| *f != Format::Original).map(|f| format!(" · {}", f.label())).unwrap_or_default();
        let text = format!("{} · {size}{format}", p.file_name);
        ui.painter().text(pos2(full.left() + 24.0, full.bottom() - 15.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(12.0), t.dim);
    }
    message(app, ui, t, full);
}

fn message(app: &mut BrApp, ui: &Ui, t: &Theme, full: Rect) {
    let now = ui.ctx().input(|i| i.time);
    let Some((text, error, at)) = app.simple.message.clone() else { return };
    let ttl = if error { 8.0 } else { 3.5 };
    if now - at > ttl {
        app.simple.message = None;
        return;
    }
    ui.ctx().request_repaint_after(Duration::from_millis(250));
    let font = egui::FontId::proportional(13.0);
    let galley = ui.painter().layout(text, font, if error { Color32::from_rgb(0xff, 0xd2, 0xcc) } else { t.text }, full.width() - 120.0);
    let size = galley.size() + vec2(28.0, 16.0);
    let r = Rect::from_center_size(pos2(full.center().x, full.bottom() - 52.0), size);
    let fill = if error { Color32::from_rgb(0x6a, 0x1f, 0x1a) } else { Color32::from_black_alpha(200) };
    ui.painter().rect_filled(r, CornerRadius::same(10), fill);
    ui.painter().galley(r.min + vec2(14.0, 8.0), galley, t.text);
}

fn strip(app: &mut BrApp, ui: &mut Ui, t: &Theme, photos: &[PhotoId], active: PhotoId) {
    let ctx = ui.ctx().clone();
    let cell = THUMB.x + 8.0;
    let jump = app.simple.shown != Some(active);
    let mut clicked = None;
    egui::ScrollArea::horizontal().auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let total = vec2(cell * photos.len() as f32 + 16.0, STRIP_H);
        let (area, _) = ui.allocate_exact_size(total, Sense::hover());
        let first = ((viewport.left() - 8.0) / cell).floor().max(0.0) as usize;
        let last = (((viewport.right() - 8.0) / cell).ceil().max(0.0) as usize + 1).min(photos.len());
        for (i, &id) in photos.iter().enumerate().take(last).skip(first) {
            let r = Rect::from_min_size(pos2(area.left() + 8.0 + i as f32 * cell, area.top() + (STRIP_H - THUMB.y) / 2.0), THUMB);
            let resp = ui.interact(r, ui.id().with(("thumb", id.0)), Sense::click());
            ui.painter().rect_filled(r, CornerRadius::same(6), t.button);
            lightcraft_ui_egui::panels::grid::request_thumb(&mut app.lc, id, 128, if id == active { 40 } else { 3 });
            if let Some(tex) = app.lc.renderer.thumb(id) {
                let (tw, th) = (tex.size[0].max(1) as f32, tex.size[1].max(1) as f32);
                // fill the cell (cover), cropping the thumbnail's middle
                let k = (r.width() / tw).max(r.height() / th);
                let (u, v) = ((r.width() / (tw * k)).min(1.0), (r.height() / (th * k)).min(1.0));
                let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(u, v));
                let painter = ui.painter().with_clip_rect(r);
                painter.image(tex.tex.id(), r, uv, Color32::WHITE);
            }
            if id == active {
                ui.painter().rect_stroke(r, CornerRadius::same(6), Stroke::new(2.5, ACCENT), StrokeKind::Inside);
                if jump {
                    ui.scroll_to_rect(r, Some(Align::Center));
                }
            } else if resp.hovered() {
                ui.painter().rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, t.dim), StrokeKind::Inside);
            }
            app.widgets.insert(format!("photo.{}", id.0), r);
            if resp.clicked() {
                clicked = Some(id);
            }
        }
        if jump && !(first..last).contains(&photos.iter().position(|p| *p == active).unwrap_or(0)) {
            let i = photos.iter().position(|p| *p == active).unwrap_or(0);
            let r = Rect::from_min_size(pos2(area.left() + 8.0 + i as f32 * cell, area.top()), THUMB);
            ui.scroll_to_rect(r, Some(Align::Center));
        }
    });
    if let Some(id) = clicked.filter(|id| *id != active) {
        finish_drags(app);
        run(app, &ctx, "Auswahl", |app| app.lc.session.execute("library.select", &json!({"ids": [id.0], "active": id.0})).map_err(|e| e.to_string()));
    }
}

fn side(app: &mut BrApp, ui: &mut Ui, t: &Theme, id: PhotoId) {
    // tab bar
    let w = SIDE_W / Tab::ALL.len() as f32;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for tab in Tab::ALL {
            let r = style::tab(ui, t, tab.label(), app.simple.tab == tab, w);
            app.widgets.insert(format!("tab.{}", tab.key()), r.rect);
            if r.clicked() {
                app.simple.tab = tab;
            }
        }
    });
    let line = ui.available_rect_before_wrap().with_max_y(ui.cursor().top() + 1.0);
    ui.painter().rect_filled(line, CornerRadius::ZERO, t.divider);
    ui.add_space(4.0);
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
        ui.set_width(SIDE_W - 28.0);
        match app.simple.tab {
            Tab::Looks => looks_tab(app, ui, t, id),
            Tab::Adjust => adjust_tab(app, ui, t, id),
            Tab::Format => format_tab(app, ui, t, id),
            Tab::Export => export_tab(app, ui, t, id),
        }
    });
}

fn looks_tab(app: &mut BrApp, ui: &mut Ui, t: &Theme, id: PhotoId) {
    let ctx = ui.ctx().clone();
    let current = looks::current(&app.simple.br.look, &app.lc.session, id).cloned();

    // Amount of the current look
    style::heading(ui, t, "STÄRKE");
    match &current {
        Some(l) => {
            let name = lightcraft_ui_egui::i18n::builtin_label(l.label.trim_start_matches("Look: "), true).to_string();
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).size(13.5).color(t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("{:.0} %", l.amount)).size(13.0).color(t.dim));
                });
            });
            let mut amount = l.amount;
            ui.spacing_mut().slider_width = ui.available_width();
            let r = ui.add(egui::Slider::new(&mut amount, 0.0..=looks::MAX_AMOUNT).show_value(false).step_by(1.0));
            app.widgets.insert("look.amount".into(), r.rect);
            if r.drag_started() {
                app.simple.look_drag = true;
            }
            if r.changed() {
                let live = app.simple.look_drag && !r.drag_stopped();
                let (preset, st, s) = (l.preset_id.clone(), &mut app.simple, &mut app.lc.session);
                if let Err(e) = looks::set_look(s, &mut st.br.look, id, &preset, amount, live) {
                    say(app, &ctx, e.to_string(), true);
                }
            }
            if r.drag_stopped() {
                finish_drags(app);
            }
        }
        None => {
            ui.label(RichText::new("Tippe auf einen Look. Danach lässt sich seine Stärke hier einstellen.").size(12.5).color(t.dim));
        }
    }
    ui.add_space(6.0);

    // the tiles: rendered from the settings before the current look
    let Some(base) = current.as_ref().map(|l| l.base.clone()).or_else(|| app.lc.session.develop_of(id)) else { return };
    let stale = app.simple.looks.as_ref().is_none_or(|c| c.photo != id || !Arc::ptr_eq(&c.base, &base));
    if stale {
        let s = &app.lc.session;
        let looks = looks::list(s)
            .into_iter()
            .filter_map(|l| {
                let p = looks::find(s, &l.id)?;
                let name = lightcraft_ui_egui::i18n::builtin_label(&p.name, p.builtin).to_string();
                let group = lightcraft_ui_egui::i18n::builtin_label(&p.group, p.builtin).to_string();
                Some((l.id, name, group, looks::blend_look(&base, p, 1.0)))
            })
            .collect();
        app.simple.looks = Some(LooksCache { photo: id, base: base.clone(), looks });
    }
    let tiles: Vec<(String, String, String, DevelopSettings)> = app.simple.looks.as_ref().map(|c| c.looks.clone()).unwrap_or_default();
    let mut groups: Vec<String> = Vec::new();
    for (_, _, g, _) in &tiles {
        if !groups.contains(g) {
            groups.push(g.clone());
        }
    }
    let cols = 3;
    let gap = 8.0;
    // room for the scroll bar on the right
    let tw = (ui.available_width() - 10.0 - gap * (cols as f32 - 1.0)) / cols as f32;
    let mut picked: Option<(String, bool)> = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut entries: Vec<(Option<&str>, Vec<(&str, &str, &DevelopSettings)>)> = vec![(None, vec![("", "Original", &*base)])];
        for g in &groups {
            entries.push((Some(g), tiles.iter().filter(|x| &x.2 == g).map(|x| (x.0.as_str(), x.1.as_str(), &x.3)).collect()));
        }
        for (group, items) in entries {
            if let Some(g) = group {
                style::heading(ui, t, &g.to_uppercase());
            }
            for row in items.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (look_id, name, settings) in row {
                        let (rect, resp) = ui.allocate_exact_size(vec2(tw, LOOK_TILE_H + 20.0), Sense::click());
                        let img = Rect::from_min_size(rect.min, vec2(tw, LOOK_TILE_H));
                        ui.painter().rect_filled(img, CornerRadius::same(7), t.well);
                        if ui.is_rect_visible(rect)
                            && let Some(job) = app.lc.session.variant_job(id, settings, LOOK_EDGE)
                            && let Some(tex) = app.lc.renderer.variant(job)
                        {
                            let (w, h) = (tex.size[0].max(1) as f32, tex.size[1].max(1) as f32);
                            let k = (img.width() / w).max(img.height() / h);
                            let uv =
                                Rect::from_center_size(pos2(0.5, 0.5), vec2((img.width() / (w * k)).min(1.0), (img.height() / (h * k)).min(1.0)));
                            ui.painter().with_clip_rect(img).image(tex.tex.id(), img, uv, Color32::WHITE);
                        }
                        let on = match &current {
                            Some(l) => l.preset_id == *look_id,
                            None => look_id.is_empty(),
                        };
                        if on {
                            style::selected_outline(ui, img);
                        } else if resp.hovered() {
                            ui.painter().rect_stroke(img, CornerRadius::same(7), Stroke::new(1.0, t.dim), StrokeKind::Inside);
                        }
                        let galley = style::elided(ui, name, 11.5, if on { t.text } else { t.dim }, tw);
                        let label_pos = pos2(img.center().x - galley.size().x / 2.0, img.bottom() + 3.0);
                        ui.painter().galley(label_pos, galley, t.text);
                        app.widgets.insert(format!("look.{}", if look_id.is_empty() { "original" } else { look_id }), rect);
                        if resp.clicked() {
                            picked = Some((look_id.to_string(), on));
                        }
                    }
                });
                ui.add_space(gap);
            }
        }
    });
    match picked {
        // "Original": take the current look off (amount 0, still one step)
        Some((look_id, _)) if look_id.is_empty() => {
            if let Some(l) = &current {
                let preset = l.preset_id.clone();
                let (st, s) = (&mut app.simple, &mut app.lc.session);
                if let Err(e) = looks::set_look(s, &mut st.br.look, id, &preset, 0.0, false) {
                    say(app, &ctx, e.to_string(), true);
                }
            }
        }
        Some((_, true)) => {}
        Some((look_id, false)) => {
            finish_drags(app);
            let (st, s) = (&mut app.simple, &mut app.lc.session);
            if let Err(e) = looks::set_look(s, &mut st.br.look, id, &look_id, 100.0, false) {
                say(app, &ctx, e.to_string(), true);
            }
        }
        None => {}
    }
}

fn adjust_tab(app: &mut BrApp, ui: &mut Ui, t: &Theme, id: PhotoId) {
    let ctx = ui.ctx().clone();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let r = style::button(ui, t, "  Auto  ", true, true).on_hover_text("Belichtung, Kontrast und Tonwerte automatisch");
        app.widgets.insert("adjust.auto".into(), r.rect);
        if r.clicked() {
            finish_drags(app);
            run(app, &ctx, "Auto", |app| adjust::auto(&mut app.lc.session, id).map_err(|e| e.to_string()));
        }
        let r = style::button(ui, t, "Zurücksetzen", false, true).on_hover_text("Alle sechs Regler auf 0");
        app.widgets.insert("adjust.reset".into(), r.rect);
        if r.clicked() {
            finish_drags(app);
            run(app, &ctx, "Zurücksetzen", |app| adjust::reset(&mut app.lc.session, id).map_err(|e| e.to_string()));
        }
    });
    ui.add_space(10.0);
    let values = adjust::values(&app.lc.session, id);
    for (slider, value) in values {
        ui.horizontal(|ui| {
            let l = ui.add(egui::Label::new(RichText::new(slider.label()).size(13.5).color(t.text)).sense(Sense::click()));
            if l.on_hover_text(format!("{} (Doppelklick: 0)", slider.hint())).double_clicked() {
                finish_drags(app);
                run(app, &ctx, slider.label(), |app| adjust::set(&mut app.lc.session, id, &[(slider, 0.0)]).map_err(|e| e.to_string()));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let text = if value.abs() < 0.05 { "0".to_string() } else { format!("{value:+.0}") };
                ui.label(RichText::new(text).size(13.0).color(if value.abs() < 0.05 { t.dim } else { ACCENT }));
            });
        });
        let mut v = value;
        ui.spacing_mut().slider_width = ui.available_width();
        let r = ui.add(egui::Slider::new(&mut v, adjust::MIN..=adjust::MAX).show_value(false).step_by(1.0));
        app.widgets.insert(format!("slider.{}", slider.key()), r.rect);
        if r.drag_started() {
            finish_drags(app);
            match adjust::begin(&mut app.lc.session, slider) {
                Ok(()) => app.simple.drag = Some(slider),
                Err(e) => say(app, &ctx, e.to_string(), true),
            }
        }
        if r.changed() {
            let result = if app.simple.drag == Some(slider) {
                adjust::drag(&mut app.lc.session, id, slider, v).map(|_| Value::Null)
            } else {
                adjust::set(&mut app.lc.session, id, &[(slider, v)])
            };
            if let Err(e) = result {
                say(app, &ctx, e.to_string(), true);
            }
        }
        if r.drag_stopped() {
            finish_drags(app);
        }
        ui.add_space(10.0);
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Mehr Regler (Kurven, Farben einzeln, Masken) gibt es im Pro-Modus.").size(12.0).color(t.dim));
}

/// The ratio as a small box, `size` points at most.
fn ratio_icon(ui: &Ui, t: &Theme, center: egui::Pos2, f: Format, size: f32, on: bool) {
    let (w, h) = match f.ratio() {
        Some((w, h)) => (w as f32, h as f32),
        None => (3.0, 2.0),
    };
    let k = size / w.max(h);
    let r = Rect::from_center_size(center, vec2(w * k, h * k));
    let color = if on { ACCENT } else { t.dim };
    if f == Format::Original {
        ui.painter().rect_stroke(r, CornerRadius::same(2), Stroke::new(1.5, color), StrokeKind::Inside);
        let inner = r.shrink(4.0);
        ui.painter().rect_filled(inner, CornerRadius::same(1), color.gamma_multiply(0.35));
    } else {
        ui.painter().rect_stroke(r, CornerRadius::same(2), Stroke::new(1.5, color), StrokeKind::Inside);
    }
}

fn format_tab(app: &mut BrApp, ui: &mut Ui, t: &Theme, id: PhotoId) {
    let ctx = ui.ctx().clone();
    let current = crop::current(&app.lc.session, id);
    style::heading(ui, t, "FORMAT");
    let width = ui.available_width();
    for f in Format::ALL {
        let on = current == Some(f);
        let (rect, r) = ui.allocate_exact_size(vec2(width, 52.0), Sense::click());
        let fill = if on {
            t.button
        } else if r.hovered() {
            t.hover
        } else {
            t.well
        };
        ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
        if on {
            ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.5, ACCENT), StrokeKind::Inside);
        }
        ratio_icon(ui, t, pos2(rect.left() + 30.0, rect.center().y), f, 26.0, on);
        let p = ui.painter();
        p.text(pos2(rect.left() + 58.0, rect.top() + 10.0), egui::Align2::LEFT_TOP, f.label(), egui::FontId::proportional(14.0), t.text);
        p.text(pos2(rect.left() + 58.0, rect.top() + 29.0), egui::Align2::LEFT_TOP, f.hint(), egui::FontId::proportional(11.5), t.dim);
        app.widgets.insert(format!("format.{}", f.key()), rect);
        if r.clicked() && !on {
            finish_drags(app);
            run(app, &ctx, "Format", |app| crop::apply(&mut app.lc.session, id, f).map_err(|e| e.to_string()));
        }
        ui.add_space(6.0);
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new("Der Ausschnitt wird mittig gesetzt. Verschieben oder drehen: im Pro-Modus mit dem Zuschneide-Werkzeug.")
            .size(12.0)
            .color(t.dim),
    );
}

fn export_tab(app: &mut BrApp, ui: &mut Ui, t: &Theme, id: PhotoId) {
    let ctx = ui.ctx().clone();
    style::heading(ui, t, "VORLAGE");
    let width = ui.available_width();
    for p in Preset::ALL {
        let on = app.simple.export_preset == p;
        let r = style::card(ui, t, p.label(), p.detail(), on, width);
        app.widgets.insert(format!("preset.{}", p.key()), r.rect);
        if r.clicked() {
            app.simple.export_preset = p;
        }
        ui.add_space(6.0);
    }
    style::heading(ui, t, "ZIEL");
    ui.label(RichText::new(app.simple.export_dir.to_string_lossy()).size(12.0).color(t.text));
    let r = style::button(ui, t, "Ordner wählen…", false, true);
    app.widgets.insert("export.dir".into(), r.rect);
    if r.clicked()
        && let Some(dir) = rfd::FileDialog::new().set_title("Export-Ordner").set_directory(&app.simple.export_dir).pick_folder()
    {
        app.simple.export_dir = dir;
    }
    ui.add_space(14.0);
    let busy = app.simple.exporting();
    let label = if busy { "Wird exportiert…" } else { "Exportieren" };
    let r = style::wide_button(ui, label, width, !busy);
    app.widgets.insert("export.go".into(), r.rect);
    if r.clicked() {
        start_export(app, &ctx);
    }
    if busy {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Foto wird berechnet und gespeichert…").size(12.5).color(t.dim));
        });
    } else if let Some(files) = &app.simple.last_export {
        ui.add_space(8.0);
        for f in files {
            if let Some(path) = f.get("path").and_then(Value::as_str) {
                let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(RichText::new(format!("Gespeichert: {name}")).size(12.5).color(t.text)).on_hover_text(path);
                ui.label(RichText::new(format!("{} × {} Pixel", f["width"], f["height"])).size(11.5).color(t.dim));
            }
        }
    }
    let _ = id;
}

/// Start exporting the active photo with the selected preset (on another thread).
pub fn start_export(app: &mut BrApp, ctx: &egui::Context) {
    if app.simple.exporting() {
        return;
    }
    let Some(id) = app.lc.session.active() else { return };
    finish_drags(app);
    let preset = app.simple.export_preset;
    let target = Target::Dir(app.simple.export_dir.clone());
    match export::prepare(&mut app.lc.session, id, preset, &target) {
        Ok(plan) => match std::thread::Builder::new().name("br-export".into()).spawn(move || plan.run()) {
            Ok(handle) => {
                app.simple.export = Some(ExportJob { handle, preset });
                app.simple.last_export = None;
            }
            Err(e) => say(app, ctx, format!("Export: {e}"), true),
        },
        Err(e) => say(app, ctx, format!("Export: {e}"), true),
    }
}

fn poll_export(app: &mut BrApp, ctx: &egui::Context) {
    let Some(job) = &app.simple.export else { return };
    if !job.handle.is_finished() {
        ctx.request_repaint_after(Duration::from_millis(100));
        return;
    }
    let Some(job) = app.simple.export.take() else { return };
    match job.handle.join() {
        Ok(Ok(files)) => {
            let first = files.first().cloned().unwrap_or(Value::Null);
            let name =
                first.get("path").and_then(Value::as_str).and_then(|p| std::path::Path::new(p).file_name()).map(|n| n.to_string_lossy().into_owned());
            let text = match name {
                Some(n) => format!("{} gespeichert: {n} ({} × {})", job.preset.label(), first["width"], first["height"]),
                None => format!("{}: nichts gespeichert", job.preset.label()),
            };
            say(app, ctx, text, false);
            app.simple.last_export = Some(files);
        }
        Ok(Err(e)) => say(app, ctx, format!("Export fehlgeschlagen: {e}"), true),
        Err(_) => say(app, ctx, "Export unerwartet abgebrochen", true),
    }
}
