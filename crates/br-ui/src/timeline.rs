//! BR's timeline on the Schnitt page: CapCut's view of FilmCraft's sequence. The main track runs
//! through the middle, tall, with its pictures and its sound; titles, overlays and captions sit
//! above it, music and sound effects below. Clicking selects (FilmCraft's panels follow), dragging
//! a main clip reorders the video, dragging anything else moves it, dragging an edge trims, the
//! ruler moves the playhead. Every gesture is one of BR's commands (`br.video.*`), so it is one
//! undo step and what hangs on the main clips follows.
//!
//! The timeline holds no state of the video: it draws FilmCraft's sequence each frame. "Profi"
//! shows FilmCraft's own Timeline panel in the same place, with all of Premiere's tools.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use br_video::main_track;
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use filmcraft_project::{AudioType, ClipId, ItemId, ItemKind, Sequence, TrackId, TrackItem, TrackKind};
use filmcraft_time::Tick;
use filmcraft_ui_egui::icons::Icon;
use serde_json::json;

use crate::BrApp;
use crate::style::{ACCENT, Theme};

/// The control row above the tracks.
const BAR_H: f32 = 32.0;
const RULER_H: f32 = 22.0;
const HEADER_W: f32 = 92.0;
const MAIN_H: f32 = 62.0;
const OVERLAY_H: f32 = 32.0;
const AUDIO_H: f32 = 30.0;
const CAPTION_H: f32 = 22.0;
/// The empty band above the rows, to drop overlays and titles into.
const DROP_H: f32 = 26.0;
const GAP: f32 = 3.0;
/// How near an edge a press trims instead of moving.
const EDGE_PX: f32 = 7.0;
/// How near a cut, the playhead or a marker a dragged clip or edge snaps.
const SNAP_PX: f32 = 8.0;
/// How far the pointer moves before a press on a clip becomes a drag.
const DRAG_PX: f32 = 4.0;

/// What the timeline remembers between frames: the view and a drag in progress.
#[derive(Default)]
pub struct TimelineState {
    /// FilmCraft's own Timeline panel instead of BR's ("Profi").
    pub full: bool,
    /// Points per second (0: fit the video on the next frame).
    pps: f64,
    /// The time at the left edge, in seconds.
    scroll: f64,
    /// The video the view was fitted to.
    fitted: Option<ItemId>,
    drag: Option<Drag>,
    /// Waveforms being worked out in the background.
    peaks_pending: Arc<Mutex<HashSet<(ItemId, usize)>>>,
    /// Where things were in the last frame, for drops (files from the file manager arrive
    /// before the frame).
    layout: Option<Layout>,
}

/// The timeline's geometry of one frame.
#[derive(Clone, Debug)]
struct Layout {
    lane: Rect,
    pps: f64,
    scroll: f64,
    rows: Vec<(RowKind, TrackId, Rect)>,
    clips: Vec<(ClipId, Rect)>,
}

