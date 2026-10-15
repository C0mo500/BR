//! What the window needs from the desktop: file dialogs, opening links, showing files, external
//! editors, the screen's colour profile and the system's dark/light setting, and where BR keeps
//! its settings and logs.
//!
//! Ported from LightCraft's desktop app (`apps/lightcraft/src/main.rs` `services()` and
//! `apps/lightcraft/src/dialog_filter.rs`, MIT OR Apache-2.0), which keeps these in its binary;
//! the submodules are its `editors.rs`, `appearance.rs`, `colord.rs` (Linux), `screen_profile.rs`
//! (macOS) and the settings file ([`prefs`]). [`services`] gives LightCraft's interface its
//! dialogs (Import Photos…, Open Library…, Import Presets…) and the rest; [`pick`] is the same
//! kind of dialog for BR's own tools. Dialogs run on a thread of their own, so the window keeps
//! answering the compositor while one is open.

mod appearance;
#[cfg(feature = "audio")]
pub mod audio;
#[cfg(feature = "audio")]
pub mod audio_in;
#[cfg(target_os = "linux")]
mod colord;
mod editors;
pub mod film;
pub mod pointer;
pub mod prefs;
#[cfg(target_os = "macos")]
mod screen_profile;
#[cfg(unix)]
pub mod stream;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use lightcraft_ui_egui::Services;
use lightcraft_ui_egui::i18n::tr;
use rfd::FileDialog;

#[derive(Clone, Copy)]
enum Platform {
    Mac,
    Windows,
    Other,
}

impl Platform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Other
        }
    }
}

/// `name` from the environment, when it is set to something.
fn var(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

/// An XDG base folder: only an absolute path counts (the spec says to ignore relative ones).
fn xdg(env: &impl Fn(&str) -> Option<OsString>, name: &str) -> Option<PathBuf> {
    env(name).map(PathBuf::from).filter(|d| d.is_absolute())
}

/// BR's settings folder (LightCraft's interface settings, [`prefs`]): `$BR_CONFIG_DIR` when set
/// (set empty: none, so nothing is kept), else macOS `~/Library/Application Support/BR`, Windows
/// `%APPDATA%\BR`, elsewhere `$XDG_CONFIG_HOME/BR` or `~/.config/BR`.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_with(Platform::current(), |name| std::env::var_os(name))
}

fn config_dir_with(platform: Platform, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(dir) = env("BR_CONFIG_DIR") {
        return (!dir.is_empty()).then(|| PathBuf::from(dir));
    }
    let set = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = match platform {
        Platform::Mac => set("HOME").map(|h| h.join("Library/Application Support")),
        Platform::Windows => set("APPDATA"),
        Platform::Other => xdg(&env, "XDG_CONFIG_HOME").or_else(|| set("HOME").map(|h| h.join(".config"))),
    };
    base.map(|b| b.join("BR"))
}

/// BR's log folder: `Logs` in `$BR_CONFIG_DIR` when that is set (set empty: none, no log file),
/// else in BR's folder of the platform's data folder: macOS `~/Library/Application Support/BR/Logs`,
/// Windows `%LOCALAPPDATA%\BR\Logs`, elsewhere `$XDG_DATA_HOME/BR/Logs` or `~/.local/share/BR/Logs`.
pub fn log_dir() -> Option<PathBuf> {
    log_dir_with(Platform::current(), |name| std::env::var_os(name))
}

fn log_dir_with(platform: Platform, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(dir) = env("BR_CONFIG_DIR") {
        return (!dir.is_empty()).then(|| PathBuf::from(dir).join("Logs"));
    }
    let set = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = match platform {
        Platform::Mac => set("HOME").map(|h| h.join("Library/Application Support")),
        Platform::Windows => set("LOCALAPPDATA").or_else(|| set("APPDATA")),
        Platform::Other => xdg(&env, "XDG_DATA_HOME").or_else(|| set("HOME").map(|h| h.join(".local/share"))),
    };
    base.map(|b| b.join("BR").join("Logs"))
}

/// Where the video side keeps its settings, workspaces, auto-saves and crash-recovery journal
/// (FilmCraft's data folder): `Video` beside BR's `Logs` folder.
pub fn video_dir() -> Option<PathBuf> {
    log_dir().and_then(|l| l.parent().map(|p| p.join("Video")))
}

