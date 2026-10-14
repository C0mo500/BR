//! What the window needs from the desktop: file dialogs, opening links, showing files.
//!
//! Ported from LightCraft's desktop app (`apps/lightcraft/src/main.rs` `services()` and
//! `apps/lightcraft/src/dialog_filter.rs`, MIT OR Apache-2.0), which keeps these in its binary.
//! [`services`] gives LightCraft's Pro interface its dialogs (Import Photos…, Open Library…,
//! Import Presets…); [`pick`] is the same kind of dialog for BR's Simple mode. Dialogs run on a
//! thread of their own, so the window keeps answering the compositor while one is open.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use lightcraft_ui_egui::Services;
use lightcraft_ui_egui::i18n::tr;
use rfd::FileDialog;

/// The patterns to hand to `rfd` for `exts` (plain extensions without the dot). On Linux, `rfd`
/// turns each into a `*.ext` glob that GTK and the desktop portal match case-sensitively, so each
/// extension also gets its upper-case form and a `[nN][eE][fF]`-style form: `.NEF` and `.Jpg`
/// files stay visible.
pub fn dialog_extensions_for(exts: &[&str], linux: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    for ext in exts {
        let ext = ext.trim_start_matches('.');
        let lower = ext.to_lowercase();
        push(lower.clone());
        if !linux {
            continue;
        }
        push(ext.to_uppercase());
        if lower.chars().any(|c| c.is_alphabetic()) && !lower.contains(['[', ']', '*', '?', '\\']) {
            push(
                lower
                    .chars()
                    .map(|c| {
                        let up: String = c.to_uppercase().collect();
                        if up.chars().count() == 1 && up != c.to_string() { format!("[{c}{up}]") } else { c.to_string() }
                    })
                    .collect(),
            );
        }
    }
    out
}

/// A file-type filter that matches `exts` in any letter case.
pub fn add_filter(d: FileDialog, name: impl Into<String>, exts: &[&str]) -> FileDialog {
    d.add_filter(name, &dialog_extensions_for(exts, cfg!(all(unix, not(target_os = "macos")))))
}

/// What a [`pick`] dialog asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Photos to import.
    Photos,
    /// A folder.
    Folder,
}

/// Open a file dialog on its own thread; its answer arrives on the returned channel (empty when
/// cancelled), and `ctx` is repainted then.
pub fn pick(ctx: &egui::Context, what: Pick, title: &str, start: Option<PathBuf>) -> Result<Receiver<Vec<PathBuf>>, String> {
    let (tx, rx) = channel();
    let repaint = ctx.clone();
    let title = title.to_string();
    std::thread::Builder::new()
        .name("file-dialog".into())
        .spawn(move || {
            let mut d = FileDialog::new().set_title(&title);
            if let Some(dir) = start {
                d = d.set_directory(dir);
            }
            let chosen = match what {
                Pick::Photos => add_filter(d, "Fotos", lightcraft_engine::import::EXTENSIONS).pick_files().unwrap_or_default(),
                Pick::Folder => d.pick_folder().into_iter().collect(),
            };
            let _ = tx.send(chosen);
            repaint.request_repaint();
        })
        .map_err(|e| format!("could not open the file dialog: {e}"))?;
    Ok(rx)
}

fn path(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

fn open_command(target: &str) -> std::process::Command {
    if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(target);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("explorer");
        c.arg(target);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(target);
        c
    }
}

/// Explorer's `/select,"<path>"`: the quotes keep a path with spaces (`BR Exports`) whole, and
/// the `\\?\` prefix Windows adds to long paths is something Explorer does not understand.
fn explorer_select_arg(path: &str) -> String {
    let path = path.replace('/', "\\").replace('"', "");
    let path = match path.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!(r"\\{unc}"),
        None => path.strip_prefix(r"\\?\").unwrap_or(path.as_str()).to_string(),
    };
    format!("/select,\"{path}\"")
}

/// `explorer /select,"<path>"`, passed as written (Rust would otherwise quote the whole argument
/// again, which Explorer reads as a folder to open).
fn explorer_select_command(path: &str) -> std::process::Command {
    let mut c = std::process::Command::new("explorer");
    let arg = explorer_select_arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.raw_arg(arg);
    }
    #[cfg(not(windows))]
    c.arg(arg);
    c
}

