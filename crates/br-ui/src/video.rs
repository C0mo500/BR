//! The video pages: FilmCraft's complete editor (timeline, colour, sound, captions, export),
//! arranged per page the way DaVinci Resolve arranges its pages.
//!
//! - **Medien**: FilmCraft's Import mode (media browser, bins, ingest).
//! - **Schnitt**: CapCut's arrangement made of FilmCraft panels: media, effects and titles on
//!   the left, the picture in the middle, the selected clip's properties on the right, the
//!   timeline across the bottom.
//! - **Farbe**: Lumetri Color with the scopes (FilmCraft's Color workspace).
//! - **Ton**: mixer, Essential Sound, meters (FilmCraft's Audio workspace).
//! - **Ausgabe**: FilmCraft's Export mode.
//!
//! Every page is the whole FilmCraft: its menu bar sits in BR's header, every panel can be
//! opened from Fenster, every shortcut works. A layout the user changes on a page stays with
//! that page.

use std::collections::HashMap;

use egui::FontFamily;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::dock::{DockNode, PanelKind, SplitSize};
use filmcraft_ui_egui::state::Mode as FcMode;

use crate::Page;

/// The video pages' own state.
pub struct VideoState {
    /// The layout each page had when it was left (the user may have rearranged it).
    docks: HashMap<Page, DockNode>,
    /// The page whose layout FilmCraft has now.
    shown: Option<Page>,
    /// FilmCraft's `eframe::App` methods take a frame they don't use; headless runs have none.
    frame: eframe::Frame,
}

impl Default for VideoState {
    fn default() -> Self {
        Self { docks: HashMap::new(), shown: None, frame: eframe::Frame::_new_kittest() }
    }
}

fn tabs(panels: &[PanelKind]) -> DockNode {
    DockNode::Tabs { panels: panels.to_vec(), active: 0 }
}

fn split(vertical: bool, size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical, size, a: Box::new(a), b: Box::new(b) }
}

/// CapCut's arrangement from FilmCraft's panels.
pub fn schnitt_layout() -> DockNode {
    use PanelKind::*;
    let top = split(
        false,
        SplitSize::FixedA(340.0),
        tabs(&[Project, Effects, EssentialGraphics, Text, MediaBrowser]),
        split(false, SplitSize::FixedB(360.0), tabs(&[Program]), tabs(&[Properties, EffectControls, LumetriColor, EssentialSound])),
    );
    let bottom =
        split(false, SplitSize::FixedA(52.0), tabs(&[Tools]), split(false, SplitSize::FixedB(96.0), tabs(&[Timeline]), tabs(&[AudioMeters])));
    split(true, SplitSize::Ratio(0.58), top, bottom)
}

/// FilmCraft's mode and layout for a video page.
fn layout(page: Page) -> (FcMode, DockNode) {
    match page {
        Page::Medien => (FcMode::Import, schnitt_layout()),
        Page::Farbe => (FcMode::Edit, filmcraft_ui_egui::dock::workspace("Color")),
        Page::Ton => (FcMode::Edit, filmcraft_ui_egui::dock::workspace("Audio")),
        Page::Ausgabe => (FcMode::Export, schnitt_layout()),
        Page::Schnitt | Page::Foto => (FcMode::Edit, schnitt_layout()),
    }
}

/// A new FilmCraft as BR shows it: in German, without its own header (BR's header has the pages
/// and FilmCraft's menu bar), on the Schnitt page's layout.
pub fn new_app(mut session: br_video::Session) -> FilmcraftApp {
    german_by_default(&mut session);
    let mut fc = FilmcraftApp::new(session);
    fc.ui.show_header = false;
    fc.ui.show_menu_bar = false;
    let (mode, dock) = layout(Page::Schnitt);
    fc.ui.mode = mode;
    fc.ui.dock = dock;
    fc
}

/// BR speaks German: FilmCraft's interface language follows its setting, which starts as "the
/// system's" (English on most servers); that default becomes German. A language chosen in
/// FilmCraft's settings stays.
pub fn german_by_default(session: &mut br_video::Session) {
    if session.prefs.general.interface_language == "system" {
        session.prefs.general.interface_language = "de".into();
    }
}

/// Give FilmCraft the layout of `page` (a video page), keeping the layout of the page it leaves.
pub fn show_page(fc: &mut FilmcraftApp, st: &mut VideoState, page: Page) {
    if st.shown == Some(page) {
        return;
    }
    if let Some(old) = st.shown {
        st.docks.insert(old, fc.ui.dock.clone());
    }
    let (mode, default) = layout(page);
    fc.ui.mode = mode;
    fc.ui.dock = st.docks.remove(&page).unwrap_or(default);
    st.shown = Some(page);
}

/// Playback stops when the video pages are left (the picture would no longer be drawn).
pub fn leave(fc: &mut FilmcraftApp) {
    if fc.playback.playing {
        fc.stop();
    }
}

/// The font families both apps draw with. LightCraft and FilmCraft each install their fonts
/// for the whole window; whichever did it last must not take a family the other one uses away.
const FAMILIES: [&str; 2] = ["semibold", "medium"];

/// Every family the apps use is bound. If one is missing (an app just replaced the fonts), bind
/// it to the regular face and say so: the fonts change at the start of the next frame, and
/// FilmCraft must not draw before (an unbound family is a panic in egui).
pub fn fonts_ready(ctx: &egui::Context) -> bool {
    let missing: Vec<&str> =
        ctx.fonts(|f| FAMILIES.into_iter().filter(|n| !f.definitions().families.contains_key(&FontFamily::Name((*n).into()))).collect());
    if missing.is_empty() {
        return true;
    }
    let mut defs = ctx.fonts(|f| f.definitions().clone());
    let regular = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for name in missing {
        defs.families.insert(FontFamily::Name(name.into()), regular.clone());
    }
    ctx.set_fonts(defs);
    ctx.request_repaint();
    false
}

/// FilmCraft's per-frame work before the UI pass (dialogs, its control channel, the first-frame
/// style). Runs on every page, like LightCraft's.
pub fn logic(fc: &mut FilmcraftApp, st: &mut VideoState, ctx: &egui::Context) {
    eframe::App::logic(fc, ctx, &mut st.frame);
}

/// FilmCraft's raw-input hook (window focus, synthetic input of its control channel).
pub fn raw_input_hook(fc: &mut FilmcraftApp, ctx: &egui::Context, raw: &mut egui::RawInput) {
    eframe::App::raw_input_hook(fc, ctx, raw);
}

/// Draw FilmCraft in the rest of the window. FilmCraft fills its `Ui`'s whole `max_rect`, so it
/// gets a child `Ui` that ends where BR's header ends.
pub fn ui(fc: &mut FilmcraftApp, st: &mut VideoState, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect);
        eframe::App::ui(fc, ui, &mut st.frame);
    });
}

/// FilmCraft's menu bar, in BR's header.
pub fn menu_bar(fc: &mut FilmcraftApp, ui: &mut egui::Ui) {
    filmcraft_ui_egui::menus::menu_bar(fc, ui);
}

/// When the window closes: FilmCraft flushes its recovery journal and stops its auto-save.
pub fn on_exit(fc: &mut FilmcraftApp) {
    eframe::App::on_exit(fc);
}
