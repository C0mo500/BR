//! BR's look: LightCraft's theme colours (so light and dark mode follow its settings) with BR's
//! accent, and the few widgets the simple editor is built from.

use std::collections::HashMap;

use egui::{Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use lightcraft_ui_egui::theme::Tokens;

use crate::Mode;

/// BR's accent: selection, primary buttons.
pub const ACCENT: Color32 = Color32::from_rgb(0x1f, 0xc8, 0xb4);
/// Text on [`ACCENT`].
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x06, 0x22, 0x1f);

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// Top bar and side panel.
    pub bar: Color32,
    /// Behind the photo.
    pub canvas: Color32,
    /// Filmstrip, tiles.
    pub well: Color32,
    pub button: Color32,
    pub hover: Color32,
    pub divider: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub disabled: Color32,
}

impl Theme {
    pub fn get(ctx: &egui::Context) -> Theme {
        let t = Tokens::get(ctx);
        Theme {
            bar: t.chrome,
            canvas: t.canvas,
            well: t.grid_bg,
            button: t.button,
            hover: t.hover,
            divider: t.divider,
            text: t.text,
            dim: t.text_dim,
            disabled: t.text_disabled,
        }
    }
}

/// "BR" wordmark.
pub fn brand(ui: &mut Ui, t: &Theme) {
    ui.label(RichText::new("BR").strong().size(17.0).color(ACCENT));
    ui.label(RichText::new("Foto").size(14.0).color(t.dim));
}

/// A rounded button. `primary` fills it with the accent.
pub fn button(ui: &mut Ui, t: &Theme, text: &str, primary: bool, enabled: bool) -> egui::Response {
    let (fill, color) = if primary { (ACCENT, ON_ACCENT) } else { (t.button, t.text) };
    let b = egui::Button::new(RichText::new(text).size(13.5).color(if enabled { color } else { t.disabled }))
        .fill(fill)
        .corner_radius(CornerRadius::same(7))
        .min_size(Vec2::new(0.0, 30.0));
    ui.add_enabled(enabled, b)
}

/// The "Einfach | Pro" switch; returns the mode clicked.
pub fn mode_switch(ui: &mut Ui, t: &Theme, current: Mode, widgets: &mut HashMap<String, Rect>) -> Option<Mode> {
    let mut clicked = None;
    egui::Frame::NONE.fill(t.well).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (mode, label, name) in [(Mode::Simple, "Einfach", "mode.simple"), (Mode::Pro, "Pro", "mode.pro")] {
            let on = current == mode;
            let b = egui::Button::new(RichText::new(label).size(12.5).color(if on { ON_ACCENT } else { t.text }))
                .fill(if on { ACCENT } else { Color32::TRANSPARENT })
                .stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(6))
                .min_size(Vec2::new(64.0, 22.0));
            let r = ui.add(b).on_hover_text(match mode {
                Mode::Simple => "Einfach bearbeiten: Looks, Regler, Formate, Export",
                Mode::Pro => "Alle Werkzeuge (LightCraft): Masken, Kurven, Bibliothek, …",
            });
            widgets.insert(name.into(), r.rect);
            if r.clicked() && !on {
                clicked = Some(mode);
            }
        }
    });
    clicked
}

/// A tab in the side panel's tab bar.
pub fn tab(ui: &mut Ui, t: &Theme, label: &str, on: bool, width: f32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click());
    let color = if on || r.hovered() { t.text } else { t.dim };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(13.5), color);
    if on {
        let line = Rect::from_min_max(egui::pos2(rect.left() + 10.0, rect.bottom() - 2.5), egui::pos2(rect.right() - 10.0, rect.bottom()));
        ui.painter().rect_filled(line, CornerRadius::same(1), ACCENT);
    }
    r
}

/// A small grey heading.
pub fn heading(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(11.5).color(t.dim).strong());
    ui.add_space(2.0);
}

/// Outline around a selected tile.
pub fn selected_outline(ui: &Ui, rect: Rect) {
    ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(8), Stroke::new(2.0, ACCENT), StrokeKind::Outside);
}

/// A card that can be selected: a box with a title and a detail line. Returns the response for
/// the whole card.
pub fn card(ui: &mut Ui, t: &Theme, title: &str, detail: &str, on: bool, width: f32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, 50.0), Sense::click());
    let fill = if on {
        t.button
    } else if r.hovered() {
        t.hover
    } else {
        t.well
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    if on {
        ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.5, ACCENT), StrokeKind::Inside);
    }
    let p = ui.painter();
    p.text(rect.left_top() + Vec2::new(12.0, 9.0), egui::Align2::LEFT_TOP, title, egui::FontId::proportional(13.5), t.text);
    p.text(rect.left_top() + Vec2::new(12.0, 28.0), egui::Align2::LEFT_TOP, detail, egui::FontId::proportional(11.5), t.dim);
    r
}

/// `text` on one line, cut with "…" when wider than `max_width`.
pub fn elided(ui: &Ui, text: &str, size: f32, color: Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping { max_width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.ctx().fonts_mut(|f| f.layout_job(job))
}

/// A wide primary button with its label centred.
pub fn wide_button(ui: &mut Ui, text: &str, width: f32, enabled: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, 40.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        ACCENT.gamma_multiply(0.45)
    } else if r.is_pointer_button_down_on() {
        ACCENT.gamma_multiply(0.85)
    } else if r.hovered() {
        ACCENT.gamma_multiply(1.1)
    } else {
        ACCENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(14.5), ON_ACCENT);
    r
}
