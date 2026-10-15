//! LightCraft's interface settings across restarts: `<settings folder>/lightcraft-ui.json`
//! ([`super::config_dir`]), the same `UiState` LightCraft keeps in its own `ui.json` (panels,
//! language, appearance, the library last opened…).
//!
//! Ported from LightCraft's desktop app (`apps/lightcraft/src/main.rs`: `load_prefs`,
//! `decode_saved_ui`, `write_atomic`, `PrefsWriter`; MIT OR Apache-2.0), which keeps them in its
//! binary. Only the window reads and writes the file: headless runs, `shnyp snapshot` and tests never
//! do, and a `--demo` window reads it but writes nothing (a `PrefsWriter` without a path).

use std::path::{Path, PathBuf};

use lightcraft_ui_egui::{LightcraftApp, UiState};

/// The file's name in the settings folder.
pub const FILE: &str = "lightcraft-ui.json";

/// `<settings folder>/lightcraft-ui.json`; `None` when there is no settings folder (no home folder,
/// or `SHNYP_CONFIG_DIR` set empty): then nothing is read or written.
pub fn path() -> Option<PathBuf> {
    super::config_dir().map(|d| d.join(FILE))
}

/// The saved settings, a warning for the user when the file exists but can't be used, and whether
/// the file must be kept as it is (never written this session).
///
/// A damaged file is kept as `lightcraft-ui.json.corrupt-<unix time>` first (`set_aside_damaged`;
/// LightCraft issue #103) and defaults are used; one that can't be read at all is not overwritten.
/// A `--demo` window passes `set_aside_damaged = false`: it reads the settings (so it looks like
/// the user's shnyp) but changes nothing on disk, so a damaged file is left where it is.
pub fn load_prefs_at(path: &Path, set_aside_damaged: bool) -> (Option<UiState>, Option<String>, bool) {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, None, false),
        Err(e) => {
            let msg = format!(
                "Die Einstellungen ({}) konnten nicht gelesen werden: {e}. shnyp nutzt die Voreinstellungen und überschreibt die Datei nicht.",
                path.display()
            );
            return (None, Some(msg), true);
        }
    };
    match decode_saved_ui(&bytes) {
        Ok((ui, false)) => (Some(ui.sanitized()), None, false),
        Ok((ui, true)) => {
            let msg = format!(
                "Die gespeicherte Anordnung der Bedienfelder in {} ließ sich nicht wiederherstellen. Die übrigen Einstellungen sind geladen; \
                 die Datei bleibt, wie sie ist, und in dieser Sitzung wird nichts gespeichert.",
                path.display()
            );
            (Some(ui.sanitized()), Some(msg), true)
        }
        Err(e) if !set_aside_damaged => {
            let msg = format!(
                "Die Einstellungen ({}) sind beschädigt ({e}). shnyp nutzt die Voreinstellungen und speichert in dieser Sitzung nichts.",
                path.display()
            );
            (None, Some(msg), true)
        }
        Err(e) => {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| FILE.into());
            let keep = path.with_file_name(format!("{name}.corrupt-{secs}"));
            let kept = std::fs::rename(path, &keep);
            let what = match &kept {
                Ok(()) => format!("sie ist als {} aufbewahrt", keep.display()),
                Err(r) => format!("sie ließ sich nicht beiseitelegen ({r})"),
            };
            let msg = format!("Die Einstellungen ({}) sind beschädigt ({e}); {what}. shnyp nutzt die Voreinstellungen.", path.display());
            (None, Some(msg), kept.is_err())
        }
    }
}

/// The settings in `bytes`, and whether a saved panel layout had to be dropped: one this
/// LightCraft can't use (written by a newer one, or invalid) is left out so the rest still loads.
/// Everything else is read as strictly as LightCraft reads it.
pub fn decode_saved_ui(bytes: &[u8]) -> Result<(UiState, bool), String> {
    let mut saved: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let fields = saved.as_object_mut().ok_or("settings must be an object")?;
    let recovered =
        fields.get("docking").is_some_and(|value| match serde_json::from_value::<Option<lightcraft_ui_egui::docking::Workspace>>(value.clone()) {
            Ok(Some(workspace)) => workspace.validate().is_err(),
            Ok(None) => false,
            Err(_) => true,
        });
    if recovered {
        fields.remove("docking");
    }
    let ui = serde_json::from_value(saved).map_err(|error| error.to_string())?;
    Ok((ui, recovered))
}

/// Write `bytes` to `path` atomically: a temp file, fsynced, renamed over it (a crash or a full
/// disk leaves the old file or the new one, never a truncated one).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent()
        && let Ok(d) = std::fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Saves the settings: as soon as the library changes, every few seconds when anything else
/// changed, and when the window closes.
///
/// Nothing is written while the session is one that promises to save nothing: `--demo` (no path),
/// or the one running because the library couldn't be opened (LightCraft's question window and
/// Continue Without Saving; once a library opens, it is saved again).
#[derive(Default)]
pub struct PrefsWriter {
    /// Where to write; `None` writes nothing (`--demo`, no settings folder).
    path: Option<PathBuf>,
    written: Vec<u8>,
    library: String,
    checked: f64,
    /// The last write failed (reported once until a write works).
    failing: bool,
    /// The file couldn't be read at launch: never overwrite it this session.
    keep_file: bool,
}

