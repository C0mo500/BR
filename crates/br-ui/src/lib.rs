//! BR's window.
//!
//! One window with pages, as in DaVinci Resolve; each page is a complete Craft app, and BR's
//! header carries the page switcher and the page's quick tools (CapCut-style one-click results).
//! Nothing is hidden behind a mode: every tool of the app under a page is on that page.
//!
//! - **Medien, Schnitt, Farbe, Ton, Ausgabe**: FilmCraft's complete editor ([`video`]), arranged
//!   per page; its menu bar is in BR's header.
//! - **Foto**: LightCraft's complete interface ([`LightcraftApp`]), with BR's quick tools
//!   ([`photo`]: looks with strength, social formats, export presets) in the header.
//!
//! There is one photo session (LightCraft's, owned by the `LightcraftApp`) and one video session
//! (FilmCraft's, owned by the `FilmcraftApp`). Both apps' `logic()` runs every frame on every
//! page (rendering, importing, saving go on); only the app on the page gets keyboard shortcuts.

pub mod control;
pub mod cut;
pub mod desktop;
pub mod headless;
pub mod photo;
mod style;
pub mod video;

use std::collections::HashMap;
use std::time::Duration;

use filmcraft_ui_egui::FilmcraftApp;
use lightcraft_engine::Session;
use lightcraft_ui_egui::{LightcraftApp, Services};

pub use cut::CutState;
pub use photo::{PhotoState, Popover};
pub use video::VideoState;

/// A page of the window, in DaVinci Resolve's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    /// Media: browse, import, organise (FilmCraft's Import mode).
    Medien,
    /// Editing: the timeline in CapCut's arrangement (FilmCraft).
    Schnitt,
    /// Colour grading: Lumetri and scopes (FilmCraft).
    Farbe,
    /// Sound: mixer, Essential Sound (FilmCraft).
    Ton,
    /// Deliver: FilmCraft's Export mode.
    Ausgabe,
    /// Photos: LightCraft.
    Foto,
}

impl Page {
    pub const ALL: [Page; 6] = [Page::Medien, Page::Schnitt, Page::Farbe, Page::Ton, Page::Ausgabe, Page::Foto];

    pub fn label(self) -> &'static str {
        match self {
            Page::Medien => "Medien",
            Page::Schnitt => "Schnitt",
            Page::Farbe => "Farbe",
            Page::Ton => "Ton",
            Page::Ausgabe => "Ausgabe",
            Page::Foto => "Foto",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Page::Medien => "medien",
            Page::Schnitt => "schnitt",
            Page::Farbe => "farbe",
            Page::Ton => "ton",
            Page::Ausgabe => "ausgabe",
            Page::Foto => "foto",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Page::Medien => "Medien sichten, importieren und ordnen",
            Page::Schnitt => "Schneiden: Timeline, Vorschau, Eigenschaften, Texte, Effekte",
            Page::Farbe => "Farbkorrektur und Looks mit Scopes (Lumetri)",
            Page::Ton => "Mischpult, Pegel, Sprache, Musik und Effekte",
            Page::Ausgabe => "Exportieren: Formate, Vorlagen, Warteschlange",
            Page::Foto => "Fotos: Bibliothek, Entwickeln, Masken, Kurven, Looks (LightCraft)",
        }
    }

    /// A page FilmCraft draws.
    pub fn is_video(self) -> bool {
        self != Page::Foto
    }
}

/// Window title.
pub const TITLE: &str = "BR";

/// Height of BR's header.
const HEADER_H: f32 = 40.0;
/// Below this window width FilmCraft's menus get a row of their own on the video pages (beside
/// the page tabs and the header's tools they would be covered).
const MENUS_INLINE_WIDTH: f32 = 1480.0;
/// Height of that row.
const MENUS_H: f32 = 28.0;

pub struct BrApp {
    /// LightCraft's interface, owner of the one photo session.
    pub lc: LightcraftApp,
    /// FilmCraft's interface, owner of the one video session.
    pub fc: FilmcraftApp,
    pub page: Page,
    /// The Foto page's quick tools.
    pub photo: PhotoState,
    /// The video pages' layouts.
    pub video: VideoState,
    /// The video pages' quick tools.
    pub cut: CutState,
    /// Every font family the apps draw with was bound at the start of this frame.
    fonts_ok: bool,
    /// Screen rectangles of BR's controls from the last frame, by name (`page.foto`,
    /// `tool.looks`, `format.9:16`, `look.lc.moody`, …), for tests and automation.
    pub widgets: HashMap<String, egui::Rect>,
    /// Where the library is, for the header and messages (`None`: in memory).
    pub library: Option<String>,
    /// BR's requests from the control channel ([`control`]), when the window has one.
    control: Option<std::sync::mpsc::Receiver<lightcraft_ui_egui::ControlRequest>>,
    /// Control requests whose reply waits for background work.
    pending: Vec<control::Pending>,
    /// Whose look the window has (`true`: FilmCraft's, `false`: LightCraft's).
    styled: Option<bool>,
}