/// Where a drop at `p` lands on the timeline (as last drawn): onto a clip for an effect
/// (`onto_clip`), else the main track, above it or among the sound tracks.
pub(crate) fn spot_at(app: &BrApp, p: Pos2, onto_clip: bool) -> Option<crate::drop::Spot> {
    use crate::drop::Spot;
    if !visible(app) || app.timeline.full {
        return None;
    }
    let l = app.timeline.layout.as_ref()?;
    if !l.lane.contains(p) {
        return None;
    }
    let t = (l.scroll + (p.x - l.lane.min.x) as f64 / l.pps).max(0.0);
    if onto_clip {
        return l.clips.iter().rev().find(|c| c.1.contains(p)).map(|(c, r)| Spot::Clip(*c, t, p.x - r.min.x < r.max.x - p.x));
    }
    let row = l.rows.iter().find(|r| r.2.min.y - GAP <= p.y && p.y < r.2.max.y + GAP);
    Some(match row {
        Some((RowKind::Main, _, _)) => Spot::Main(t),
        Some((RowKind::Overlay, tr, _)) => Spot::Above(t, Some(*tr)),
        Some((RowKind::Captions, _, _)) => Spot::Above(t, None),
        Some((RowKind::Sound, tr, _)) => Spot::Sound(t, Some(*tr)),
        None => match l.rows.iter().find(|r| r.0 == RowKind::Main) {
            Some(m) if p.y < m.2.min.y => Spot::Above(t, None),
            _ => Spot::Sound(t, None),
        },
    })
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    /// The playhead, from the ruler.
    Playhead,
    /// A clip pressed and perhaps dragged: `grab` is where in it (seconds from its start).
    Clip { clip: ClipId, grab: f64, from: Pos2, moving: bool, main: bool },
    /// An edge of a clip.
    Edge { clip: ClipId, edge: Edge, from: Pos2 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    In,
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Captions,
    Overlay,
    Main,
    Sound,
}

/// One row of the timeline.
#[derive(Clone, Debug)]
struct Row {
    kind: RowKind,
    track: TrackId,
    /// The whole row (header and lane).
    rect: Rect,
    label: String,
}

/// Where things are this frame.
struct View {
    lane: Rect,
    pps: f64,
    scroll: f64,
}

impl View {
    fn x(&self, t: Tick) -> f32 {
        self.lane.min.x + ((t.seconds() - self.scroll) * self.pps) as f32
    }
    fn t(&self, x: f32) -> f64 {
        (self.scroll + (x - self.lane.min.x) as f64 / self.pps).max(0.0)
    }
}

/// The timeline is shown: on the Schnitt page, when there is a video (not on the start screen).
pub fn visible(app: &BrApp) -> bool {
    app.page == crate::Page::Schnitt && app.fc.session.active_sequence().is_some() && !crate::cut::start_screen_visible(app)
}

/// Draw the timeline as a band at the bottom of the window (before FilmCraft takes the rest).
pub fn show(app: &mut BrApp, ui: &mut Ui) {
    if !visible(app) {
        return;
    }
    let t = Theme::get(ui.ctx());
    egui::Panel::bottom("br_timeline")
        .resizable(true)
        .default_size(250.0)
        .size_range(140.0..=700.0)
        .frame(egui::Frame::NONE.fill(t.bar).stroke(Stroke::new(1.0, t.divider)))
        .show(ui, |ui| {
            bar(app, ui, &t);
            let rect = ui.available_rect_before_wrap();
            let fc_has_it = app.fc.ui.dock.contains(filmcraft_ui_egui::dock::PanelKind::Timeline);
            if app.timeline.full && !fc_has_it {
                // FilmCraft's own panel (its egui ids are its own: never drawn twice in one frame)
                if ui.rect_contains_pointer(rect) && ui.input(|i| i.pointer.any_pressed()) {
                    app.fc.ui.focused = filmcraft_ui_egui::dock::PanelKind::Timeline;
                }
                filmcraft_ui_egui::panels::timeline::show(&mut app.fc, ui, rect);
            } else {
                canvas(app, ui, &t, rect);
            }
            ui.allocate_rect(rect, Sense::hover());
        });
}

/// The control row: magnet, linkage, zoom, Profi; the time on the right.
fn bar(app: &mut BrApp, ui: &mut Ui, t: &Theme) {
    let ctx = ui.ctx().clone();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_H), Sense::hover());
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.divider));
    let mut row =
        ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 2.0;
    let settings = app.fc.session.active_sequence().map(main_track::settings).unwrap_or_default();
    let full = app.timeline.full;

    let r = crate::cut::tool(
        &mut row,
        t,
        Icon::Magnet,
        "Magnet",
        "Magnetische Hauptspur: keine Lücken, Löschen schließt auf, Einfügen schiebt weiter (aus: Lücken bleiben)",
        settings.magnet,
        !full,
    );
    app.widgets.insert("tl.magnet".into(), r.rect);
    if r.clicked() {
        crate::cut::run(app, &ctx, "br.video.magnet", json!({"on": !settings.magnet}));
    }
    let r = crate::cut::tool(
        &mut row,
        t,
        Icon::Link,
        "Verknüpfung",
        "Titel, Overlays, Soundeffekte und Untertitel gehen mit dem Clip der Hauptspur, auf dem sie beginnen (aus: wie in FilmCraft)",
        settings.linkage,
        !full,
    );
    app.widgets.insert("tl.linkage".into(), r.rect);
    if r.clicked() {
        crate::cut::run(app, &ctx, "br.video.linkage", json!({"on": !settings.linkage}));
    }
    row.add_space(10.0);
    for (key, icon, text, tip) in [
        ("zoom.out", Icon::ChevronLeft, "", "Weiter weg (Strg + Mausrad)"),
        ("zoom.in", Icon::ChevronRight, "", "Näher heran (Strg + Mausrad)"),
        ("zoom.fit", Icon::Fullscreen, "Ganz", "Das ganze Video zeigen"),
    ] {
        let r = crate::cut::tool(&mut row, t, icon, text, tip, false, !full);
        app.widgets.insert(format!("tl.{key}"), r.rect);
        if r.clicked() {
            let w = (rect.width() - HEADER_W).max(100.0);
            match key {
                "zoom.in" => zoom(&mut app.timeline, 1.5, None, w),
                "zoom.out" => zoom(&mut app.timeline, 1.0 / 1.5, None, w),
                _ => app.timeline.pps = 0.0,
            }
        }
    }
    row.add_space(10.0);
    let r = crate::cut::tool(
        &mut row,
        t,
        Icon::Wrench,
        "Profi",
        "FilmCrafts vollständige Timeline (Premieres Werkzeuge: Rollen, Slip, Slide, Spuren, Keyframes …)",
        full,
        true,
    );
    app.widgets.insert("tl.view.full".into(), r.rect);
    if r.clicked() {
        app.timeline.full = !full;
    }
    // the music's beats as markers to snap to (again: away)
    let has_beats = app.fc.session.active_sequence().is_some_and(|q| !br_video::beats::markers(q).is_empty());
    let r = crate::cut::tool(
        &mut row,
        t,
        Icon::Marker,
        "Beats",
        if has_beats { "Die Beat-Marker entfernen" } else { "Die Beats der Musik (oder des ausgewählten Tons) markieren: Clips rasten daran ein" },
        has_beats,
        true,
    );
    app.widgets.insert("tl.beats".into(), r.rect);
    if r.clicked() {
        let ctx = row.ctx().clone();
        crate::cut::run(app, &ctx, "br.video.beats", json!({"mode": if has_beats { "none" } else { "beats" }}));
    }
    // the time: where the playhead is, and how long the video is
    if let Some(q) = app.fc.session.active_sequence() {
        let rate = q.settings.frame_rate;
        let end = br_video::magnet::picture_end(q);
        let text = format!("{}  /  {}", clock(app.fc.session.playhead(), rate), clock(end, rate));
        row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(text).monospace().size(12.0).color(t.dim));
        });
    }
}

/// `mm:ss.ff` (`h:mm:ss.ff` past an hour).
fn clock(t: Tick, rate: filmcraft_time::FrameRate) -> String {
    let fps = rate.as_f64().round().max(1.0) as i64;
    let frames = (t.seconds() * rate.as_f64()).round() as i64;
    let (s, f) = (frames.div_euclid(fps), frames.rem_euclid(fps));
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{f:02}") } else { format!("{m:02}:{s:02}.{f:02}") }
}

/// Zoom by `factor` about the time under `at` (screen x; the middle without).
/// Zoom BR's timeline by `factor` around its middle (0: the whole video), as its buttons do.
pub(crate) fn zoom_by(app: &mut BrApp, factor: f64) {
    if factor <= 0.0 {
        app.timeline.pps = 0.0;
        return;
    }
    let width = app.timeline.layout.as_ref().map_or(800.0, |l| l.lane.width());
    zoom(&mut app.timeline, factor, None, width);
}

fn zoom(st: &mut TimelineState, factor: f64, at: Option<(f32, f32)>, width: f32) {
    if st.pps <= 0.0 {
        return;
    }
    let (x, left) = at.unwrap_or((width / 2.0, 0.0));
    let rel = (x - left).clamp(0.0, width);
    let anchor = st.scroll + rel as f64 / st.pps;
    st.pps = (st.pps * factor).clamp(2.0, 2000.0);
    st.scroll = (anchor - rel as f64 / st.pps).max(0.0);
}

