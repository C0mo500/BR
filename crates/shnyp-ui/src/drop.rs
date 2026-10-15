//! Drag and drop on the Schnitt page, CapCut's rules on top of FilmCraft's drags: whatever is
//! dragged (a tile of shnyp's asset bar, an item from FilmCraft's Project panel, a span from its
//! Source monitor, an effect, a look, a transition, a graphics template, files from the file
//! manager) and wherever it lands (shnyp's timeline, the Program monitor's picture or panel), one
//! resolver turns it into one of shnyp's commands, so a drop is one undo step and the same as asking
//! for it by name (`shnyp.video.place`, `shnyp.video.effect`, `shnyp.video.filter`, `shnyp.video.text`, …).

use egui::Pos2;
use filmcraft_project::{ClipId, TrackId};
use filmcraft_ui_egui::panels;
use serde_json::{Value, json};

use crate::ShnypApp;

/// shnyp's own drags: the asset bar's tiles FilmCraft has no payload for.
#[derive(Clone, Debug, PartialEq)]
pub enum ShnypDrag {
    /// "Standardtext": a title.
    Text,
    /// Files (the browser page's uploads, a folder's media).
    Files(Vec<String>),
}

/// What is being dragged.
#[derive(Clone, Debug)]
pub enum Payload {
    Files(Vec<String>),
    Items(Vec<u64>),
    /// A span of a clip from the Source monitor: item, media start and length (seconds), picture,
    /// sound.
    Source {
        item: u64,
        from: f64,
        len: f64,
        video: bool,
        audio: bool,
    },
    /// An effect id, a transition id, `lumetri:<look>` or `preset:<name>`.
    Effect(String),
    Template(String),
    Text,
}

impl Payload {
    /// It goes onto a clip (an effect, a look, a transition), not into the video.
    pub fn onto_clip(&self) -> bool {
        matches!(self, Payload::Effect(_))
    }

    /// A short name for the ghost and the log.
    pub fn label(&self) -> String {
        match self {
            Payload::Files(f) => format!("{} Datei(en)", f.len()),
            Payload::Items(i) => format!("{} Clip(s)", i.len()),
            Payload::Source { .. } => "Ausschnitt".into(),
            Payload::Effect(e) => e.trim_start_matches("lumetri:").trim_start_matches("preset:").to_string(),
            Payload::Template(t) => t.clone(),
            Payload::Text => "Text".into(),
        }
    }
}

/// Where a drop lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot {
    /// The main track at this time (inserted at the nearest cut).
    Main(f64),
    /// Above the main track at this time (on this overlay track, if free there).
    Above(f64, Option<TrackId>),
    /// Among the sound tracks at this time.
    Sound(f64, Option<TrackId>),
    /// Onto a clip: the time, and whether nearer its start than its end.
    Clip(ClipId, f64, bool),
    /// The Program monitor's picture, at this point (0…1).
    Picture(f64, f64),
    /// The Program monitor outside its picture: the main track at the playhead.
    Program,
}

/// The drag in progress, if any (shnyp's own first, then FilmCraft's).
pub fn current(app: &ShnypApp, ctx: &egui::Context) -> Option<Payload> {
    if let Some(b) = egui::DragAndDrop::payload::<ShnypDrag>(ctx) {
        return Some(match &*b {
            ShnypDrag::Text => Payload::Text,
            ShnypDrag::Files(f) => Payload::Files(f.clone()),
        });
    }
    // FilmCraft's accessors take a Ui only for its context
    let ui = egui::Ui::new(ctx.clone(), egui::Id::new("shnyp_drop_probe"), egui::UiBuilder::new());
    if let Some(s) = panels::dragged_source(&ui) {
        return Some(Payload::Source {
            item: s.item.0,
            from: s.range.start.seconds(),
            len: s.range.duration.seconds(),
            video: s.video,
            audio: s.audio,
        });
    }
    if let Some(item) = panels::dragged_project_item(&ui) {
        return Some(Payload::Items(panels::dragged_selection(&app.fc, item).iter().map(|i| i.0).collect()));
    }
    if let Some(e) = panels::dragged_effect(&ui) {
        return Some(Payload::Effect(e));
    }
    panels::dragged_template(&ui).map(Payload::Template)
}

/// The drag is over: nobody else takes it.
pub fn finish(ctx: &egui::Context) {
    egui::DragAndDrop::clear_payload(ctx);
    let ui = egui::Ui::new(ctx.clone(), egui::Id::new("shnyp_drop_probe"), egui::UiBuilder::new());
    panels::clear_drag(&ui);
    forget_release(ctx);
}

