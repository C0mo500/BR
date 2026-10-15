//! The Foto page's quick tools, over LightCraft's complete interface.
//!
//! ```text
//! ┌ shnyp  [Foto]                                  Looks  Format  [Exportieren]   ┐
//! │                                                                 ┌──────────┐ │
//! │   LightCraft: library, develop panels, masks, curves …          │ a popover │ │
//! │   (everything, with its own shortcuts)                          └──────────┘ │
//! └──────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The quick tools are one-tap ways to the common results, the same [`shnyp_photo`] code the CLI
//! and `shnyp mcp` use: a look with its strength, a social format, an export preset. They never
//! replace anything: every LightCraft control stays where it is, one click away.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::Duration;

use egui::{Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use lightcraft_engine::activity::Cancel;
use lightcraft_engine::catalog::PhotoId;
use lightcraft_engine::develop::DevelopSettings;
use serde_json::Value;
use shnyp_photo::ShnypState;
use shnyp_photo::crop::{self, Format};
use shnyp_photo::export::{self, Preset, Target};
use shnyp_photo::looks;

use crate::ShnypApp;
use crate::desktop::{self, Pick};
use crate::style::{self, ACCENT, Theme};

const LOOK_TILE_H: f32 = 72.0;
/// Long edge of Look thumbnails.
const LOOK_EDGE: usize = 192;
const LOOKS_W: f32 = 380.0;
const FORMAT_W: f32 = 300.0;
const EXPORT_W: f32 = 320.0;

/// A quick tool's popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popover {
    Looks,
    Format,
    Export,
}

impl Popover {
    pub const ALL: [Popover; 3] = [Popover::Looks, Popover::Format, Popover::Export];

    pub fn key(self) -> &'static str {
        match self {
            Popover::Looks => "looks",
            Popover::Format => "format",
            Popover::Export => "export",
        }
    }
}

struct ExportJob {
    handle: JoinHandle<Result<Vec<Value>, String>>,
    preset: Preset,
    /// Set by the activity row's ✕ or Quit Anyway: nothing more is written.
    cancel: Arc<AtomicBool>,
}

/// A file dialog that is open (the export folder).
struct Picking {
    answer: Receiver<Vec<PathBuf>>,
}

/// Looks rendered for one photo's settings.
struct LooksCache {
    photo: PhotoId,
    base: Arc<DevelopSettings>,
    /// LightCraft's preset list generation (looks added, renamed or deleted)
    presets: u64,
    /// (look id, shown name, group, settings at 100 %)
    looks: Vec<(String, String, String, DevelopSettings)>,
}

/// The Foto page's own state.
pub struct PhotoState {
    /// The open popover, and where its button is.
    pub open: Option<(Popover, Rect)>,
    /// The look whose strength can still be changed (shared with the `shnyp.*` commands).
    pub shnyp: ShnypState,
    /// A strength drag in progress.
    look_drag: bool,
    pub export_preset: Preset,
    pub export_dir: PathBuf,
    export: Option<ExportJob>,
    /// The files of the last finished export.
    pub last_export: Option<Vec<Value>>,
    looks: Option<LooksCache>,
    picking: Option<Picking>,
}

impl Default for PhotoState {
    fn default() -> Self {
        Self {
            open: None,
            shnyp: ShnypState::default(),
            look_drag: false,
            export_preset: Preset::Instagram,
            export_dir: export::default_dir(),
            export: None,
            last_export: None,
            looks: None,
            picking: None,
        }
    }
}

impl PhotoState {
    /// An export is running.
    pub fn exporting(&self) -> bool {
        self.export.is_some()
    }

    /// Which popover is open.
    pub fn popover(&self) -> Option<Popover> {
        self.open.map(|(p, _)| p)
    }
}

/// End a strength drag that is in progress: it becomes its undo step.
pub fn finish_drags(app: &mut ShnypApp) {
    let st = &mut app.photo;
    if std::mem::take(&mut st.look_drag)
        && let Some(l) = st.shnyp.look.clone()
        && let Err(e) = looks::set_look(&mut app.lc.session, &mut st.shnyp.look, l.photo, &l.preset_id, l.amount, false)
    {
        log::warn!("ending a strength drag: {e}");
    }
}

