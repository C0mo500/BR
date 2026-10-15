//! BR's keys. Strg+K (⌘K) opens the command search and Strg+Alt+1…6 the pages, on every page.
//! On the video pages FilmCraft's key set applies, which BR starts as "CapCut": Final Cut's keys
//! (A, B, Space, J/K/L, I/O, M, N …) with CapCut's own where they differ: ⌘B splits, ⌫/Entf
//! delete and close the gap, P turns the magnet on and off, ⌘R opens Tempo, ⌘E Exportieren.
//! BR's commands are FilmCraft's to bind (Tastenkürzel bearbeiten lists them, under "BR"); a key
//! bound to one runs it here, before FilmCraft sees the key.

use egui::{Event, Key, Modifiers};
use filmcraft_engine::shortcuts::{Binding, CommandInfo, DEFAULT_PRESET, FCP_PRESET, Platform, Shortcuts};
use filmcraft_ui_egui::FilmcraftApp;

use crate::{BrApp, Page};

/// The key set BR starts with.
pub const CAPCUT: &str = "CapCut";

/// What BR adds to Final Cut's keys: (command, keys). A key listed here is taken from whatever
/// else had it.
pub const CAPCUT_KEYS: &[(&str, &str)] = &[
    ("br.video.split", "Cmd+B"),
    ("br.video.delete", "Backspace"),
    ("br.video.delete", "Delete"),
    ("br.ui.magnet", "P"),
    ("br.ui.popover.speed", "Cmd+R"),
    ("br.ui.popover.export", "Cmd+E"),
    ("br.ui.palette", "Cmd+K"),
];

/// BR's commands that FilmCraft can bind: the ones that need no values, and BR's window actions.
pub fn commands() -> Vec<CommandInfo> {
    let mut v = Vec::new();
    for c in br_video::COMMANDS {
        if crate::palette::needs_values(c.params) || matches!(c.id, "br.video.commands" | "br.video.state" | "br.video.jobs" | "br.video.anchors") {
            continue;
        }
        let title = crate::palette::german(c.id).unwrap_or(c.summary);
        v.push(CommandInfo::new(c.id, title, &["BR"], None));
    }
    for (id, title, _) in crate::palette::BR_UI {
        v.push(CommandInfo::new(id, title, &["BR"], None));
    }
    v
}

/// FilmCraft's own frontend commands and BR's, for [`Shortcuts::register_external`] (which
/// replaces the list).
pub fn register(fc: &mut FilmcraftApp) {
    let mut all = filmcraft_ui_egui::menus::external_commands();
    all.extend(commands());
    fc.session.shortcuts.register_external(all);
}

/// The CapCut key set: Final Cut's with [`CAPCUT_KEYS`]; FilmCraft's ⌘K (Schnitt setzen, the
/// command search's key here) is gone. The Timeline and Program panels' own bindings of those
/// keys (Entf = Löschen ohne Aufschließen, ⌘R = Lineale) give way to CapCut's; Entf in the
/// Project panel deletes project items, as ⌫ does there.
pub fn capcut(sc: &Shortcuts) -> Vec<Binding> {
    let mut b = sc.builtin(FCP_PRESET).unwrap_or_default();
    let p = Platform::current();
    let chord = |k: &str| filmcraft_engine::shortcuts::Chord::parse(k).ok().map(|c| c.effective(p));
    for (command, keys) in CAPCUT_KEYS {
        let Ok(c) = filmcraft_engine::shortcuts::Chord::parse(keys) else { continue };
        let eff = c.effective(p);
        b.retain(|x| {
            let taken = matches!(x.panel.as_deref(), None | Some("Timeline") | Some("Program"));
            !(taken && chord(&x.keys) == Some(eff.clone()))
        });
        b.push(Binding { command: (*command).into(), keys: c.canonical(), panel: None });
    }
    if let Ok(del) = filmcraft_engine::shortcuts::Chord::parse("Delete") {
        let eff = del.effective(p);
        b.retain(|x| !(x.panel.as_deref() == Some("Project") && chord(&x.keys) == Some(eff.clone())));
        b.push(Binding { command: "project.delete".into(), keys: del.canonical(), panel: Some("Project".into()) });
    }
    b
}

/// The file that says BR has started FilmCraft's keys as CapCut once (in FilmCraft's data
/// folder, next to its settings). It holds the bindings BR wrote last (empty: the user's own set
/// was there), so BR brings only its own CapCut set up to date, never one the user changed and
/// saved under that name.
fn installed_marker(fc: &FilmcraftApp) -> Option<std::path::PathBuf> {
    fc.session.prefs_path.as_ref().and_then(|p| p.parent()).map(|d| d.join("br-tasten-capcut.json"))
}

/// Start FilmCraft's key set as CapCut, once: when it is still FilmCraft's unchanged default the
/// first time BR runs (whoever then picks another set keeps it). BR's own CapCut set, unchanged,
/// is brought up to date on every start.
pub fn install(fc: &mut FilmcraftApp) {
    let marker = installed_marker(fc);
    let wrote: Option<String> = marker.as_ref().and_then(|m| std::fs::read_to_string(m).ok());
    let sc = &mut fc.session.shortcuts;
    let json = |b: &[Binding]| serde_json::to_string(b).unwrap_or_default();
    let remember = |content: &str| {
        if let Some(m) = &marker {
            let _ = std::fs::write(m, content);
        }
    };
    let save = |sc: &mut Shortcuts| {
        if sc.save_preset(CAPCUT).is_err() {
            // no data folder (tests, a session in memory): only for now
            sc.preset = CAPCUT.into();
            sc.modified = false;
            sc.revision += 1;
        }
    };
    if sc.preset == CAPCUT && !sc.modified {
        // BR's own (as it wrote it last, or from before BR kept a note): up to date
        let ours = wrote.as_deref().is_none_or(|w| w == json(&sc.bindings));
        if ours {
            let now = capcut(sc);
            if now != sc.bindings {
                sc.bindings = now;
                save(sc);
            }
            remember(&json(&sc.bindings));
        } else if wrote.is_none() {
            remember("");
        }
        return;
    }
    // BR did this before (or had a CapCut preset already): the user's choice stays
    if wrote.is_some() || sc.user_presets().iter().any(|n| n == CAPCUT) {
        if wrote.is_none() {
            remember("");
        }
        return;
    }
    if sc.preset == DEFAULT_PRESET && !sc.modified {
        sc.bindings = capcut(sc);
        save(sc);
        remember(&json(&sc.bindings));
    } else {
        remember("");
    }
}