/// FilmCraft keeps the frame of a drag's release (its ghost lingers one frame) and clears the
/// next drag on its next release when the key is still there, which it is after a drop someone
/// else took: the next drop would be lost before shnyp saw it.
fn forget_release(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<u64>(egui::Id::new("drag-release-frame")));
}

/// The primary button was let go this frame (where a drag ends).
pub fn released(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.pointer.primary_released())
}

/// Where the Program monitor would take a drop at `p`: its picture (by the point in it), else
/// the rest of its panel.
pub fn monitor_spot(app: &ShnypApp, p: Pos2) -> Option<Spot> {
    let rect = |id: &str| app.fc.auto.find(id).map(|e| egui::Rect::from_min_size(egui::pos2(e.rect[0], e.rect[1]), egui::vec2(e.rect[2], e.rect[3])));
    if let Some(r) = rect("program.picture").filter(|r| r.contains(p) && r.width() > 1.0 && r.height() > 1.0) {
        return Some(Spot::Picture(((p.x - r.min.x) / r.width()) as f64, ((p.y - r.min.y) / r.height()) as f64));
    }
    rect("panel.Program").filter(|r| r.contains(p)).map(|_| Spot::Program)
}

fn time_of(app: &ShnypApp, spot: Spot) -> f64 {
    match spot {
        Spot::Main(t) | Spot::Above(t, _) | Spot::Sound(t, _) | Spot::Clip(_, t, _) => t,
        Spot::Picture(..) | Spot::Program => app.fc.session.playhead().seconds(),
    }
}

/// The main clip under the playhead (what an effect dropped on the monitor goes onto).
fn clip_at_playhead(app: &ShnypApp) -> Option<ClipId> {
    let q = app.fc.session.active_sequence()?;
    let t = app.fc.session.playhead();
    shnyp_video::main_track::main_clips(q).into_iter().find(|it| it.start <= t && t < it.end()).map(|it| it.id)
}

/// The request `shnyp.video.place` gets for media dropped at `spot`.
fn place_request(spot: Spot, at: f64) -> Value {
    match spot {
        Spot::Main(t) => json!({"target": "main", "seconds": t}),
        Spot::Above(t, track) => json!({"target": "overlay", "seconds": t, "track": track.map(|t| t.0)}),
        Spot::Sound(t, track) => json!({"target": "audio", "seconds": t, "track": track.map(|t| t.0)}),
        Spot::Clip(_, t, _) => json!({"target": "auto", "seconds": t}),
        Spot::Picture(x, y) => json!({"target": "overlay", "seconds": at, "position": [x, y]}),
        Spot::Program => json!({"target": "main", "seconds": at}),
    }
}

/// Make the drop: the command it stands for. `false` when it does not apply there (an effect
/// next to every clip).
pub fn apply(app: &mut ShnypApp, ctx: &egui::Context, payload: &Payload, spot: Spot) -> bool {
    let at = time_of(app, spot);
    let run = |app: &mut ShnypApp, id: &str, p: Value| crate::cut::run(app, ctx, id, p).is_some();
    match payload {
        Payload::Files(paths) => {
            // a project is opened, not placed
            let (projects, rest): (Vec<String>, Vec<String>) = paths.iter().cloned().partition(|p| shnyp_video::file_kind(p) == Some("project"));
            if !projects.is_empty() {
                crate::cut::open_files(app, ctx, projects);
            }
            if rest.is_empty() {
                return true;
            }
            let mut p = place_request(spot, at);
            p["paths"] = json!(rest);
            run(app, "shnyp.video.place", p)
        }
        Payload::Items(items) => {
            let mut p = place_request(spot, at);
            p["items"] = json!(items);
            run(app, "shnyp.video.place", p)
        }
        Payload::Source { item, from, len, video, audio } => {
            let mut p = place_request(spot, at);
            p["items"] = json!([item]);
            p["sourceIn"] = json!(from);
            p["duration"] = json!(len);
            if *audio && !*video {
                p["target"] = json!("audio");
            }
            run(app, "shnyp.video.place", p)
        }
        Payload::Text => match spot {
            Spot::Main(t) | Spot::Above(t, _) | Spot::Clip(_, t, _) => run(app, "shnyp.video.text", json!({"text": "Text", "at": t})),
            Spot::Picture(..) | Spot::Program => run(app, "shnyp.video.text", json!({"text": "Text", "at": at})),
            Spot::Sound(..) => false,
        },
        Payload::Template(t) => match spot {
            Spot::Sound(..) => false,
            _ => run(app, "graphics.template.apply", json!({"template": t, "time": filmcraft_time::Tick::from_seconds_f64(at).0})),
        },
        Payload::Effect(e) => {
            let clip = match spot {
                Spot::Clip(c, ..) => Some(c),
                Spot::Picture(..) | Spot::Program => app.fc.session.state.selection.first().copied().or_else(|| clip_at_playhead(app)),
                _ => None,
            };
            let Some(clip) = clip else { return false };
            if let Some(look) = e.strip_prefix("lumetri:") {
                return run(app, "shnyp.video.filter", json!({"preset": look, "clips": [clip.0]}));
            }
            if let Some(preset) = e.strip_prefix("preset:") {
                return run(app, "presets.apply", json!({"preset": preset, "clips": [clip.0]}));
            }
            let transition = [filmcraft_project::EffectKind::VideoTransition, filmcraft_project::EffectKind::AudioTransition]
                .into_iter()
                .any(|k| filmcraft_project::vtransition::find_transition(e, k).is_some());
            if transition {
                let near_in = matches!(spot, Spot::Clip(_, _, true));
                return run(app, "effects.apply", json!({"effect": e, "clip": clip.0, "edge": if near_in { "in" } else { "out" }}));
            }
            run(app, "shnyp.video.effect", json!({"clip": clip.0, "op": "add", "effect": e}))
        }
    }
}