/// The services every BR window has: writing files and PNG screenshots. A desktop window adds
/// file dialogs and links ([`desktop::services`]); headless runs keep these alone.
pub(crate) fn base_services() -> Services {
    Services {
        write: Some(Box::new(lightcraft_engine::export::write_file)),
        write_shared: Some(std::sync::Arc::new(lightcraft_engine::export::write_file)),
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        ..Default::default()
    }
}

fn is_shortcut_event(e: &egui::Event) -> bool {
    matches!(e, egui::Event::Key { .. } | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))
}

/// Run `f` for an app that is not on the page: the frame's keyboard shortcuts and dropped files
/// are hidden from it (they belong to the app the user sees).
fn without_page_input(ctx: &egui::Context, f: impl FnOnce(&egui::Context)) {
    let saved = ctx.input_mut(|i| {
        let saved = (i.events.clone(), std::mem::take(&mut i.raw.dropped_files), std::mem::take(&mut i.raw.hovered_files));
        i.events.retain(|e| !is_shortcut_event(e));
        saved
    });
    f(ctx);
    ctx.input_mut(|i| (i.events, i.raw.dropped_files, i.raw.hovered_files) = saved);
}

impl BrApp {
    /// BR on the photo `session` and an empty video project, without file dialogs (headless runs
    /// and tests). `library` names where the photos live (`None` for an in-memory session).
    pub fn new(session: Session, library: Option<String>) -> Self {
        Self::with_services(session, library, base_services())
    }

    /// BR on the photo `session` with `services` (a window: [`desktop::services`]) and an empty
    /// video project.
    pub fn with_services(session: Session, library: Option<String>, services: Services) -> Self {
        Self::with_sessions(session, br_video::Session::default(), library, services)
    }

    /// BR on the photo `session` and the video session `video` (a window's: its auto-save and
    /// crash recovery already started, so FilmCraft offers to recover and finds its saved
    /// workspaces).
    pub fn with_sessions(session: Session, video: br_video::Session, library: Option<String>, services: Services) -> Self {
        let mut lc = LightcraftApp::new(session, services);
        lc.ui.language = lightcraft_ui_egui::i18n::Locale::De;
        // BR shows its own title; this keeps LightCraft from replacing it with "LightCraft — …"
        lc.shown_library = Some(lc.session.library_generation());
        let fc = video::new_app(video);
        let mut app = Self {
            lc,
            fc,
            page: Page::Schnitt,
            photo: PhotoState::default(),
            video: VideoState::default(),
            cut: CutState::default(),
            fonts_ok: false,
            widgets: HashMap::new(),
            library,
            control: None,
            pending: Vec::new(),
            styled: None,
        };
        video::show_page(&mut app.fc, &mut app.video, Page::Schnitt);
        app
    }

    /// Take requests from a control channel ([`control::start`]): LightCraft's and FilmCraft's go
    /// to their own handlers, BR's are run here each frame.
    pub fn with_control(mut self, ch: control::Channels) -> Self {
        self.lc = self.lc.with_control(ch.photo);
        self.fc = self.fc.with_control(ch.video);
        self.control = Some(ch.br);
        self
    }

    /// Show a library that could not be opened (the session is then in memory). `pending_import`
    /// is imported once the user picks Try Again or Continue Without Saving.
    pub fn library_problem(&mut self, path: impl Into<String>, error: impl Into<String>, pending_import: Vec<String>) {
        let mut p = lightcraft_ui_egui::panels::library_problem::LibraryProblem::new(path, error);
        p.pending_import = pending_import;
        self.lc.library_problem = Some(p);
    }

    /// Show `page`.
    pub fn open(&mut self, page: Page) {
        if page == self.page {
            return;
        }
        photo::finish_drags(self);
        self.photo.open = None;
        self.cut.open = None;
        if page.is_video() {
            video::show_page(&mut self.fc, &mut self.video, page);
        } else {
            video::leave(&mut self.fc);
        }
        self.page = page;
    }

