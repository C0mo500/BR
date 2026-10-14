//! Formats: Original, 1:1, 4:5, 9:16 and 16:9 crops.
//!
//! The crop is taken in the orientation the format names (9:16 is always portrait), centred, as
//! large as fits, and sized to whole pixels in the exact ratio (a whole multiple of 9 × 16 pixels),
//! so a 1080-wide export comes out at exactly 1080 × 1920. LightCraft's own `crop.aspect` instead
//! turns the ratio to the photo's orientation and keeps fractional pixels.

use lightcraft_engine::catalog::PhotoId;
use lightcraft_engine::develop::DevelopSettings;
use lightcraft_engine::{Result, Session};
use lightcraft_geom::{CropGeometry, Rect, crop_fit_angle};
use serde_json::{Value, json};

use crate::photo::{bad, oriented_dims};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// The whole photo, no crop.
    Original,
    /// 1:1
    Square,
    /// 4:5 portrait (Instagram feed).
    Portrait,
    /// 9:16 portrait (Stories, Reels, TikTok).
    Story,
    /// 16:9 landscape (YouTube, screens).
    Wide,
}

impl Format {
    pub const ALL: [Format; 5] = [Format::Original, Format::Square, Format::Portrait, Format::Story, Format::Wide];

    /// Name in commands: `original`, `1:1`, `4:5`, `9:16`, `16:9`.
    pub fn key(self) -> &'static str {
        match self {
            Format::Original => "original",
            Format::Square => "1:1",
            Format::Portrait => "4:5",
            Format::Story => "9:16",
            Format::Wide => "16:9",
        }
    }

    /// `1:1`, `1x1`, `4:5`, …, `original`, or a German or English name (`quadrat`, `story`, …).
    pub fn parse(s: &str) -> Option<Format> {
        let s = s.trim().to_lowercase().replace(['x', '/', '×'], ":");
        Some(match s.as_str() {
            "original" | "none" | "full" | "kein" | "ohne" => Format::Original,
            "1:1" | "square" | "quadrat" => Format::Square,
            "4:5" | "portrait" | "hochformat" | "instagram" => Format::Portrait,
            "9:16" | "story" | "reel" | "reels" | "tiktok" => Format::Story,
            "16:9" | "wide" | "breitbild" | "querformat" | "youtube" => Format::Wide,
            _ => return None,
        })
    }

    /// Width and height of the ratio, `None` for [`Format::Original`].
    pub fn ratio(self) -> Option<(u32, u32)> {
        match self {
            Format::Original => None,
            Format::Square => Some((1, 1)),
            Format::Portrait => Some((4, 5)),
            Format::Story => Some((9, 16)),
            Format::Wide => Some((16, 9)),
        }
    }

    /// Short German label.
    pub fn label(self) -> &'static str {
        match self {
            Format::Original => "Original",
            Format::Square => "1:1",
            Format::Portrait => "4:5",
            Format::Story => "9:16",
            Format::Wide => "16:9",
        }
    }

    /// What the format is for, in German.
    pub fn hint(self) -> &'static str {
        match self {
            Format::Original => "Ganzes Foto",
            Format::Square => "Quadrat",
            Format::Portrait => "Instagram-Feed",
            Format::Story => "Story, Reels, TikTok",
            Format::Wide => "YouTube, Bildschirm",
        }
    }
}

/// `g` in whole pixels and the exact ratio `aw:ah` (a whole multiple of `aw`×`ah` pixels), as large
/// as `g` allows, around `g`'s centre. `w`/`h` is the frame the crop rectangle is normalised to.
/// An unturned crop keeps its corner on whole pixels inside the photo; a turned one stays inside
/// `g` (see [`snap_within`]). `None` when `g` is smaller than one unit of the ratio.
pub fn snap(g: CropGeometry, w: f64, h: f64, ratio: (u32, u32)) -> Option<CropGeometry> {
    if g.angle != 0.0 {
        return snap_within(g, w, h, ratio);
    }
    let r = g.rect_px(w, h);
    let (aw, ah) = (ratio.0.max(1) as f64, ratio.1.max(1) as f64);
    // a hair of tolerance so a crop that is already exact does not lose a unit to float noise
    let m = ((r.width() + 1e-6) / aw).min((r.height() + 1e-6) / ah).floor();
    if m < 1.0 {
        return None;
    }
    let (cw, ch) = (m * aw, m * ah);
    let c = r.center();
    let x0 = (c.x - cw / 2.0).round().clamp(0.0, (w - cw).max(0.0));
    let y0 = (c.y - ch / 2.0).round().clamp(0.0, (h - ch).max(0.0));
    Some(CropGeometry { rect: Rect::new(x0, y0, x0 + cw, y0 + ch).scale(1.0 / w, 1.0 / h), angle: g.angle })
}