/// Where the system keeps monitor profiles, for the Choose Monitor Profile dialog to start in: the
/// user's own profiles first (Linux: colord / DisplayCAL; macOS: ColorSync), then the system's.
fn display_profile_dir() -> Option<PathBuf> {
    let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let user = |rel: &str| home.as_ref().map(|h| h.join(rel));
    let candidates: Vec<Option<PathBuf>> = if cfg!(target_os = "macos") {
        vec![user("Library/ColorSync/Profiles"), Some("/Library/ColorSync/Profiles".into())]
    } else if cfg!(windows) {
        let root = var("SystemRoot").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        vec![Some(root.join("System32").join("spool").join("drivers").join("color"))]
    } else {
        let data = var("XDG_DATA_HOME").map(PathBuf::from).or_else(|| user(".local/share"));
        vec![data.map(|d| d.join("icc")), user(".color/icc"), Some("/usr/share/color/icc".into())]
    };
    candidates.into_iter().flatten().find(|d| d.is_dir())
}

/// The file dialogs' parent, set once the window exists ([`services`]). Without one, the
/// xdg-desktop-portal picker on Wayland is a stray top-level window: a tiling compositor puts it
/// beside BR and a full-screen window hides it, so Import Photos… looks like a hang. `FileDialog`
/// is `Clone` and `Send`, so one parented template serves every dialog, those on their own thread
/// included. Only on Linux and FreeBSD, as in LightCraft: on macOS a parented dialog becomes a
/// sheet and on Windows a modal one.
static DIALOG_PARENT: std::sync::OnceLock<FileDialog> = std::sync::OnceLock::new();

/// A file dialog owned by BR's window, or a plain one before the window exists.
fn file_dialog() -> FileDialog {
    DIALOG_PARENT.get().cloned().unwrap_or_default()
}

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
    /// Video, sound and stills for the video.
    Media,
    /// A music file.
    Music,
    /// A folder.
    Folder,
}

/// The folder BR's import dialog starts in: `$BR_IMPORT_DIR` (a hosted BR points it at the folder
/// uploads arrive in), else the dialog's own choice.
pub fn import_dir() -> Option<PathBuf> {
    std::env::var_os("BR_IMPORT_DIR").filter(|d| !d.is_empty()).map(PathBuf::from)
}

