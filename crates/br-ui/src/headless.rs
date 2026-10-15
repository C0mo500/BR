//! BR's window without a window: frames are run in memory and painted on the CPU (LightCraft's
//! `softpaint`), for screenshots (`br snapshot`) and tests. Needs no display and no GPU.

use std::time::{Duration, Instant};

use lightcraft_ui_egui::headless::HeadlessView;

use crate::BrApp;

/// Drives a [`BrApp`] frame by frame.
pub struct Driver {
    pub view: HeadlessView,
    size: egui::Vec2,
    ppp: f32,
    time: f64,
    pointer: egui::Pos2,
}

impl Driver {
    /// A window of `size` points at `ppp` pixels per point.
    pub fn new(size: [f32; 2], ppp: f32) -> Self {
        Self { view: HeadlessView::new(), size: egui::vec2(size[0], size[1]), ppp, time: 0.0, pointer: egui::Pos2::ZERO }
    }

    /// One frame with these input events.
    pub fn step(&mut self, app: &mut BrApp, events: Vec<egui::Event>) {
        let mut raw = HeadlessView::raw_input(self.size, self.ppp, self.time, events);
        app.raw_input_hook(&self.view.ctx, &mut raw);
        self.view.run(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        self.time += 1.0 / 60.0;
    }

    /// One frame in which the window is asked to close (its close button).
    pub fn request_close(&mut self, app: &mut BrApp) {
        let mut raw = HeadlessView::raw_input(self.size, self.ppp, self.time, Vec::new());
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        app.raw_input_hook(&self.view.ctx, &mut raw);
        self.view.run(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        self.time += 1.0 / 60.0;
    }

    /// One frame in which `paths` are dropped on the window (at the pointer).
    pub fn drop_files(&mut self, app: &mut BrApp, paths: &[std::path::PathBuf]) {
        let mut raw = HeadlessView::raw_input(self.size, self.ppp, self.time, Vec::new());
        raw.dropped_files = paths.iter().map(|p| std::sync::Arc::new(crate::PathFile(p.clone())) as egui::DroppedFileHandle).collect();
        app.raw_input_hook(&self.view.ctx, &mut raw);
        self.view.run(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        self.time += 1.0 / 60.0;
    }

    /// Run frames until nothing has been in progress for a dozen frames (renders, an import, an
    /// export), or `timeout` passes. Returns whether it settled.
    pub fn settle(&mut self, app: &mut BrApp, timeout: Duration) -> bool {
        let start = Instant::now();
        let mut quiet = 0;
        // LightCraft's fonts and theme take effect from the second frame
        self.step(app, Vec::new());
        while start.elapsed() < timeout {
            self.step(app, Vec::new());
            if app.busy() {
                quiet = 0;
                std::thread::sleep(Duration::from_millis(3));
            } else {
                quiet += 1;
                if quiet >= 12 {
                    return true;
                }
            }
        }
        false
    }

    /// Click at `pos` (points): move, press, release, over three frames.
    pub fn click_at(&mut self, app: &mut BrApp, pos: egui::Pos2) {
        self.pointer = pos;
        self.step(app, vec![egui::Event::PointerMoved(pos)]);
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        self.step(app, vec![button(true)]);
        self.step(app, vec![button(false)]);
        self.step(app, Vec::new());
    }

    /// Click the control BR registered as `name` (see [`BrApp::widgets`]).
    pub fn click(&mut self, app: &mut BrApp, name: &str) -> Result<(), String> {
        let r = app.widgets.get(name).copied().ok_or_else(|| {
            let mut known: Vec<&String> = app.widgets.keys().collect();
            known.sort();
            format!("no control `{name}` on screen; known: {known:?}")
        })?;
        self.click_at(app, r.center());
        Ok(())
    }

    /// Drag with the primary button from `from` to `to` in `steps` moves.
    pub fn drag(&mut self, app: &mut BrApp, from: egui::Pos2, to: egui::Pos2, steps: usize) {
        let button =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        self.step(app, vec![egui::Event::PointerMoved(from)]);
        self.step(app, vec![button(from, true)]);
        for i in 1..=steps.max(1) {
            let p = from + (to - from) * (i as f32 / steps.max(1) as f32);
            self.step(app, vec![egui::Event::PointerMoved(p)]);
        }
        self.step(app, vec![button(to, false)]);
        self.step(app, Vec::new());
        self.pointer = to;
    }

    /// A key press (and release) with modifiers.
    pub fn key(&mut self, app: &mut BrApp, key: egui::Key, modifiers: egui::Modifiers) {
        let ev = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
        self.step(app, vec![ev(true)]);
        self.step(app, vec![ev(false)]);
    }

    /// The last frame, rasterised.
    pub fn paint(&self) -> egui::ColorImage {
        self.view.paint(&Default::default())
    }

    /// Text drawn in the last frame.
    pub fn texts(&self) -> Vec<String> {
        self.view.painted_text()
    }
}

/// `img` as PNG bytes.
pub fn png(img: &egui::ColorImage) -> Vec<u8> {
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    let Some(rgba) = lightcraft_raster::Rgba8::from_bytes(img.size[0], img.size[1], &bytes) else { return Vec::new() };
    lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&rgba), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
}
