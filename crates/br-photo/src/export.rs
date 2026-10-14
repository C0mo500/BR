//! Export presets: Instagram, Story, Quadrat, Breitbild and full resolution.
//!
//! A preset with a format crops the export to that format without changing the photo's edit: if
//! the photo already has that format, its own crop is used (snapped to whole pixels), otherwise
//! the largest centred crop. The crop is swapped into the catalog only while the export is
//! prepared and restored right after, with no undo step, history entry or save in between.
//! Sizes are exact: an Instagram export is always 1080 × 1350, also from smaller photos.
//!
//! [`prepare`] needs the session; [`Plan::run`] does the heavy part (decode, render, encode,
//! write) without it, so the app can run it on another thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lightcraft_engine::Session;
use lightcraft_engine::catalog::{Op, PhotoId};
use lightcraft_engine::develop::DevelopSettings;
use lightcraft_engine::export::{Destination, ExportOptions, PreparedExport, prepare_batch, run_batch, write_file};
use lightcraft_engine::pipeline::geometry::Frame;
use serde_json::{Value, json};

use crate::crop::{self, Format};
use crate::photo::oriented_dims;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    /// 4:5, 1080 × 1350 JPEG.
    Instagram,
    /// 9:16, 1080 × 1920 JPEG.
    Story,
    /// 1:1, 1080 × 1080 JPEG.
    Square,
    /// 16:9, 1920 × 1080 JPEG.
    Wide,
    /// The photo as edited, full size, high-quality JPEG.
    Full,
}

impl Preset {
    pub const ALL: [Preset; 5] = [Preset::Instagram, Preset::Story, Preset::Square, Preset::Wide, Preset::Full];

    /// Name in commands.
    pub fn key(self) -> &'static str {
        match self {
            Preset::Instagram => "instagram",
            Preset::Story => "story",
            Preset::Square => "square",
            Preset::Wide => "wide",
            Preset::Full => "full",
        }
    }

    /// The key, or a German or common name (`quadrat`, `tiktok`, `reels`, `voll`, …).
    pub fn parse(s: &str) -> Option<Preset> {
        Some(match s.trim().to_lowercase().as_str() {
            "instagram" | "insta" | "feed" | "4:5" => Preset::Instagram,
            "story" | "stories" | "tiktok" | "reel" | "reels" | "9:16" => Preset::Story,
            "square" | "quadrat" | "1:1" => Preset::Square,
            "wide" | "breitbild" | "youtube" | "16:9" => Preset::Wide,
            "full" | "voll" | "original" | "volle auflösung" | "jpeg" => Preset::Full,
            _ => return None,
        })
    }

    /// German name shown in the app.
    pub fn label(self) -> &'static str {
        match self {
            Preset::Instagram => "Instagram",
            Preset::Story => "Story / TikTok / Reels",
            Preset::Square => "Quadrat",
            Preset::Wide => "Breitbild",
            Preset::Full => "Volle Auflösung",
        }
    }

    /// Format and size, in German.
    pub fn detail(self) -> &'static str {
        match self {
            Preset::Instagram => "4:5 · 1080 × 1350 · JPEG",
            Preset::Story => "9:16 · 1080 × 1920 · JPEG",
            Preset::Square => "1:1 · 1080 × 1080 · JPEG",
            Preset::Wide => "16:9 · 1920 × 1080 · JPEG",
            Preset::Full => "Originalgröße · JPEG",
        }
    }

    /// The format the export is cropped to, if any.
    pub fn format(self) -> Option<Format> {
        match self {
            Preset::Instagram => Some(Format::Portrait),
            Preset::Story => Some(Format::Story),
            Preset::Square => Some(Format::Square),
            Preset::Wide => Some(Format::Wide),
            Preset::Full => None,
        }
    }

    /// The exact output size, if the preset has one.
    pub fn size(self) -> Option<(u32, u32)> {
        match self {
            Preset::Instagram => Some((1080, 1350)),
            Preset::Story => Some((1080, 1920)),
            Preset::Square => Some((1080, 1080)),
            Preset::Wide => Some((1920, 1080)),
            Preset::Full => None,
        }
    }

    /// LightCraft export options for this preset.
    pub fn options(self) -> Result<ExportOptions, String> {
        let mut p = json!({
            "format": "jpeg",
            "quality": if self == Preset::Full { 92 } else { 90 },
            "colorSpace": "srgb",
            "naming": format!("{{name}}-{}", self.key()),
        });
        if let Some((w, _)) = self.size() {
            // the crop already has the exact ratio: fixing the width fixes the height, and small
            // photos are scaled up so the size is always the one promised
            p["width"] = json!(w);
            p["dontEnlarge"] = json!(false);
        }
        ExportOptions::from_params(&p).map_err(|e| e.to_string())
    }
}

