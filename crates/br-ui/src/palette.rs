//! The command search (Ctrl+K): every command of BR, FilmCraft and LightCraft in one list, found
//! by its German or English name, its id, its menu or a CapCut word ("teilen", "Standbild"), run
//! from there. A command that needs values gets a small form (one field per parameter, or JSON).
//! The list shows each command's key, so it is also where one learns the keys.

use egui::{CornerRadius, RichText, Stroke, vec2};
use serde_json::{Value, json};

use crate::style::{ACCENT, Theme};
use crate::{BrApp, Page};

/// Where a command comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// FilmCraft's engine (video).
    Film,
    /// FilmCraft's window (panels, views, dialogs).
    FilmUi,
    /// LightCraft's engine (photo).
    Photo,
    /// LightCraft's window.
    PhotoUi,
    /// BR's video and photo commands.
    Br,
    /// BR's window (pages, popovers, columns).
    BrUi,
}

impl Source {
    fn app(self) -> &'static str {
        match self {
            Source::Film | Source::FilmUi => "Video",
            Source::Photo | Source::PhotoUi => "Foto",
            Source::Br | Source::BrUi => "BR",
        }
    }
}

/// One command in the list.
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub source: Source,
    /// The German name (the English one where there is none).
    pub title: String,
    pub title_en: String,
    /// Where it is in the menus, or what it belongs to.
    pub path: String,
    /// Its parameters as the command describes them.
    pub params: String,
    /// It cannot run without values (it has no dialog of its own).
    pub needs_params: bool,
    /// Its key, if it has one.
    pub key: Option<String>,
    /// Words it is also found by.
    synonyms: &'static str,
}

/// What the command search remembers.
#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    query: String,
    selected: usize,
    /// The command whose values are being filled in, and the values (as typed).
    form: Option<(Entry, Vec<(String, String)>)>,
    /// The commands run last, newest first.
    recent: Vec<String>,
    /// Focus the search field on the next frame.
    focus: bool,
    /// The list keys of this frame ([`crate::keys::handle`] takes them before the apps do).
    pub nav: crate::keys::Nav,
    /// Where the search was drawn (a click elsewhere closes it).
    rect: Option<egui::Rect>,
}

impl PaletteState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.form = None;
        self.focus = self.open;
        if self.open {
            self.query.clear();
            self.selected = 0;
        }
    }
}