/// Files dropped from the file manager onto the Schnitt page (before the frame, from
/// [`ShnypApp::raw_input_hook`]): onto shnyp's timeline or the Program monitor they are placed by the
/// rules above; anywhere else FilmCraft imports them into the project. The files taken.
pub fn os_files(app: &mut ShnypApp, ctx: &egui::Context, paths: Vec<String>, at: Pos2) -> Result<(), Vec<String>> {
    let spot = crate::timeline::spot_at(app, at, false).or_else(|| monitor_spot(app, at));
    let Some(spot) = spot else { return Err(paths) };
    let paths: Vec<String> = paths.into_iter().map(|p| std::path::absolute(&p).map(|a| a.to_string_lossy().into_owned()).unwrap_or(p)).collect();
    apply(app, ctx, &Payload::Files(paths), spot);
    Ok(())
}

/// Where the pointer is while files are dragged in from the file manager: the OS's pointer when
/// the platform can say (X11), else the last one egui saw.
pub fn os_pointer(app: &ShnypApp, ctx: &egui::Context) -> Option<Pos2> {
    // dropped on the browser page over shnyp: where the page says
    if let Some(at) = app.drop_at {
        return Some(at);
    }
    let screen = app.cursor.and_then(|f| f()).and_then(|(x, y)| {
        ctx.input(|i| i.viewport().inner_rect.map(|r| egui::pos2(x as f32 / i.pixels_per_point - r.min.x, y as f32 / i.pixels_per_point - r.min.y)))
    });
    screen.or_else(|| ctx.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos())))
}

/// A drag let go over the Program monitor (after FilmCraft drew it): an overlay where it was
/// dropped in the picture, the main track elsewhere on the panel; an effect onto the selected
/// clip (else the one at the playhead).
pub fn monitor(app: &mut ShnypApp, ctx: &egui::Context, held: Option<Payload>) {
    if !crate::timeline::visible(app) {
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    if ctx.layer_id_at(p).is_some_and(|l| l.order != egui::Order::Background && l.order != egui::Order::Tooltip) {
        return;
    }
    let Some(spot) = monitor_spot(app, p) else { return };
    // what was dragged as the frame began (FilmCraft may have cleared it since)
    let Some(payload) = current(app, ctx).or(held) else { return };
    let outline = match spot {
        Spot::Picture(..) => app.fc.auto.find("program.picture"),
        _ => app.fc.auto.find("panel.Program"),
    }
    .map(|e| egui::Rect::from_min_size(egui::pos2(e.rect[0], e.rect[1]), egui::vec2(e.rect[2], e.rect[3])));
    if let Some(r) = outline {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("shnyp_drop_monitor")));
        painter.rect_stroke(r.shrink(2.0), egui::CornerRadius::same(4), egui::Stroke::new(2.5, crate::style::ACCENT), egui::StrokeKind::Inside);
    }
    if released(ctx) {
        apply(app, ctx, &payload, spot);
        finish(ctx);
    }
}
