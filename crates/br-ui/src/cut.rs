//! The video pages' quick tools: CapCut's one-tap results over FilmCraft's complete editor.
//!
//! ```text
//! ┌ BR  Medien [Schnitt] Farbe Ton Ausgabe Foto   Datei Bearbeiten Clip …     Format  [Exportieren] ┐
//! ├ + Medien  Text  Untertitel  Musik  Filter  Übergang │ Teilen  Löschen  Tempo  Ton trennen │ ↶  ↷ ┤
//! │  FilmCraft: Projekt · Programm · Eigenschaften … Timeline (everything, with its own shortcuts)  │
//! └──────────────────────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Every tool is one of BR's `br.video.*` commands (one undo step, the same code `br run` and
//! `br mcp` use), built from FilmCraft's own, so whatever it does can be refined in FilmCraft's
//! panels. Nothing is hidden or replaced: FilmCraft's menus, panels and shortcuts stay as they are.
//! A new, empty project opens on CapCut's start screen: pick a format, add media, or open a
//! project.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use br_video::speech::{CaptionJob, CaptionOptions};
use br_video::{EXPORT_PRESETS, ExportPreset, Format};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use filmcraft_ui_egui::dock::PanelKind;
use filmcraft_ui_egui::icons::Icon;
use serde_json::{Value, json};

use crate::desktop::{self, Pick};
use crate::style::{self, ACCENT, Theme};
use crate::{BrApp, Page};

/// The tool row's height.
pub const TOOLS_H: f32 = 40.0;
const FORMAT_W: f32 = 300.0;
const EXPORT_W: f32 = 340.0;
const CAPTIONS_W: f32 = 320.0;
const FILTER_W: f32 = 360.0;
const SPEED_W: f32 = 280.0;

/// A quick tool's popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popover {
    Captions,
    Filter,
    Speed,
    Format,
    Export,
}

impl Popover {
    pub const ALL: [Popover; 5] = [Popover::Captions, Popover::Filter, Popover::Speed, Popover::Format, Popover::Export];

    pub fn key(self) -> &'static str {
        match self {
            Popover::Captions => "captions",
            Popover::Filter => "filter",
            Popover::Speed => "speed",
            Popover::Format => "format",
            Popover::Export => "export",
        }
    }

    fn width(self) -> f32 {
        match self {
            Popover::Captions => CAPTIONS_W,
            Popover::Filter => FILTER_W,
            Popover::Speed => SPEED_W,
            Popover::Format => FORMAT_W,
            Popover::Export => EXPORT_W,
        }
    }
}

/// The spoken languages offered for automatic captions (Whisper knows many more; FilmCraft's
/// transcript settings list them all).
pub const LANGUAGES: [(&str, &str); 9] = [
    ("auto", "Automatisch erkennen"),
    ("de", "Deutsch"),
    ("en", "Englisch"),
    ("fr", "Französisch"),
    ("es", "Spanisch"),
    ("it", "Italienisch"),
    ("nl", "Niederländisch"),
    ("pl", "Polnisch"),
    ("tr", "Türkisch"),
];

/// Speeds in the Tempo popover.
pub const SPEEDS: [f64; 7] = [0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0];

/// What an open file dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickFor {
    Media,
    Music,
    ExportFolder,
}

/// A video export FilmCraft runs (a job in its session).
struct ExportJob {
    job: u64,
    /// Where the file goes once it is complete.
    path: PathBuf,
    /// Where it is written until then (hidden).
    part: PathBuf,
}

/// The video pages' own state.
pub struct CutState {
    /// The open popover, and where its button is.
    pub open: Option<(Popover, Rect)>,
    pub export_preset: &'static ExportPreset,
    pub export_dir: PathBuf,
    export: Option<ExportJob>,
    /// The file of the last finished export.
    pub last_export: Option<PathBuf>,
    captions: Option<CaptionJob>,
    /// The language chosen for automatic captions (a key of [`LANGUAGES`]).
    pub caption_language: &'static str,
    /// A new format makes the clips fill the frame instead of fitting into it.
    pub fill: bool,
    /// The start screen was left without a video ("Leerer Editor").
    pub start_dismissed: bool,
    picking: Option<(PickFor, Receiver<Vec<PathBuf>>)>,
    /// FilmCraft's looks, read once.
    looks: Option<Vec<(String, String)>>,
    /// Where FilmCraft was drawn this frame (the start screen covers it).
    area: Option<Rect>,
    /// How the last caption recognition ended (for a control request waiting on it).
    caption_outcome: Option<Result<Value, String>>,
    /// The window closes once the running export is written (closing was asked for).
    close_after_export: bool,
}

impl Default for CutState {
    fn default() -> Self {
        Self {
            open: None,
            export_preset: &EXPORT_PRESETS[0],
            export_dir: br_video::export_dir(),
            export: None,
            last_export: None,
            captions: None,
            caption_language: "auto",
            fill: false,
            start_dismissed: false,
            picking: None,
            looks: None,
            area: None,
            caption_outcome: None,
            close_after_export: false,
        }
    }
}

impl CutState {
    /// Which popover is open.
    pub fn popover(&self) -> Option<Popover> {
        self.open.map(|(p, _)| p)
    }

    /// A video export is running.
    pub fn exporting(&self) -> bool {
        self.export.is_some()
    }

    /// Captions are being recognised.
    pub fn recognising(&self) -> bool {
        self.captions.is_some()
    }

    /// Something runs in the background (keep drawing frames for its progress).
    pub fn busy(&self) -> bool {
        self.export.is_some() || self.captions.is_some() || self.picking.is_some()
    }
}

/// A message in FilmCraft's toast (shown over the video pages).
pub(crate) fn toast(app: &mut BrApp, ctx: &egui::Context, text: impl Into<String>) {
    let text = text.into();
    log::info!("{text}");
    app.fc.toast = Some((text, ctx.input(|i| i.time)));
}

/// How long a message stays: long enough to read it.
fn toast_seconds(text: &str) -> f64 {
    (3.0 + text.chars().count() as f64 / 18.0).min(10.0)
}