/// German names (and CapCut's words) for commands whose German name FilmCraft lacks, and for
/// BR's own.
fn overlay(id: &str) -> Option<(&'static str, &'static str)> {
    Some(match id {
        "br.video.new" => ("Neues Video", "projekt anlegen format"),
        "br.video.add" => ("Medien hinzufügen", "importieren clip video foto"),
        "br.video.place" => ("Medien platzieren", "einfügen overlay ablegen"),
        "br.video.split" => ("Teilen", "schneiden splitten trennen razor cut"),
        "br.video.delete" => ("Löschen", "entfernen ripple"),
        "br.video.speed" => ("Tempo", "geschwindigkeit schneller langsamer speed"),
        "br.video.move" => ("Clip verschieben", "reihenfolge umsortieren"),
        "br.video.trim" => ("Kante trimmen", "kürzen verlängern"),
        "br.video.duplicate" => ("Duplizieren", "kopieren doppeln"),
        "br.video.pack" => ("Lücken schließen", "zusammenschieben"),
        "br.video.magnet" => ("Magnetische Hauptspur", "magnet lücken"),
        "br.video.linkage" => ("Verknüpfung", "anker folgen linkage"),
        "br.video.anchors" => ("Anker anzeigen", "verankert"),
        "br.video.anchor" => ("Anker setzen", "verankern lösen"),
        "br.video.detachAudio" => ("Ton trennen", "audio abtrennen extrahieren"),
        "br.video.text" => ("Text hinzufügen", "titel schrift"),
        "br.video.captions" => ("Untertitel aus Datei", "srt vtt"),
        "br.video.autoCaptions" => ("Automatische Untertitel", "spracherkennung whisper transkript"),
        "br.video.music" => ("Musik hinzufügen", "audio song ducking"),
        "br.video.filter" => ("Filter", "look lut farbe"),
        "br.video.format" => ("Format", "seitenverhältnis 9:16 16:9 hochformat querformat"),
        "br.video.transition" => ("Übergang", "blende überblendung transition"),
        "br.video.export" => ("Video exportieren", "rendern speichern mp4"),
        "br.video.inspect" => ("Inspector lesen", "eigenschaften werte"),
        "br.video.set" => ("Wert setzen", "eigenschaft parameter"),
        "br.video.keyframe" => ("Keyframe", "animation raute"),
        "br.video.effect" => ("Effekt", "hinzufügen entfernen"),
        "br.video.schema" => ("Effekt-Schemata", "parameter liste"),
        "br.video.transform" => ("Transformieren", "position skalieren drehen größe"),
        "br.video.fit" => ("Einpassen / Füllen", "fit fill"),
        "br.video.background" => ("Hintergrund füllen", "canvas unschärfe blur farbe"),
        "br.video.reframe" => ("Motiv folgen", "auto reframe zuschneiden"),
        "br.video.curve" => ("Tempo-Kurve", "kurve montage bullet speed ramp"),
        "br.video.freeze" => ("Einfrieren", "standbild freeze still"),
        "br.video.reverse" => ("Rückwärts", "umkehren reverse"),
        "br.video.beats" => ("Beats markieren", "takt rhythmus beat marker musik"),
        "br.video.template.list" => ("Vorlagen", "template"),
        "br.video.template.new" => ("Video aus Vorlage", "template platzhalter"),
        "br.video.template.fill" => ("Vorlage füllen", "medien einsetzen template"),
        "br.video.template.save" => ("Als Vorlage speichern", "template"),
        "br.video.state" => ("Video-Zustand", "status"),
        "br.video.jobs" => ("Exporte und Aufträge", "fortschritt"),
        "br.video.cancel" => ("Export abbrechen", "stop"),
        "br.video.commands" => ("FilmCraft-Befehle", "liste"),
        "sequence.addEdit" => ("Schnitt setzen (FilmCraft)", "teilen splitten razor"),
        "clip.frameHold" => ("Standbild (ganzer Clip)", "einfrieren freeze"),
        "clip.insertFrameHoldSegment" => ("Standbild-Stück einfügen", "einfrieren freeze segment"),
        "clip.nest" => ("Verschachteln", "verbund compound"),
        "clip.speedDuration" => ("Geschwindigkeit/Dauer…", "tempo rückwärts"),
        "edit.undo" => ("Rückgängig", "undo zurück"),
        "edit.redo" => ("Wiederholen", "redo"),
        "app.keyboardShortcuts" => ("Tastenkürzel bearbeiten", "tasten shortcuts"),
        "br.state" => ("Foto-Zustand", "werte status"),
        "br.looks" => ("Looks", "filter liste"),
        "br.look" => ("Look anwenden", "filter preset stärke"),
        "br.adjust" => ("Anpassen", "licht kontrast farbe wärme schärfe vignette"),
        "br.auto" => ("Auto", "automatisch belichtung"),
        "br.crop" => ("Foto-Format", "zuschneiden seitenverhältnis 9:16 4:5"),
        "br.export" => ("Foto exportieren", "speichern jpg instagram story"),
        "br.reset" => ("Alles zurücksetzen", "original"),
        "br.undo" => ("Rückgängig (Foto)", "undo"),
        "br.redo" => ("Wiederholen (Foto)", "redo"),
        _ => return None,
    })
}

/// The German name of one of BR's commands.
pub(crate) fn german(id: &str) -> Option<&'static str> {
    overlay(id).map(|o| o.0)
}