/// Where an export goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A folder; the file is named `<photo>-<preset>.jpg` (with `-2`, `-3`, … if it exists).
    Dir(PathBuf),
    /// This exact file.
    File(PathBuf),
}

/// BR's export folder: `$BR_EXPORTS`, else `~/Pictures/BR Exports`.
pub fn default_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("BR_EXPORTS").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Pictures").join("BR Exports")
}

/// A prepared export: everything [`Plan::run`] needs, without the session.
pub struct Plan {
    pub photo: PhotoId,
    pub preset: Preset,
    items: Vec<PreparedExport>,
    opts: ExportOptions,
    dest: Destination,
}

impl Plan {
    /// Render, encode and write the file. Returns LightCraft's result per file:
    /// `{path, width, height, bytes, …}`.
    pub fn run(self) -> Result<Vec<Value>, String> {
        let mut write = |path: &str, bytes: &[u8]| write_file(path, bytes);
        let exists = |path: &str| Path::new(path).exists();
        run_batch(self.items, &self.opts, &self.dest, &mut write, &exists, true, &mut |_, _| true)
    }
}

/// Settings photo `id` is exported with under `preset`, or `None` to export it as it is.
///
/// The crop is the photo's own when it already has the preset's format, else the largest centred
/// one; either way in whole pixels and the exact ratio. With Constrain Crop and a perspective or
/// lens warp, LightCraft would shrink the crop by a fraction of a pixel at render time; the
/// constrained crop is worked out here instead and snapped again, so the size stays exact.
pub fn export_settings(s: &Session, id: PhotoId, preset: Preset) -> Result<Option<DevelopSettings>, String> {
    let Some(format) = preset.format() else { return Ok(None) };
    let Some((aw, ah)) = format.ratio() else { return Ok(None) };
    let p = s.catalog.photo(id).ok_or_else(|| format!("no such photo {}", id.0))?;
    let (w, h) = oriented_dims(s, id).ok_or_else(|| format!("no such photo {}", id.0))?;
    let d = p.develop.clone();
    let mut out = (*d).clone();
    let mut g = if crop::detect(&d, w, h) == Some(format) {
        d.crop.geometry
    } else {
        lightcraft_geom::crop_fit_angle(w, h, d.crop.geometry.angle, Some(aw as f64 / ah as f64))
    };
    let constrained = out.geometry.constrain_crop;
    if constrained {
        out.crop.geometry = g;
        let info = s.source_info(id);
        g = Frame::with_lens(p.width as usize, p.height as usize, &out, true, info.lens.as_ref()).crop;
        out.geometry.constrain_crop = false;
    }
    let snapped = if constrained { crop::snap_within(g, w, h, (aw, ah)) } else { crop::snap(g, w, h, (aw, ah)) };
    out.crop.geometry = snapped.ok_or_else(|| format!("the photo is too small for {} (it needs at least {aw} × {ah} pixels)", format.label()))?;
    out.crop.aspect = Some((aw * 100, ah * 100));
    Ok(Some(out))
}