impl PrefsWriter {
    /// A writer for `path` (`None`: write nothing) that takes `ui` as what is on disk now, so only
    /// changes are written. `keep_file` is [`load_prefs_at`]'s answer.
    pub fn new(path: Option<PathBuf>, ui: &UiState, keep_file: bool) -> Self {
        Self {
            path,
            written: serde_json::to_vec_pretty(ui).unwrap_or_default(),
            library: ui.settings.library_path.clone(),
            keep_file,
            ..Default::default()
        }
    }

    /// Write `app`'s settings if they changed (on exit; [`PrefsWriter::tick`] otherwise).
    pub fn save(&mut self, app: &LightcraftApp) -> Result<(), String> {
        self.save_ui(&app.ui, app.library_problem.is_some())
    }

    /// Write `ui` if it changed since the last write; `temporary` (the library couldn't be
    /// opened) saves nothing.
    fn save_ui(&mut self, ui: &UiState, temporary: bool) -> Result<(), String> {
        if self.keep_file || temporary {
            return Ok(());
        }
        let Some(path) = self.path.clone() else { return Ok(()) };
        if let Some(workspace) = &ui.docking {
            workspace.validate()?;
        }
        let bytes = serde_json::to_vec_pretty(ui).map_err(|e| e.to_string())?;
        if bytes == self.written {
            return Ok(());
        }
        path.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| write_atomic(&path, &bytes))
            .map_err(|e| format!("saving the settings to {} failed: {e}", path.display()))?;
        self.written = bytes;
        self.library = ui.settings.library_path.clone();
        Ok(())
    }

    /// Per frame: save when the library changed, else at most every three seconds.
    pub fn tick(&mut self, app: &mut LightcraftApp, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let moved = app.ui.settings.library_path != self.library;
        if !moved && now - self.checked < 3.0 {
            return;
        }
        self.checked = now;
        match self.save(app) {
            Ok(()) => self.failing = false,
            Err(e) => {
                if !self.failing {
                    log::warn!("{e}");
                    app.notices.push(format!("Die Einstellungen konnten nicht gespeichert werden ({e}). shnyp versucht es weiter."));
                }
                self.failing = true;
                // don't retry every frame
                self.library = app.ui.settings.library_path.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_ui_egui::Services;
    use lightcraft_ui_egui::i18n::Locale;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("shnyp-prefs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// What shnyp's window does with what it finds: the saved settings replace shnyp's defaults, so
    /// German is the language only for a first start (or after the file was set aside).
    #[test]
    fn the_saved_language_wins_over_shnyps_german_default() {
        let d = dir("language");
        let path = d.join(FILE);
        let first = crate::ShnypApp::new(lightcraft_engine::Session::new(), None);
        assert_eq!(first.lc.ui.language, Locale::De);
        // nothing changed yet: nothing is written
        let mut w = PrefsWriter::new(Some(path.clone()), &first.lc.ui, false);
        w.save(&first.lc).unwrap();
        assert!(!path.exists());
        // the user picks English and a bigger grid
        let mut ui = first.lc.ui.clone();
        ui.language = Locale::En;
        ui.thumb_size = 300.0;
        w.save_ui(&ui, false).unwrap();
        let (loaded, warning, keep) = load_prefs_at(&path, true);
        assert!(warning.is_none() && !keep);
        let mut next = crate::ShnypApp::new(lightcraft_engine::Session::new(), None);
        next.lc.ui = loaded.unwrap();
        assert_eq!(next.lc.ui.language, Locale::En);
        assert_eq!(next.lc.ui.thumb_size, 300.0);
        // no file: no settings, shnyp's default stays
        std::fs::remove_file(&path).unwrap();
        assert!(load_prefs_at(&path, true).0.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A damaged file (it may hold the library location) is kept aside and reported, not silently
    /// replaced by the defaults.
    #[test]
    fn a_damaged_file_is_kept_and_reported() {
        let d = dir("damaged");
        let path = d.join(FILE);
        std::fs::write(&path, br#"{"settings": {"library_path": "/Volumes/Photos/Lib"#).unwrap();
        let (ui, warning, keep) = load_prefs_at(&path, true);
        assert!(ui.is_none() && !keep);
        let warning = warning.unwrap();
        assert!(warning.contains("beschädigt") && warning.contains("lightcraft-ui.json.corrupt-"), "{warning}");
        assert!(!path.exists());
        let kept: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        assert!(kept.len() == 1 && kept[0].starts_with("lightcraft-ui.json.corrupt-"), "{kept:?}");
        // a missing file is just "no settings yet"
        assert_eq!(load_prefs_at(&path, true).1, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `--demo` reads the settings but changes nothing: a damaged file is reported and left where it is.
    #[test]
    fn a_demo_window_leaves_a_damaged_file_alone() {
        let d = dir("damaged-demo");
        let path = d.join(FILE);
        let bytes = br#"{"settings": {"library_path": "/Volumes/Photos/Lib"#;
        std::fs::write(&path, bytes).unwrap();
        let (ui, warning, keep) = load_prefs_at(&path, false);
        assert!(ui.is_none() && keep);
        assert!(warning.unwrap().contains("speichert in dieser Sitzung nichts"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_docked_layout_survives_a_restart() {
        use lightcraft_ui_egui::docking::Panel;
        let d = dir("docking");
        let path = d.join(FILE);
        let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
        app.run("ui.dock", serde_json::json!({"operation":"float", "panel":"tools", "rect":[70,80,310,500]})).unwrap();
        app.run("ui.dock", serde_json::json!({"operation":"close", "panel":"tools"})).unwrap();
        let mut writer = PrefsWriter { path: Some(path.clone()), ..Default::default() };
        writer.save_ui(&app.ui, false).unwrap();
        let (restored, warning, keep) = load_prefs_at(&path, false);
        assert!(warning.is_none() && !keep);
        let mut restored_app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
        restored_app.ui = restored.unwrap();
        restored_app.run("ui.dock", serde_json::json!({"operation":"open", "panel":"tools"})).unwrap();
        assert_eq!(
            restored_app.ui.docking.as_ref().unwrap().layout.floating.iter().find(|g| g.panels.contains(&Panel::Tools)).unwrap().rect,
            [70.0, 80.0, 310.0, 500.0]
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    /// A layout from a newer LightCraft (or an invalid one) is dropped; the rest loads and the
    /// file is left exactly as it was.
    #[test]
    fn an_unknown_layout_keeps_the_other_settings_and_the_file() {
        let d = dir("future-docking");
        let path = d.join(FILE);
        let mut ui = UiState { thumb_size: 210.0, filmstrip: false, ..Default::default() };
        ui.settings.library_path = "/Volumes/Photos/Synthetic Library".into();
        for docking in [serde_json::json!({"layout":{"root":{"FutureLayout":{}}}}), serde_json::json!(false)] {
            let mut saved = serde_json::to_value(&ui).unwrap();
            saved["docking"] = docking;
            let bytes = serde_json::to_vec(&saved).unwrap();
            for set_aside in [false, true] {
                std::fs::write(&path, &bytes).unwrap();
                let (loaded, warning, keep) = load_prefs_at(&path, set_aside);
                let loaded = loaded.unwrap();
                assert_eq!(loaded.settings.library_path, ui.settings.library_path);
                assert_eq!(loaded.thumb_size, ui.thumb_size);
                assert!(!loaded.filmstrip && loaded.docking.is_none());
                assert!(warning.unwrap().contains("übrigen Einstellungen sind geladen"));
                assert!(keep);
                let mut writer = PrefsWriter::new(Some(path.clone()), &UiState::default(), keep);
                writer.save_ui(&loaded, false).unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
                assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "the settings file is not renamed or replaced");
            }
        }
        std::fs::remove_dir_all(d).unwrap();
    }

    /// The writer saves changed settings to its file, and nothing at all without one (`--demo`)
    /// or while the session is the temporary one offered when the library can't be opened.
    #[test]
    fn the_writer_writes_nothing_for_a_demo_or_temporary_session() {
        let d = dir("writer");
        let path = d.join("sub").join(FILE);
        let mut ui = UiState::default();
        // an ordinary session: a change is written, the folder created on the way
        let mut w = PrefsWriter::new(Some(path.clone()), &ui, false);
        ui.thumb_size += 50.0;
        w.save_ui(&ui, false).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(String::from_utf8_lossy(&saved).contains("thumbSize"), "{}", String::from_utf8_lossy(&saved));
        // the temporary session (library problem): a further change is not written…
        ui.thumb_size += 50.0;
        w.save_ui(&ui, true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        // …until a library opens
        w.save_ui(&ui, false).unwrap();
        assert_ne!(std::fs::read(&path).unwrap(), saved);
        // --demo: no path, so no file and no folder, whatever changes
        let mut w = PrefsWriter::new(None, &ui, false);
        let before = w.written.clone();
        ui.thumb_size += 50.0;
        w.save_ui(&ui, false).unwrap();
        assert_eq!(w.written, before);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "only the ordinary session's folder");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_file_is_written_atomically() {
        let d = dir("atomic");
        let path = d.join(FILE);
        std::fs::write(&path, b"old").unwrap();
        write_atomic(&path, b"{\"new\": true}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"new\": true}");
        assert!(!d.join("lightcraft-ui.json.tmp").exists());
        // a failed write leaves the old file
        let missing = d.join("gone").join(FILE);
        assert!(write_atomic(&missing, b"x").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"new\": true}");
        write_atomic(&path, &serde_json::to_vec_pretty(&UiState::default()).unwrap()).unwrap();
        let (loaded, warning, _) = load_prefs_at(&path, true);
        assert!(loaded.is_some() && warning.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