/// The services LightCraft's Pro interface uses in a window: the base set plus file dialogs,
/// links and Show in Folder. `ctx` repaints when a dialog closes.
pub fn services(ctx: &egui::Context) -> Services {
    let ctx = ctx.clone();
    let mut s = crate::base_services();
    // commands' dialogs (Import Photos…, Add Folder…, Locate…) on their own thread
    s.picker = Some(Box::new(move |req: lightcraft_ui_egui::pick::PickRequest| {
        use lightcraft_ui_egui::pick::PickKind;
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("file-dialog".into())
            .spawn(move || {
                let mut d = FileDialog::new();
                if let Some(t) = &req.title {
                    d = d.set_title(t);
                }
                if let Some((name, exts)) = &req.filter {
                    d = add_filter(d, name.clone(), exts);
                }
                if let Some(n) = &req.file_name {
                    d = d.set_file_name(n);
                }
                let paths: Vec<String> = match req.kind {
                    PickKind::Files => d.pick_files().unwrap_or_default().into_iter().map(path).collect(),
                    PickKind::File => d.pick_file().map(path).into_iter().collect(),
                    PickKind::Folder => d.pick_folder().map(path).into_iter().collect(),
                    PickKind::Save => d.save_file().map(path).into_iter().collect(),
                };
                let _ = tx.send(paths);
                repaint.request_repaint();
            })
            .map_err(|e| format!("could not open the file dialog: {e}"))?;
        Ok(rx)
    }));
    s.pick_files = Some(Box::new(|| {
        add_filter(FileDialog::new(), tr("Photos"), lightcraft_engine::import::EXTENSIONS)
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(path)
            .collect()
    }));
    s.pick_folder = Some(Box::new(|| FileDialog::new().set_title(tr("Open Library")).pick_folder().map(path)));
    s.pick_lightroom_catalog = Some(Box::new(|| {
        add_filter(FileDialog::new().set_title(tr("Import Lightroom Catalog")), tr("Lightroom Classic Catalog"), &["lrcat"]).pick_file().map(path)
    }));
    s.pick_preset_files = Some(Box::new(|| {
        add_filter(
            FileDialog::new().set_title(tr("Import Presets")),
            tr("Presets & Profiles"),
            &["lcpreset", "xmp", "lrtemplate", "zip", "dng", "lmp", "mplumpack", "cube"],
        )
        .pick_files()
        .unwrap_or_default()
        .into_iter()
        .map(path)
        .collect()
    }));
    s.save_preset_file = Some(Box::new(|name: &str| {
        add_filter(FileDialog::new().set_title(tr("Export Presets")), tr("LightCraft Preset"), &["lcpreset"])
            .set_file_name(name)
            .save_file()
            .map(path)
    }));
    s.pick_denoise_model =
        Some(Box::new(|| add_filter(FileDialog::new(), tr("ONNX model"), &["onnx"]).pick_file().map(|p| vec![path(p)]).unwrap_or_default()));
    s.pick_model_file = Some(Box::new(|| {
        add_filter(FileDialog::new().set_title(tr("Add a Face Recognition Model")), tr("Face models (ONNX)"), &["onnx"])
            .pick_file()
            .map(|p| vec![path(p)])
            .unwrap_or_default()
    }));
    s.pick_tracklog = Some(Box::new(|| {
        add_filter(FileDialog::new().set_title(tr("Auto-Tag from Tracklog")), tr("GPS Track Log"), &["gpx"])
            .pick_file()
            .map(|p| vec![path(p)])
            .unwrap_or_default()
    }));
    s.pick_keyword_list = Some(Box::new(|| {
        add_filter(FileDialog::new().set_title(tr("Import Keywords")), tr("Keyword Lists"), &["txt", "utf8"])
            .pick_file()
            .map(|p| vec![path(p)])
            .unwrap_or_default()
    }));
    s.save_keyword_list = Some(Box::new(|name: &str| {
        add_filter(FileDialog::new().set_title(tr("Export Keywords")), tr("Keyword Lists"), &["txt"]).set_file_name(name).save_file().map(path)
    }));
    s.pick_curve_preset_files = Some(Box::new(|| {
        add_filter(FileDialog::new().set_title(tr("Import Point Curve Presets")), tr("Point Curve Presets"), &["lccurve", "json"])
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(path)
            .collect()
    }));
    s.save_curve_preset_file = Some(Box::new(|name: &str| {
        add_filter(FileDialog::new().set_title(tr("Export Point Curve Presets")), tr("Point Curve Presets"), &["lccurve"])
            .set_file_name(name)
            .save_file()
            .map(path)
    }));
    s.open_url = Some(Box::new(|url: &str| {
        if !url.starts_with("https://") {
            return Err("only https links are opened".into());
        }
        open_command(url).status().map(|_| ()).map_err(|e| e.to_string())
    }));
    s.reveal = Some(Box::new(|file: &str| {
        let status = if cfg!(target_os = "macos") {
            std::process::Command::new("open").args(["-R", file]).status()
        } else if cfg!(target_os = "windows") {
            explorer_select_command(file).status()
        } else {
            let dir = std::path::Path::new(file).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|| ".".into());
            open_command(&dir).status()
        };
        status.map(|_| ()).map_err(|e| e.to_string())
    }));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_platforms_get_lower_case_once() {
        assert_eq!(dialog_extensions_for(&["NEF", "nef", ".Jpg", "jpg", ""], false), ["nef", "jpg"]);
    }

    #[test]
    fn explorer_select_quotes_the_path() {
        assert_eq!(explorer_select_arg(r"C:\Users\a b\Pictures\BR Exports\x.jpg"), r#"/select,"C:\Users\a b\Pictures\BR Exports\x.jpg""#);
        assert_eq!(explorer_select_arg(r"\\?\C:\long\x.jpg"), r#"/select,"C:\long\x.jpg""#);
        assert_eq!(explorer_select_arg(r"\\?\UNC\server\share\x.jpg"), r#"/select,"\\server\share\x.jpg""#);
        assert_eq!(explorer_select_arg("C:/a/b.jpg"), r#"/select,"C:\a\b.jpg""#);
    }

    #[test]
    fn linux_gets_lower_upper_and_any_mix() {
        assert_eq!(dialog_extensions_for(&["nef"], true), ["nef", "NEF", "[nN][eE][fF]"]);
        assert_eq!(dialog_extensions_for(&["mp4", "MP4"], true), ["mp4", "MP4", "[mM][pP]4"]);
    }
}