/// Like [`snap`], but the result always lies inside `g` (for turned crops, and crops LightCraft
/// constrained to a warped photo, where the area outside `g` may not be image).
pub fn snap_within(g: CropGeometry, w: f64, h: f64, (aw, ah): (u32, u32)) -> Option<CropGeometry> {
    let r = g.rect_px(w, h);
    let (aw, ah) = (aw.max(1) as f64, ah.max(1) as f64);
    let c = r.center();
    // where a side of `size` starts: on a whole pixel inside [lo, hi] if it can, else centred
    let place = |lo: f64, hi: f64, center: f64, size: f64| {
        let (min, max) = ((lo - 1e-6).ceil(), (hi + 1e-6).floor() - size);
        if min <= max { (center - size / 2.0).round().clamp(min, max) } else { center - size / 2.0 }
    };
    let mut m = ((r.width() + 1e-6) / aw).min((r.height() + 1e-6) / ah).floor();
    while m >= 1.0 {
        let (cw, ch) = (m * aw, m * ah);
        let (x0, y0) = (place(r.x0, r.x1, c.x, cw), place(r.y0, r.y1, c.y, ch));
        let inside = x0 >= r.x0 - 1e-6 && x0 + cw <= r.x1 + 1e-6 && y0 >= r.y0 - 1e-6 && y0 + ch <= r.y1 + 1e-6;
        if inside {
            return Some(CropGeometry { rect: Rect::new(x0, y0, x0 + cw, y0 + ch).scale(1.0 / w, 1.0 / h), angle: g.angle });
        }
        m -= 1.0;
    }
    None
}

/// The largest centred crop of `format` in a `w`×`h` frame at `angle` degrees, in whole pixels
/// (on a photo smaller than one unit of the ratio, the exact ratio without whole pixels).
pub fn fit(w: f64, h: f64, angle: f64, format: Format) -> CropGeometry {
    match format.ratio() {
        None => crop_fit_angle(w, h, angle, None),
        Some((aw, ah)) => {
            let g = crop_fit_angle(w, h, angle, Some(aw as f64 / ah as f64));
            snap(g, w, h, (aw, ah)).unwrap_or(g)
        }
    }
}

/// `d` cropped to `format` (keeping its straighten angle), with the ratio locked for Pro mode.
pub fn with_format(d: &DevelopSettings, w: f64, h: f64, format: Format) -> DevelopSettings {
    let mut d = d.clone();
    d.crop.geometry = fit(w, h, d.crop.geometry.angle, format);
    d.crop.aspect = format.ratio().map(|(aw, ah)| (aw * 100, ah * 100));
    d
}

/// The format `d`'s crop has, if it is one of ours (within half a pixel).
pub fn detect(d: &DevelopSettings, w: f64, h: f64) -> Option<Format> {
    let g = d.crop.geometry;
    let r = g.rect_px(w, h);
    let full = crop_fit_angle(w, h, g.angle, None).rect_px(w, h);
    if (r.width() - full.width()).abs() < 0.5 && (r.height() - full.height()).abs() < 0.5 {
        return Some(Format::Original);
    }
    let a = r.width() / r.height().max(1e-9);
    Format::ALL.into_iter().filter(|f| *f != Format::Original).find(|f| {
        let (aw, ah) = f.ratio().unwrap_or((1, 1));
        // half a pixel on the shorter side
        (a - aw as f64 / ah as f64).abs() <= 0.5 / r.width().min(r.height()).max(1.0) * a.max(1.0)
    })
}

/// The format of photo `id`'s current crop.
pub fn current(s: &Session, id: PhotoId) -> Option<Format> {
    let (w, h) = oriented_dims(s, id)?;
    detect(&*s.develop_of(id)?, w, h)
}