/// BR's window actions: (id, title, words).
pub(crate) const BR_UI: &[(&str, &str, &str)] = &[
    ("br.ui.palette", "Befehlssuche", "suchen ctrl k"),
    ("br.ui.popover.captions", "Untertitel …", "popover"),
    ("br.ui.popover.filter", "Filter …", "look popover"),
    ("br.ui.popover.speed", "Tempo …", "geschwindigkeit kurve popover"),
    ("br.ui.popover.format", "Format …", "seitenverhältnis hintergrund popover"),
    ("br.ui.popover.export", "Exportieren …", "speichern popover"),
    ("br.ui.inspector.basis", "Inspector: Basis", "eigenschaften"),
    ("br.ui.inspector.mehr", "Inspector: Mehr", "alle effekte parameter"),
    ("br.ui.inspector.voll", "Inspector: Voll", "effekteinstellungen filmcraft"),
    ("br.ui.timeline.profi", "Timeline: Profi", "filmcraft timeline premiere"),
    ("br.ui.timeline.zoomIn", "Timeline: näher heran", "zoom vergrößern"),
    ("br.ui.timeline.zoomOut", "Timeline: weiter weg", "zoom verkleinern"),
    ("br.ui.timeline.zoomFit", "Timeline: ganzes Video", "zoom einpassen"),
    ("br.ui.start.skip", "Ohne Video weiter", "startbildschirm leerer editor"),
    ("br.ui.captions.edit", "Untertitel bearbeiten (Text-Panel)", "untertitel liste transkript"),
    ("br.ui.export.quick", "Schnell-Export (FilmCraft)", "export vorlagen"),
    ("br.ui.photo.looks", "Foto: Looks …", "filter popover"),
    ("br.ui.photo.format", "Foto: Format …", "zuschneiden popover"),
    ("br.ui.photo.export", "Foto: Exportieren …", "speichern popover"),
    ("br.ui.magnet", "Magnet an/aus", "magnetische hauptspur lücken"),
    ("br.ui.linkage", "Verknüpfung an/aus", "anker folgen"),
    ("br.ui.assets.media", "Asset-Leiste: Medien", ""),
    ("br.ui.assets.audio", "Asset-Leiste: Audio", "musik"),
    ("br.ui.assets.text", "Asset-Leiste: Text", "titel vorlagen"),
    ("br.ui.assets.effects", "Asset-Leiste: Effekte", ""),
    ("br.ui.assets.transitions", "Asset-Leiste: Übergänge", ""),
    ("br.ui.assets.looks", "Asset-Leiste: Filter", ""),
    ("br.ui.assets.templates", "Asset-Leiste: Vorlagen", "template"),
    ("br.ui.assets.profi", "Asset-Leiste: Profi", "projekt filmcraft"),
];