/// The message in FilmCraft's toast (BR's and FilmCraft's own), over the bottom of the video
/// pages (FilmCraft keeps it but does not draw it).
pub fn draw_toast(app: &mut BrApp, ctx: &egui::Context) {
    let Some((text, since)) = app.fc.toast.clone() else { return };
    let age = ctx.input(|i| i.time) - since;
    if age > toast_seconds(&text) || age < 0.0 {
        app.fc.toast = None;
        return;
    }
    // on the video pages over FilmCraft's area, on Foto over the window
    let area = app.cut.area.filter(|_| app.page.is_video()).unwrap_or(ctx.content_rect());
    let t = Theme::get(ctx);
    let max_w = (area.width() - 48.0).clamp(200.0, 640.0);
    let r = egui::Area::new(egui::Id::new("br_toast"))
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -(ctx.content_rect().bottom() - area.bottom()) - 36.0))
        .interactable(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(t.bar).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(14, 10)).show(
                ui,
                |ui| {
                    ui.set_max_width(max_w);
                    ui.label(RichText::new(&text).size(13.0).color(t.text));
                },
            )
        });
    app.widgets.insert("toast".into(), r.response.rect);
    // a click takes it away
    if r.response.clicked() {
        app.fc.toast = None;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(200));
}

/// The window is asked to close while a video export or a caption recognition runs: the export
/// is finished first (the window then closes); asked again, it is stopped and the window closes.
pub fn on_close_request(app: &mut BrApp, ctx: &egui::Context) {
    let running = running_exports(app);
    if running.is_empty() || app.cut.close_after_export {
        // the window closes: what runs in the background stops
        for job in running {
            let _ = br_video::execute(&mut app.fc.session, "jobs.cancel", &json!({"job": job}));
        }
        if let Some(job) = &app.cut.captions {
            job.cancel();
        }
        return;
    }
    app.cut.close_after_export = true;
    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    if !app.page.is_video() {
        app.open(Page::Schnitt);
    }
    toast(app, ctx, "Das Video wird noch gespeichert: BR schließt sich, sobald es fertig ist. Noch einmal schließen bricht den Export ab.");
}

/// FilmCraft's export jobs that are still running (BR's Exportieren, the Ausgabe page, its queue,
/// an agent's).
fn running_exports(app: &BrApp) -> Vec<u64> {
    app.fc.session.jobs.iter().filter(|j| j.result.lock().unwrap_or_else(|e| e.into_inner()).is_none()).map(|j| j.id).collect()
}

/// Run one of BR's video commands (or FilmCraft's) on the video project; a failure is told in the
/// toast.
pub(crate) fn run(app: &mut BrApp, ctx: &egui::Context, id: &str, params: Value) -> Option<Value> {
    match br_video::execute_with(&mut app.fc.session, &mut app.video.watch, id, &params) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("{id}: {e}");
            toast(app, ctx, readable(&e));
            None
        }
    }
}

/// FilmCraft's errors start with the command id: the reader wants only the reason.
fn readable(e: &str) -> String {
    let mut e = e.trim();
    // "br.video.x: …", FilmCraft's "invalid parameters for `x`: …" and "command `x` is not
    // available right now: …"
    loop {
        // a command id: `br.video.add`, `file.import` (not a file name, which has a `/` or an
        // extension after the last dot only)
        let is_id =
            |h: &str| h.starts_with(|c: char| c.is_ascii_lowercase()) && h.contains('.') && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
        let next = match e.split_once(": ") {
            Some((cmd, rest))
                if is_id(cmd)
                    && !cmd.rsplit('.').next().is_some_and(|ext| matches!(ext, "mp4" | "mov" | "jpg" | "png" | "wav" | "mp3" | "m4a" | "srt")) =>
            {
                rest
            }
            Some((head, rest)) if head.starts_with("invalid parameters for `") || (head.starts_with("command `") && head.ends_with("right now")) => {
                rest
            }
            _ => break,
        };
        e = next.trim();
    }
    match e {
        "cancelled" => "abgebrochen".to_string(),
        _ => e.to_string(),
    }
}

/// Run one of FilmCraft's menu commands as its menus do (dialogs, file pickers, status line).
fn invoke(app: &mut BrApp, ctx: &egui::Context, id: &str) {
    if let Err(e) = filmcraft_ui_egui::menus::invoke(&mut app.fc, ctx, id, json!({})) {
        toast(app, ctx, readable(&e));
    }
}

fn has_video(app: &BrApp) -> bool {
    app.fc.session.active_sequence().is_some()
}

fn toggle(app: &mut BrApp, p: Popover, button: Rect) {
    app.cut.open = if app.cut.popover() == Some(p) { None } else { Some((p, button)) };
}

/// Add media files to the video: one after another at the end (a new video in the first clip's
/// shape when there is none yet).
pub fn add_paths(app: &mut BrApp, ctx: &egui::Context, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    let n = paths.len();
    if let Some(st) = run(app, ctx, "br.video.add", json!({"paths": paths})) {
        let failed: Vec<String> =
            st["errors"].as_array().into_iter().flatten().map(|e| e.as_str().map_or_else(|| e.to_string(), str::to_string)).collect();
        let ok = n - failed.len().min(n);
        let mut text = if ok == 1 { "1 Datei hinzugefügt".to_string() } else { format!("{ok} Dateien hinzugefügt") };
        if !failed.is_empty() {
            text += &format!("; nicht lesbar: {}", failed.join(" · "));
        }
        toast(app, ctx, text);
    }
}