/// The rows of `q`, top to bottom: captions, overlays (the highest first), the main track, sound
/// (the main clips' own sound is drawn in the main track).
fn rows(q: &Sequence, top: f32, left: f32, right: f32) -> Vec<Row> {
    let main = main_track::main_track_id(q);
    let main_audio = main_track::main_audio_track(q);
    let mut out = Vec::new();
    // an empty band on top: something dropped there becomes an overlay on a track of its own
    let mut y = top + DROP_H;
    let mut push = |kind: RowKind, track: TrackId, h: f32, label: String, out: &mut Vec<Row>| {
        out.push(Row { kind, track, rect: Rect::from_min_max(pos2(left, y), pos2(right, y + h)), label });
        y += h + GAP;
    };
    for (i, c) in q.caption_tracks.iter().enumerate() {
        let label = if q.caption_tracks.len() > 1 { format!("Untertitel {}", i + 1) } else { "Untertitel".into() };
        push(RowKind::Captions, c.id, CAPTION_H, label, &mut out);
    }
    // tracks without anything on them are not shown (a drop above the top row makes a new one)
    let overlays: Vec<&filmcraft_project::Track> =
        q.video_tracks.iter().filter(|t| Some(t.id) != main && !main_track::is_background_track(t) && !t.items.is_empty()).collect();
    for (n, t) in overlays.iter().enumerate().rev() {
        let titles = !t.items.is_empty() && t.items.iter().all(|it| it.graphic_layers().next().is_some());
        let label = if titles { format!("Text {}", n + 1) } else { format!("Overlay {}", n + 1) };
        push(RowKind::Overlay, t.id, OVERLAY_H, label, &mut out);
    }
    if let Some(m) = main {
        push(RowKind::Main, m, MAIN_H, "Hauptspur".into(), &mut out);
    }
    let mut n = 0;
    for t in q.audio_tracks.iter().filter(|t| !t.items.is_empty()) {
        // the main track's sound track shows only what is not a main clip's own sound
        if Some(t.id) == main_audio && t.items.iter().all(|it| it.link.is_some()) {
            continue;
        }
        n += 1;
        let music = !t.items.is_empty() && t.items.iter().all(|it| it.essential.as_ref().is_some_and(|e| e.kind == AudioType::Music));
        let label = if music { format!("Musik {n}") } else { format!("Ton {n}") };
        push(RowKind::Sound, t.id, AUDIO_H, label, &mut out);
    }
    out
}

/// The colours of a clip by what it is.
fn clip_colour(it: &TrackItem, kind: RowKind, project: &filmcraft_project::Project) -> Color32 {
    let graphic = it.graphic_layers().next().is_some();
    match kind {
        _ if graphic => Color32::from_rgb(0x8a, 0x5c, 0xd6),
        RowKind::Sound if it.essential.as_ref().is_some_and(|e| e.kind == AudioType::Music) => Color32::from_rgb(0x2f, 0x9e, 0x6a),
        RowKind::Sound => Color32::from_rgb(0x2f, 0x7f, 0xc4),
        RowKind::Overlay if matches!(project.item(it.item).map(|i| &i.kind), Some(ItemKind::AdjustmentLayer { .. })) => {
            Color32::from_rgb(0x8c, 0x8c, 0x5a)
        }
        _ => Color32::from_rgb(0x2d, 0x4d, 0x63),
    }
}

/// The edge of `r` near `x`, if any.
fn edge_at(r: Rect, x: f32) -> Option<Edge> {
    let e = EDGE_PX.min(r.width() / 3.0);
    if x <= r.min.x + e {
        Some(Edge::In)
    } else if x >= r.max.x - e {
        Some(Edge::Out)
    } else {
        None
    }
}

/// Times a dragged clip or edge snaps to: the cuts and clip edges on every track, the playhead,
/// the markers.
fn snap_points(q: &Sequence, playhead: Tick, skip: ClipId) -> Vec<f64> {
    let mut v: Vec<f64> =
        q.all_tracks().flat_map(|t| t.items.iter()).filter(|it| it.id != skip).flat_map(|it| [it.start.seconds(), it.end().seconds()]).collect();
    v.push(playhead.seconds());
    v.push(0.0);
    v.extend(q.markers.iter().map(|m| m.start.seconds()));
    v
}

fn snap(points: &[f64], secs: f64, pps: f64) -> (f64, Option<f64>) {
    let best = points.iter().copied().min_by(|a, b| (a - secs).abs().total_cmp(&(b - secs).abs()));
    match best {
        Some(p) if ((p - secs).abs() * pps) as f32 <= SNAP_PX => (p, Some(p)),
        _ => (secs, None),
    }
}