/// The parameters a description names: (key, type, optional), from `{"key":type?,…}` (FilmCraft,
/// LightCraft, BR's video) or `{key?: type, …} — text` (BR's photo commands).
fn param_keys(params: &str) -> Vec<(String, String, bool)> {
    let head = params.split(" — ").next().unwrap_or("");
    let Some(open) = head.find('{') else { return Vec::new() };
    // the body up to the matching brace
    let mut depth = 0i32;
    let mut end = head.len();
    for (k, c) in head[open..].char_indices() {
        match c {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    end = open + k;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &head[open + 1..end.max(open + 1)];
    // its fields at the top level
    let mut fields = vec![String::new()];
    let mut depth = 0i32;
    let mut quoted = false;
    for c in body.chars() {
        match c {
            '"' => quoted = !quoted,
            '[' | '{' | '(' if !quoted => depth += 1,
            ']' | '}' | ')' if !quoted => depth -= 1,
            ',' if depth == 0 && !quoted => {
                fields.push(String::new());
                continue;
            }
            _ => {}
        }
        if let Some(f) = fields.last_mut() {
            f.push(c);
        }
    }
    let mut out = Vec::new();
    for f in fields {
        let Some((key, ty)) = f.split_once(':') else { continue };
        let key = key.trim().trim_matches('"');
        let (key, optional_key) = match key.strip_suffix('?') {
            Some(k) => (k, true),
            None => (key, false),
        };
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let ty = ty.trim();
        let optional = optional_key || ty.contains('?') || ty.contains('=') || ty.contains("default");
        out.push((key.to_string(), ty.to_string(), optional));
    }
    out
}

/// A command that cannot run without values (it has no dialog of its own).
pub(crate) fn needs_values(params: &str) -> bool {
    param_keys(params).iter().any(|p| !p.2)
}

/// Every command, built fresh (about 1 500; the list is built when the search opens).
pub fn index(app: &BrApp) -> Vec<Entry> {
    let mut v = Vec::new();
    // BR first: its commands are the CapCut ones
    for c in br_video::COMMANDS {
        let (title, synonyms) = overlay(c.id).unwrap_or((c.summary, ""));
        v.push(Entry {
            id: c.id.into(),
            source: Source::Br,
            title: title.into(),
            title_en: c.summary.into(),
            path: "Video".into(),
            params: c.params.into(),
            needs_params: needs_values(c.params),
            key: None,
            synonyms,
        });
    }
    for (id, label, desc) in br_photo::commands::COMMANDS {
        let (title, synonyms) = overlay(id).unwrap_or((label.trim_end_matches(" (BR)"), ""));
        v.push(Entry {
            id: (*id).into(),
            source: Source::Br,
            title: title.into(),
            title_en: label.trim_end_matches(" (BR)").into(),
            path: "Foto".into(),
            params: (*desc).into(),
            needs_params: needs_values(desc),
            key: None,
            synonyms,
        });
    }
    for (id, title, synonyms) in BR_UI {
        v.push(Entry {
            id: (*id).into(),
            source: Source::BrUi,
            title: (*title).into(),
            title_en: String::new(),
            path: "Fenster".into(),
            params: String::new(),
            needs_params: false,
            key: None,
            synonyms,
        });
    }
    for p in Page::ALL {
        v.push(Entry {
            id: format!("br.ui.page.{}", p.key()),
            source: Source::BrUi,
            title: format!("Seite: {}", p.label()),
            title_en: String::new(),
            path: "Seiten".into(),
            params: String::new(),
            needs_params: false,
            key: None,
            synonyms: "",
        });
    }
    // FilmCraft: its engine, then its window's own commands
    let shortcuts = &app.fc.session.shortcuts;
    let lang = app.fc.ui.language;
    let de = |s: &str| lang.tr(s).to_string();
    for c in filmcraft_engine::command_specs() {
        let (title, synonyms) = overlay(c.id).map_or((de(c.label), ""), |(t, s)| (t.to_string(), s));
        let in_menu = !c.menu.is_empty();
        v.push(Entry {
            id: c.id.into(),
            source: Source::Film,
            title,
            title_en: c.label.into(),
            path: if in_menu { c.menu.iter().map(|m| de(m)).collect::<Vec<_>>().join(" › ") } else { "FilmCraft".into() },
            params: c.params.into(),
            needs_params: !in_menu && c.shortcut.is_none() && needs_values(c.params),
            key: shortcuts.primary(c.id).map(|k| filmcraft_ui_egui::menus::shortcut_text(&k)),
            synonyms,
        });
    }
    for c in filmcraft_ui_egui::menus::external_commands() {
        if v.iter().any(|e| e.id == c.id) {
            continue;
        }
        let (title, synonyms) = overlay(&c.id).map_or((de(&c.label), ""), |(t, s)| (t.to_string(), s));
        v.push(Entry {
            key: shortcuts.primary(&c.id).map(|k| filmcraft_ui_egui::menus::shortcut_text(&k)),
            id: c.id,
            source: Source::FilmUi,
            title,
            title_en: c.label,
            path: if c.menu.is_empty() { de(&c.category) } else { c.menu.iter().map(|m| de(m)).collect::<Vec<_>>().join(" › ") },
            params: String::new(),
            needs_params: false,
            synonyms,
        });
    }
    // LightCraft
    let lde = |s: &str| lightcraft_ui_egui::i18n::Locale::De.tr(s).to_string();
    for c in lightcraft_engine::command_specs() {
        let in_menu = !c.menu.is_empty();
        v.push(Entry {
            id: c.id.into(),
            source: Source::Photo,
            title: lde(c.label),
            title_en: c.label.into(),
            path: if in_menu { c.menu.iter().map(|m| lde(m)).collect::<Vec<_>>().join(" › ") } else { "LightCraft".into() },
            params: c.params.into(),
            needs_params: !in_menu && c.shortcut.is_none() && needs_values(c.params),
            key: c.shortcut.map(str::to_string),
            synonyms: "",
        });
    }
    for &(id, label, key, menu) in lightcraft_ui_egui::menus::ui_commands() {
        if v.iter().any(|e| e.id == id && e.source == Source::Photo) {
            continue;
        }
        v.push(Entry {
            id: id.into(),
            source: Source::PhotoUi,
            title: lde(label),
            title_en: label.into(),
            path: menu.split('>').map(lde).collect::<Vec<_>>().join(" › "),
            params: String::new(),
            needs_params: false,
            key: key.map(str::to_string),
            synonyms: "",
        });
    }
    v
}

/// Lowercase without accents and punctuation (ä → a, ß → ss), for matching.
fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push('a'),
            'ö' => out.push('o'),
            'ü' => out.push('u'),
            'ß' => out.push_str("ss"),
            'é' | 'è' => out.push('e'),
            c if c.is_alphanumeric() || c == '.' || c == ':' => out.push(c),
            _ => out.push(' '),
        }
    }
    out
}