fn error(app: &mut ShnypApp, ctx: &egui::Context, text: impl Into<String>) {
    app.lc.toast_error(ctx, text);
}

/// The page's tools in shnyp's header (right-aligned): Looks, Format, Exportieren.
pub fn header_tools(app: &mut ShnypApp, ui: &mut Ui, t: &Theme) {
    let has_photo = app.lc.session.active().is_some();
    let open = app.photo.popover();
    // right to left: the primary action sits at the edge
    let r =
        style::button(ui, t, "  Exportieren  ", true, has_photo).on_hover_text("Das aktive Foto als Instagram-, Story- oder Originaldatei speichern");
    app.widgets.insert("export".into(), r.rect);
    if r.clicked() {
        toggle(app, Popover::Export, r.rect);
    }
    for (p, label, tip) in
        [(Popover::Format, "Format", "1:1, 4:5, 9:16, 16:9 mit einem Klick"), (Popover::Looks, "Looks", "Fertige Looks mit Stärke-Regler")]
    {
        let r = style::tool_button(ui, t, label, open == Some(p), has_photo).on_hover_text(tip);
        app.widgets.insert(format!("tool.{}", p.key()), r.rect);
        if r.clicked() {
            toggle(app, p, r.rect);
        }
    }
}

/// Open one of the Foto page's popovers by its key (`looks`, `format`, `export`) under its
/// button, as a click on it does; `toggle_it`: closed when it is open.
pub(crate) fn open_popover(app: &mut ShnypApp, key: &str, toggle_it: bool) -> bool {
    let Some(p) = Popover::ALL.into_iter().find(|p| p.key() == key) else { return false };
    if toggle_it && app.photo.popover() == Some(p) {
        finish_drags(app);
        app.photo.open = None;
        return true;
    }
    // Exportieren is the header's button, the others are tools
    let button = if p == Popover::Export { "export".to_string() } else { format!("tool.{key}") };
    let rect = app.widgets.get(&button).copied().unwrap_or_else(|| Rect::from_center_size(egui::pos2(1200.0, 20.0), egui::vec2(80.0, 28.0)));
    finish_drags(app);
    app.photo.open = Some((p, rect));
    true
}

fn toggle(app: &mut ShnypApp, p: Popover, button: Rect) {
    finish_drags(app);
    app.photo.open = if app.photo.popover() == Some(p) { None } else { Some((p, button)) };
}

/// Per frame on the Foto page, after LightCraft drew: the open popover, a dialog's answer.
pub fn show(app: &mut ShnypApp, ctx: &egui::Context) {
    // a strength drag whose release we never saw (the pointer left the window) ends here
    if app.photo.look_drag && !ctx.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
        finish_drags(app);
    }
    poll_pick(app);
    let Some((which, button)) = app.photo.open else { return };
    let Some(id) = app.lc.session.active() else {
        app.photo.open = None;
        return;
    };
    let t = Theme::get(ctx);
    let width = match which {
        Popover::Looks => LOOKS_W,
        Popover::Format => FORMAT_W,
        Popover::Export => EXPORT_W,
    };
    let screen = ctx.content_rect();
    let x = (button.right() - width).clamp(screen.left() + 8.0, (screen.right() - width - 8.0).max(screen.left() + 8.0));
    let pos = pos2(x, button.bottom() + 6.0);
    let max_h = (screen.bottom() - pos.y - 12.0).max(200.0);
    let area = egui::Area::new(egui::Id::new("shnyp_photo_popover")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(t.bar).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
            ui.set_width(width - 28.0);
            ui.set_max_height(max_h);
            match which {
                Popover::Looks => looks_panel(app, ui, &t, id),
                Popover::Format => format_panel(app, ui, &t, id),
                Popover::Export => export_panel(app, ui, &t),
            }
        });
    });
    app.widgets.insert(format!("popover.{}", which.key()), area.response.rect);
    // a click elsewhere (not on the popover, not on its button) or Escape closes it
    let outside =
        ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !button.contains(p)));
    if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        finish_drags(app);
        app.photo.open = None;
    }
}