/// Files given to the window (command line, "open with"): a video project is opened, videos and
/// sound are added to the video, subtitle files become captions. Problems are told in the toast.
pub fn open_files(app: &mut BrApp, ctx: &egui::Context, paths: Vec<String>) {
    // absolute: FilmCraft keeps paths as given (a saved project would lose its media)
    let paths: Vec<String> = paths.into_iter().map(|p| std::path::absolute(&p).map(|a| a.to_string_lossy().into_owned()).unwrap_or(p)).collect();
    // a dropped folder: the media in it
    let paths = br_video::expand_folders(&paths);
    if paths.is_empty() {
        toast(app, ctx, "Darin sind keine Videos, Fotos oder Musik, die BR öffnen kann");
        return;
    }
    let kind = |p: &String| br_video::file_kind(p).unwrap_or("");
    if let Some(project) = paths.iter().find(|p| kind(p) == "project") {
        if let Err(e) = filmcraft_ui_egui::menus::invoke(&mut app.fc, ctx, "file.open", json!({"path": project})) {
            toast(app, ctx, readable(&e));
        }
        app.cut.start_dismissed = true;
    }
    let media: Vec<String> = paths.iter().filter(|p| matches!(kind(p), "video" | "audio" | "image")).cloned().collect();
    add_paths(app, ctx, media);
    for subs in paths.iter().filter(|p| kind(p) == "caption") {
        run(app, ctx, "br.video.captions", json!({"path": subs}));
    }
}

// ── the header: Format, Exportieren ─────────────────────────────────────────────────────────────

/// The video pages' tools in BR's header (right-aligned): Format and Exportieren.
pub fn header_tools(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let video = has_video(app);
    let open = app.cut.popover();
    let job = app.cut.export.as_ref().map(|j| j.job).or_else(|| running_exports(app).first().copied());
    let label = match job.and_then(|j| job_progress(app, j)) {
        Some(f) => format!("  Export {} %  ", (f * 100.0).round()),
        None => "  Exportieren  ".to_string(),
    };
    // something to export: a video with something in it
    let has_content = app.fc.session.active_sequence().is_some_and(|q| q.duration().0 > 0);
    let r = style::button(ui, t, &label, true, has_content).on_hover_text("Das Video als MP4 für TikTok, Reels, YouTube … speichern");
    app.widgets.insert("video.export".into(), r.rect);
    if r.clicked() {
        toggle(app, Popover::Export, r.rect);
    }
    let r = style::tool_button(ui, t, "Format", open == Some(Popover::Format), video).on_hover_text("9:16, 16:9, 1:1 oder 4:5 mit einem Klick");
    app.widgets.insert("video.format".into(), r.rect);
    if r.clicked() {
        toggle(app, Popover::Format, r.rect);
    }
}

// ── the tool row (Schnitt) ──────────────────────────────────────────────────────────────────────

/// A tool in the row: FilmCraft's icon and a label (CapCut's row of tools), highlighted while its
/// popover is open. Without a label it is an icon button.
pub(crate) fn tool(ui: &mut Ui, t: &Theme, icon: Icon, text: &str, tip: &str, open: bool, enabled: bool) -> egui::Response {
    let color = if enabled { t.text } else { t.disabled };
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(13.0), color);
    let w = if text.is_empty() { 30.0 } else { 8.0 + 16.0 + 6.0 + galley.size().x + 10.0 };
    let (rect, r) = ui.allocate_exact_size(vec2(w, 28.0), if enabled { Sense::click() } else { Sense::hover() });
    if open || (enabled && r.hovered()) {
        ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
    }
    if open {
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, ACCENT), StrokeKind::Inside);
    }
    let icon_rect = if text.is_empty() {
        Rect::from_center_size(rect.center(), vec2(16.0, 16.0))
    } else {
        Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0))
    };
    filmcraft_ui_egui::icons::paint(ui.painter(), icon_rect, icon, if enabled && (open || r.hovered()) { ACCENT } else { color });
    if !text.is_empty() {
        ui.painter().galley(pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, color);
    }
    let r = r.on_hover_text(tip);
    if enabled { r.on_hover_cursor(egui::CursorIcon::PointingHand) } else { r }
}

fn divider(ui: &mut Ui, t: &Theme) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::ZERO, t.divider);
    ui.add_space(4.0);
}

/// The Schnitt page's tool row, above FilmCraft.
pub fn toolbar(app: &mut BrApp, ui: &mut Ui) {
    let t = Theme::get(ui.ctx());
    egui::Panel::top("br_cut_tools")
        .exact_size(TOOLS_H)
        .frame(egui::Frame::NONE.fill(t.bar).inner_margin(egui::Margin::symmetric(10, 0)).stroke(Stroke::new(1.0, t.divider)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                tools(app, ui, &t);
            });
        });
    // what runs in the background, in a row of its own under the tools
    if app.cut.captions.is_some() {
        egui::Panel::top("br_cut_progress").exact_size(28.0).frame(egui::Frame::NONE.fill(t.bar).inner_margin(egui::Margin::symmetric(12, 0))).show(
            ui,
            |ui| {
                ui.horizontal_centered(|ui| {
                    let Some(job) = &app.cut.captions else { return };
                    let (f, text) = job.progress();
                    ui.label(RichText::new("Untertitel").size(12.0).color(t.text));
                    ui.add(egui::ProgressBar::new(f).desired_width(120.0).desired_height(6.0));
                    let w = (ui.available_width() - 110.0).max(40.0);
                    let g = style::elided(ui, &text, 12.0, t.dim, w);
                    ui.label(g);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = tool(ui, &t, Icon::Close, "Abbrechen", "Die Spracherkennung abbrechen", false, true);
                        app.widgets.insert("cut.captions.cancel".into(), r.rect);
                        if r.clicked()
                            && let Some(job) = &app.cut.captions
                        {
                            job.cancel();
                        }
                    });
                });
            },
        );
    }
}

