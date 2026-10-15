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
    /// Frames in a row in which a font family was missing.
    fonts_missing: u32,
    /// What BR last reconciled of the video: edits made with FilmCraft's own tools are followed by
    /// what hangs on the main clips they changed ([`br_video::reconcile`]).
    pub watch: br_video::reconcile::Watch,
}

impl Default for VideoState {
    fn default() -> Self {
        Self {
            docks: HashMap::new(),
            shown: None,
            frame: eframe::Frame::_new_kittest(),
            fonts_missing: 0,
            watch: br_video::reconcile::Watch::default(),
        }
    }
}

fn tabs(panels: &[PanelKind]) -> DockNode {
    DockNode::Tabs { panels: panels.to_vec(), active: 0 }
}

fn split(vertical: bool, size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical, size, a: Box::new(a), b: Box::new(b) }
}

/// CapCut's arrangement: the picture in the middle, FilmCraft's Program monitor (the Source
/// monitor shares its place: a double-clicked clip opens there). The asset bar on the left, the
/// timeline under it and the inspector on the right are BR's ([`crate::assets`],
/// [`crate::timeline`], [`crate::inspector`]); FilmCraft's own panels are their "Profi" and
/// "Voll" views, and the Window menu docks them again.
pub fn schnitt_layout() -> DockNode {
    use PanelKind::*;
    tabs(&[Program, Source])
}

/// The Schnitt layout with FilmCraft's media panels on the left (before BR's asset bar).
fn schnitt_layout_v3() -> DockNode {
    use PanelKind::*;
    split(false, SplitSize::FixedA(340.0), tabs(&[Project, Effects, EssentialGraphics, Text, MediaBrowser]), tabs(&[Program, Source]))
}

/// The Schnitt layout with FilmCraft's properties on the right (before BR's inspector).
fn schnitt_layout_v2() -> DockNode {
    use PanelKind::*;
    split(
        false,
        SplitSize::FixedA(340.0),
        tabs(&[Project, Effects, EssentialGraphics, Text, MediaBrowser]),
        split(false, SplitSize::FixedB(360.0), tabs(&[Program, Source]), tabs(&[Properties, EffectControls, LumetriColor, EssentialSound])),
    )
}

/// The Schnitt layout of BR's first version (FilmCraft's timeline docked under the picture).
fn schnitt_layout_v1() -> DockNode {
    use PanelKind::*;
    let bottom =
        split(false, SplitSize::FixedA(52.0), tabs(&[Tools]), split(false, SplitSize::FixedB(96.0), tabs(&[Timeline]), tabs(&[AudioMeters])));
    split(true, SplitSize::Ratio(0.58), schnitt_layout_v2(), bottom)
}

/// BR's Schnitt arrangement as one of FilmCraft's workspaces: Window ▸ Workspaces lists it,
/// "Reset to Saved Layout" goes back to it, "Save Changes" keeps the user's version of it.
pub const SCHNITT_WORKSPACE: &str = "Schnitt";

/// The FilmCraft workspace a video page is made from.
fn workspace_of(page: Page) -> &'static str {
    match page {
        Page::Farbe => "Color",
        Page::Ton => "Audio",
        _ => SCHNITT_WORKSPACE,
    }
}

/// FilmCraft's mode for a video page.
fn mode_of(page: Page) -> FcMode {
    match page {
        Page::Medien => FcMode::Import,
        Page::Ausgabe => FcMode::Export,
        _ => FcMode::Edit,
    }
}

/// FilmCraft's mode and layout for a video page: its workspace as saved (the user may have saved
/// changes to it), else as it comes.
fn layout(fc: &FilmcraftApp, page: Page) -> (FcMode, DockNode) {
    (mode_of(page), filmcraft_ui_egui::dock::saved_docking(&fc.workspaces, workspace_of(page)).root)
}

/// A new FilmCraft as BR shows it: in German, without its own header (BR's header has the pages
/// and FilmCraft's menu bar), on the Schnitt page's layout.
pub fn new_app(mut session: br_video::Session) -> FilmcraftApp {
    german_by_default(&mut session);
    let mut fc = FilmcraftApp::new(session);
    // BR's commands among FilmCraft's keys, which start as CapCut's
    crate::keys::register(&mut fc);
    crate::keys::install(&mut fc);
    fc.ui.show_header = false;
    fc.ui.show_menu_bar = false;
    // the pages bring their own layouts: FilmCraft must not reopen its last workspace over them on
    // its first frame (the saved workspaces stay in its Window menu)
    fc.workspaces.current.clear();
    let layout_now = serde_json::to_value(schnitt_layout()).unwrap_or_default();
    let old = [schnitt_layout_v1(), schnitt_layout_v2(), schnitt_layout_v3()].map(|l| serde_json::to_value(l).unwrap_or_default());
    match fc.workspaces.saved.iter_mut().find(|w| w.name == SCHNITT_WORKSPACE) {
        // an earlier version's layout, unchanged by the user: BR's timeline and inspector take
        // the place of FilmCraft's panels
        Some(w) if old.contains(&w.layout) => w.layout = layout_now,
        Some(_) => {}
        None => fc.workspaces.saved.push(filmcraft_ui_egui::dock::SavedWorkspace {
            name: SCHNITT_WORKSPACE.into(),
            layout: layout_now,
            floating: serde_json::Value::Null,
        }),
    }
    let (mode, dock) = layout(&fc, Page::Schnitt);
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
        // the page's mode, should FilmCraft have left it
        fc.ui.mode = mode_of(page);
        return;
    }
    if let Some(old) = st.shown {
        st.docks.insert(old, fc.ui.dock.clone());
    }
    let (mode, default) = layout(fc, page);
    fc.ui.mode = mode;
    fc.ui.dock = st.docks.remove(&page).unwrap_or(default);
    // Window ▸ Workspaces names (and resets to) the workspace a page is made from
    fc.ui.workspace = workspace_of(page).into();
    st.shown = Some(page);
}

