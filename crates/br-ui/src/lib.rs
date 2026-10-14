//! BR's window.
//!
//! [`BrApp`] hosts LightCraft's complete interface ([`LightcraftApp`]) and shows one of two modes:
//!
//! - **Einfach** (Simple, [`simple`]): BR's own CapCut-style editor. A big preview, a filmstrip,
//!   and four tabs: Looks, Anpassen (six sliders and Auto), Format (1:1, 4:5, 9:16, 16:9) and
//!   Export (Instagram, Story, …).
//! - **Pro**: LightCraft's Lightroom-style interface on the same library and the same photo.
//!
//! There is one [`lightcraft_engine::Session`], owned by the `LightcraftApp`. LightCraft's
//! `logic()` runs every frame in both modes (it renders, imports, saves and keeps the Pro view
//! ready); in Simple mode its keyboard shortcuts are hidden from it so they don't act behind BR's
//! back. Everything that only LightCraft's `ui()` would do (export polling, notices, the quit
//! prompt) is done by BR in Simple mode.

pub mod headless;
pub mod simple;
mod style;

use std::collections::HashMap;
use std::time::Duration;

use lightcraft_engine::Session;
use lightcraft_ui_egui::docking::Panel as DockPanel;
use lightcraft_ui_egui::state::{RightPanel, ViewMode};
use lightcraft_ui_egui::{LightcraftApp, Services};
use serde_json::json;

pub use simple::{SimpleState, Tab};

/// Which interface the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// BR's simple editor.
    Simple,
    /// LightCraft's full interface.
    Pro,
}

/// Window title.
pub const TITLE: &str = "BR";

/// Height of BR's top bar in Pro mode (in Simple mode the bar is taller and carries the tools).
const PRO_BAR_H: f32 = 30.0;

pub struct BrApp {
    /// LightCraft's interface, owner of the one session.
    pub lc: LightcraftApp,
    pub mode: Mode,
    pub simple: SimpleState,
    /// Screen rectangles of BR's controls from the last frame, by name (`tab.looks`,
    /// `format.9:16`, `look.lc.moody`, `mode.pro`, …), for tests and automation.
    pub widgets: HashMap<String, egui::Rect>,
    /// Where the library is, for the title bar and messages (`None`: in memory).
    pub library: Option<String>,
}

fn services() -> Services {
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
    /// BR on `session`. `library` names where it lives (`None` for an in-memory session).
    pub fn new(session: Session, library: Option<String>) -> Self {
        let mut lc = LightcraftApp::new(session, services());
        lc.ui.language = lightcraft_ui_egui::i18n::Locale::De;
        // BR shows its own title; this keeps LightCraft from replacing it with "LightCraft — …"
        lc.shown_library = Some(lc.session.library_generation());
        Self { lc, mode: Mode::Simple, simple: SimpleState::default(), widgets: HashMap::new(), library }
    }

    /// Show a library that could not be opened (the session is then in memory).
    pub fn library_problem(&mut self, path: impl Into<String>, error: impl Into<String>) {
        self.lc.library_problem = Some(lightcraft_ui_egui::panels::library_problem::LibraryProblem::new(path, error));
    }

    /// Switch to Pro mode on the active photo: LightCraft's detail view with its Edit panel.
    pub fn open_pro(&mut self) {
        // a slider or look drag in progress becomes its undo step before LightCraft takes over
        simple::finish_drags(self);
        let edit_shown = self.lc.ui.docking.as_ref().is_none_or(|w| w.exposed(DockPanel::Tools));
        if !(self.lc.ui.right == RightPanel::Edit && edit_shown) {
            // `panel.edit` toggles: only call it when the Edit panel is not already showing
            if let Err(e) = self.lc.run("panel.edit", json!({})) {
                log::warn!("panel.edit: {e}");
            }
        }
        self.lc.ui.view = ViewMode::Detail;
        self.mode = Mode::Pro;
    }

    /// Back to Simple mode.
    pub fn open_simple(&mut self) {
        if self.lc.session.interaction.is_some() {
            let _ = self.lc.session.end_interaction();
        }
        self.mode = Mode::Simple;
    }

    /// Per frame, before `ui`: LightCraft's background work (renders, imports, saving, control
    /// requests). In Simple mode its keyboard shortcuts are hidden from it.
    pub fn logic(&mut self, ctx: &egui::Context) {
        if self.mode == Mode::Simple {
            let saved = ctx.input_mut(|i| {
                let saved = i.events.clone();
                i.events.retain(|e| !is_shortcut_event(e));
                saved
            });
            self.lc.logic(ctx);
            ctx.input_mut(|i| i.events = saved);
        } else {
            self.lc.logic(ctx);
        }
        self.lc.shown_library = Some(self.lc.session.library_generation());
    }

    /// Before each frame: input LightCraft's control channel injects.
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        self.lc.raw_input_hook(raw);
    }

    /// Draw the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.widgets.clear();
        match self.mode {
            Mode::Pro => {
                let t = style::Theme::get(ui.ctx());
                let mut go_simple = false;
                egui::Panel::top("br_pro_bar")
                    .exact_size(PRO_BAR_H)
                    .frame(egui::Frame::NONE.fill(t.bar).inner_margin(egui::Margin::symmetric(10, 0)))
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            style::brand(ui, &t);
                            ui.add_space(12.0);
                            go_simple = style::mode_switch(ui, &t, Mode::Pro, &mut self.widgets) == Some(Mode::Simple);
                        });
                    });
                self.lc.ui(ui);
                if go_simple {
                    self.open_simple();
                }
            }
            Mode::Simple => {
                simple::show(self, ui);
                self.simple_duties(ui.ctx());
            }
        }
    }

    /// What LightCraft's `ui()` does that Simple mode still needs.
    fn simple_duties(&mut self, ctx: &egui::Context) {
        use lightcraft_ui_egui::panels;
        panels::notices::show(&mut self.lc, ctx);
        panels::library_problem::show(&mut self.lc, ctx);
        lightcraft_ui_egui::export_task::poll(&mut self.lc, ctx);
        lightcraft_ui_egui::export_task::poll_photos(&mut self.lc, ctx);
        // LightCraft's dialogs (a dropped preset file, a confirmation) over BR's interface
        panels::dialogs::show(&mut self.lc, ctx);
        panels::toast(&mut self.lc, ctx);
        if std::mem::take(&mut self.lc.ui.quit) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Something is still being worked on (renders, an import or export): keep drawing frames.
    pub fn busy(&self) -> bool {
        let lc = &self.lc;
        lc.renderer.in_flight() > 0 || lc.import.is_some() || lc.export.is_some() || !lc.tasks.is_empty() || self.simple.exporting()
    }

    /// Save and close the library, then stop LightCraft's workers. Call once, when the window
    /// closes.
    pub fn on_exit(&mut self) {
        simple::finish_drags(self);
        if let Err(e) = self.lc.session.close_library() {
            log::error!("closing the library: {e}");
        }
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
    }
}