fn tools(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    let video = has_video(app);
    let selected = br_video::selection(&app.fc.session) > 0;
    let (cuts, linked) = {
        let s = &app.fc.session;
        let q = s.active_sequence();
        let main = q.and_then(|q| q.video_tracks.first()).map_or(0, |t| t.items.len());
        let linked = q.is_some_and(|q| s.state.selection.iter().any(|c| q.find_item(*c).is_some_and(|(_, it)| it.link.is_some())));
        (main >= 2, linked)
    };
    let open = app.cut.popover();
    let picking = app.cut.picking.is_some();
    let click = |app: &mut BrApp, key: &str, r: egui::Response| {
        app.widgets.insert(format!("cut.{key}"), r.rect);
        r.clicked().then_some(r.rect)
    };

    // add
    let r = tool(ui, t, Icon::Plus, "Medien", "Videos, Fotos und Ton ans Ende des Videos (oder hierher ziehen)", false, !picking);
    if click(app, "add", r).is_some() {
        pick(app, &ctx, PickFor::Media);
    }
    let r = tool(ui, t, Icon::Type, "Text", "Ein Titel an der Abspielposition; Schrift und Stil rechts unter Eigenschaften", false, video);
    if click(app, "text", r).is_some() && run(app, &ctx, "br.video.text", json!({"text": "Dein Text"})).is_some() {
        app.fc.show_panel(PanelKind::Properties);
    }
    let r = tool(ui, t, Icon::Captions, "Untertitel", "Automatisch aus der Sprache oder aus einer Datei", open == Some(Popover::Captions), video);
    if let Some(rect) = click(app, "captions", r) {
        toggle(app, Popover::Captions, rect);
    }
    let r = tool(ui, t, Icon::Audio, "Musik", "Ein Musikstück unter das Video, unter Sprache leiser", false, video && !picking);
    if click(app, "music", r).is_some() {
        pick(app, &ctx, PickFor::Music);
    }
    let r = tool(ui, t, Icon::Sparkle, "Filter", "Ein Look für die ausgewählten Clips (ohne Auswahl: alle)", open == Some(Popover::Filter), video);
    if let Some(rect) = click(app, "filter", r) {
        toggle(app, Popover::Filter, rect);
    }
    let r = tool(ui, t, Icon::Automate, "Übergang", "Überblenden zwischen den ausgewählten Clips (ohne Auswahl: an allen Schnitten)", false, cuts);
    if click(app, "transition", r).is_some()
        && let Some(v) = run(app, &ctx, "br.video.transition", json!({}))
    {
        let n = v["transitions"].as_u64().unwrap_or(0);
        toast(app, &ctx, if n == 1 { "1 Übergang".to_string() } else { format!("{n} Übergänge") });
    }
    divider(ui, t);

    // the clips
    let r = tool(ui, t, Icon::Razor, "Teilen", "Alle Clips an der Abspielposition teilen", false, video);
    if click(app, "split", r).is_some() {
        run(app, &ctx, "br.video.split", json!({}));
    }
    let r = tool(ui, t, Icon::Trash, "Löschen", "Die ausgewählten Clips löschen und die Lücke schließen", false, selected);
    if click(app, "delete", r).is_some() {
        run(app, &ctx, "br.video.delete", json!({}));
    }
    let r = tool(ui, t, Icon::Stopwatch, "Tempo", "Die ausgewählten Clips schneller oder langsamer", open == Some(Popover::Speed), selected);
    if let Some(rect) = click(app, "speed", r) {
        toggle(app, Popover::Speed, rect);
    }
    let r =
        tool(ui, t, Icon::Link, "Ton trennen", "Bild und Ton der ausgewählten Clips trennen (wieder verbinden: Clip › Verknüpfen)", false, linked);
    if click(app, "detach", r).is_some() && run(app, &ctx, "br.video.detachAudio", json!({})).is_some() {
        toast(app, &ctx, "Ton getrennt: Bild und Ton lassen sich jetzt einzeln bewegen und löschen");
    }
    divider(ui, t);
    let h = &app.fc.session.history;
    let (can_undo, can_redo) = (!h.undo.is_empty(), !h.redo.is_empty());
    let undo_tip = h.undo.last().map_or("Rückgängig".to_string(), |u| format!("Rückgängig: {}", u.0));
    let redo_tip = h.redo.last().map_or("Wiederholen".to_string(), |u| format!("Wiederholen: {}", u.0));
    let r = tool(ui, t, Icon::Undo, "", &undo_tip, false, can_undo);
    if click(app, "undo", r).is_some() {
        invoke(app, &ctx, "edit.undo");
    }
    let r = tool(ui, t, Icon::Redo, "", &redo_tip, false, can_redo);
    if click(app, "redo", r).is_some() {
        invoke(app, &ctx, "edit.redo");
    }
}

// ── file dialogs ────────────────────────────────────────────────────────────────────────────────

fn pick(app: &mut BrApp, ctx: &egui::Context, what: PickFor) {
    let (kind, title, start) = match what {
        PickFor::Media => (Pick::Media, "Medien hinzufügen", desktop::import_dir()),
        PickFor::Music => (Pick::Music, "Musik hinzufügen", desktop::import_dir()),
        PickFor::ExportFolder => (Pick::Folder, "Export-Ordner", Some(app.cut.export_dir.clone()).filter(|d| d.is_dir())),
    };
    match desktop::pick(ctx, kind, title, start) {
        Ok(answer) => app.cut.picking = Some((what, answer)),
        Err(e) => toast(app, ctx, e),
    }
}

fn poll_pick(app: &mut BrApp, ctx: &egui::Context) {
    let Some((what, answer)) = &app.cut.picking else { return };
    let chosen = match answer.try_recv() {
        Ok(chosen) => chosen,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Vec::new(),
    };
    let what = *what;
    app.cut.picking = None;
    let paths: Vec<String> = chosen.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    match what {
        PickFor::Media => add_paths(app, ctx, paths),
        PickFor::Music => {
            if let Some(path) = paths.into_iter().next()
                && run(app, ctx, "br.video.music", json!({"path": path})).is_some()
            {
                toast(app, ctx, "Musik hinzugefügt: unter Sprache automatisch leiser");
            }
        }
        PickFor::ExportFolder => {
            if let Some(dir) = chosen.into_iter().next() {
                app.cut.export_dir = dir;
            }
        }
    }
}

// ── background work: export, captions ───────────────────────────────────────────────────────────

/// How far a FilmCraft job is (`None`: no such job).
fn job_progress(app: &BrApp, job: u64) -> Option<f32> {
    app.fc.session.jobs.iter().find(|j| j.id == job).map(|j| j.progress.fraction())
}

