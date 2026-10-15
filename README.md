# shnyp

**Eine Media-Suite für Foto und Video: der Funktionsumfang von DaVinci Resolve, so direkt bedienbar wie
CapCut, und komplett steuerbar durch Claude.**

shnyp baut auf den quelloffenen [Crafting Apps](https://getartcraft.com/apps) von ArtCraft auf (alle in Rust,
MIT/Apache-2.0): [FilmCraft](https://github.com/storytold/filmcraft) (ein Premiere-Nachbau) für Video und
[LightCraft](https://github.com/storytold/lightcraft) (ein Lightroom-Nachbau) für Fotos. Beide stecken
vollständig in shnyp, mit allen Panels, Menüs, Tastenkürzeln und Einstellungen. Nichts ist weggelassen oder
vereinfacht. shnyp ordnet sie in Seiten wie DaVinci Resolve und legt CapCuts Bedienung darüber: Werkzeuge,
die mit einem Klick zum Ergebnis führen, und hinter jedem davon das volle Werkzeug.

## Seiten

| Seite | Was | Darunter |
|---|---|---|
| **Medien** | Medien sichten, importieren, ordnen | FilmCrafts Import-Modus |
| **Schnitt** | Schneiden in CapCuts Anordnung: Medien, Effekte und Texte links, Bild in der Mitte, Eigenschaften rechts, Timeline unten | FilmCraft |
| **Farbe** | Farbkorrektur und Looks mit Scopes | Lumetri (FilmCraft) |
| **Ton** | Mischpult, Pegel, Sprache, Musik, Effekte | Essential Sound, Mixer (FilmCraft) |
| **Ausgabe** | Exportieren: Formate, Bereich, Warteschlange | FilmCrafts Export-Modus |
| **Foto** | Bibliothek, RAW-Entwicklung, Masken, Kurven, Looks | LightCraft |

Jede Seite behält das Layout, das man ihr gibt. FilmCrafts Menüleiste steht in shnyps Kopfzeile, und jedes
Panel lässt sich über „Fenster“ auf jede Seite holen. Tastenkürzel gehen an die App der sichtbaren Seite.

![Schnitt: Asset-Leiste, Programm-Monitor, Inspector und shnyps Timeline mit Titel, Untertiteln, Musik und Beats](docs/images/shnyp-schnitt.png)

### Schnitt wie in CapCut

Die Schnitt-Seite ist CapCuts Anordnung, gebaut aus FilmCrafts Teilen: links die **Asset-Leiste**, in der
Mitte der **Programm-Monitor**, rechts shnyps **Inspector**, unten shnyps **Timeline**. Alles schreibt in dieselbe
FilmCraft-Sequenz; FilmCrafts eigene Timeline und Panels sind einen Klick entfernt (**Profi**, **Voll**,
„Fenster“).

- **Startbildschirm:** Format-Kacheln (9:16, 16:9, 1:1, 4:5), Vorlagen („Schnelle Story“, „Reise-Montage“,
  „Im Takt“, „YouTube-Intro“ und eigene), Medien hinzufügen, Projekt öffnen, zuletzt Geöffnetes.
  Hineingezogene Dateien werden direkt zum Video; das erste gibt das Format vor.
- **Timeline:** eine **magnetische Hauptspur** ohne Lücken (Löschen schließt auf, Einfügen schiebt weiter),
  darüber Overlays und Texte, darunter Ton und Musik, oben Untertitel. Was auf einem Hauptclip beginnt,
  hängt an ihm (**Verknüpfung**) und geht beim Verschieben, Trimmen, Tempo-Ändern und Löschen mit; ein
  Rückgängig holt alles zurück. Ziehen ordnet um und trimmt, das Lineal setzt die Abspielposition,
  **Beats** markiert den Takt der Musik.
- **Ziehen und Ablegen überallhin:** Dateien aus dem Dateimanager oder Clips aus der Asset-Leiste auf die
  Hauptspur (an die nächste Schnittstelle), darüber (Overlay), darunter (Ton), auf einen Clip (Effekt,
  Übergang, Look) oder auf den Monitor (Overlay an genau dieser Stelle im Bild).
- **Monitor:** Ein Klick wählt den Clip im Bild, Griffe verschieben, skalieren und drehen ihn.
- **Inspector** in drei Stufen: **Basis** zeigt CapCuts Reiter je nach Clip (Video, Text, Ton, Musik,
  Untertitel, Übergang …) mit Reglern, Rauten für Keyframes und Zurücksetzen; **Mehr** jeden Effekt des
  Clips mit allen Parametern und einen Effekt-Wähler; **Voll** FilmCrafts Effekteinstellungen, Lumetri,
  Essential Sound und Text-Panel an derselben Stelle. Jeder Ziehvorgang ist ein Rückgängig-Schritt.
- **Werkzeugleiste:** Medien, Text, Untertitel, Musik, Filter, Übergang | Teilen, Löschen, Tempo, Ton
  trennen | Rückgängig, Wiederholen.
- **Tempo:** Faktor, **Tempo-Kurven** (Montage, Bullet, Sprung, Blitz rein/raus …, gebacken in Stücke
  konstanten Tempos mit passendem Ton), **Einfrieren** und **Rückwärts**.
- **Format** (oben rechts): anderes Seitenverhältnis mit Einpassen, Füllen oder **Motiv folgen**
  (Auto-Reframe); **Hintergrund füllen** mit dem unscharfen Clip selbst, einer Farbe oder einem Bild;
  Titel wandern und skalieren mit.
- **Untertitel** automatisch aus der Sprache (Whisper, lokal, im Hintergrund) oder aus einer Datei.
- **Musik** als eigene Spur, so lang wie das Video und unter Sprache leiser.
- **Vorlagen:** Platzhalter, die man in Reihenfolge mit Medien füllt (zu kurze Clips laufen langsamer);
  „Im Takt“ schneidet auf die Beats der Musik. Jedes Video lässt sich als eigene Vorlage speichern.
- **Exportieren** (oben rechts) mit Vorlagen für TikTok/Reels/Shorts (1080 × 1920), YouTube 1080p und 4K,
  „wie das Video“ und ProRes 422 HQ, im Hintergrund mit Fortschritt.

![Startbildschirm für ein neues Video](docs/images/shnyp-start.png)

### Befehlssuche und Tasten

**Strg+K** (⌘K) sucht unter jedem Befehl von shnyp, FilmCraft und LightCraft, auf Deutsch, Englisch, nach
Kennung, Menü oder CapCut-Wort („teilen“, „Standbild“, „Beats“), zeigt seine Taste und führt ihn aus;
Befehle, die Werte brauchen, bekommen ein kleines Formular. **Strg+Alt+1 … 6** wechselt die Seiten.

FilmCrafts Tastenbelegung startet als **CapCut**: Final Cuts Tasten (A, B, Leertaste, J/K/L, I/O, M, N …)
und dazu ⌘B Teilen, ⌫/Entf Löschen mit Aufschließen, P Magnet, ⌘R Tempo, ⌘E Exportieren. shnyps Befehle
stehen in FilmCrafts Tastenkürzel-Editor (Abschnitt „shnyp“) und lassen sich dort umlegen; eine andere
Belegung (FilmCraft, Premiere, Final Cut, Avid) bleibt, wenn man sie wählt.

![Befehlssuche: „standbild“ findet shnyps Einfrieren und FilmCrafts Standbild-Befehle](docs/images/shnyp-befehlssuche.png)

### Foto

Die Foto-Seite ist LightCraft vollständig (Bibliothek, RAW-Entwicklung, Masken, Kurven), mit shnyps schnellen
Werkzeugen darüber: Looks mit Stärke-Regler, Formate auf den Pixel genau und Export-Vorlagen (Instagram,
Story, Quadrat, Breitbild, volle Auflösung).

![Foto: LightCraft mit shnyps Looks](docs/images/shnyp-foto.png)

## Für Claude: `shnyp mcp`

```sh
claude mcp add shnyp -- /pfad/zu/shnyp mcp                 # eigenes Projekt im Speicher
claude mcp add shnyp -- /pfad/zu/shnyp mcp --connect       # im laufenden Fenster (shnyp --control)
```

- **Video** (`shnyp_video_*`): anlegen und füllen (`shnyp_video_new`, `shnyp_video_add`, `shnyp_video_place` – wie ein
  Ablegen mit der Maus), schneiden auf der magnetischen Hauptspur (`shnyp_video_split`, `shnyp_video_delete`,
  `shnyp_video_move`, `shnyp_video_trim`, `shnyp_video_duplicate`, `shnyp_video_pack`, `shnyp_video_magnet`,
  `shnyp_video_linkage`, `shnyp_video_anchor(s)`), Tempo (`shnyp_video_speed`, `shnyp_video_curve`, `shnyp_video_freeze`,
  `shnyp_video_reverse`), Bild (`shnyp_video_transform`, `shnyp_video_fit`, `shnyp_video_background`,
  `shnyp_video_reframe`, `shnyp_video_format`), der Inspector (`shnyp_video_inspect`, `shnyp_video_set`,
  `shnyp_video_keyframe`, `shnyp_video_effect`, `shnyp_video_schema`), Text, Ton und Look (`shnyp_video_text`,
  `shnyp_video_captions`, `shnyp_video_auto_captions`, `shnyp_video_music`, `shnyp_video_beats`, `shnyp_video_detach_audio`,
  `shnyp_video_looks`, `shnyp_video_filter`, `shnyp_video_transition`), Vorlagen (`shnyp_video_template_list|new|fill|save`),
  `shnyp_video_preview` (gibt ein Bild zurück), `shnyp_video_export`, `shnyp_video_state`, `shnyp_video_undo|redo`.
  Lange Arbeit läuft mit `wait: false` im Hintergrund; `shnyp_video_jobs` zeigt den Fortschritt,
  `shnyp_video_cancel` bricht ab.
- **Der ganze FilmCraft:** `film_commands` sucht unter seinen über 300 Befehlen (Timeline, Lumetri,
  Keyframes, Essential Sound, Untertitel, Export-Einstellungen …), `film_run` führt jeden davon aus.
- **Fotos** (`shnyp_*`): `shnyp_looks`, `shnyp_look`, `shnyp_adjust`, `shnyp_auto`, `shnyp_crop`, `shnyp_export`, `shnyp_preview`,
  `shnyp_state`, `shnyp_reset`, `shnyp_undo`, `shnyp_redo`, dahinter alle Werkzeuge von LightCraft (`import`,
  `query_photos`, `set_develop`, `command_run` …). Mit `--full` kommt jeder LightCraft-Befehl als eigenes
  Werkzeug dazu.

**Im laufenden Fenster:** Mit `shnyp --control` (oder `SHNYP_CONTROL=1`) öffnet shnyp einen Steuerkanal auf
127.0.0.1. Dann arbeitet `shnyp mcp --connect` in dem Fenster, das man sieht: Claude schneidet, und man schaut
zu. Den Kanal schützt ein Token, das shnyp bei jedem Start neu erzeugt und nur für den eigenen Benutzer lesbar
ablegt (`control.json` im Einstellungsordner). Ohne dieses Token wird keine Anfrage ausgeführt. `shnyp.inspect`
nennt dort jedes Bedienelement des Fensters mit seinem Platz und dem Befehl, den es ausführt
(`controls`); `shnyp.palette.search` und `shnyp.palette.run` sind die Befehlssuche, `shnyp.drop` legt Dateien an
einer Stelle des Fensters ab, als wären sie dorthin gezogen worden.

Das Repo enthält eine fertige [`.mcp.json`](.mcp.json).

## Kommandozeile

```sh
shnyp                                   # Fenster (startet auf „Schnitt“)
shnyp clip.mp4 musik.mp3                # Fenster mit einem Video aus diesen Dateien
shnyp --demo urlaub.jpg                 # Fotos öffnen die Foto-Seite
shnyp --control                         # mit Steuerkanal für `shnyp mcp --connect`
shnyp run --import clip.mp4 shnyp.video.split seconds=2 shnyp.video.text text=Hallo \
       shnyp.video.export preset=social path=reel.mp4 wait=true
shnyp run film.sequence.inspect         # jeder FilmCraft-Befehl als film.<id>
shnyp photo urlaub.jpg --look "Golden Hour:70" --light 10 --preset instagram -o insta.jpg
shnyp commands [--film]                 # shnyps Befehle (--film: alle von FilmCraft)
shnyp snapshot --page farbe -o shot.png # Fenster ohne Bildschirm als PNG
```

## Im Browser

`deploy/browser` bringt shnyp auf einen Linux-Server, bedient wird es im Browser. Fotos und Videos werden auf
der Seite hochgeladen (auf das shnyp-Bild gezogen, landen sie dort, wo man sie loslässt), die Exporte dort
heruntergeladen, und der Ton der Wiedergabe spielt im Browser. Das Ganze steht hinter einem Login, und
Claude kann im selben Fenster mitarbeiten. Siehe [deploy/browser/README.md](deploy/browser/README.md).

## Aufbau

| Crate | Inhalt |
|---|---|
| `crates/shnyp-video` | shnyps Video-Befehle (`shnyp.video.*`) auf FilmCrafts Engine, jeder ein Rückgängig-Schritt: Hauptspur und Anker, Inspector-Zeilen, Platzieren, Hintergrund und Reframe, Tempo-Kurven, Beats, Vorlagen, Auto-Untertitel. Gemeinsam genutzt von Fenster, CLI und MCP. |
| `crates/shnyp-photo` | Dasselbe für Fotos (`shnyp.*`) auf LightCraft: Looks, Regler, Formate, Export-Vorlagen. |
| `crates/shnyp-ui` | Das Fenster: die Seiten, FilmCraft und LightCraft darin, shnyps Timeline, Inspector, Asset-Leiste, Monitor-Griffe, Werkzeuge, Startbildschirm, Befehlssuche und Tasten, der Steuerkanal und ein Headless-Treiber für Screenshots und Tests. |
| `crates/shnyp-mcp` | Der MCP-Server: der von LightCraft, mit shnyps Foto- und Video-Werkzeugen davor; im Speicher oder verbunden mit einem Fenster. |
| `apps/shnyp` | Das Programm `shnyp`. |

FilmCraft und LightCraft werden als Git-Abhängigkeiten eingebunden, jeweils fest auf einen Commit gesetzt
(`Cargo.toml`). Ein Update heißt: den Commit hochsetzen und die Tests laufen lassen.

## Bauen und testen

Voraussetzung ist Rust 1.95 oder neuer. Auf Linux braucht der Build die ALSA-Header (`libasound2-dev`,
für den Ton, abschaltbar mit `--no-default-features`), und das Fenster braucht zur Laufzeit die üblichen
X11/Wayland-Bibliotheken (`libxkbcommon-x11`, Mesa). Für die Video-Tests muss `ffmpeg` vorhanden sein;
ohne es werden diese Tests übersprungen.

```sh
cargo run --release -p shnyp
cargo test --workspace               # läuft ohne Bildschirm und ohne GPU
```

Das Fenster nutzt wgpu (Vulkan, Metal oder DX12). Startet das nicht, startet shnyp von selbst neu, dann mit
OpenGL. Mit `SHNYP_RENDERER=gl` oder `SHNYP_RENDERER=wgpu` lässt sich der Weg erzwingen. Unter Linux nutzt shnyp
X11 (XWayland), weil winit nur dort Dateien per Drag-and-drop annimmt; `SHNYP_WAYLAND=1` bleibt bei nativem
Wayland.

## Früher BR

shnyp hieß bis Oktober 2026 BR. Was BR angelegt hat, liest shnyp weiter: Videoprojekte (Hauptspur,
Hintergründe, Anker, Tempo-Kurven, Platzhalter, Beats), eigene Vorlagen (`.brvt`) und die
Tastenbelegung. Beim ersten Start übernimmt shnyp die Ordner von BR (`~/.config/BR` und
`~/.local/share/BR`, unter macOS `~/Library/Application Support/BR`, unter Windows `%APPDATA%\BR` und
`%LOCALAPPDATA%\BR`); eine Bibliothek in `~/Pictures/BR Library` bleibt, wo sie ist, und wird weiter
benutzt. Die Befehle heißen jetzt `shnyp.*`, die MCP-Werkzeuge `shnyp_*`, die Umgebungsvariablen
`SHNYP_*` statt `BR_*`; in Claude Code `claude mcp remove br`, dann `claude mcp add shnyp …` wie oben.
Für eine Einrichtung im Browser siehe [deploy/browser](deploy/browser/README.md).

## Nächste Schritte

- **Fusion:** EffectCraft für Titel, Animationen und Motion Graphics als eigene Seite.
- **Ton in der Tiefe:** Fairlight-artige Spurbearbeitung, Plugins.
- FilmCrafts deutsche Übersetzung vervollständigen. Bisher ist sie nur für die Menüs vorhanden; der Rest
  von FilmCraft ist noch englisch.

## Lizenz

MIT oder Apache-2.0, nach Wahl. LightCraft, FilmCraft und EffectCraft sind Projekte des ArtCraft-Teams unter
denselben Lizenzen. Ihre Namen und Logos sind Marken von ArtCraft und gehören nicht zu shnyp.
