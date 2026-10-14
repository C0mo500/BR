//! What FilmCraft needs from the desktop in BR's window: file dialogs, opening files and links,
//! the system theme, pasting files and images.
//!
//! Ported from FilmCraft's desktop app (`apps/filmcraft/src/main.rs` and `file_filters.rs` in
//! storytold/filmcraft, MIT OR Apache-2.0), which keeps them in its binary. The case-insensitive
//! file patterns are BR's own [`super::dialog_extensions_for`], which does the same.

use filmcraft_ui_egui::HostHooks;
use filmcraft_ui_egui::i18n::t;

/// File dialog patterns for `exts` in any letter case where the picker needs it (Linux).
fn patterns(exts: &[&str]) -> Vec<String> {
    super::dialog_extensions_for(exts, cfg!(all(unix, not(target_os = "macos"))))
}

fn path(p: std::path::PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

/// FilmCraft's host hooks for a desktop window.
#[allow(clippy::field_reassign_with_default)] // one hook per statement, some only on one system
pub fn hooks() -> HostHooks {
    let mut h = HostHooks::default();
    h.pick_files = Some(Box::new(|exts: &[&str]| {
        super::file_dialog().add_filter(t("Media"), &patterns(exts)).pick_files().unwrap_or_default().into_iter().map(path).collect()
    }));
    // Link Media ▸ Locate…, Attach Proxies, Reconnect Full Resolution: one path, not imported
    h.pick_file_for_relink =
        Some(Box::new(|exts: &[&str], _hint| super::file_dialog().add_filter(t("Media"), &patterns(exts)).pick_file().map(path)));
    h.pick_save = Some(Box::new(|name: &str| {
        super::file_dialog().add_filter(t("FilmCraft Project"), &patterns(&["fcproj"])).set_file_name(name).save_file().map(path)
    }));
    h.pick_save_as = Some(Box::new(|filter: &str, exts: &[&str], name: &str| {
        // a full path (Quick Export, Export mode) opens the dialog in its folder
        let p = std::path::Path::new(name);
        let mut d =
            super::file_dialog().add_filter(t(filter), &patterns(exts)).set_file_name(p.file_name().map_or(name.into(), |n| n.to_string_lossy()));
        if let Some(dir) = p.parent().filter(|d| d.is_dir()) {
            d = d.set_directory(dir);
        }
        d.save_file().map(path)
    }));
    h.pick_folder = Some(Box::new(|| super::file_dialog().pick_folder().map(path)));
    h.pick_folder_at = Some(Box::new(|dir: &str| super::file_dialog().set_directory(dir).pick_folder().map(path)));
    h.pick_open_file =
        Some(Box::new(|filter: &str, exts: &[&str]| super::file_dialog().add_filter(t(filter), &patterns(exts)).pick_file().map(path)));
    h.pick_open_project = Some(Box::new(|| super::file_dialog().add_filter(t("FilmCraft Project"), &patterns(&["fcproj"])).pick_file().map(path)));
    // Linux: the XDG portal call blocks until the user chooses, which would stop the frame loop
    // (GNOME then offers to force-quit the window): the dialog runs on its own thread instead
    #[cfg(target_os = "linux")]
    {
        h.run_dialog = Some(Box::new(run_dialog_off_thread));
    }
    h.open_path = Some(Box::new(open_path));
    h.open_url = Some(Box::new(|url: &str| {
        if !url.starts_with("https://") {
            return Err("only https links are opened".into());
        }
        super::open_command(url).spawn().map(|_| ()).map_err(|e| e.to_string())
    }));
    h.system_theme = super::appearance::service();
    h.clipboard_media = Some(Box::new(clipboard_media));
    h
}

/// Files copied in the file manager, else an image, on the system clipboard (Paste).
fn clipboard_media() -> Option<filmcraft_ui_egui::ClipboardMedia> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    if let Ok(files) = clipboard.get().file_list()
        && !files.is_empty()
    {
        return Some(filmcraft_ui_egui::ClipboardMedia::Files(files.iter().map(|p| p.to_string_lossy().into_owned()).collect()));
    }
    let image = clipboard.get_image().ok()?;
    Some(filmcraft_ui_egui::ClipboardMedia::Image { width: image.width, height: image.height, rgba: image.bytes.into_owned() })
}

/// Open a file in its default application, or show it in the file manager (Edit Original, Reveal
/// Log Files).
fn open_path(path: &str, reveal: bool) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        if reveal {
            c.arg("-R");
        }
        c.arg(path);
        c
    } else if cfg!(target_os = "windows") {
        if reveal {
            super::explorer_select_command(path)
        } else {
            let mut c = std::process::Command::new("explorer");
            c.arg(path);
            c
        }
    } else {
        // Linux file managers have no select-file flag: the folder opens instead
        let p = std::path::Path::new(path);
        let mut c = std::process::Command::new("xdg-open");
        c.arg(if reveal { p.parent().unwrap_or(p) } else { p });
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("can't open {path}: {e}"))
}