fn looks_panel(app: &mut ShnypApp, ui: &mut Ui, t: &Theme, id: PhotoId) {
    let ctx = ui.ctx().clone();
    let current = looks::current(&app.photo.shnyp.look, &app.lc.session, id).cloned();

    // strength of the current look
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
                app.photo.look_drag = true;
            }
            if r.changed() {
                let live = app.photo.look_drag && !r.drag_stopped();
                let (preset, st, s) = (l.preset_id.clone(), &mut app.photo, &mut app.lc.session);
                if let Err(e) = looks::set_look(s, &mut st.shnyp.look, id, &preset, amount, live) {
                    error(app, &ctx, e.to_string());
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
    let presets = app.lc.session.presets.generation();
    let stale = app.photo.looks.as_ref().is_none_or(|c| c.photo != id || !Arc::ptr_eq(&c.base, &base) || c.presets != presets);
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
        app.photo.looks = Some(LooksCache { photo: id, base: base.clone(), presets, looks });
    }
    let tiles: Vec<(String, String, String, DevelopSettings)> = app.photo.looks.as_ref().map(|c| c.looks.clone()).unwrap_or_default();
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
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
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
        // "Original": take the current look off (strength 0, still one step)
        Some((look_id, _)) if look_id.is_empty() => {
            if let Some(l) = &current {
                let preset = l.preset_id.clone();
                let (st, s) = (&mut app.photo, &mut app.lc.session);
                if let Err(e) = looks::set_look(s, &mut st.shnyp.look, id, &preset, 0.0, false) {
                    error(app, &ctx, e.to_string());
                }
            }
        }
        Some((_, true)) => {}
        Some((look_id, false)) => {
            finish_drags(app);
            let (st, s) = (&mut app.photo, &mut app.lc.session);
            if let Err(e) = looks::set_look(s, &mut st.shnyp.look, id, &look_id, 100.0, false) {
                error(app, &ctx, e.to_string());
            }
        }
        None => {}
    }
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
    ui.painter().rect_stroke(r, CornerRadius::same(2), Stroke::new(1.5, color), StrokeKind::Inside);
    if f == Format::Original {
        ui.painter().rect_filled(r.shrink(4.0), CornerRadius::same(1), color.gamma_multiply(0.35));
    }
}

fn format_panel(app: &mut ShnypApp, ui: &mut Ui, t: &Theme, id: PhotoId) {
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
            if let Err(e) = crop::apply(&mut app.lc.session, id, f) {
                error(app, &ctx, format!("Format: {e}"));
            }
        }
        ui.add_space(6.0);
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new("Mittig gesetzt. Verschieben, drehen oder frei zuschneiden: mit dem Zuschneide-Werkzeug darunter.").size(12.0).color(t.dim),
    );
}

fn export_panel(app: &mut ShnypApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    style::heading(ui, t, "VORLAGE");
    let width = ui.available_width();
    for p in Preset::ALL {
        let on = app.photo.export_preset == p;
        let r = style::card(ui, t, p.label(), p.detail(), on, width);
        app.widgets.insert(format!("preset.{}", p.key()), r.rect);
        if r.clicked() {
            app.photo.export_preset = p;
        }
        ui.add_space(6.0);
    }
    style::heading(ui, t, "ZIEL");
    ui.label(RichText::new(app.photo.export_dir.to_string_lossy()).size(12.0).color(t.text));
    let r = style::button(ui, t, "Ordner wählen…", false, app.photo.picking.is_none());
    app.widgets.insert("export.dir".into(), r.rect);
    if r.clicked() {
        let start = Some(app.photo.export_dir.clone()).filter(|d| d.is_dir());
        match desktop::pick(&ctx, Pick::Folder, "Export-Ordner", start) {
            Ok(answer) => app.photo.picking = Some(Picking { answer }),
            Err(e) => error(app, &ctx, e),
        }
    }
    ui.add_space(14.0);
    let busy = app.photo.exporting();
    let r = style::wide_button(ui, if busy { "Wird exportiert…" } else { "Exportieren" }, width, !busy);
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
    } else if let Some(files) = &app.photo.last_export {
        ui.add_space(8.0);
        for f in files {
            if let Some(path) = f.get("path").and_then(Value::as_str) {
                let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(RichText::new(format!("Gespeichert: {name}")).size(12.5).color(t.text)).on_hover_text(path);
                ui.label(RichText::new(format!("{} × {} Pixel", f["width"], f["height"])).size(11.5).color(t.dim));
            }
        }
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Mehr Formate, Wasserzeichen und Stapel-Export: Datei › Exportieren in LightCraft.").size(12.0).color(t.dim));
}