    /// Per frame, before `ui`: both apps' background work (renders, imports, saving, control
    /// requests). Only the app on the page sees keyboard shortcuts.
    pub fn logic(&mut self, ctx: &egui::Context) {
        if self.page == Page::Foto {
            self.lc.logic(ctx);
        } else {
            without_page_input(ctx, |ctx| self.lc.logic(ctx));
        }
        video::logic(&mut self.fc, &mut self.video, ctx, self.page.is_video());
        if self.page.is_video() {
            // LightCraft's background results that only its interface collects (a finished export)
            lightcraft_ui_egui::export_task::poll(&mut self.lc, ctx);
            lightcraft_ui_egui::export_task::poll_photos(&mut self.lc, ctx);
        }
        control::drain(self, ctx);
        self.fonts_ok = video::fonts_ready(ctx, &mut self.video);
        self.lc.shown_library = Some(self.lc.session.library_generation());
        // LightCraft can open another library (Try Again, Open Library…)
        self.library = self.lc.session.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.to_string_lossy().into_owned());
        // closing the window: a question an app asks (save the project? stop the export?) is
        // shown on its page
        if ctx.input(|i| i.viewport().close_requested()) {
            // FilmCraft asks about unsaved changes in its quit dialog, LightCraft about running
            // work in its quit prompt; each is drawn on its app's page
            let film_asks = self.fc.dialog.is_some() || self.fc.ui.clip_dialog.as_ref().is_some_and(|d| d.command == "app.quit");
            let photo_asks = self.lc.quit_prompt.is_some() || self.lc.ui.dialog.is_some();
            if film_asks && !self.page.is_video() {
                self.open(Page::Schnitt);
            } else if photo_asks && self.page.is_video() {
                self.open(Page::Foto);
            } else if !film_asks && !photo_asks {
                cut::on_close_request(self, ctx);
            }
        }
    }

    /// Before each frame: input the apps' control channels inject, and files dropped where BR
    /// takes them itself.
    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.lc.raw_input_hook(raw);
        video::raw_input_hook(&mut self.fc, ctx, raw);
        self.route_drops(ctx, raw);
    }

    /// Dropped files: on the start screen they make the video (instead of only landing in the
    /// project); on the Foto page, videos go to the Schnitt page (photos stay with LightCraft).
    fn route_drops(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if raw.dropped_files.is_empty() {
            return;
        }
        let path = |f: &egui::DroppedFileHandle| Some(f.path().to_string_lossy().into_owned()).filter(|p| !p.is_empty());
        if cut::start_screen_visible(self) {
            let paths: Vec<String> = std::mem::take(&mut raw.dropped_files).iter().filter_map(path).collect();
            cut::open_files(self, ctx, paths);
        } else if self.page == Page::Foto {
            let (videos, photos): (Vec<_>, Vec<_>) =
                std::mem::take(&mut raw.dropped_files).into_iter().partition(|f| path(f).is_some_and(|p| br_video::is_video_file(&p)));
            raw.dropped_files = photos;
            if !videos.is_empty() {
                cut::open_files(self, ctx, videos.iter().filter_map(path).collect());
                if raw.dropped_files.is_empty() {
                    self.open(Page::Schnitt);
                }
            }
        }
    }

    /// Draw the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.widgets.clear();
        let ctx = ui.ctx().clone();
        // an export (and a caption recognition) finishes whichever page is showing
        photo::poll_export(self, &ctx);
        cut::poll(self, &ctx);
        self.style_for_page(&ctx);
        self.header(ui);
        match self.page {
            Page::Foto => {
                self.lc.ui(ui);
                photo::show(self, &ctx);
                // a message from the video side (an export finished, closing waits for one)
                cut::draw_toast(self, &ctx);
            }
            _ if !self.fonts_ok => {
                // the fonts were just completed: FilmCraft draws from the next frame on
                egui::CentralPanel::default().show(ui, |_| {});
            }
            _ => {
                if self.page == Page::Schnitt {
                    cut::toolbar(self, ui);
                }
                let area = video::ui(&mut self.fc, &mut self.video, ui);
                cut::set_area(self, area);
                cut::show(self, &ctx);
                cut::draw_toast(self, &ctx);
                // FilmCraft switched its own mode (File ▸ Export ▸ Media…, Import mode's cards):
                // show the page that has it
                if let Some(page) = video::page_of_mode(self.fc.ui.mode, self.page) {
                    self.open(page);
                }
            }
        }
    }

    /// Each app draws with its own look (both style the one egui context): the page's app's look
    /// is applied when the page changes between LightCraft and FilmCraft.
    fn style_for_page(&mut self, ctx: &egui::Context) {
        let video = self.page.is_video();
        if self.styled == Some(video) {
            return;
        }
        if video {
            let kind = self.fc.appearance_kind(ctx);
            self.fc.set_theme(ctx, kind);
        } else {
            let kind = self.lc.ui.settings.theme(self.lc.system_theme(ctx));
            lightcraft_ui_egui::theme::apply_kind(ctx, kind);
        }
        self.styled = Some(video);
    }

    /// BR's header: the wordmark, the pages, the page's quick tools.
    fn header(&mut self, ui: &mut egui::Ui) {
        let t = style::Theme::get(ui.ctx());
        let menus = self.page.is_video() && self.fonts_ok;
        let menus_inline = menus && ui.ctx().content_rect().width() >= MENUS_INLINE_WIDTH;
        egui::Panel::top("br_header").exact_size(HEADER_H).frame(egui::Frame::NONE.fill(t.bar).inner_margin(egui::Margin::symmetric(12, 0))).show(
            ui,
            |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    style::brand(ui, &t);
                    ui.add_space(14.0);
                    if let Some(page) = style::page_tabs(ui, &t, self.page, &mut self.widgets) {
                        self.open(page);
                    }
                    if menus_inline {
                        ui.add_space(10.0);
                        video::menu_bar(&mut self.fc, ui);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if self.page == Page::Foto {
                            photo::header_tools(self, ui, &t);
                        } else if self.fonts_ok {
                            cut::header_tools(self, ui, &t);
                        }
                        // the photo library on the Foto page; the video project's name and
                        // whether it has unsaved changes on the video pages
                        if self.page == Page::Foto {
                            if let Some(lib) = &self.library {
                                ui.add_space(6.0);
                                ui.label(egui::RichText::new(lib).size(11.5).color(t.dim)).on_hover_text("Bibliothek");
                            }
                        } else {
                            ui.add_space(6.0);
                            let name = video::project_name(&self.fc);
                            let dirty = self.fc.session.is_dirty();
                            let text = if dirty { format!("{name} •") } else { name };
                            let tip = if dirty {
                                format!("Video-Projekt „{text}“: ungesicherte Änderungen (Datei › Speichern)")
                            } else {
                                format!("Video-Projekt „{text}“")
                            };
                            ui.label(style::elided(ui, &text, 11.5, t.dim, 180.0)).on_hover_text(tip);
                        }
                    });
                });
            },
        );
        if menus && !menus_inline {
            egui::Panel::top("br_menus").exact_size(MENUS_H).frame(egui::Frame::NONE.fill(t.bar).inner_margin(egui::Margin::symmetric(12, 0))).show(
                ui,
                |ui| {
                    ui.horizontal_centered(|ui| video::menu_bar(&mut self.fc, ui));
                },
            );
        }
    }

    /// Something is still being worked on (renders, an import or export): keep drawing frames.
    pub fn busy(&self) -> bool {
        let lc = &self.lc;
        lc.renderer.in_flight() > 0
            || lc.import.is_some()
            || lc.export.is_some()
            || !lc.tasks.is_empty()
            || self.photo.exporting()
            || self.cut.busy()
            || !self.pending.is_empty()
    }

    /// Everything closing the window does short of stopping LightCraft's workers: end a drag,
    /// let a running export finish (unless it was cancelled), save and close the library, flush
    /// the video project's recovery journal.
    pub fn close(&mut self) {
        photo::finish_drags(self);
        video::leave(&mut self.fc);
        video::on_exit(&mut self.fc);
        // an export still running gets written, unless Quit Anyway (or ✕) cancelled it
        photo::wait_for_export(self, Duration::from_secs(120));
        if let Err(e) = self.lc.session.close_library() {
            log::error!("closing the library: {e}");
        }
    }

    /// [`BrApp::close`], then stop LightCraft's workers. Call once, when the window closes: the
    /// stop is for the whole process (LightCraft's GPU and render workers), so nothing renders
    /// after it.
    pub fn on_exit(&mut self) {
        self.close();
        if !self.lc.shutdown(Duration::from_secs(2)) {
            log::warn!("background work did not stop in time");
        }
    }
}

impl eframe::App for BrApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        BrApp::logic(self, ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        BrApp::raw_input_hook(self, ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        BrApp::ui(self, ui);
        if self.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }

    fn on_exit(&mut self) {
        BrApp::on_exit(self);
        // macOS (as LightCraft, its issue #1331): end the process before eframe tears the window
        // down, or AppKit's teardown ends in a crash report. Everything is saved at this point.
        #[cfg(target_os = "macos")]
        std::process::exit(0);
    }
}