/// Per frame on every page: a finished export or recognition is told (and captions are put in).
pub fn poll(app: &mut BrApp, ctx: &egui::Context) {
    poll_pick(app, ctx);
    if app.cut.close_after_export && running_exports(app).is_empty() && app.cut.export.is_none() {
        app.cut.close_after_export = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    if let Some(e) = &app.cut.export {
        let (job, path, part) = (e.job, e.path.clone(), e.part.clone());
        if let Some(r) = br_video::settle_export(&app.fc.session, job, &part, &path) {
            app.cut.export = None;
            match r {
                Ok(_) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    toast(app, ctx, format!("Video gespeichert: {name}"));
                    app.cut.last_export = Some(path);
                }
                Err(e) => toast(app, ctx, format!("Export: {}", readable(&e))),
            }
        }
    }
    if let Some(job) = &app.cut.captions
        && let Some(r) = job.poll()
    {
        let Some(job) = app.cut.captions.take() else { return };
        let answer = r.and_then(|done| br_video::speech::apply(&mut app.fc.session, job.sequence, done));
        match &answer {
            Ok(v) => {
                let n = v["captions"].as_u64().unwrap_or(0);
                toast(app, ctx, format!("{n} Untertitel erstellt: Text und Stil im Text-Panel › Untertitel"));
            }
            Err(e) => toast(app, ctx, format!("Untertitel: {e}")),
        }
        app.cut.caption_outcome = Some(answer);
    }
}

fn start_export(app: &mut BrApp, ctx: &egui::Context) {
    let dir = app.cut.export_dir.clone();
    if dir.to_str().is_none() {
        toast(app, ctx, format!("{}: der Ordnername ist kein gültiger Text (UTF-8); bitte einen anderen Ordner wählen", dir.display()));
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        toast(app, ctx, format!("{}: {e}", dir.display()));
        return;
    }
    let name = app.fc.session.state.active_sequence.and_then(|q| app.fc.session.project.item(q)).map(|i| i.name.clone()).unwrap_or_default();
    let preset = app.cut.export_preset;
    let path = br_video::export_path(&dir, &name, preset);
    let params = json!({"preset": preset.key, "path": path.to_string_lossy(), "wait": false});
    if let Some(v) = run(app, ctx, "br.video.export", params)
        && let Some(job) = v.get("job").and_then(Value::as_u64)
    {
        let part = v.get("part").and_then(Value::as_str).map_or_else(|| br_video::part_path(&path), PathBuf::from);
        app.cut.export = Some(ExportJob { job, path, part });
        app.cut.last_export = None;
    }
}

fn start_captions(app: &mut BrApp, ctx: &egui::Context) {
    let opts = CaptionOptions { language: Some(app.cut.caption_language.to_string()), model: None };
    if let Err(e) = start_captions_with(app, ctx, opts) {
        toast(app, ctx, e);
    }
}

/// Start recognising captions in the background (the tool row shows the progress).
pub fn start_captions_with(app: &mut BrApp, _ctx: &egui::Context, opts: CaptionOptions) -> Result<(), String> {
    if app.cut.captions.is_some() {
        return Err("Untertitel werden schon erkannt".into());
    }
    let job = br_video::speech::start(&app.fc.session, &opts)?;
    app.cut.captions = Some(job);
    app.cut.caption_outcome = None;
    app.cut.open = None;
    Ok(())
}

/// The caption recognition for `br.video.jobs`: running (progress, status) or how it last ended.
pub fn caption_status(app: &BrApp) -> Value {
    match (&app.cut.captions, &app.cut.caption_outcome) {
        (Some(j), _) => {
            let (f, text) = j.progress();
            json!({"running": true, "progress": f, "status": text})
        }
        (None, Some(Ok(v))) => json!({"done": true, "result": v}),
        (None, Some(Err(e))) => json!({"done": true, "error": e}),
        (None, None) => Value::Null,
    }
}

/// Stop the caption recognition; whether one was running.
pub fn cancel_captions(app: &BrApp) -> bool {
    app.cut.captions.as_ref().map(CaptionJob::cancel).is_some()
}

/// How the recognition ended, once (for a control request waiting on it).
pub fn caption_result(app: &mut BrApp) -> Option<Result<Value, String>> {
    if app.cut.captions.is_some() {
        return None;
    }
    Some(app.cut.caption_outcome.take().unwrap_or_else(|| Err("die Erkennung wurde beendet".into())))
}

// ── popovers ────────────────────────────────────────────────────────────────────────────────────

/// Per frame on the video pages, after FilmCraft drew: the start screen, the open popover.
pub fn show(app: &mut BrApp, ctx: &egui::Context) {
    if start_screen_visible(app) {
        start_screen(app, ctx);
    }
    // FilmCraft's Quick Export (its own header had the button), under BR's Exportieren
    if app.fc.ui.export.quick_open {
        let anchor = app
            .widgets
            .get("video.export")
            .map_or(egui::pos2(ctx.content_rect().right() - 360.0, 44.0), |r| egui::pos2(r.right() - 340.0, r.bottom() + 6.0));
        filmcraft_ui_egui::panels::export_mode::quick_export(&mut app.fc, ctx, anchor);
    }
    let Some((which, button)) = app.cut.open else { return };
    // a tool that needs a video closes when there is none (undo of its creation)
    if !has_video(app) {
        app.cut.open = None;
        return;
    }
    let t = Theme::get(ctx);
    let width = which.width();
    let screen = ctx.content_rect();
    let x = match which {
        // the header's tools sit at the right edge: their popovers open to the left
        Popover::Format | Popover::Export => button.right() - width,
        _ => button.left(),
    }
    .clamp(screen.left() + 8.0, (screen.right() - width - 8.0).max(screen.left() + 8.0));
    let pos = pos2(x, button.bottom() + 6.0);
    let max_h = (screen.bottom() - pos.y - 12.0).max(200.0);
    let area = egui::Area::new(egui::Id::new("br_cut_popover")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(t.bar).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
            ui.set_width(width - 28.0);
            ui.set_max_height(max_h);
            egui::ScrollArea::vertical().max_height(max_h - 28.0).show(ui, |ui| match which {
                Popover::Captions => captions_panel(app, ui, &t),
                Popover::Filter => filter_panel(app, ui, &t),
                Popover::Speed => speed_panel(app, ui, &t),
                Popover::Format => format_panel(app, ui, &t),
                Popover::Export => export_panel(app, ui, &t),
            });
        });
    });
    app.widgets.insert(format!("popover.{}", which.key()), area.response.rect);
    let outside =
        ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !button.contains(p)));
    if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.cut.open = None;
    }
}

