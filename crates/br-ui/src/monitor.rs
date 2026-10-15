//! The Program monitor, CapCut's way, on top of FilmCraft's: a clip selected anywhere shows its
//! transform box at once (FilmCraft shows it only once Motion is the selected effect), and a
//! single click on the picture selects the clip under the pointer (FilmCraft wants a double
//! click). Moving, scaling and rotating stay FilmCraft's own handles: keyframes at the playhead,
//! one undo step per drag, Escape.

use egui::Pos2;
use filmcraft_geom::Vec2;
use filmcraft_project::{ClipId, ItemKind};
use serde_json::json;

use crate::BrApp;

/// What the monitor remembers between frames.
#[derive(Default)]
pub struct MonitorState {
    /// The selection the box was last shown for.
    seen: Vec<ClipId>,
}

fn rect_of(app: &BrApp, id: &str) -> Option<egui::Rect> {
    app.fc.auto.find(id).map(|e| egui::Rect::from_min_size(egui::pos2(e.rect[0], e.rect[1]), egui::vec2(e.rect[2], e.rect[3])))
}

/// After FilmCraft drew the window: the box for a newly selected clip, a click on the picture.
pub fn after(app: &mut BrApp, ctx: &egui::Context) {
    if !crate::timeline::visible(app) {
        return;
    }
    click(app, ctx);
    show_box(app);
}

/// A clip selected on its own (in BR's timeline, FilmCraft's or by a command): its transform box.
fn show_box(app: &mut BrApp) {
    let sel = app.fc.session.state.selection.clone();
    if sel == app.monitor.seen {
        return;
    }
    app.monitor.seen = sel.clone();
    let Some(q) = app.fc.session.active_sequence() else { return };
    // the picture of the selection: one video clip (its sound may be selected with it)
    let pictures: Vec<ClipId> = sel.iter().copied().filter(|c| q.video_tracks.iter().any(|t| t.item(*c).is_some())).collect();
    let [c] = pictures[..] else { return };
    let Some((_, it)) = q.find_item(c) else { return };
    if br_video::main_track::is_background(it) || it.graphic_layers().next().is_some() {
        return;
    }
    if app.fc.session.state.selected_effect.as_ref().is_some_and(|e| e.clip == c) {
        return;
    }
    let _ = app.fc.session.execute("effects.select", json!({"clip": c.0, "effect": "motion"}));
}

/// A click on the Program picture with the Selection tool: the topmost clip there is selected
/// (nothing, on an empty spot). Clicks on FilmCraft's handles and layers are FilmCraft's.
fn click(app: &mut BrApp, ctx: &egui::Context) {
    let clicked = ctx.input(|i| i.pointer.primary_clicked() && !i.modifiers.any());
    if !clicked || app.fc.ui.program.zoom.is_some() || app.fc.ui.tool != filmcraft_ui_egui::state::Tool::Selection {
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    // a popover, menu or floating panel over the picture took the click
    if ctx.layer_id_at(p).is_some_and(|l| l.order != egui::Order::Background) {
        return;
    }
    let Some(pic) = rect_of(app, "program.picture").filter(|r| r.contains(p) && r.width() > 1.0) else { return };
    // the box, its handles and text layers belong to FilmCraft
    let on_handles = app.fc.auto.elements.iter().any(|e| {
        (e.id.starts_with("program.transform.") || e.id.starts_with("program.layer.") || e.id.starts_with("program.mask"))
            && egui::Rect::from_min_size(egui::pos2(e.rect[0], e.rect[1]), egui::vec2(e.rect[2], e.rect[3])).contains(p)
    });
    if on_handles {
        return;
    }
    let hit = clip_at(app, pic, p);
    let now = app.fc.session.state.selection.clone();
    match hit {
        Some(c) if now.contains(&c) => {}
        Some(c) => {
            crate::cut::run(app, ctx, "timeline.select", json!({"clips": [c.0]}));
        }
        None if !now.is_empty() => {
            crate::cut::run(app, ctx, "timeline.select", json!({"clips": []}));
            let _ = app.fc.session.execute("effects.select", json!({"none": true}));
        }
        None => {}
    }
}

/// The topmost clip whose picture covers point `p` of the monitor's picture `pic` at the playhead
/// (a background stands for its main clip; a title only where its text is).
pub fn clip_at(app: &mut BrApp, pic: egui::Rect, p: Pos2) -> Option<ClipId> {
    let titles: Vec<(ClipId, Vec<[Vec2; 4]>)> = {
        let q = app.fc.session.active_sequence()?;
        let t = app.fc.session.playhead();
        let ids: Vec<ClipId> = q
            .video_tracks
            .iter()
            .flat_map(|tr| &tr.items)
            .filter(|it| it.start <= t && t < it.end() && it.graphic_layers().next().is_some())
            .map(|it| it.id)
            .collect();
        ids.into_iter().map(|c| (c, graphic_quads(app, c))).collect()
    };
    let s = &app.fc.session;
    let q = s.active_sequence()?;
    let (w, h) = (q.settings.width as f64, q.settings.height as f64);
    let at = Vec2::new((p.x - pic.min.x) as f64 * w / pic.width() as f64, (p.y - pic.min.y) as f64 * h / pic.height() as f64);
    let t = s.playhead();
    for track in q.video_tracks.iter().rev().filter(|t| t.enabled) {
        for it in track.items.iter().filter(|it| it.enabled && it.start <= t && t < it.end()) {
            let Some(size) = filmcraft_render::source_size(&s.project, it.item) else { continue };
            let m = filmcraft_render::motion_matrix(q, it, size, None, it.source_time_at(t));
            let graphic = matches!(s.project.item(it.item).map(|i| &i.kind), Some(ItemKind::Graphic { .. }));
            // in the clip's own pixels: the whole picture, or a title's text
            let quads: Vec<[Vec2; 4]> = if graphic {
                titles.iter().find(|x| x.0 == it.id).map(|x| x.1.clone()).unwrap_or_default()
            } else {
                vec![[Vec2::new(0.0, 0.0), Vec2::new(size.0 as f64, 0.0), Vec2::new(size.0 as f64, size.1 as f64), Vec2::new(0.0, size.1 as f64)]]
            };
            let inside = quads.iter().any(|quad| {
                let pts: Vec<Vec2> = quad.iter().map(|v| m.apply(*v)).collect();
                in_convex(&pts, at)
            });
            if inside {
                if br_video::main_track::is_background(it) {
                    return br_video::main_track::main_of(q, it.id);
                }
                return Some(it.id);
            }
        }
    }
    None
}

/// A title's layers in its canvas pixels.
fn graphic_quads(app: &mut BrApp, clip: ClipId) -> Vec<[Vec2; 4]> {
    // graphics.list is a query: it changes nothing
    let g = app.fc.session.execute("graphics.list", json!({"clip": clip.0})).unwrap_or_default();
    g["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let q = l["quad"].as_array()?;
            let v = |i: usize| Some(Vec2::new(q.get(i)?[0].as_f64()?, q.get(i)?[1].as_f64()?));
            Some([v(0)?, v(1)?, v(2)?, v(3)?])
        })
        .collect()
}

/// Whether `p` lies inside the convex polygon `pts` (either winding).
fn in_convex(pts: &[Vec2], p: Vec2) -> bool {
    let mut sign = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
        if cross.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    sign != 0.0
}