/// How well one search word matches a field: a word's start 100, the initials 70, inside 50,
/// its letters in order 20.
fn word_score(field: &str, w: &str) -> f64 {
    if w.is_empty() {
        return 0.0;
    }
    let words: Vec<&str> = field.split([' ', '.']).filter(|x| !x.is_empty()).collect();
    if words.iter().any(|x| x.starts_with(w)) {
        return 100.0;
    }
    let initials: String = words.iter().filter_map(|x| x.chars().next()).collect();
    if w.len() >= 2 && initials.starts_with(w) {
        return 70.0;
    }
    if field.contains(w) {
        return 50.0;
    }
    // its letters in order inside one word ("dplz" → duplizieren)
    if w.len() >= 3
        && words.iter().any(|x| {
            let mut it = x.chars();
            x.starts_with(w.chars().next().unwrap_or(' ')) && w.chars().all(|c| it.any(|f| f == c))
        })
    {
        return 20.0;
    }
    0.0
}

/// The commands for `query`, best first.
pub fn search<'a>(entries: &'a [Entry], query: &str, recent: &[String], video: bool) -> Vec<&'a Entry> {
    let q = query.trim();
    let exact = entries.iter().filter(|e| e.id == q);
    let words: Vec<String> = norm(q).split_whitespace().map(str::to_string).collect();
    let mut scored: Vec<(f64, usize, &Entry)> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.id != q)
        .filter_map(|(i, e)| {
            let fields = [
                (norm(&e.title), 1.0),
                (norm(&e.title_en), 0.9),
                (norm(e.synonyms), 0.9),
                (norm(&e.id), 0.8),
                (norm(&e.path), 0.5),
                (norm(e.key.as_deref().unwrap_or("")), 1.0),
            ];
            let mut total = 0.0;
            for w in &words {
                let best = fields.iter().map(|(f, wt)| word_score(f, w) * wt).fold(0.0, f64::max);
                if best <= 0.0 {
                    return None;
                }
                total += best;
            }
            // the page's own app, BR's commands, what was used lately
            let video_cmd = match e.source {
                Source::Film | Source::FilmUi => true,
                Source::Br => br_video::is_video(&e.id),
                Source::BrUi => !e.id.starts_with("br.ui.page."),
                Source::Photo | Source::PhotoUi => false,
            };
            let page = video == video_cmd;
            total += if page { 15.0 } else { 0.0 } + if matches!(e.source, Source::Br | Source::BrUi) { 12.0 } else { 0.0 };
            if let Some(r) = recent.iter().position(|x| *x == e.id) {
                total += 30.0 - r as f64 * 3.0;
            }
            Some((total, i, e))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    exact.chain(scored.into_iter().map(|x| x.2)).collect()
}

/// Run a command from the list with `params`; a failure is told in the toast.
pub fn run(app: &mut BrApp, ctx: &egui::Context, e: &Entry, params: Value) {
    if let Err(err) = execute(app, ctx, e, params) {
        crate::cut::toast(app, ctx, crate::cut::readable(&err));
    }
}

/// Run a command from the list with `params` on the page of its app.
pub fn execute(app: &mut BrApp, ctx: &egui::Context, e: &Entry, params: Value) -> Result<Value, String> {
    app.palette.recent.retain(|x| *x != e.id);
    app.palette.recent.insert(0, e.id.clone());
    app.palette.recent.truncate(20);
    match e.source {
        Source::Film | Source::FilmUi => {
            if !app.page.is_video() {
                app.open(Page::Schnitt);
            }
            filmcraft_ui_egui::menus::invoke(&mut app.fc, ctx, &e.id, params)
        }
        Source::Photo | Source::PhotoUi => {
            app.open(Page::Foto);
            app.lc.run(&e.id, params)
        }
        Source::Br if br_video::is_video(&e.id) => {
            if !app.page.is_video() {
                app.open(Page::Schnitt);
            }
            br_video::execute_with(&mut app.fc.session, &mut app.video.watch, &e.id, &params)
        }
        Source::Br => {
            app.open(Page::Foto);
            crate::photo::finish_drags(app);
            br_photo::commands::execute(&mut app.lc.session, &mut app.photo.br, &e.id, &params).map_err(|e| e.to_string())
        }
        Source::BrUi => ui_action(app, ctx, &e.id, false),
    }
}

/// `br.palette.run` (control channel): run a command by its id as the list does.
pub fn run_id(app: &mut BrApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {
    let e = index(app).into_iter().find(|e| e.id == id).ok_or_else(|| format!("no command `{id}`"))?;
    execute(app, ctx, &e, params)
}

/// BR's window actions (`br.ui.*`).
/// `toggle`: a second press closes what the first opened (keys); the list only opens.
pub fn ui_action(app: &mut BrApp, ctx: &egui::Context, id: &str, toggle: bool) -> Result<Value, String> {
    let Some(rest) = id.strip_prefix("br.ui.") else { return Err(format!("unknown action `{id}`")) };
    if rest == "palette" {
        app.palette.toggle();
        return Ok(Value::Null);
    }
    if let Some(k) = rest.strip_prefix("page.") {
        let page = Page::ALL.into_iter().find(|p| p.key() == k).ok_or_else(|| format!("unknown page `{k}`"))?;
        app.open(page);
        return Ok(json!({"page": k}));
    }
    if let Some(k) = rest.strip_prefix("photo.") {
        app.open(Page::Foto);
        return crate::photo::open_popover(app, k, toggle).then_some(json!({"popover": k})).ok_or_else(|| format!("no popover `{k}` here"));
    }
    if !app.page.is_video() {
        app.open(Page::Schnitt);
    }
    if rest == "captions.edit" {
        app.cut.open = None;
        app.fc.show_panel(filmcraft_ui_egui::dock::PanelKind::Text);
        return Ok(json!({"panel": "text"}));
    }
    if rest == "export.quick" {
        app.cut.open = None;
        app.fc.ui.export.quick_open = true;
        return Ok(json!({"quickExport": true}));
    }
    if rest == "start.skip" {
        app.cut.start_dismissed = true;
        return Ok(json!({"start": false}));
    }
    if let Some(z) = rest.strip_prefix("timeline.zoom") {
        let factor = match z {
            "In" => 1.5,
            "Out" => 1.0 / 1.5,
            "Fit" => 0.0,
            _ => return Err(format!("unknown action `{id}`")),
        };
        crate::timeline::zoom_by(app, factor);
        return Ok(json!({"zoom": z}));
    }
    if let Some(k) = rest.strip_prefix("popover.") {
        return crate::cut::open_popover(app, k, toggle).then_some(json!({"popover": k})).ok_or_else(|| format!("no popover `{k}` here"));
    }
    if let Some(l) = rest.strip_prefix("inspector.") {
        return crate::inspector::control(app, &json!({"level": l}));
    }
    if let Some(t) = rest.strip_prefix("assets.") {
        return crate::assets::control(app, &json!({"tab": t}));
    }
    if rest == "timeline.profi" {
        app.timeline.full = !app.timeline.full;
        return Ok(json!({"profi": app.timeline.full}));
    }
    if rest == "magnet" || rest == "linkage" {
        let settings = app.fc.session.active_sequence().map(br_video::main_track::settings).ok_or("noch kein Video")?;
        let on = !if rest == "magnet" { settings.magnet } else { settings.linkage };
        crate::cut::run(app, ctx, &format!("br.video.{rest}"), json!({"on": on}));
        crate::cut::toast(
            app,
            ctx,
            match (rest, on) {
                ("magnet", true) => "Magnet an",
                ("magnet", false) => "Magnet aus",
                (_, true) => "Verknüpfung an",
                _ => "Verknüpfung aus",
            },
        );
        return Ok(json!({rest: on}));
    }
    Err(format!("unknown action `{id}`"))
}

/// Draw the search when it is open.
pub fn show(app: &mut BrApp, ctx: &egui::Context) {
    if !app.palette.open {
        return;
    }
    let t = Theme::get(ctx);
    let entries = index(app);
    let video = app.page.is_video();
    let screen = ctx.content_rect();
    let width = 600.0_f32.min(screen.width() - 32.0);
    let pos = egui::pos2(screen.center().x - width / 2.0, screen.top() + 70.0);
    // the keys of the list (taken before the apps saw them)
    let crate::keys::Nav { down, up, enter, shift_enter, escape: esc } = std::mem::take(&mut app.palette.nav);
    // a click beside it closes it
    let clicked_away =
        ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| app.palette.rect.is_some_and(|r| !r.contains(p))));
    if clicked_away {
        app.palette.open = false;
        app.palette.form = None;
        return;
    }
    if esc {
        if app.palette.form.is_some() {
            app.palette.form = None;
        } else {
            app.palette.open = false;
        }
        return;
    }
    let mut run_now: Option<(Entry, Value)> = None;
    let mut to_form: Option<Entry> = None;
    let shown = egui::Area::new(egui::Id::new("br_palette")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(t.bar).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(width);
            if let Some((e, values)) = app.palette.form.clone() {
                if let Some(v) = form(app, ui, &t, &e, values, enter) {
                    run_now = Some((e, v));
                }
                return;
            }
            let mut q = app.palette.query.clone();
            let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Befehl suchen – Name, Wort oder Kennung").desired_width(f32::INFINITY));
            app.widgets.insert("palette.search".into(), r.rect);
            if app.palette.focus {
                r.request_focus();
                app.palette.focus = false;
            }
            if q != app.palette.query {
                app.palette.selected = 0;
            }
            app.palette.query = q.clone();
            let found = search(&entries, &q, &app.palette.recent, video);
            let n = found.len().min(50);
            if down && n > 0 {
                app.palette.selected = (app.palette.selected + 1).min(n - 1);
            }
            if up {
                app.palette.selected = app.palette.selected.saturating_sub(1);
            }
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (i, e) in found.iter().take(n).enumerate() {
                    let on = i == app.palette.selected;
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), egui::Sense::click());
                    if on || r.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
                    }
                    if on {
                        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
                        r.scroll_to_me(None);
                    }
                    let p = ui.painter();
                    p.text(rect.min + vec2(10.0, 5.0), egui::Align2::LEFT_TOP, &e.title, egui::FontId::proportional(13.5), t.text);
                    let sub = format!(
                        "{} · {}{}",
                        e.source.app(),
                        e.path,
                        if e.title_en != e.title && !e.title_en.is_empty() && e.title_en.len() < 50 {
                            format!(" · {}", e.title_en)
                        } else {
                            String::new()
                        }
                    );
                    let g = crate::style::elided(ui, &sub, 11.0, t.dim, rect.width() - 140.0);
                    ui.painter().galley(rect.min + vec2(10.0, 23.0), g, t.dim);
                    if let Some(k) = &e.key {
                        ui.painter().text(rect.right_center() - vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, k, egui::FontId::monospace(11.5), t.dim);
                    } else if e.needs_params {
                        ui.painter().text(
                            rect.right_center() - vec2(10.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            "…",
                            egui::FontId::proportional(13.0),
                            t.dim,
                        );
                    }
                    app.widgets.insert(format!("palette.row.{i}"), rect);
                    app.widgets.insert(format!("palette.cmd.{}", e.id), rect);
                    if r.clicked() {
                        app.palette.selected = i;
                        if e.needs_params { to_form = Some((*e).clone()) } else { run_now = Some(((*e).clone(), json!({}))) }
                    }
                }
                if n == 0 {
                    ui.label(RichText::new("Nichts gefunden").color(t.dim));
                }
            });
            if let Some(e) = found.get(app.palette.selected) {
                if shift_enter || (enter && e.needs_params) {
                    to_form = Some((*e).clone());
                } else if enter {
                    run_now = Some(((*e).clone(), json!({})));
                }
            }
        });
    });
    app.palette.rect = Some(shown.response.rect);
    app.widgets.insert("palette".into(), shown.response.rect);
    if let Some(e) = to_form {
        // the values it must have first
        let mut keys = param_keys(&e.params);
        keys.sort_by_key(|k| k.2);
        let values = keys.into_iter().map(|(k, _, _)| (k, String::new())).collect();
        app.palette.form = Some((e, values));
        return;
    }
    if let Some((e, params)) = run_now {
        app.palette.open = false;
        app.palette.form = None;
        run(app, ctx, &e, params);
    }
}