/// Crop photo `id` to `format` as one undo step ("Format 4:5").
pub fn apply(s: &mut Session, id: PhotoId, format: Format) -> Result<Value> {
    s.execute_fn("br.crop", |s| {
        let (w, h) = oriented_dims(s, id).ok_or_else(|| bad("br.crop", format!("no such photo {}", id.0)))?;
        let d = s.develop_of(id).ok_or_else(|| bad("br.crop", "no photo"))?;
        let next = with_format(&d, w, h, format);
        if next != *d {
            s.set_develop(id, next, &format!("Format {}", format.label()))?;
        }
        let r = s.develop_of(id).map(|d| d.crop.geometry.rect_px(w, h)).unwrap_or(Rect::UNIT);
        Ok(json!({"id": id.0, "format": format.key(), "width": r.width().round(), "height": r.height().round()}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: [(f64, f64); 9] = [
        (6000.0, 4000.0),
        (4000.0, 6000.0),
        (4032.0, 3024.0),
        (1003.0, 752.0),
        (752.0, 1003.0),
        (2000.0, 1003.0),
        (1080.0, 1080.0),
        (5472.0, 3648.0),
        (321.0, 457.0),
    ];

    #[test]
    fn formats_are_whole_pixels_in_the_exact_ratio_and_orientation() {
        for (w, h) in SIZES {
            for f in Format::ALL {
                let g = fit(w, h, 0.0, f);
                let r = g.rect_px(w, h);
                assert!(g.is_within_image(w, h), "{f:?} {w}x{h}");
                assert!(r.x0 >= -1e-9 && r.y0 >= -1e-9 && r.x1 <= w + 1e-9 && r.y1 <= h + 1e-9);
                for v in [r.x0, r.y0, r.width(), r.height()] {
                    assert!((v - v.round()).abs() < 1e-6, "{f:?} {w}x{h}: {v}");
                }
                match f.ratio() {
                    None => assert!((r.width() - w).abs() < 1e-6 && (r.height() - h).abs() < 1e-6, "{w}x{h}: {r:?}"),
                    Some((aw, ah)) => {
                        let m = r.width() / aw as f64;
                        assert!((m - m.round()).abs() < 1e-6 && (r.height() - m * ah as f64).abs() < 1e-6, "{f:?} {w}x{h}: {r:?}");
                        // as large as the photo allows: one more unit would not fit
                        assert!((m + 1.0) * aw as f64 > w + 1e-6 || (m + 1.0) * ah as f64 > h + 1e-6, "{f:?} {w}x{h} not maximal");
                    }
                }
                let mut d = DevelopSettings::default();
                d.crop.geometry = g;
                // a photo already in that ratio is not cropped at all: that is "Original"
                let whole = (r.width() - w).abs() < 0.5 && (r.height() - h).abs() < 0.5;
                assert_eq!(detect(&d, w, h), Some(if whole { Format::Original } else { f }), "{w}x{h}");
            }
        }
    }

    #[test]
    fn snapping_stays_inside_the_crop_it_starts_from() {
        for (w, h) in SIZES {
            for f in Format::ALL.into_iter().filter(|f| f.ratio().is_some()) {
                let ratio = f.ratio().unwrap();
                // an off-centre crop with fractional edges, roughly in the ratio
                let g = CropGeometry { rect: Rect::new(0.13, 0.071, 0.77, 0.93), angle: 0.0 };
                let Some(s) = snap_within(g, w, h, ratio) else { continue };
                let (r, o) = (g.rect_px(w, h), s.rect_px(w, h));
                assert!(o.x0 >= r.x0 - 1e-6 && o.x1 <= r.x1 + 1e-6 && o.y0 >= r.y0 - 1e-6 && o.y1 <= r.y1 + 1e-6, "{f:?} {w}x{h}");
                assert!((o.width() * ratio.1 as f64 - o.height() * ratio.0 as f64).abs() < 1e-6);
            }
        }
        // smaller than one unit of 9:16
        let g = CropGeometry { rect: Rect::UNIT, angle: 0.0 };
        assert!(snap(g, 8.0, 12.0, (9, 16)).is_none());
        assert!(fit(8.0, 12.0, 0.0, Format::Story).is_within_image(8.0, 12.0));
    }

    #[test]
    fn a_turned_crop_stays_inside_the_photo() {
        for (w, h) in SIZES {
            for f in Format::ALL {
                let g = fit(w, h, 7.5, f);
                assert!(g.is_within_image(w, h), "{f:?} {w}x{h}");
            }
        }
    }

    #[test]
    fn formats_parse() {
        assert_eq!(Format::parse("9x16"), Some(Format::Story));
        assert_eq!(Format::parse(" Quadrat "), Some(Format::Square));
        assert_eq!(Format::parse("16/9"), Some(Format::Wide));
        assert_eq!(Format::parse("3:2"), None);
        for f in Format::ALL {
            assert_eq!(Format::parse(f.key()), Some(f));
        }
    }

    #[test]
    fn apply_is_one_undo_step_and_locks_the_ratio() {
        let mut s = Session::with_demo();
        let id = s.active().unwrap();
        let undo0 = s.undo.len();
        let r = apply(&mut s, id, Format::Story).unwrap();
        assert_eq!(s.undo.len(), undo0 + 1);
        assert_eq!(s.undo.last().unwrap().label, "Format 9:16");
        let (w, h) = (r["width"].as_f64().unwrap(), r["height"].as_f64().unwrap());
        assert_eq!(w * 16.0, h * 9.0);
        assert_eq!(current(&s, id), Some(Format::Story));
        assert_eq!(s.develop_of(id).unwrap().crop.aspect, Some((900, 1600)));
        apply(&mut s, id, Format::Original).unwrap();
        assert_eq!(current(&s, id), Some(Format::Original));
        assert_eq!(s.develop_of(id).unwrap().crop.aspect, None);
    }
}