/// A row that can be chosen: a title, a detail line, an outline while it is the current one.
fn choice(ui: &mut Ui, t: &Theme, title: &str, detail: &str, on: bool, width: f32) -> egui::Response {
    let h = if detail.is_empty() { 34.0 } else { 50.0 };
    let (rect, r) = ui.allocate_exact_size(vec2(width, h), Sense::click());
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
    let p = ui.painter();
    if detail.is_empty() {
        p.text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, title, FontId::proportional(13.5), t.text);
    } else {
        p.text(rect.left_top() + vec2(12.0, 9.0), Align2::LEFT_TOP, title, FontId::proportional(13.5), t.text);
        p.text(rect.left_top() + vec2(12.0, 28.0), Align2::LEFT_TOP, detail, FontId::proportional(11.5), t.dim);
    }
    r
}

fn note(ui: &mut Ui, t: &Theme, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(t.dim));
}

fn link(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(ACCENT)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn captions_panel(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    let width = ui.available_width();
    style::heading(ui, t, "AUTOMATISCH AUS DER SPRACHE");
    let available = br_video::speech::available(&app.fc.session);
    egui::ComboBox::from_id_salt("br_caption_language")
        .width(width)
        .selected_text(LANGUAGES.iter().find(|l| l.0 == app.cut.caption_language).map_or("", |l| l.1))
        .show_ui(ui, |ui| {
            for (key, label) in LANGUAGES {
                ui.selectable_value(&mut app.cut.caption_language, key, label);
            }
        });
    ui.add_space(8.0);
    let busy = app.cut.recognising();
    let r = style::wide_button(ui, if busy { "Wird erkannt …" } else { "Untertitel erkennen" }, width, available && !busy);
    app.widgets.insert("captions.auto".into(), r.rect);
    if r.clicked() {
        start_captions(app, &ctx);
    }
    ui.add_space(6.0);
    if !available {
        note(ui, t, "Dieses BR wurde ohne Spracherkennung gebaut (Funktion `whisper`).");
    } else if !br_video::speech::model_ready(&app.fc.session) {
        note(ui, t, "Beim ersten Mal wird das Sprachmodell geladen (etwa 150 MB). Die Erkennung läuft auf diesem Rechner, nichts verlässt ihn.");
    } else {
        note(ui, t, "Die Erkennung läuft auf diesem Rechner, nichts verlässt ihn.");
    }
    style::heading(ui, t, "AUS EINER DATEI");
    let r = style::button(ui, t, "Untertitel-Datei laden (SRT, VTT …)…", false, true);
    app.widgets.insert("captions.file".into(), r.rect);
    if r.clicked() {
        app.cut.open = None;
        invoke(app, &ctx, "captions.import");
    }
    ui.add_space(8.0);
    if link(ui, "Untertitel bearbeiten: Text-Panel › Untertitel").clicked() {
        app.cut.open = None;
        app.fc.show_panel(PanelKind::Text);
    }
}

fn filter_panel(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    if app.cut.looks.is_none() {
        let list = br_video::execute(&mut app.fc.session, "lumetri.presets", &json!({})).unwrap_or_default();
        let looks = list["presets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p["name"].as_str()?.to_string(), p["folder"].as_str().unwrap_or("").to_string())))
            .collect();
        app.cut.looks = Some(looks);
    }
    let looks = app.cut.looks.clone().unwrap_or_default();
    let selected = br_video::selection(&app.fc.session);
    style::heading(ui, t, if selected > 0 { "LOOK FÜR DIE AUSGEWÄHLTEN CLIPS" } else { "LOOK FÜR ALLE CLIPS" });
    let width = ui.available_width();
    let mut chosen = None;
    let r = choice(ui, t, "Ohne Filter", "", false, width);
    app.widgets.insert("filter.none".into(), r.rect);
    if r.clicked() {
        chosen = Some("none".to_string());
    }
    ui.add_space(4.0);
    let mut group = String::new();
    // FilmCraft's technical looks (log, broadcast) are grading tools, not filters
    for (name, folder) in looks.iter().filter(|(_, f)| f != "Technical") {
        if *folder != group {
            group = folder.clone();
            style::heading(ui, t, &look_group(&group).to_uppercase());
        }
        let r = choice(ui, t, name, "", false, width);
        app.widgets.insert(format!("filter.{name}"), r.rect);
        if r.clicked() {
            chosen = Some(name.clone());
        }
        ui.add_space(4.0);
    }
    if let Some(name) = chosen
        && run(app, &ctx, "br.video.filter", json!({"preset": name})).is_some()
    {
        toast(app, &ctx, if name == "none" { "Filter entfernt".to_string() } else { format!("Filter: {name}") });
    }
    ui.add_space(6.0);
    if link(ui, "Feiner einstellen: Seite Farbe (Lumetri, Scopes)").clicked() {
        app.cut.open = None;
        app.open(Page::Farbe);
    }
}

fn speed_panel(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    // the speed of the selected clips when they all have the same
    let speeds: Vec<f64> = {
        let s = &app.fc.session;
        let q = s.active_sequence();
        s.state.selection.iter().filter_map(|c| q.and_then(|q| q.find_item(*c)).map(|(_, it)| it.speed)).collect()
    };
    let current = speeds.first().copied().filter(|f| speeds.iter().all(|g| (g - f).abs() < 1e-9));
    style::heading(ui, t, "TEMPO");
    let width = ui.available_width();
    let cols = 4;
    let w = (width - 6.0 * (cols - 1) as f32) / cols as f32;
    let mut chosen = None;
    for row in SPEEDS.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for &f in row {
                let label = format!("{}×", format_speed(f));
                let on = current.is_some_and(|c| (c * 100.0 - f * 100.0).abs() < 0.5);
                let (rect, r) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
                ui.painter().rect_filled(rect, CornerRadius::same(8), if on || r.hovered() { t.hover } else { t.well });
                if on {
                    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.5, ACCENT), StrokeKind::Inside);
                }
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(14.0), t.text);
                app.widgets.insert(format!("speed.{}", format_speed(f)), rect);
                if r.clicked() {
                    chosen = Some(f);
                }
            }
        });
        ui.add_space(6.0);
    }
    if let Some(f) = chosen {
        run(app, &ctx, "br.video.speed", json!({"speed": f}));
    }
    note(ui, t, "Die Clips danach rücken nach. Tempo-Rampen und Rückwärts: Clip › Geschwindigkeit/Dauer.");
}