/// The values a command needs: one field per parameter (numbers, text, JSON).
fn form(app: &mut BrApp, ui: &mut egui::Ui, t: &Theme, e: &Entry, mut values: Vec<(String, String)>, enter: bool) -> Option<Value> {
    ui.label(RichText::new(&e.title).size(15.0).strong().color(t.text));
    ui.label(RichText::new(&e.id).monospace().size(11.0).color(t.dim));
    ui.add_space(6.0);
    let kinds: Vec<(String, String, bool)> = param_keys(&e.params);
    egui::Grid::new("br_palette_form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        for (i, (k, v)) in values.iter_mut().enumerate() {
            let (ty, optional) = kinds.iter().find(|x| x.0 == *k).map_or(("", true), |x| (x.1.as_str(), x.2));
            ui.label(RichText::new(k.as_str()).color(if optional { t.dim } else { t.text })).on_hover_text(ty);
            let r = ui.add(egui::TextEdit::singleline(v).hint_text(ty).desired_width(380.0));
            app.widgets.insert(format!("palette.field.{k}"), r.rect);
            if i == 0 && !r.has_focus() && v.is_empty() && !ui.ctx().memory(|m| m.focused().is_some()) {
                r.request_focus();
            }
            ui.end_row();
        }
    });
    app.palette.form = Some((e.clone(), values.clone()));
    ui.add_space(8.0);
    let mut go = enter;
    ui.horizontal(|ui| {
        let r = crate::style::button(ui, t, "Ausführen", true, true);
        app.widgets.insert("palette.run".into(), r.rect);
        go |= r.clicked();
        let r = crate::style::button(ui, t, "Zurück", false, true);
        app.widgets.insert("palette.back".into(), r.rect);
        if r.clicked() {
            app.palette.form = None;
        }
    });
    if !go {
        return None;
    }
    // a value is JSON where it reads as JSON (numbers, lists, true), else text
    let mut p = serde_json::Map::new();
    for (k, v) in values {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        p.insert(k, serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_string())));
    }
    Some(Value::Object(p))
}

/// `br.ui.palette.search` (control channel): the commands a query finds.
pub fn search_json(app: &BrApp, query: &str, limit: usize) -> Value {
    let entries = index(app);
    let found = search(&entries, query, &app.palette.recent, app.page.is_video());
    json!(found
        .iter()
        .take(limit)
        .map(|e| json!({"id": e.id, "title": e.title, "titleEn": e.title_en, "path": e.path, "app": e.source.app(), "key": e.key, "needsParams": e.needs_params, "params": e.params}))
        .collect::<Vec<_>>())
}

/// What `br.inspect` says about the command search.
pub fn state(app: &BrApp) -> Value {
    json!({"open": app.palette.open, "query": app.palette.query, "form": app.palette.form.as_ref().map(|f| f.0.id.clone()), "recent": app.palette.recent})
}
