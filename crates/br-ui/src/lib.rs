//! BR's window.
//!
//! One window with pages, as in DaVinci Resolve; each page is a complete Craft app, and BR's
//! header carries the page switcher and the page's quick tools (CapCut-style one-click results).
//! Nothing is hidden behind a mode: every tool of the app under a page is on that page.
//!
//! - **Foto**: LightCraft's complete interface ([`LightcraftApp`]), with BR's quick tools
//!   ([`photo`]: looks with strength, social formats, export presets) in the header.
//!
//! There is one [`lightcraft_engine::Session`], owned by the `LightcraftApp`. LightCraft's
//! `logic()` runs every frame on every page (it renders, imports, saves); on other pages its
//! keyboard shortcuts are hidden from it so they don't act behind the page's back.

pub mod desktop;
pub mod headless;
pub mod photo;
mod style;

use std::collections::HashMap;
use std::time::Duration;

use lightcraft_engine::Session;
use lightcraft_ui_egui::{LightcraftApp, Services};

pub use photo::{PhotoState, Popover};

/// A page of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    /// Photos: LightCraft.
    Foto,
}

impl Page {
    pub const ALL: [Page; 1] = [Page::Foto];

    pub fn label(self) -> &'static str {
        match self {
            Page::Foto => "Foto",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Page::Foto => "foto",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Page::Foto => "Fotos: Bibliothek, Entwickeln, Masken, Kurven, Looks (LightCraft)",
        }
    }
}

/// Window title.
pub const TITLE: &str = "BR";

/// Height of BR's header.
const HEADER_H: f32 = 40.0;

pub struct BrApp {
    /// LightCraft's interface, owner of the one photo session.
    pub lc: LightcraftApp,
    pub page: Page,
    /// The Foto page's quick tools.
    pub photo: PhotoState,
    /// Screen rectangles of BR's controls from the last frame, by name (`page.foto`,
    /// `tool.looks`, `format.9:16`, `look.lc.moody`, …), for tests and automation.
    pub widgets: HashMap<String, egui::Rect>,
    /// Where the library is, for the header and messages (`None`: in memory).
    pub library: Option<String>,
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

impl BrApp {
    /// BR on `session`, without file dialogs (headless runs and tests). `library` names where it
    /// lives (`None` for an in-memory session).
    pub fn new(session: Session, library: Option<String>) -> Self {
        Self::with_services(session, library, base_services())
    }

    /// BR on `session` with `services` (a window: [`desktop::services`]).
    pub fn with_services(session: Session, library: Option<String>, services: Services) -> Self {
        let mut lc = LightcraftApp::new(session, services);
        lc.ui.language = lightcraft_ui_egui::i18n::Locale::De;
        // BR shows its own title; this keeps LightCraft from replacing it with "LightCraft — …"
        lc.shown_library = Some(lc.session.library_generation());
        Self { lc, page: Page::Foto, photo: PhotoState::default(), widgets: HashMap::new(), library }
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
        if page != self.page {
            photo::finish_drags(self);
            self.photo.open = None;
            self.page = page;
        }
    }

    /// Per frame, before `ui`: LightCraft's background work (renders, imports, saving, control
    /// requests). On other pages its keyboard shortcuts are hidden from it.
    pub fn logic(&mut self, ctx: &egui::Context) {
        if self.page == Page::Foto {
            self.lc.logic(ctx);
        } else {
            let saved = ctx.input_mut(|i| {
                let saved = i.events.clone();
                i.events.retain(|e| !is_shortcut_event(e));
                saved
            });
            self.lc.logic(ctx);
            ctx.input_mut(|i| i.events = saved);
        }
        self.lc.shown_library = Some(self.lc.session.library_generation());
        // LightCraft can open another library (Try Again, Open Library…)
        self.library = self.lc.session.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.to_string_lossy().into_owned());
    }

    /// Before each frame: input LightCraft's control channel injects.
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        self.lc.raw_input_hook(raw);
    }

    /// Draw the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.widgets.clear();
        let ctx = ui.ctx().clone();
        // an export finishes whichever page is showing
        photo::poll_export(self, &ctx);
        self.header(ui);
        match self.page {
            Page::Foto => {
                self.lc.ui(ui);
                photo::show(self, &ctx);
            }
        }
    }

    /// BR's header: the wordmark, the pages, the page's quick tools.
    fn header(&mut self, ui: &mut egui::Ui) {
        let t = style::Theme::get(ui.ctx());
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        match self.page {
                            Page::Foto => photo::header_tools(self, ui, &t),
                        }
                        if let Some(lib) = &self.library {
                            ui.add_space(6.0);
                            ui.label(egui::RichText::new(lib).size(11.5).color(t.dim)).on_hover_text("Bibliothek");
                        }
                    });
                });
            },
        );
    }

    /// Something is still being worked on (renders, an import or export): keep drawing frames.
    pub fn busy(&self) -> bool {
        let lc = &self.lc;
        lc.renderer.in_flight() > 0 || lc.import.is_some() || lc.export.is_some() || !lc.tasks.is_empty() || self.photo.exporting()
    }

    /// Everything closing the window does short of stopping LightCraft's workers: end a drag,
    /// let a running export finish (unless it was cancelled), save and close the library.
    pub fn close(&mut self) {
        photo::finish_drags(self);
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

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        BrApp::raw_input_hook(self, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        BrApp::ui(self, ui);
        if self.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        BrApp::on_exit(self);
        // macOS (as LightCraft, its issue #1331): end the process before eframe tears the window
        // down, or AppKit's teardown ends in a crash report. Everything is saved at this point.
        #[cfg(target_os = "macos")]
        std::process::exit(0);
    }
}