/// `rfd` falls back to zenity on Linux when there is no desktop portal (a server display, for
/// one). zenity has no starting folder, only a file to preselect; a name that is not there makes
/// it open the folder with nothing selected. The portal ignores the name in open dialogs.
fn zenity_start(dir: &std::path::Path) -> String {
    dir.join(" ").to_string_lossy().into_owned()
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
            let mut d = file_dialog().set_title(&title);
            if let Some(dir) = start {
                if cfg!(all(unix, not(target_os = "macos"))) {
                    d = d.set_file_name(zenity_start(&dir));
                }
                d = d.set_directory(dir);
            }
            let chosen = match what {
                Pick::Photos => add_filter(d, "Fotos", lightcraft_engine::import::EXTENSIONS).pick_files().unwrap_or_default(),
                Pick::Media => add_filter(d, "Medien", &br_video::media_extensions()).pick_files().unwrap_or_default(),
                Pick::Music => add_filter(d, "Musik", br_video::audio_extensions()).pick_file().into_iter().collect(),
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

/// The services LightCraft's interface uses in a window: the base set plus file dialogs, links,
/// Show in Folder, external editors, the screen's colour profile and the system's dark/light
/// setting. `cc` is the window the dialogs belong to (its `egui_ctx` repaints when one closes);
/// `log_file` is the log Help ▸ Open Log Folder shows (`None`: no log file this run).
pub fn services(cc: &eframe::CreationContext<'_>, log_file: Option<&Path>) -> Services {
    let ctx = cc.egui_ctx.clone();
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let _ = DIALOG_PARENT.set(FileDialog::new().set_parent(cc));
    let mut s = crate::base_services();
    // commands' dialogs (Import Photos…, Add Folder…, Locate…) on their own thread
    s.picker = Some(Box::new(move |req: lightcraft_ui_egui::pick::PickRequest| {
        use lightcraft_ui_egui::pick::PickKind;
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("file-dialog".into())
            .spawn(move || {
                let mut d = file_dialog();
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
        add_filter(file_dialog(), tr("Photos"), lightcraft_engine::import::EXTENSIONS)
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(path)
            .collect()
    }));
    s.pick_folder = Some(Box::new(|| file_dialog().set_title(tr("Open Library")).pick_folder().map(path)));
    s.pick_lightroom_catalog = Some(Box::new(|| {
        add_filter(file_dialog().set_title(tr("Import Lightroom Catalog")), tr("Lightroom Classic Catalog"), &["lrcat"]).pick_file().map(path)
    }));
    s.pick_preset_files = Some(Box::new(|| {
        add_filter(
            file_dialog().set_title(tr("Import Presets")),
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
        add_filter(file_dialog().set_title(tr("Export Presets")), tr("LightCraft Preset"), &["lcpreset"]).set_file_name(name).save_file().map(path)
    }));
    s.pick_denoise_model =
        Some(Box::new(|| add_filter(file_dialog(), tr("ONNX model"), &["onnx"]).pick_file().map(|p| vec![path(p)]).unwrap_or_default()));
    s.pick_model_file = Some(Box::new(|| {
        add_filter(file_dialog().set_title(tr("Add a Face Recognition Model")), tr("Face models (ONNX)"), &["onnx"])
            .pick_file()
            .map(|p| vec![path(p)])
            .unwrap_or_default()
    }));
    s.pick_tracklog = Some(Box::new(|| {
        add_filter(file_dialog().set_title(tr("Auto-Tag from Tracklog")), tr("GPS Track Log"), &["gpx"])
            .pick_file()
            .map(|p| vec![path(p)])
            .unwrap_or_default()
    }));
    s.pick_keyword_list = Some(Box::new(|| {
        add_filter(file_dialog().set_title(tr("Import Keywords")), tr("Keyword Lists"), &["txt", "utf8"])
            .pick_file()
            .map(|p| vec![path(p)])
            .unwrap_or_default()
    }));
    s.save_keyword_list = Some(Box::new(|name: &str| {
        add_filter(file_dialog().set_title(tr("Export Keywords")), tr("Keyword Lists"), &["txt"]).set_file_name(name).save_file().map(path)
    }));
    s.pick_curve_preset_files = Some(Box::new(|| {
        add_filter(file_dialog().set_title(tr("Import Point Curve Presets")), tr("Point Curve Presets"), &["lccurve", "json"])
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(path)
            .collect()
    }));
    s.save_curve_preset_file = Some(Box::new(|name: &str| {
        add_filter(file_dialog().set_title(tr("Export Point Curve Presets")), tr("Point Curve Presets"), &["lccurve"])
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
    s.log_file = log_file.map(|p| p.display().to_string());
    // Photo ▸ Edit In and Open Original: PhotoCraft or the editor named in Settings
    s.open_with = Some(Box::new(editors::open_with));
    s.open_all_with = Some(Box::new(editors::open_all_with));
    s.check_editor = Some(Box::new(editors::check));
    // the SAM 3 model's folder (AI masks); LightCraft runs this on a worker and waits for the opener
    #[cfg(target_os = "linux")]
    {
        s.open_model_folder = Some(std::sync::Arc::new(|dir: &str| {
            open_command(dir)
                .status()
                .map_err(|e| e.to_string())
                .and_then(|s| if s.success() { Ok(()) } else { Err(format!("opening folder failed: {s}")) })
        }));
    }
    s.pick_display_profile = Some(Box::new(|| {
        let mut d = add_filter(file_dialog().set_title(tr("Choose Monitor Profile")), tr("ICC Profiles"), &["icc", "icm"]);
        if let Some(dir) = display_profile_dir() {
            d = d.set_directory(dir);
        }
        d.pick_file().map(|p| vec![path(p)]).unwrap_or_default()
    }));
    // macOS doesn't colour-manage the window: previews follow the screen's profile (LightCraft #787)
    #[cfg(target_os = "macos")]
    {
        s.screen_profile = Some(Box::new(screen_profile::icc));
    }
    // Linux: the profile the desktop assigns in colord, read off the UI thread
    #[cfg(target_os = "linux")]
    {
        s.screen_profile = Some(colord::service());
    }
    s.system_theme = appearance::service();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(v))
    }

    #[test]
    fn settings_and_logs_live_in_brs_own_folders() {
        let p = |s: &str| Some(PathBuf::from(s));
        let home = [("HOME", "/home/anna")];
        assert_eq!(config_dir_with(Platform::Other, env(&home)), p("/home/anna/.config/BR"));
        assert_eq!(log_dir_with(Platform::Other, env(&home)), p("/home/anna/.local/share/BR/Logs"));
        let xdg = [("HOME", "/home/anna"), ("XDG_CONFIG_HOME", "/cfg"), ("XDG_DATA_HOME", "/data")];
        assert_eq!(config_dir_with(Platform::Other, env(&xdg)), p("/cfg/BR"));
        assert_eq!(log_dir_with(Platform::Other, env(&xdg)), p("/data/BR/Logs"));
        // relative or empty XDG folders don't count (the XDG spec): the home folder's are used
        let odd = [("HOME", "/home/anna"), ("XDG_CONFIG_HOME", "cfg"), ("XDG_DATA_HOME", "")];
        assert_eq!(config_dir_with(Platform::Other, env(&odd)), p("/home/anna/.config/BR"));
        assert_eq!(log_dir_with(Platform::Other, env(&odd)), p("/home/anna/.local/share/BR/Logs"));
        assert_eq!(config_dir_with(Platform::Mac, env(&home)), p("/home/anna/Library/Application Support/BR"));
        assert_eq!(log_dir_with(Platform::Mac, env(&home)), p("/home/anna/Library/Application Support/BR/Logs"));
        let win = [("APPDATA", r"C:\Users\anna\AppData\Roaming"), ("LOCALAPPDATA", r"C:\Users\anna\AppData\Local")];
        assert_eq!(config_dir_with(Platform::Windows, env(&win)), Some(PathBuf::from(win[0].1).join("BR")));
        assert_eq!(log_dir_with(Platform::Windows, env(&win)), Some(PathBuf::from(win[1].1).join("BR").join("Logs")));
        // no home folder (an empty one is none): nothing is kept, never a folder in the working directory
        for platform in [Platform::Other, Platform::Mac, Platform::Windows] {
            assert_eq!(config_dir_with(platform, env(&[("HOME", "")])), None);
            assert_eq!(log_dir_with(platform, env(&[])), None);
        }
    }

    /// `BR_CONFIG_DIR` keeps a separate profile (settings and logs) together; set empty, BR keeps
    /// nothing at all (scripts, tests).
    #[test]
    fn br_config_dir_overrides_or_turns_off_both() {
        let own = [("HOME", "/home/anna"), ("BR_CONFIG_DIR", "/tmp/profile")];
        assert_eq!(config_dir_with(Platform::Other, env(&own)), Some(PathBuf::from("/tmp/profile")));
        assert_eq!(log_dir_with(Platform::Windows, env(&own)), Some(PathBuf::from("/tmp/profile/Logs")));
        let off = [("HOME", "/home/anna"), ("BR_CONFIG_DIR", "")];
        for platform in [Platform::Other, Platform::Mac, Platform::Windows] {
            assert_eq!(config_dir_with(platform, env(&off)), None);
            assert_eq!(log_dir_with(platform, env(&off)), None);
        }
    }

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
    fn zenity_opens_inside_the_folder() {
        let dir = std::path::Path::new("/srv/br/Eingang");
        let name = zenity_start(dir);
        assert_eq!(std::path::Path::new(&name).parent(), Some(dir));
        assert!(!std::path::Path::new(&name).exists());
    }

    #[test]
    fn linux_gets_lower_upper_and_any_mix() {
        assert_eq!(dialog_extensions_for(&["nef"], true), ["nef", "NEF", "[nN][eE][fF]"]);
        assert_eq!(dialog_extensions_for(&["mp4", "MP4"], true), ["mp4", "MP4", "[mM][pP]4"]);
    }
}