fn poll_pick(app: &mut ShnypApp) {
    let Some(p) = &app.photo.picking else { return };
    let chosen = match p.answer.try_recv() {
        Ok(chosen) => chosen,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
    };
    app.photo.picking = None;
    if let Some(dir) = chosen.into_iter().next() {
        app.photo.export_dir = dir;
    }
}

/// Start exporting the active photo with the selected preset (on another thread).
pub fn start_export(app: &mut ShnypApp, ctx: &egui::Context) {
    if app.photo.exporting() {
        return;
    }
    let Some(id) = app.lc.session.active() else { return };
    finish_drags(app);
    let preset = app.photo.export_preset;
    let target = Target::Dir(app.photo.export_dir.clone());
    match export::prepare(&mut app.lc.session, id, preset, &target) {
        Ok(plan) => {
            // an activity task: LightCraft's quit prompt warns about it, and Quit Anyway (or the
            // row's ✕) cancels it: the photo being rendered is then not written
            let cancel = Arc::new(AtomicBool::new(false));
            let row = app.lc.session.activity.start("export", "Exporting", Cancel::Flag(cancel.clone()));
            let flag = cancel.clone();
            let spawned = std::thread::Builder::new().name("shnyp-export".into()).spawn(move || {
                let _row = row; // listed exactly as long as the export runs
                plan.run_with_cancel(&flag)
            });
            match spawned {
                Ok(handle) => {
                    app.photo.export = Some(ExportJob { handle, preset, cancel });
                    app.photo.last_export = None;
                }
                Err(e) => error(app, ctx, format!("Export: {e}")),
            }
        }
        Err(e) => error(app, ctx, format!("Export: {e}")),
    }
}

/// Wait up to `timeout` for a running export to finish (when the window closes), unless it was
/// cancelled: then nothing more gets written and there is nothing to wait for.
pub(crate) fn wait_for_export(app: &mut ShnypApp, timeout: Duration) {
    let start = std::time::Instant::now();
    let waiting = |j: &ExportJob| !j.handle.is_finished() && !j.cancel.load(std::sync::atomic::Ordering::Relaxed);
    while app.photo.export.as_ref().is_some_and(waiting) && start.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(20));
    }
    if app.photo.export.as_ref().is_some_and(|j| j.handle.is_finished())
        && let Some(job) = app.photo.export.take()
    {
        match job.handle.join() {
            Ok(Ok(files)) => log::info!("export finished on exit: {} file(s)", files.len()),
            Ok(Err(e)) => log::error!("export on exit: {e}"),
            Err(_) => log::error!("export on exit: the export thread stopped unexpectedly"),
        }
    }
}

/// Collect a finished export (every frame, whichever page is showing).
pub(crate) fn poll_export(app: &mut ShnypApp, ctx: &egui::Context) {
    let Some(job) = &app.photo.export else { return };
    if !job.handle.is_finished() {
        ctx.request_repaint_after(Duration::from_millis(100));
        return;
    }
    let Some(job) = app.photo.export.take() else { return };
    match job.handle.join() {
        Ok(Ok(files)) => {
            let first = files.first().cloned().unwrap_or(Value::Null);
            let name =
                first.get("path").and_then(Value::as_str).and_then(|p| std::path::Path::new(p).file_name()).map(|n| n.to_string_lossy().into_owned());
            match name {
                Some(n) => app.lc.toast(ctx, format!("{} gespeichert: {n} ({} × {})", job.preset.label(), first["width"], first["height"])),
                None => error(app, ctx, format!("{}: nichts gespeichert", job.preset.label())),
            }
            app.photo.last_export = Some(files);
        }
        Ok(Err(e)) if e == export::CANCELLED => app.lc.toast(ctx, "Export abgebrochen"),
        Ok(Err(e)) => error(app, ctx, format!("Export fehlgeschlagen: {e}")),
        Err(_) => error(app, ctx, "Export unerwartet abgebrochen"),
    }
}