/// FilmCraft's look folders in German.
fn look_group(folder: &str) -> &str {
    match folder {
        "Cinematic" => "Kino",
        "Film Emulation" => "Filmlook",
        "Monochrome" => "Schwarzweiß",
        "Creative" => "Kreativ",
        "Vintage" => "Vintage",
        "Portrait" => "Porträt",
        "Landscape" => "Landschaft",
        other => other,
    }
}

/// `0.25`, `0.5`, `1`, `1.5` …
pub fn format_speed(f: f64) -> String {
    let s = format!("{f:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A format's shape, drawn as an outline.
fn ratio_icon(ui: &Ui, center: egui::Pos2, f: Format, size: f32, color: Color32) {
    let (w, h) = f.size();
    let k = size / w.max(h) as f32;
    let r = Rect::from_center_size(center, vec2(w as f32 * k, h as f32 * k));
    ui.painter().rect_stroke(r, CornerRadius::same(2), Stroke::new(1.5, color), StrokeKind::Inside);
}

fn current_format(app: &BrApp) -> Option<Format> {
    let q = app.fc.session.active_sequence()?;
    Format::of_size(q.settings.width, q.settings.height)
}

fn format_panel(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    let current = current_format(app);
    style::heading(ui, t, "FORMAT DES VIDEOS");
    let width = ui.available_width();
    for f in Format::ALL {
        let on = current == Some(f);
        let r = choice(ui, t, f.label(), f.hint(), on, width);
        ratio_icon(ui, pos2(r.rect.right() - 28.0, r.rect.center().y), f, 24.0, if on { ACCENT } else { t.dim });
        app.widgets.insert(format!("vformat.{}", f.key()), r.rect);
        if r.clicked() && !on {
            let fit = if app.cut.fill { "fill" } else { "fit" };
            run(app, &ctx, "br.video.format", json!({"format": f.key(), "fit": fit}));
        }
        ui.add_space(6.0);
    }
    ui.add_space(2.0);
    let mut refit = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Clips").size(12.5).color(t.dim));
        let r = ui.selectable_label(!app.cut.fill, "einpassen");
        app.widgets.insert("vformat.fit".into(), r.rect);
        if r.clicked() && app.cut.fill {
            refit = Some(false);
        }
        let r = ui.selectable_label(app.cut.fill, "füllend");
        app.widgets.insert("vformat.fill".into(), r.rect);
        if r.clicked() && !app.cut.fill {
            refit = Some(true);
        }
    });
    // the clips of this video, now
    if let Some(fill) = refit {
        app.cut.fill = fill;
        if let Some(f) = current {
            run(app, &ctx, "br.video.format", json!({"format": f.key(), "fit": if fill { "fill" } else { "fit" }}));
        }
    }
    note(ui, t, "Einpassen zeigt das ganze Bild, füllend schneidet die Ränder ab. Einzelne Clips: Eigenschaften › Transform.");
}

fn export_panel(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    style::heading(ui, t, "VORLAGE");
    let width = ui.available_width();
    for p in &EXPORT_PRESETS {
        let on = std::ptr::eq(app.cut.export_preset, p);
        let r = style::card(ui, t, p.label, p.detail, on, width);
        app.widgets.insert(format!("vpreset.{}", p.key), r.rect);
        if r.clicked() {
            app.cut.export_preset = p;
        }
        ui.add_space(6.0);
    }
    style::heading(ui, t, "ZIEL");
    let dir = app.cut.export_dir.to_string_lossy().into_owned();
    let g = style::elided(ui, &dir, 12.0, t.text, width);
    let (rect, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
    ui.painter().galley(rect.min, g, t.text);
    let r = style::button(ui, t, "Ordner wählen…", false, app.cut.picking.is_none());
    app.widgets.insert("vexport.dir".into(), r.rect);
    if r.clicked() {
        pick(app, &ctx, PickFor::ExportFolder);
    }
    ui.add_space(14.0);
    match app.cut.export.as_ref().map(|e| (e.job, e.path.clone())) {
        Some((job, path)) => {
            let f = job_progress(app, job).unwrap_or(0.0);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.label(RichText::new(format!("{name} wird gespeichert …")).size(12.5).color(t.text));
            ui.add(egui::ProgressBar::new(f).desired_width(width).show_percentage());
            ui.add_space(6.0);
            let r = style::button(ui, t, "Abbrechen", false, true);
            app.widgets.insert("vexport.cancel".into(), r.rect);
            if r.clicked() {
                run(app, &ctx, "jobs.cancel", json!({"job": job}));
            }
        }
        None => {
            let r = style::wide_button(ui, "Exportieren", width, true);
            app.widgets.insert("vexport.go".into(), r.rect);
            if r.clicked() {
                start_export(app, &ctx);
            }
            if let Some(path) = &app.cut.last_export {
                ui.add_space(8.0);
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(RichText::new(format!("Gespeichert: {name}")).size(12.5).color(t.text)).on_hover_text(path.to_string_lossy());
            }
        }
    }
    ui.add_space(8.0);
    let r = link(ui, "Schnell-Export mit FilmCrafts Vorlagen …");
    app.widgets.insert("vexport.quick".into(), r.rect);
    if r.clicked() {
        app.cut.open = None;
        app.fc.ui.export.quick_open = true;
    }
    ui.add_space(4.0);
    if link(ui, "Alle Formate, Bereich, Warteschlange: Seite Ausgabe").clicked() {
        app.cut.open = None;
        app.open(Page::Ausgabe);
    }
}

// ── the start screen ────────────────────────────────────────────────────────────────────────────

/// The start screen shows on the Schnitt page of a project that holds nothing yet, unless the
/// user left it or one of FilmCraft's dialogs is open.
pub fn start_screen_visible(app: &BrApp) -> bool {
    app.page == Page::Schnitt
        && !app.cut.start_dismissed
        && br_video::is_empty(&app.fc.session)
        && app.fc.dialog.is_none()
        && !app.fc.native_dialog_open()
}

/// Remember where FilmCraft was drawn (the start screen covers exactly that).
pub fn set_area(app: &mut BrApp, rect: Rect) {
    app.cut.area = Some(rect);
}

fn start_screen(app: &mut BrApp, ctx: &egui::Context) {
    let Some(area) = app.cut.area else { return };
    let t = Theme::get(ctx);
    egui::Area::new(egui::Id::new("br_start")).order(egui::Order::Middle).fixed_pos(area.min).show(ctx, |ui| {
        // covers FilmCraft: clicks end here
        let (_, _) = ui.allocate_exact_size(area.size(), Sense::click());
        ui.painter().rect_filled(area, CornerRadius::ZERO, t.canvas);
        let card_w = 640.0_f32.min(area.width() - 32.0);
        let card = Rect::from_center_size(area.center(), vec2(card_w, 0.0));
        let mut child =
            ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(card.left(), area.top()), vec2(card_w, area.height()))));
        // centred when it fits, scrolling when it does not (a low window, many recent projects)
        let id = egui::Id::new("br_start_height");
        let height = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(480.0);
        egui::ScrollArea::vertical().max_height(area.height()).auto_shrink([false, false]).show(&mut child, |ui| {
            ui.add_space(((area.height() - height) / 2.0).max(16.0));
            let top = ui.cursor().top();
            start_card(app, ui, &t, card_w);
            let h = ui.cursor().top() - top;
            ui.ctx().data_mut(|d| d.insert_temp(id, h));
            ui.add_space(16.0);
        });
    });
}