#[cfg(target_os = "linux")]
fn run_dialog_off_thread(dialog: filmcraft_ui_egui::native_dialogs::FileDialog) -> std::sync::mpsc::Receiver<Vec<String>> {
    use filmcraft_ui_egui::native_dialogs::FileDialog;
    // filter names translated here, on the UI thread: the interface language is per thread
    let (media, project) = (t("Media").to_string(), t("FilmCraft Project").to_string());
    let list = |exts: Vec<String>| patterns(&exts.iter().map(String::as_str).collect::<Vec<_>>());
    let dialog = match dialog {
        FileDialog::OpenFiles { exts } => FileDialog::OpenFiles { exts: list(exts) },
        FileDialog::OpenFile { filter, exts } => FileDialog::OpenFile { filter: t(&filter).to_string(), exts: list(exts) },
        FileDialog::SaveAs { filter, exts, name } => FileDialog::SaveAs { filter: t(&filter).to_string(), exts: list(exts), name },
        FileDialog::Relink { exts, hint } => FileDialog::Relink { exts: list(exts), hint },
        other => other,
    };
    let project_exts = patterns(&["fcproj"]);
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("file-dialog".into()).spawn(move || {
        let shown = std::panic::catch_unwind(move || match dialog {
            FileDialog::OpenFiles { exts } => {
                super::file_dialog().add_filter(&media, &exts).pick_files().unwrap_or_default().into_iter().map(path).collect()
            }
            FileDialog::OpenFile { filter, exts } => super::file_dialog().add_filter(&filter, &exts).pick_file().map(path).into_iter().collect(),
            FileDialog::Relink { exts, .. } => super::file_dialog().add_filter(&media, &exts).pick_file().map(path).into_iter().collect(),
            FileDialog::OpenProject => super::file_dialog().add_filter(&project, &project_exts).pick_file().map(path).into_iter().collect(),
            FileDialog::SaveProject { name } => {
                super::file_dialog().add_filter(&project, &project_exts).set_file_name(name).save_file().map(path).into_iter().collect()
            }
            FileDialog::SaveAs { filter, exts, name } => {
                super::file_dialog().add_filter(&filter, &exts).set_file_name(name).save_file().map(path).into_iter().collect()
            }
            FileDialog::Folder { at } => {
                let d = super::file_dialog();
                let d = match at {
                    Some(dir) if !dir.is_empty() => d.set_directory(dir),
                    _ => d,
                };
                d.pick_folder().map(path).into_iter().collect::<Vec<_>>()
            }
        });
        match shown {
            Ok(paths) => {
                let _ = tx.send(paths);
            }
            Err(_) => log::warn!("the file dialog thread panicked; taken as cancelled"),
        }
    });
    if let Err(e) = spawned {
        log::warn!("could not start the file dialog thread: {e}");
    }
    rx
}

/// FilmCraft's exports composited on the GPU (its off-screen compositor behind the export's
/// frame-renderer hook), falling back to the CPU renderer wherever it cannot.
struct GpuFrameRenderer(filmcraft_gpu::ExportRenderer);

impl filmcraft_export::FrameRenderer for GpuFrameRenderer {
    fn render(
        &mut self,
        project: &filmcraft_project::Project,
        seq: filmcraft_project::ItemId,
        t: filmcraft_time::Tick,
        opts: filmcraft_render::RenderOptions,
        sources: &dyn filmcraft_render::SourceProvider,
    ) -> Option<filmcraft_render::Image> {
        self.0.render(project, seq, t, opts, sources)
    }
}

/// Process-wide set-up for FilmCraft, once before the window opens: the system's hardware video
/// decoders in front of FilmCraft's own, exports composited on the GPU (`gpu_export`: only when
/// the window itself renders with wgpu, so a driver that failed is not loaded again), and
/// FilmCraft's crash log, which its panels use to say what went wrong when they catch a panic.
pub fn register_process(log_dir: Option<std::path::PathBuf>, gpu_export: bool) {
    let hardware = filmcraft_platform::register();
    log::info!("hardware video decoding: {hardware:?}");
    if gpu_export {
        filmcraft_export::register_frame_renderer(|| {
            filmcraft_gpu::ExportRenderer::new().map(|r| Box::new(GpuFrameRenderer(r)) as Box<dyn filmcraft_export::FrameRenderer>)
        });
    }
    filmcraft_ui_egui::crash::install(log_dir);
}

/// A video session for a window: auto-save and crash recovery in `data_dir` (FilmCraft's settings,
/// workspaces and recovery journal live there). Started before FilmCraft's interface is made, so
/// it offers to recover what a crash left and finds the workspaces saved before.
pub fn session(data_dir: Option<std::path::PathBuf>) -> br_video::Session {
    let mut s = br_video::Session::default();
    if let Some(dir) = data_dir
        && let Err(e) = s.start_autosave(filmcraft_engine::autosave::AutosaveConfig::new(dir))
    {
        log::warn!("video auto-save and crash recovery unavailable: {e}");
    }
    s
}

/// FilmCraft in BR's window: the desktop hooks, the window's GPU device for the monitors (without
/// one they composite on the CPU), and sound.
pub fn attach(fc: &mut filmcraft_ui_egui::FilmcraftApp, cc: &eframe::CreationContext<'_>) {
    fc.hooks = hooks();
    if let Some(rs) = cc.wgpu_render_state.clone() {
        fc.set_wgpu(rs);
    }
    #[cfg(feature = "audio")]
    {
        fc.audio = Some(Box::new(super::audio::CpalOut::new()));
    }
}