/// The page that shows FilmCraft's `mode` when it is not `page`'s (FilmCraft switched it itself:
/// File ▸ Export ▸ Media…, a card in Import mode).
pub fn page_of_mode(mode: FcMode, page: Page) -> Option<Page> {
    match mode {
        FcMode::Export if page != Page::Ausgabe => Some(Page::Ausgabe),
        FcMode::Import if page != Page::Medien => Some(Page::Medien),
        FcMode::Edit if matches!(page, Page::Medien | Page::Ausgabe) => Some(Page::Schnitt),
        _ => None,
    }
}

/// Playback stops when the video pages are left (the picture would no longer be drawn), the
/// Source monitor's too.
pub fn leave(fc: &mut FilmcraftApp) {
    if fc.playback.playing {
        fc.stop();
    }
    if fc.source_playback.clock.playing {
        fc.stop_source();
    }
}

/// The font families both apps draw with. LightCraft and FilmCraft each install their fonts
/// for the whole window; whichever did it last must not take a family the other one uses away.
const FAMILIES: [&str; 2] = ["semibold", "medium"];

/// Every family the apps use is bound. FilmCraft must not draw while one is missing (an unbound
/// family is a panic in egui). An app that just installed its fonts has them pending until the
/// next frame (FilmCraft's include the families), so a first missing frame is waited out; only
/// when they are still missing (LightCraft replaced the fonts afterwards) are they bound to the
/// regular face, on top of the fonts in use, from the next frame on.
pub fn fonts_ready(ctx: &egui::Context, st: &mut VideoState) -> bool {
    let missing: Vec<&str> =
        ctx.fonts(|f| FAMILIES.into_iter().filter(|n| !f.definitions().families.contains_key(&FontFamily::Name((*n).into()))).collect());
    if missing.is_empty() {
        st.fonts_missing = 0;
        return true;
    }
    st.fonts_missing += 1;
    ctx.request_repaint();
    if st.fonts_missing < 2 {
        return false;
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
/// style). Runs on every page, like LightCraft's. Off the video pages (`shown` false) its
/// session's upkeep runs here too, which FilmCraft otherwise does while drawing: the export
/// queue moves on, proxies, mask tracking and scene detection deliver, auto-save reports.
pub fn logic(fc: &mut FilmcraftApp, st: &mut VideoState, ctx: &egui::Context, shown: bool) {
    eframe::App::logic(fc, ctx, &mut st.frame);
    if !shown {
        fc.session.poll_persistence();
        if !fc.session.jobs.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
    // edits FilmCraft's control channel (or a finished job) just made
    follow(fc, st, ctx);
}

/// Let what hangs on the main clips follow an edit made with FilmCraft's own tools, in its step.
pub fn follow(fc: &mut FilmcraftApp, st: &mut VideoState, ctx: &egui::Context) {
    if br_video::reconcile::catch_up(&mut fc.session, &mut st.watch) {
        ctx.request_repaint();
    }
}

/// FilmCraft's raw-input hook (window focus, synthetic input of its control channel).
pub fn raw_input_hook(fc: &mut FilmcraftApp, ctx: &egui::Context, raw: &mut egui::RawInput) {
    eframe::App::raw_input_hook(fc, ctx, raw);
}

/// Draw FilmCraft in the rest of the window, and say where. FilmCraft fills its `Ui`'s whole
/// `max_rect`, so it gets a child `Ui` that ends where BR's header (and tool row) ends.
pub fn ui(fc: &mut FilmcraftApp, st: &mut VideoState, ui: &mut egui::Ui) -> egui::Rect {
    let rect = ui.available_rect_before_wrap();
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect);
        eframe::App::ui(fc, ui, &mut st.frame);
    });
    // in the same frame as the gesture in FilmCraft's timeline, menus or panels
    follow(fc, st, ui.ctx());
    rect
}

/// The video project's name: its file's, else FilmCraft's ("Untitled" for a new one).
pub fn project_name(fc: &FilmcraftApp) -> String {
    fc.session
        .path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).file_stem())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| fc.session.project.name.clone())
}

/// FilmCraft's menu bar, in BR's header.
pub fn menu_bar(fc: &mut FilmcraftApp, ui: &mut egui::Ui) {
    filmcraft_ui_egui::menus::menu_bar(fc, ui);
}

/// When the window closes: FilmCraft flushes its recovery journal and stops its auto-save.
pub fn on_exit(fc: &mut FilmcraftApp) {
    eframe::App::on_exit(fc);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fonts an app just installed are pending for a frame: the families are waited for once, and
    /// bound only when they are still missing after that.
    #[test]
    fn missing_font_families_are_waited_for_then_bound() {
        let ctx = egui::Context::default();
        let mut st = VideoState::default();
        let run = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.textures_delta.clear();
        };
        run(&ctx);
        assert!(!fonts_ready(&ctx, &mut st), "missing: FilmCraft does not draw");
        run(&ctx);
        let bound =
            |ctx: &egui::Context| ctx.fonts(|f| FAMILIES.iter().all(|n| f.definitions().families.contains_key(&FontFamily::Name((*n).into()))));
        assert!(!bound(&ctx), "the first frame only waits (a pending install is not replaced)");
        assert!(!fonts_ready(&ctx, &mut st), "still missing: bound now, ready next frame");
        run(&ctx);
        assert!(bound(&ctx));
        assert!(fonts_ready(&ctx, &mut st));
    }
}