fn start_card(app: &mut BrApp, ui: &mut Ui, t: &Theme, width: f32) {
    let ctx = ui.ctx().clone();
    ui.label(RichText::new("Neues Video").size(24.0).strong().color(t.text));
    ui.add_space(4.0);
    ui.label(RichText::new("Wähle ein Format, oder füge einfach Medien hinzu: dann gibt das erste Video das Format vor.").size(13.5).color(t.dim));
    ui.add_space(18.0);
    // the formats as tiles
    let gap = 12.0;
    let tile_w = (width - gap * 3.0) / 4.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for f in Format::ALL {
            let (rect, r) = ui.allocate_exact_size(vec2(tile_w, 150.0), Sense::click());
            ui.painter().rect_filled(rect, CornerRadius::same(10), if r.hovered() { t.hover } else { t.well });
            if r.hovered() {
                ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.5, ACCENT), StrokeKind::Inside);
            }
            ratio_icon(ui, pos2(rect.center().x, rect.top() + 52.0), f, 56.0, if r.hovered() { ACCENT } else { t.text });
            let p = ui.painter();
            p.text(pos2(rect.center().x, rect.top() + 100.0), Align2::CENTER_TOP, f.key(), FontId::proportional(16.0), t.text);
            p.text(pos2(rect.center().x, rect.top() + 122.0), Align2::CENTER_TOP, short_hint(f), FontId::proportional(11.5), t.dim);
            app.widgets.insert(format!("start.{}", f.key()), rect);
            if r.on_hover_text(f.hint()).clicked() {
                run(app, &ctx, "br.video.new", json!({"format": f.key()}));
            }
        }
    });
    ui.add_space(18.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let r = style::button(ui, t, "  + Medien hinzufügen…  ", true, app.cut.picking.is_none());
        app.widgets.insert("start.add".into(), r.rect);
        if r.clicked() {
            pick(app, &ctx, PickFor::Media);
        }
        let r = style::button(ui, t, "Projekt öffnen…", false, true);
        app.widgets.insert("start.open".into(), r.rect);
        if r.clicked() {
            invoke(app, &ctx, "file.open");
        }
        let r = style::button(ui, t, "Beispielprojekt", false, true).on_hover_text("FilmCrafts Demo-Projekt: Clips, Musik, Titel");
        app.widgets.insert("start.demo".into(), r.rect);
        if r.clicked() {
            run(app, &ctx, "file.openDemoProject", json!({}));
        }
    });
    ui.add_space(10.0);
    note(ui, t, "Oder Videos, Fotos und Musik einfach hierher ziehen.");
    let recent: Vec<String> =
        app.fc.session.prefs.general.recent_projects.iter().filter(|p| std::path::Path::new(p).exists()).take(5).cloned().collect();
    if !recent.is_empty() {
        style::heading(ui, t, "ZULETZT GEÖFFNET");
        for (i, path) in recent.iter().enumerate() {
            let name = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let r = choice(ui, t, &name, path, false, width);
            app.widgets.insert(format!("start.recent.{i}"), r.rect);
            if r.clicked() {
                let _ = filmcraft_ui_egui::menus::invoke(&mut app.fc, &ctx, "file.open", json!({"path": path}))
                    .map_err(|e| toast(app, &ctx, readable(&e)));
            }
            ui.add_space(4.0);
        }
    }
    ui.add_space(12.0);
    let r = link(ui, "Ohne Video weiter (leerer Editor)");
    app.widgets.insert("start.skip".into(), r.rect);
    if r.clicked() {
        app.cut.start_dismissed = true;
    }
}

fn short_hint(f: Format) -> &'static str {
    match f {
        Format::Vertical => "TikTok, Reels, Shorts",
        Format::Wide => "YouTube, TV",
        Format::Square => "Feed",
        Format::Portrait => "Instagram-Feed",
    }
}