/// Set up the export of photo `id` with `preset` to `target`. Cheap: no pixels are read yet.
pub fn prepare(s: &mut Session, id: PhotoId, preset: Preset, target: &Target) -> Result<Plan, String> {
    if s.catalog.photo(id).is_none() {
        return Err(format!("no such photo {}", id.0));
    }
    let opts = preset.options()?;
    let dest = match target {
        Target::Dir(dir) => Destination { dir: dir.to_string_lossy().into_owned(), exact: None },
        Target::File(path) => {
            let path = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
                Some("jpg" | "jpeg") => path.clone(),
                None => path.with_extension("jpg"),
                Some(other) => return Err(format!("{}: BR exports JPEG; use a .jpg or .jpeg file name (not .{other})", path.display())),
            };
            let path = path.to_string_lossy().into_owned();
            s.check_write_target(&path)?;
            let dir = Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            Destination { dir, exact: Some(path) }
        }
    };
    let items = match export_settings(s, id, preset)? {
        None => prepare_batch(s, &[id], &opts)?,
        Some(temp) => {
            // the export's crop, only while LightCraft sets the export up
            let edited = s.catalog.photo(id).and_then(|p| p.edited.clone());
            let inverse =
                s.catalog.apply(Op::SetDevelop { id, settings: Arc::new(temp), label: "BR export".into(), edited }).map_err(|e| e.to_string())?;
            let items = prepare_batch(s, &[id], &opts);
            s.catalog.apply(inverse).map_err(|e| e.to_string())?;
            items?
        }
    };
    Ok(Plan { photo: id, preset, items, opts, dest })
}