/// The page Strg+Alt+`n` opens.
fn page_key(k: Key) -> Option<Page> {
    let i = match k {
        Key::Num1 => 0,
        Key::Num2 => 1,
        Key::Num3 => 2,
        Key::Num4 => 3,
        Key::Num5 => 4,
        Key::Num6 => 5,
        _ => return None,
    };
    Page::ALL.get(i).copied()
}

/// The modifiers of a pressed key exactly as a binding names them (egui's own matching lets extra
/// Shift and Alt through, and BR runs before FilmCraft: ⇧⌫ must stay FilmCraft's Lift).
fn same_mods(pressed: Modifiers, wanted: Modifiers) -> bool {
    pressed.command == wanted.command
        && pressed.shift == wanted.shift
        && pressed.alt == wanted.alt
        && (!cfg!(target_os = "macos") || pressed.ctrl == wanted.ctrl)
}

/// What the list keys of the command search did this frame.
#[derive(Default, Clone, Copy, Debug)]
pub struct Nav {
    pub down: bool,
    pub up: bool,
    pub enter: bool,
    pub shift_enter: bool,
    pub escape: bool,
}

/// Before the apps see this frame's keys: BR's own (all pages), the command search's list keys
/// while it is open, and FilmCraft's keys bound to BR's commands (video pages).
pub fn handle(app: &mut BrApp, ctx: &egui::Context) {
    let mut actions: Vec<String> = Vec::new();
    let mut nav = Nav::default();
    let open = app.palette.open;
    // FilmCraft's bindings of BR's commands, as (modifiers, key, command)
    let video = app.page.is_video() && app.fc.dialog.is_none();
    // text being typed: a field, or a title typed on the Program monitor (FilmCraft's own)
    let typing = ctx.text_edit_focused() || ctx.memory(|m| m.has_focus(egui::Id::new("gfx-text-edit")));
    // FilmCraft's shortcut editor records the next key
    let recording = app.page.is_video() && app.fc.shortcut_editor.recording.is_some();
    let bound: Vec<(Modifiers, Key, String)> = if video && !open {
        let focused = app.fc.ui.focused.title();
        let sc = &app.fc.session.shortcuts;
        sc.bindings
            .iter()
            .filter(|b| b.command.starts_with("br.") && b.panel.is_none())
            // a key the focused panel binds itself stays the panel's (Entf in the Project panel)
            .filter(|b| {
                filmcraft_engine::shortcuts::Chord::parse(&b.keys)
                    .is_ok_and(|c| sc.resolve(&c, Some(focused), Platform::current()).is_some_and(|r| r.command == b.command))
            })
            .filter_map(|b| filmcraft_ui_egui::menus::parse_shortcut(&b.keys).map(|(m, k)| (m, k, b.command.clone())))
            .collect()
    } else {
        Vec::new()
    };
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            let Event::Key { key, pressed: true, modifiers, .. } = e else { return true };
            let (key, m) = (*key, *modifiers);
            // Strg+K: the command search (also to close it)
            if !recording && key == Key::K && m.command && !m.shift && !m.alt {
                actions.push("br.ui.palette".into());
                return false;
            }
            if !recording
                && m.command
                && m.alt
                && !m.shift
                && let Some(p) = page_key(key)
            {
                actions.push(format!("br.ui.page.{}", p.key()));
                return false;
            }
            if open {
                let plain = !m.command && !m.alt;
                match key {
                    Key::ArrowDown if plain => nav.down = true,
                    Key::ArrowUp if plain => nav.up = true,
                    Key::Enter if plain && m.shift => nav.shift_enter = true,
                    Key::Enter if plain => nav.enter = true,
                    Key::Escape => nav.escape = true,
                    _ => return true,
                }
                return false;
            }
            // a field being typed in keeps its keys, chords too (⌘B in a title's text is not
            // Teilen), as FilmCraft keeps its own shortcuts away from a focused field
            if typing || recording {
                return true;
            }
            if let Some((_, _, id)) = bound.iter().find(|(bm, bk, _)| *bk == key && same_mods(m, *bm)) {
                actions.push(id.clone());
                return false;
            }
            true
        });
    });
    app.palette.nav = nav;
    for id in actions {
        run(app, ctx, &id);
    }
}

/// Run what a key is bound to.
fn run(app: &mut BrApp, ctx: &egui::Context, id: &str) {
    if id.starts_with("br.ui.") {
        if let Err(e) = crate::palette::ui_action(app, ctx, id, true) {
            crate::cut::toast(app, ctx, crate::cut::readable(&e));
        }
        return;
    }
    let r = crate::cut::run(app, ctx, id, serde_json::json!({}));
    // what a key did, where there is nothing to see it by
    if r.is_some() && id == "br.video.detachAudio" {
        crate::cut::toast(app, ctx, "Ton getrennt");
    }
}