/// The tracks and clips.
fn canvas(app: &mut BrApp, ui: &mut Ui, t: &Theme, rect: Rect) {
    let ctx = ui.ctx().clone();
    let project = app.fc.session.project.clone();
    let Some(active) = app.fc.session.state.active_sequence else { return };
    let Some(q) = project.sequence(active) else { return };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, t.canvas);
    let ruler = Rect::from_min_max(pos2(rect.min.x + HEADER_W, rect.min.y), pos2(rect.max.x, rect.min.y + RULER_H));
    let lane_x = ruler.min.x;
    let rows = rows(q, ruler.max.y + 4.0, rect.min.x, rect.max.x);
    let lane = Rect::from_min_max(pos2(lane_x, ruler.max.y), rect.max);
    let width = lane.width().max(50.0);
    let playhead = app.fc.session.playhead();
    let end = br_video::magnet::picture_end(q).max(q.duration());

    // the view: fitted to the video when it is new (or asked for)
    let st = &mut app.timeline;
    if st.pps <= 0.0 || st.fitted != Some(active) {
        let secs = end.seconds().max(5.0);
        st.pps = (width as f64 * 0.92 / secs).clamp(2.0, 2000.0);
        st.scroll = 0.0;
        st.fitted = Some(active);
    }
    // during playback the playhead stays in view
    if app.fc.playback.playing {
        let ph = playhead.seconds();
        let px = (ph - st.scroll) * st.pps;
        if px < 0.0 || px > width as f64 * 0.95 {
            st.scroll = (ph - width as f64 * 0.1 / st.pps).max(0.0);
        }
    }
    // wheel: scroll in time; with Ctrl (or a pinch) zoom about the pointer
    let hover = ui.rect_contains_pointer(rect);
    if hover {
        let (scroll, zoom_delta, ctrl, pointer) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers.command, i.pointer.hover_pos()));
        if zoom_delta != 1.0 || (ctrl && scroll.y != 0.0) {
            let f = if zoom_delta != 1.0 { zoom_delta as f64 } else { (scroll.y as f64 / 200.0).exp() };
            zoom(st, f, pointer.map(|p| (p.x, lane_x)), width);
        } else {
            let d = if scroll.x != 0.0 { scroll.x } else { scroll.y };
            if d != 0.0 {
                st.scroll = (st.scroll - d as f64 / st.pps).clamp(0.0, (end.seconds() + 60.0).max(0.0));
            }
        }
    }
    let view = View { lane, pps: st.pps, scroll: st.scroll };

    // ruler, with the beats of the music
    paint_ruler(&painter, t, ruler, &view, q.settings.frame_rate);
    for b in br_video::beats::markers(q) {
        let x = view.x(Tick::from_seconds_f64(b));
        if x >= ruler.min.x && x <= ruler.max.x {
            painter.circle_filled(pos2(x, ruler.max.y - 3.0), 2.5, Color32::from_rgb(0xff, 0xd0, 0x40));
        }
    }
    let rid = ui.id().with("br_tl_ruler");
    let rr = ui.interact(ruler, rid, Sense::click_and_drag());
    app.widgets.insert("tl.ruler".into(), ruler);

    // rows: headers, lanes, clips
    let sel: HashSet<ClipId> = app.fc.session.state.selection.iter().copied().collect();
    let cap_sel: HashSet<ClipId> = app.fc.session.state.caption_selection.iter().copied().collect();
    let lane_painter = painter.with_clip_rect(Rect::from_min_max(pos2(lane_x, ruler.max.y), rect.max));
    let mut hits: Vec<(ClipId, Rect, RowKind)> = Vec::new();
    for row in &rows {
        let header = Rect::from_min_max(row.rect.min, pos2(lane_x - 4.0, row.rect.max.y));
        let lane_r = Rect::from_min_max(pos2(lane_x, row.rect.min.y), row.rect.max);
        if row.kind == RowKind::Main {
            painter.rect_filled(lane_r, CornerRadius::ZERO, t.well);
        }
        painter.text(
            pos2(header.min.x + 8.0, header.center().y),
            Align2::LEFT_CENTER,
            &row.label,
            FontId::proportional(11.5),
            if row.kind == RowKind::Main { t.text } else { t.dim },
        );
        match row.kind {
            RowKind::Captions => {
                let Some(ct) = q.caption_track(row.track) else { continue };
                for c in &ct.captions {
                    let r = Rect::from_min_max(pos2(view.x(c.start), lane_r.min.y + 1.0), pos2(view.x(c.end()), lane_r.max.y - 1.0));
                    if r.max.x < lane.min.x || r.min.x > lane.max.x {
                        continue;
                    }
                    let fill = Color32::from_rgb(0xc9, 0xa2, 0x27);
                    lane_painter.rect_filled(r, CornerRadius::same(3), fill);
                    if cap_sel.contains(&c.id) {
                        lane_painter.rect_stroke(r, CornerRadius::same(3), Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
                    }
                    let text = c.text.replace('\n', " ");
                    let clip_p = lane_painter.with_clip_rect(r.shrink(2.0).intersect(lane_painter.clip_rect()));
                    clip_p.text(
                        pos2(r.min.x + 4.0, r.center().y),
                        Align2::LEFT_CENTER,
                        text,
                        FontId::proportional(10.5),
                        Color32::from_rgb(0x2a, 0x20, 0x05),
                    );
                    hits.push((c.id, r, RowKind::Captions));
                }
            }
            _ => {
                let Some(track) = q.track(row.track) else { continue };
                for it in track.items.iter().filter(|it| !main_track::is_background(it)) {
                    if row.kind == RowKind::Sound && Some(track.id) == main_track::main_audio_track(q) && it.link.is_some() {
                        continue;
                    }
                    let r = Rect::from_min_max(pos2(view.x(it.start), lane_r.min.y), pos2(view.x(it.end()), lane_r.max.y));
                    if r.max.x < lane.min.x || r.min.x > lane.max.x || r.width() < 1.0 {
                        continue;
                    }
                    hits.push((it.id, r, row.kind));
                    draw_clip(app, &ctx, &lane_painter, r, it, row.kind, sel.contains(&it.id), q, &project);
                    app.widgets.insert(format!("tl.clip.{}", it.id.0), r);
                }
            }
        }
    }

    app.timeline.layout = Some(Layout {
        lane,
        pps: view.pps,
        scroll: view.scroll,
        rows: rows.iter().map(|r| (r.kind, r.track, Rect::from_min_max(pos2(lane_x, r.rect.min.y), r.rect.max))).collect(),
        clips: hits.iter().filter(|h| h.2 != RowKind::Captions).map(|h| (h.0, h.1)).collect(),
    });
    // something dragged in (an asset, a project item, an effect, files from the file manager):
    // where it would land, and on release the drop
    let os_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
    let over = if os_files { crate::drop::os_pointer(app, &ctx) } else { ui.input(|i| i.pointer.interact_pos()) };
    if let Some(p) = over.filter(|p| lane.contains(*p)) {
        let payload = if os_files { Some(crate::drop::Payload::Files(Vec::new())) } else { crate::drop::current(app, &ctx) };
        if let Some(payload) = payload
            && let Some(spot) = spot_at(app, p, payload.onto_clip())
        {
            drop_preview(&lane_painter, q, &view, &rows, &hits, spot);
            if !os_files && crate::drop::released(&ctx) {
                crate::drop::apply(app, &ctx, &payload, spot);
                crate::drop::finish(&ctx);
            }
        }
    }

    // a drag in progress: where it would land
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut snap_x = None;
    if let (Some(drag), Some(p)) = (app.timeline.drag, pointer) {
        match drag {
            Drag::Clip { clip, grab, moving: true, main, .. } => {
                if let Some((_, it)) = q.find_item(clip) {
                    let w = (it.duration.seconds() * view.pps) as f32;
                    if main
                        && rows
                            .iter()
                            .find(|r| r.kind == RowKind::Main)
                            .is_some_and(|r| r.rect.contains(p) || (p.y - r.rect.center().y).abs() < MAIN_H)
                    {
                        // the cut it would go to
                        let at = main_track::main_clips(q)
                            .iter()
                            .map(|m| m.start)
                            .chain(main_track::main_clips(q).last().map(|m| m.end()))
                            .min_by(|a, b| (view.x(*a) - p.x).abs().total_cmp(&(view.x(*b) - p.x).abs()));
                        if let (Some(at), Some(row)) = (at, rows.iter().find(|r| r.kind == RowKind::Main)) {
                            let x = view.x(at);
                            lane_painter.line_segment([pos2(x, row.rect.min.y - 4.0), pos2(x, row.rect.max.y + 4.0)], Stroke::new(3.0, ACCENT));
                        }
                    } else {
                        let pts = snap_points(q, playhead, clip);
                        let (start, s) = snap(&pts, view.t(p.x) - grab, view.pps);
                        snap_x = s.map(|s| view.x(Tick::from_seconds_f64(s)));
                        let x = view.x(Tick::from_seconds_f64(start));
                        let row = rows.iter().find(|r| r.rect.min.y <= p.y && p.y < r.rect.max.y + GAP);
                        let (y0, y1) = row.map(|r| (r.rect.min.y, r.rect.max.y)).unwrap_or((p.y - 14.0, p.y + 14.0));
                        let ghost = Rect::from_min_max(pos2(x, y0), pos2(x + w, y1));
                        lane_painter.rect_filled(ghost, CornerRadius::same(4), Color32::from_white_alpha(40));
                        lane_painter.rect_stroke(ghost, CornerRadius::same(4), Stroke::new(1.5, ACCENT), StrokeKind::Inside);
                    }
                }
            }
            Drag::Edge { clip, edge, .. } => {
                if let Some((_, it)) = q.find_item(clip) {
                    let pts = snap_points(q, playhead, clip);
                    let (secs, s) = snap(&pts, view.t(p.x), view.pps);
                    snap_x = s.map(|s| view.x(Tick::from_seconds_f64(s)));
                    let x = view.x(Tick::from_seconds_f64(secs));
                    if let Some((_, r, _)) = hits.iter().find(|h| h.0 == clip) {
                        let (a, b) = match edge {
                            Edge::In => (x, r.max.x),
                            Edge::Out => (r.min.x, x),
                        };
                        let _ = it;
                        let ghost = Rect::from_min_max(pos2(a.min(b), r.min.y), pos2(a.max(b), r.max.y));
                        lane_painter.rect_stroke(ghost, CornerRadius::same(4), Stroke::new(2.0, ACCENT), StrokeKind::Inside);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(x) = snap_x {
        lane_painter.line_segment([pos2(x, ruler.max.y), pos2(x, rect.max.y)], Stroke::new(1.0, Color32::from_rgb(0xff, 0xd0, 0x40)));
    }

    // the playhead
    let px = view.x(playhead);
    if px >= lane.min.x && px <= lane.max.x {
        painter.line_segment([pos2(px, ruler.min.y), pos2(px, rect.max.y)], Stroke::new(1.5, Color32::WHITE));
        painter.add(egui::Shape::convex_polygon(
            vec![pos2(px - 6.0, ruler.min.y), pos2(px + 6.0, ruler.min.y), pos2(px, ruler.min.y + 9.0)],
            Color32::WHITE,
            Stroke::NONE,
        ));
    }

    // input
    interact(app, ui, &ctx, rect, &view, rr, &hits, &rows);
}

/// Where a drop would land: the cut it goes in at, the row it goes on, the clip it goes onto.
fn drop_preview(p: &egui::Painter, q: &Sequence, view: &View, rows: &[Row], hits: &[(ClipId, Rect, RowKind)], spot: crate::drop::Spot) {
    use crate::drop::Spot;
    let lane = view.lane;
    let band = |p: &egui::Painter, y0: f32, y1: f32, t: f64| {
        let x = view.x(Tick::from_seconds_f64(t));
        let r = Rect::from_min_max(pos2(x, y0), pos2((x + (2.0 * view.pps) as f32).min(lane.max.x), y1));
        p.rect_filled(r, CornerRadius::same(4), Color32::from_white_alpha(36));
        p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, ACCENT), StrokeKind::Inside);
    };
    let main = rows.iter().find(|r| r.kind == RowKind::Main);
    match spot {
        Spot::Main(t) => {
            let cuts: Vec<Tick> =
                main_track::main_clips(q).iter().map(|m| m.start).chain(main_track::main_clips(q).last().map(|m| m.end())).collect();
            let at = cuts.into_iter().min_by(|a, b| (a.seconds() - t).abs().total_cmp(&(b.seconds() - t).abs())).unwrap_or(Tick::ZERO);
            if let Some(row) = main {
                let x = view.x(at);
                p.line_segment([pos2(x, row.rect.min.y - 4.0), pos2(x, row.rect.max.y + 4.0)], Stroke::new(3.0, ACCENT));
            }
        }
        Spot::Above(t, track) => {
            let row = rows.iter().find(|r| Some(r.track) == track);
            let (y0, y1) = match (row, main) {
                (Some(r), _) => (r.rect.min.y, r.rect.max.y),
                (None, Some(m)) => (m.rect.min.y - OVERLAY_H.min(m.rect.min.y - lane.min.y).max(8.0), m.rect.min.y - 2.0),
                _ => (lane.min.y, lane.min.y + OVERLAY_H),
            };
            band(p, y0, y1, t);
        }
        Spot::Sound(t, track) => {
            let row = rows.iter().find(|r| Some(r.track) == track && r.kind == RowKind::Sound);
            let last = rows.iter().rev().find(|r| r.kind == RowKind::Sound).or(main);
            let (y0, y1) = match (row, last) {
                (Some(r), _) => (r.rect.min.y, r.rect.max.y),
                (None, Some(l)) => (l.rect.max.y + GAP, (l.rect.max.y + GAP + AUDIO_H).min(lane.max.y)),
                _ => (lane.max.y - AUDIO_H, lane.max.y),
            };
            band(p, y0, y1, t);
        }
        Spot::Clip(c, _, _) => {
            if let Some((_, r, _)) = hits.iter().find(|h| h.0 == c) {
                p.rect_stroke(*r, CornerRadius::same(4), Stroke::new(2.5, ACCENT), StrokeKind::Outside);
            }
        }
        Spot::Picture(..) | Spot::Program => {}
    }
}

fn paint_ruler(p: &egui::Painter, t: &Theme, r: Rect, view: &View, rate: filmcraft_time::FrameRate) {
    p.rect_filled(r, CornerRadius::ZERO, t.bar);
    // a label every ~90 points, on a round step
    let steps = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
    let step = steps.iter().copied().find(|s| s * view.pps >= 90.0).unwrap_or(600.0);
    let first = (view.scroll / step).floor() * step;
    let mut s = first;
    while view.x(Tick::from_seconds_f64(s)) < r.max.x {
        let x = view.x(Tick::from_seconds_f64(s));
        if x >= r.min.x {
            p.line_segment([pos2(x, r.max.y - 7.0), pos2(x, r.max.y)], Stroke::new(1.0, t.dim));
            p.text(pos2(x + 3.0, r.min.y + 3.0), Align2::LEFT_TOP, clock(Tick::from_seconds_f64(s), rate), FontId::monospace(10.0), t.dim);
        }
        for k in 1..5 {
            let xm = view.x(Tick::from_seconds_f64(s + step * k as f64 / 5.0));
            if xm >= r.min.x && xm <= r.max.x {
                p.line_segment([pos2(xm, r.max.y - 3.0), pos2(xm, r.max.y)], Stroke::new(1.0, t.divider));
            }
        }
        s += step;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_clip(
    app: &mut BrApp,
    ctx: &egui::Context,
    p: &egui::Painter,
    r: Rect,
    it: &TrackItem,
    kind: RowKind,
    selected: bool,
    q: &Sequence,
    project: &filmcraft_project::Project,
) {
    let fill = clip_colour(it, kind, project);
    let fill = if it.enabled { fill } else { Color32::from_rgb(fill.r() / 2, fill.g() / 2, fill.b() / 2) };
    let body = r.shrink2(vec2(0.5, 0.0));
    p.rect_filled(body, CornerRadius::same(4), fill);
    let cp = p.with_clip_rect(body.intersect(p.clip_rect()));
    let graphic = it.graphic_layers().next().is_some();
    // pictures: frames side by side (the main track's, and an overlay's)
    let has_picture = matches!(project.item(it.item).map(|i| &i.kind), Some(ItemKind::Media(m)) if m.info.video.is_some());
    if matches!(kind, RowKind::Main | RowKind::Overlay) && has_picture && !graphic {
        let pic_h = if kind == RowKind::Main { body.height() * 0.62 } else { body.height() - 2.0 };
        let th = Rect::from_min_max(pos2(body.min.x, body.min.y + 1.0), pos2(body.max.x, body.min.y + 1.0 + pic_h));
        let aspect = project.item(it.item).and_then(|pi| match &pi.kind {
            ItemKind::Media(m) => m.info.video.as_ref().map(|v| v.width as f32 / v.height.max(1) as f32),
            _ => None,
        });
        if let Some(aspect) = aspect.filter(|a| a.is_finite() && *a > 0.0) {
            let tw = (th.height() * aspect).max(8.0);
            let rate = q.settings.frame_rate;
            let tp = cp.with_clip_rect(th.intersect(cp.clip_rect()));
            let visible = (tp.clip_rect().min.x, tp.clip_rect().max.x);
            let tiles = filmcraft_ui_egui::panels::timeline::thumbnail_tiles(
                filmcraft_ui_egui::state::ThumbnailMode::Continuous,
                th.min.x,
                th.max.x,
                tw,
                visible,
                it.duration,
                rate.frame_duration(),
            );
            for (x, offset) in tiles {
                let mt = rate.snap(it.source_time_at(it.start + offset));
                if let Some((tex, _)) = app.fc.thumbnail(ctx, it.item, mt, 160) {
                    let tr = Rect::from_min_size(pos2(x, th.min.y), vec2(tw, th.height()));
                    tp.image(tex, tr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
        }
    }
    // sound: the clip's own (a main clip's partner), as a waveform at the bottom
    let sound = match kind {
        RowKind::Main => main_track::partners(q, it.id)
            .into_iter()
            .filter_map(|c| q.find_item(c))
            .find(|(t, _)| q.track(*t).is_some_and(|t| t.kind == TrackKind::Audio))
            .map(|(_, a)| a.clone()),
        RowKind::Sound => Some(it.clone()),
        _ => None,
    };
    if let Some(a) = sound {
        let zone = if kind == RowKind::Main {
            Rect::from_min_max(pos2(body.min.x, body.max.y - body.height() * 0.36), body.max)
        } else {
            Rect::from_min_max(pos2(body.min.x, body.min.y + 12.0), body.max)
        };
        if kind == RowKind::Main {
            cp.rect_filled(zone, CornerRadius::ZERO, Color32::from_rgb(0x1d, 0x3a, 0x4a));
        }
        waveform(app, &cp, zone, &a, if kind == RowKind::Main { Color32::from_rgb(0x5f, 0xb6, 0xd8) } else { Color32::from_white_alpha(150) });
    }
    // the name (a title's text), the speed
    let mut label = it.name.clone();
    if (it.speed - 1.0).abs() > 1e-6 || it.reverse {
        label = format!("{label}  {}{}×", if it.reverse { "◀ " } else { "" }, (it.speed * 100.0).round() / 100.0);
    }
    if let Some((curve, _)) = br_video::speed::curve_of(q, it.id) {
        let name = br_video::speed::find_curve(&curve).map_or(curve.as_str(), |c| c.label);
        label = format!("{label}  ∿ {name}");
    }
    if main_track::partners(q, it.id).iter().any(|c| q.find_item(*c).is_some_and(|(_, x)| main_track::is_background(x))) {
        label = format!("{label}  · Hintergrund");
    }
    if body.width() > 26.0 {
        let text_c = Color32::from_rgb(0xf0, 0xf0, 0xf0);
        let galley = cp.layout_no_wrap(label, FontId::proportional(11.0), text_c);
        let pos = pos2(body.min.x + 5.0, body.min.y + 2.0);
        cp.rect_filled(Rect::from_min_size(pos, galley.size() + vec2(4.0, 0.0)), CornerRadius::same(2), Color32::from_black_alpha(110));
        cp.galley(pos + vec2(2.0, 0.0), galley, text_c);
    }
    if selected {
        p.rect_stroke(body, CornerRadius::same(4), Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
    } else {
        p.rect_stroke(body, CornerRadius::same(4), Stroke::new(1.0, Color32::from_black_alpha(120)), StrokeKind::Inside);
    }
}

/// A clip's waveform (FilmCraft's peaks, worked out in the background the first time).
fn waveform(app: &mut BrApp, p: &egui::Painter, zone: Rect, it: &TrackItem, col: Color32) {
    let Some(peaks) = peaks(app, it.item, it.audio_stream) else { return };
    let clip = p.clip_rect().intersect(zone);
    if clip.width() <= 0.0 || peaks.is_empty() {
        return;
    }
    let peak = peaks.iter().fold(0f32, |m, (a, b)| m.max(a.abs()).max(b.abs())).max(1e-4);
    let mut mesh = egui::Mesh::default();
    let w = zone.width().max(1.0);
    for x in (clip.min.x.floor() as i32)..(clip.max.x.ceil() as i32) {
        let f0 = (x as f32 - zone.min.x) / w;
        let f1 = (x as f32 + 1.0 - zone.min.x) / w;
        let t0 = it.start + Tick((it.duration.0 as f64 * f0 as f64) as i64);
        let t1 = it.start + Tick((it.duration.0 as f64 * f1 as f64) as i64);
        let (a, b) = (it.source_time_at(t0), it.source_time_at(t1));
        let s0 = (a.min(b).seconds() * 48_000.0 / 256.0) as usize;
        let s1 = ((a.max(b).seconds() * 48_000.0 / 256.0) as usize).max(s0 + 1);
        let m = peaks.iter().skip(s0).take(s1 - s0).fold(0f32, |m, (lo, hi)| m.max(lo.abs()).max(hi.abs())) / peak;
        let h = m.clamp(0.0, 1.0) * zone.height() * 0.95;
        if h > 0.3 {
            let cy = zone.center().y;
            mesh.add_colored_rect(Rect::from_min_max(pos2(x as f32, cy - h / 2.0), pos2(x as f32 + 1.0, cy + h / 2.0)), col);
        }
    }
    p.add(mesh);
}

/// The peaks of one audio stream (min and max per 256 samples at 48 kHz, as FilmCraft keeps them,
/// so its panel and BR's timeline share them); missing ones are worked out on a thread.
fn peaks(app: &mut BrApp, item: ItemId, stream: usize) -> Option<Arc<Vec<(f32, f32)>>> {
    let key = (item, stream);
    if let Some(p) = app.fc.tl.peaks.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Some(p.clone());
    }
    if app.fc.session.project.sequence(item).is_some() {
        return None;
    }
    let src = app.fc.session.source(item)?;
    {
        let mut pending = app.timeline.peaks_pending.lock().unwrap_or_else(|e| e.into_inner());
        if !pending.insert(key) {
            return None;
        }
    }
    let (out, pending) = (app.fc.tl.peaks.clone(), app.timeline.peaks_pending.clone());
    let dur = src.info().duration;
    let run = move || {
        let sr = 48_000u32;
        let total = dur.to_units_floor(sr as i64).max(0) as usize;
        let mut v = Vec::with_capacity(total / 256 + 1);
        let chunk = 48_000 * 4;
        let mut s = 0usize;
        while s < total {
            let n = chunk.min(total - s);
            match src.audio_stream(stream, s as i64, n, sr) {
                Ok(buf) => {
                    let ch = buf.channels.first().cloned().unwrap_or_default();
                    for c in ch.chunks(256) {
                        let (lo, hi) = c.iter().fold((0f32, 0f32), |(l, h), v| (l.min(*v), h.max(*v)));
                        v.push((lo, hi));
                    }
                }
                Err(_) => break,
            }
            s += n;
        }
        out.lock().unwrap_or_else(|e| e.into_inner()).insert(key, Arc::new(v));
        pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
    };
    if std::thread::Builder::new().name("br-peaks".into()).spawn(run).is_err() {
        app.timeline.peaks_pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
    }
    None
}

/// Presses, drags and releases on the timeline.
#[allow(clippy::too_many_arguments)]
fn interact(
    app: &mut BrApp,
    ui: &mut Ui,
    ctx: &egui::Context,
    rect: Rect,
    view: &View,
    ruler: egui::Response,
    hits: &[(ClipId, Rect, RowKind)],
    rows: &[Row],
) {
    let (pointer, pressed, released, down, mods, double) = ui.input(|i| {
        (
            i.pointer.interact_pos(),
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.primary_down(),
            i.modifiers,
            i.pointer.button_double_clicked(egui::PointerButton::Primary),
        )
    });
    let id = ui.id().with("br_tl_lane");
    let lane_rect = Rect::from_min_max(pos2(view.lane.min.x, view.lane.min.y), rect.max);
    let lane = ui.interact(lane_rect, id, Sense::click_and_drag());
    if (ruler.dragged() || ruler.is_pointer_button_down_on()) && down {
        app.timeline.drag = Some(Drag::Playhead);
    }
    // a press: on the ruler the playhead, on a clip that clip, elsewhere nothing selected
    if pressed && let Some(p) = pointer.filter(|p| rect.contains(*p)) {
        app.fc.ui.focused = filmcraft_ui_egui::dock::PanelKind::Timeline;
        if ruler.rect.contains(p) {
            app.fc.stop();
            app.timeline.drag = Some(Drag::Playhead);
        } else if lane_rect.contains(p) {
            let hit = hits.iter().rev().find(|h| h.1.contains(p));
            match hit {
                Some((clip, r, RowKind::Captions)) => {
                    app.fc.session.state.caption_selection = vec![*clip];
                    app.fc.session.state.selection.clear();
                    let _ = r;
                }
                Some((clip, r, kind)) => {
                    let main = *kind == RowKind::Main;
                    let already = app.fc.session.state.selection.contains(clip);
                    if mods.command || mods.shift {
                        crate::cut::run(app, ctx, "timeline.select", json!({"clips": [clip.0], "toggle": true}));
                    } else if !already {
                        crate::cut::run(app, ctx, "timeline.select", json!({"clips": [clip.0]}));
                    }
                    app.fc.session.state.caption_selection.clear();
                    app.timeline.drag = Some(match edge_at(*r, p.x) {
                        Some(edge) => Drag::Edge { clip: *clip, edge, from: p },
                        None => Drag::Clip { clip: *clip, grab: view.t(p.x) - view.t(r.min.x), from: p, moving: false, main },
                    });
                }
                None => {
                    // empty space: nothing selected, the playhead there
                    if !(mods.command || mods.shift) {
                        crate::cut::run(app, ctx, "timeline.select", json!({"clips": []}));
                        app.fc.session.state.caption_selection.clear();
                    }
                    app.fc.stop();
                    app.fc.session.set_playhead(Tick::from_seconds_f64(view.t(p.x)));
                }
            }
        }
    }
    // double click on a clip: its picture at the playhead
    if double
        && let Some(p) = pointer
        && let Some((clip, _, _)) = hits.iter().rev().find(|h| h.1.contains(p))
    {
        let _ = clip;
        app.fc.session.set_playhead(Tick::from_seconds_f64(view.t(p.x)));
    }
    // a drag going on
    if let (Some(drag), Some(p)) = (app.timeline.drag, pointer) {
        match drag {
            Drag::Playhead => {
                app.fc.session.set_playhead(Tick::from_seconds_f64(view.t(p.x)));
            }
            Drag::Clip { clip, grab, from, moving: false, main } if down && (p - from).length() > DRAG_PX => {
                app.timeline.drag = Some(Drag::Clip { clip, grab, from, moving: true, main });
            }
            _ => {}
        }
        if down {
            ctx.request_repaint();
        }
    }
    // a release ends the drag with one command
    if released || (!down && app.timeline.drag.is_some()) {
        let drag = app.timeline.drag.take();
        if let (Some(drag), Some(p)) = (drag, pointer) {
            finish(app, ctx, drag, p, view, rows);
        }
    }
    // hover cursors: edges trim, clips move
    if let Some(p) = pointer.filter(|p| lane_rect.contains(*p))
        && let Some((_, r, kind)) = hits.iter().rev().find(|h| h.1.contains(p))
        && *kind != RowKind::Captions
    {
        if edge_at(*r, p.x).is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        } else {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    let _ = lane;
}

/// The command a finished drag makes.
fn finish(app: &mut BrApp, ctx: &egui::Context, drag: Drag, p: Pos2, view: &View, rows: &[Row]) {
    let Some(q) = app.fc.session.active_sequence().cloned() else { return };
    let playhead = app.fc.session.playhead();
    match drag {
        Drag::Playhead => {}
        Drag::Clip { moving: false, .. } => {}
        Drag::Clip { clip, grab, main, .. } => {
            let row = rows.iter().find(|r| r.rect.min.y - GAP <= p.y && p.y < r.rect.max.y + GAP);
            let main_row = rows.iter().find(|r| r.kind == RowKind::Main);
            let on_main = main_row.is_some_and(|r| row.map(|x| x.track) == Some(r.track) || (main && (p.y - r.rect.center().y).abs() < MAIN_H));
            if main && on_main {
                // the cut it lands on: its place in the order
                let mains = main_track::main_clips(&q);
                let cuts: Vec<Tick> = mains.iter().map(|m| m.start).chain(mains.last().map(|m| m.end())).collect();
                let i = cuts
                    .iter()
                    .enumerate()
                    .min_by(|a, b| (view.x(*a.1) - p.x).abs().total_cmp(&(view.x(*b.1) - p.x).abs()))
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let from = mains.iter().position(|m| m.id == clip).unwrap_or(0);
                // dropped behind itself counts one place less (it leaves its own place first)
                let index = if i > from { i - 1 } else { i };
                if index != from {
                    crate::cut::run(app, ctx, "br.video.move", json!({"clip": clip.0, "index": index}));
                }
                return;
            }
            let pts = snap_points(&q, playhead, clip);
            let (start, _) = snap(&pts, view.t(p.x) - grab, view.pps);
            let target = match row {
                Some(r) if r.kind == RowKind::Main => json!("main"),
                Some(r) if matches!(r.kind, RowKind::Overlay | RowKind::Sound) => json!(r.track.0),
                // above the top row: a new overlay track; elsewhere where it was
                _ if rows.first().is_some_and(|r| p.y < r.rect.min.y) => json!("overlay"),
                _ => json!(q.find_item(clip).map(|(t, _)| t.0)),
            };
            let kind_ok = |tid: u64| {
                let want = q.find_item(clip).and_then(|(t, _)| q.track(t)).map(|t| t.kind);
                q.track(TrackId(tid)).map(|t| t.kind) == want
            };
            let target = match target.as_u64() {
                Some(tid) if !kind_ok(tid) => json!(q.find_item(clip).map(|(t, _)| t.0)),
                _ => target,
            };
            crate::cut::run(app, ctx, "br.video.move", json!({"clip": clip.0, "seconds": start.max(0.0), "track": target}));
        }
        Drag::Edge { clip, edge, from } => {
            let Some((_, it)) = q.find_item(clip) else { return };
            if (p.x - from.x).abs() < 1.0 {
                return;
            }
            let pts = snap_points(&q, playhead, clip);
            let (secs, _) = snap(&pts, view.t(p.x), view.pps);
            let delta = match edge {
                Edge::In => secs - it.start.seconds(),
                Edge::Out => secs - it.end().seconds(),
            };
            if delta.abs() > 1e-6 {
                let e = if edge == Edge::In { "in" } else { "out" };
                crate::cut::run(app, ctx, "br.video.trim", json!({"clip": clip.0, "edge": e, "seconds": delta}));
            }
        }
    }
}