/// [`prepare`] and [`Plan::run`] in one go.
pub fn export(s: &mut Session, id: PhotoId, preset: Preset, target: &Target) -> Result<Vec<Value>, String> {
    prepare(s, id, preset, target)?.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("br-photo-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A JPEG of `w`×`h` on disk, imported into a fresh session.
    fn session_with_jpeg(dir: &Path, w: usize, h: usize) -> (Session, PhotoId) {
        let mut img = lightcraft_raster::Rgba8 { width: w, height: h, data: vec![[0, 0, 0, 255]; w * h] };
        for y in 0..h {
            for x in 0..w {
                img.data[y * w + x] = [(x * 255 / w) as u8, (y * 255 / h) as u8, 128, 255];
            }
        }
        let opts = ExportOptions::from_params(&json!({"format": "jpeg", "quality": 90})).unwrap();
        let bytes = lightcraft_engine::export::encode_image(&img, &opts).unwrap();
        let path = dir.join(format!("in-{w}x{h}.jpg"));
        std::fs::write(&path, bytes).unwrap();
        let mut s = Session::new().with_fs();
        let r = s.execute("library.import", &json!({"paths": [path.to_string_lossy()]})).unwrap();
        let id = PhotoId(r["imported"][0].as_u64().unwrap());
        (s, id)
    }

    #[test]
    fn presets_parse_and_have_exact_sizes() {
        for p in Preset::ALL {
            assert_eq!(Preset::parse(p.key()), Some(p));
            assert!(p.options().is_ok());
            if let (Some((w, h)), Some(f)) = (p.size(), p.format()) {
                let (aw, ah) = f.ratio().unwrap();
                assert_eq!(w * ah, h * aw, "{p:?}");
            }
        }
        assert_eq!(Preset::parse("TikTok"), Some(Preset::Story));
    }

    #[test]
    fn social_presets_come_out_at_exactly_their_size_and_leave_the_edit_alone() {
        lightcraft_engine::gpu::set_enabled(false);
        let dir = temp_dir("exact");
        // odd sizes that LightCraft's own aspect crop exports one pixel off
        for (w, h) in [(1003, 752), (752, 1003), (640, 480)] {
            let (mut s, id) = session_with_jpeg(&dir, w, h);
            let before = s.catalog.photo(id).unwrap().clone();
            let (undo0, commits0) = (s.undo.len(), s.commits());
            for preset in [Preset::Instagram, Preset::Story, Preset::Square, Preset::Wide] {
                let files = export(&mut s, id, preset, &Target::Dir(dir.clone())).unwrap();
                let f = &files[0];
                let (ew, eh) = preset.size().unwrap();
                assert_eq!((f["width"].as_u64().unwrap(), f["height"].as_u64().unwrap()), (ew as u64, eh as u64), "{preset:?} from {w}x{h}");
                let path = f["path"].as_str().unwrap();
                assert!(path.ends_with(&format!("-{}.jpg", preset.key())), "{path}");
                assert!(Path::new(path).exists());
            }
            let after = s.catalog.photo(id).unwrap();
            assert_eq!(after.develop, before.develop);
            assert_eq!(after.edited, before.edited);
            assert_eq!((s.undo.len(), s.commits()), (undo0, commits0));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_matching_crop_keeps_its_framing() {
        lightcraft_engine::gpu::set_enabled(false);
        let dir = temp_dir("framing");
        let (mut s, id) = session_with_jpeg(&dir, 1200, 800);
        // a 4:5 crop at the left edge, slightly off whole pixels
        let mut d = (*s.develop_of(id).unwrap()).clone();
        d.crop.geometry.rect = lightcraft_geom::Rect::new(0.0, 0.0, 400.4 / 1200.0, 500.5 / 800.0);
        s.set_develop(id, d, "test").unwrap();
        let t = export_settings(&s, id, Preset::Instagram).unwrap().unwrap();
        let r = t.crop.geometry.rect_px(1200.0, 800.0);
        assert!((r.width() - 400.0).abs() < 1e-6 && (r.height() - 500.0).abs() < 1e-6, "{r:?}");
        assert!(r.x0 < 1.0, "kept at the left edge: {r:?}");
        let files = export(&mut s, id, Preset::Instagram, &Target::File(dir.join("exact name.jpg"))).unwrap();
        assert!(files[0]["path"].as_str().unwrap().ends_with("exact name.jpg"));
        assert_eq!(files[0]["height"], 1350);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn constrain_crop_with_a_perspective_fix_keeps_the_size_exact() {
        lightcraft_engine::gpu::set_enabled(false);
        let dir = temp_dir("constrain");
        for ((w, h), vertical) in [((1600, 1067), 10.0), ((752, 1003), 25.0), ((752, 1003), -40.0), ((1003, 752), 25.0)] {
            let (mut s, id) = session_with_jpeg(&dir, w, h);
            let mut d = (*s.develop_of(id).unwrap()).clone();
            assert!(lightcraft_engine::develop::controls::set(&mut d, "geometry.vertical", vertical));
            d.geometry.constrain_crop = true;
            s.set_develop(id, d, "test").unwrap();
            for preset in [Preset::Instagram, Preset::Story, Preset::Wide] {
                let f = &export(&mut s, id, preset, &Target::Dir(dir.clone())).unwrap()[0];
                let (ew, eh) = preset.size().unwrap();
                assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(ew as u64), Some(eh as u64)), "{preset:?} {w}x{h} v{vertical}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_photo_too_small_for_the_format_is_refused() {
        let dir = temp_dir("tiny");
        let (mut s, id) = session_with_jpeg(&dir, 12, 12);
        let e = prepare(&mut s, id, Preset::Story, &Target::Dir(dir.clone())).err().unwrap();
        assert!(e.contains("too small"), "{e}");
        assert!(prepare(&mut s, id, Preset::Square, &Target::Dir(dir.clone())).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_exact_file_is_a_jpeg_file() {
        lightcraft_engine::gpu::set_enabled(false);
        let dir = temp_dir("names");
        let (mut s, id) = session_with_jpeg(&dir, 400, 300);
        let e = prepare(&mut s, id, Preset::Square, &Target::File(dir.join("out.png"))).err().unwrap();
        assert!(e.contains(".jpg"), "{e}");
        let files = export(&mut s, id, Preset::Square, &Target::File(dir.join("noext"))).unwrap();
        assert!(files[0]["path"].as_str().unwrap().ends_with("noext.jpg"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
