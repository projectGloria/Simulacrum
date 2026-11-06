//! The one image pipeline used by preview *and* export, so what you see is what you write.
//!
//! load → centre-crop → (≤1024 source) → compose master (pad + mask, 1024², premultiplied f32)
//!      → render each size straight from the master with Lanczos3.

use std::io::{Cursor, Read};
use std::path::Path;

use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::{self, FilterType};
use image::{
    DynamicImage, ImageDecoder, ImageEncoder, ImageError, ImageFormat, ImageReader, Rgba,
    Rgba32FImage, RgbaImage,
};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::mask::rounded_square_coverage;

/// Side of the masked master every output is resampled from. 4× the largest ICO entry and 2×
/// the 512 PNG — plenty of headroom for Lanczos while keeping a slider-drag re-render cheap.
pub const MASTER_SIDE: u32 = 1024;
/// Below this the crop is rejected outright: even a 16px icon would be pure upscaling mush.
pub const MIN_SIDE: u32 = 16;
/// Below this we still work, but warn that the large ICO entries are upscaled.
pub const WARN_SIDE: u32 = 256;
pub const ALLOWED_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
pub const PNG_EXPORT_SIDE: u32 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Rounded,
    Circle,
    None,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub shape: Shape,
    /// Corner radius, percent of side (0–50). Ignored unless `shape == Rounded`.
    pub radius: f32,
    /// Transparent margin on *each* side, percent of side (0–30).
    pub padding: f32,
    pub sizes: Vec<u32>,
}

impl Settings {
    /// Never trust the webview: clamp/validate here so a tampered or stale localStorage value
    /// can't produce a 0-entry or >256 ICO.
    pub fn validated(&self) -> AppResult<Settings> {
        if !self.radius.is_finite() || !self.padding.is_finite() {
            return Err(AppError::InvalidSettings("radius/padding must be numbers".into()));
        }
        let mut sizes: Vec<u32> =
            self.sizes.iter().copied().filter(|s| ALLOWED_SIZES.contains(s)).collect();
        sizes.sort_unstable();
        sizes.dedup();
        if sizes.is_empty() {
            return Err(AppError::InvalidSettings("select at least one icon size".into()));
        }
        Ok(Settings {
            shape: self.shape,
            radius: self.radius.clamp(0.0, 50.0),
            padding: self.padding.clamp(0.0, 30.0),
            sizes,
        })
    }
}

/// A decoded, square-cropped image, capped at `MASTER_SIDE`. Kept as straight-alpha u8 to use
/// 4 bytes/px in the cache rather than 16 for f32 — matters for 50-image batches.
pub struct Source {
    pub img: RgbaImage,
    pub orig_w: u32,
    pub orig_h: u32,
}

impl Source {
    pub fn warning(&self) -> Option<String> {
        let side = self.orig_w.min(self.orig_h);
        (side < WARN_SIDE).then(|| {
            format!("Only {side}×{side} px after cropping — larger icon sizes will be upscaled and look soft.")
        })
    }
}

pub fn load(path: &Path) -> AppResult<Source> {
    let p = path.display().to_string();
    // Detect format from magic bytes only. `with_guessed_format()` silently falls back to the
    // extension, so a text file named .png would be reported as a "corrupt PNG".
    let file = std::fs::File::open(path).map_err(|source| AppError::Open { path: p.clone(), source })?;
    let mut head = Vec::with_capacity(32);
    std::io::Read::take(&file, 32)
        .read_to_end(&mut head)
        .map_err(|source| AppError::Open { path: p.clone(), source })?;
    let format = match image::guess_format(&head) {
        Ok(f @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Bmp)) => f,
        _ => return Err(AppError::UnsupportedFormat { path: p }),
    };
    let reader = ImageReader::with_format(
        std::io::BufReader::new(std::fs::File::open(path).map_err(|source| AppError::Open { path: p.clone(), source })?),
        format,
    );
    let map_err = |e: ImageError| match e {
        ImageError::Unsupported(_) => AppError::UnsupportedFormat { path: p.clone() },
        ImageError::IoError(source) => AppError::Open { path: p.clone(), source },
        other => AppError::Decode { path: p.clone(), detail: other.to_string() },
    };
    let mut decoder = reader.into_decoder().map_err(map_err)?;
    // Phone JPEGs store pixels sideways + an EXIF flag; honour it or icons come out rotated.
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(map_err)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    from_rgba(img.into_rgba8())
}

/// Centre-crop to square and cap at `MASTER_SIDE`. Split out so tests can skip the filesystem.
pub fn from_rgba(img: RgbaImage) -> AppResult<Source> {
    let (w, h) = img.dimensions();
    let side = w.min(h);
    if side < MIN_SIDE {
        return Err(AppError::TooSmall { w, h, min: MIN_SIDE });
    }
    let (x, y) = ((w - side) / 2, (h - side) / 2);
    let square = imageops::crop_imm(&img, x, y, side, side).to_image();
    let img = if side > MASTER_SIDE {
        from_premul(&resize_premul(&to_premul(&square), MASTER_SIDE))
    } else {
        square
    };
    Ok(Source { img, orig_w: w, orig_h: h })
}

/// The masked high-res image plus the geometry needed to clip resampling ringing at the
/// target size. Geometry is stored as fractions of the side so it maps to any output size.
pub struct Master {
    img: Rgba32FImage,
    /// Corner radius as a fraction of the side; `None` = no shape mask.
    radius: Option<f32>,
    /// Padded art occupies [art_lo, art_hi] (fraction of the side) on both axes.
    art_lo: f32,
    art_hi: f32,
}

/// Build the masked master. Everything downstream (preview, ICO entries, 512 PNG) is a single
/// Lanczos3 resample of this — never a resample of a resample.
pub fn compose(source: &Source, s: &Settings) -> Master {
    let n = MASTER_SIDE;
    let inner =
        ((n as f32) * (1.0 - 2.0 * s.padding / 100.0)).round().clamp(1.0, n as f32) as u32;
    let art = resize_premul(&to_premul(&source.img), inner);

    let mut canvas = Rgba32FImage::new(n, n); // all zeros == fully transparent (premultiplied)
    let off = (n - inner) / 2;
    imageops::replace(&mut canvas, &art, off as i64, off as i64);

    let radius = match s.shape {
        Shape::None => None,
        Shape::Circle => Some(0.5),
        Shape::Rounded => Some(s.radius / 100.0),
    };
    if let Some(r) = radius {
        // The mask spans the whole canvas, not just the padded art (per spec).
        let radius_px = n as f32 * r;
        for (x, y, px) in canvas.enumerate_pixels_mut() {
            let c = rounded_square_coverage(x, y, n, radius_px);
            if c < 1.0 {
                // Premultiplied: scaling all four channels is exactly "multiply alpha by coverage".
                px.0.iter_mut().for_each(|v| *v *= c);
            }
        }
    }
    let side = n as f32;
    Master { img: canvas, radius, art_lo: off as f32 / side, art_hi: (off + inner) as f32 / side }
}

pub fn render(master: &Master, size: u32) -> RgbaImage {
    let mut out = from_premul(&resize_premul(&master.img, size));
    // Lanczos3's positive side lobes leak a few alpha units (seen: α=3 in a 24px corner) into
    // pixels that lie entirely outside the shape. Zero exactly those: a pixel is "entirely
    // outside" when the target-resolution mask gives it 0 coverage, or its footprint misses
    // the padded art rect. Edge pixels with partial coverage keep their resampled AA untouched.
    let n = size as f32;
    let (lo, hi) = (master.art_lo * n, master.art_hi * n);
    let r_px = master.radius.map(|r| r * n);
    for (x, y, px) in out.enumerate_pixels_mut() {
        let outside_art = |v: u32| (v + 1) as f32 <= lo || v as f32 >= hi;
        let outside_shape = r_px.is_some_and(|r| rounded_square_coverage(x, y, size, r) == 0.0);
        if outside_shape || outside_art(x) || outside_art(y) {
            *px = Rgba([0, 0, 0, 0]);
        }
    }
    out
}

pub fn encode_png(img: &RgbaImage, fast: bool) -> AppResult<Vec<u8>> {
    let mut buf = Vec::new();
    let comp = if fast { CompressionType::Fast } else { CompressionType::Best };
    PngEncoder::new_with_quality(Cursor::new(&mut buf), comp, PngFilter::Adaptive)
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)
        .map_err(|e| AppError::Encode(e.to_string()))?;
    Ok(buf)
}

/// Encode every selected size as PNG and pack them into an ICO.
pub fn build_ico(master: &Master, sizes: &[u32]) -> AppResult<Vec<u8>> {
    let entries = sizes
        .iter()
        .filter(|&&s| s <= 256) // ICO's width byte can't express more; 256 is stored as 0.
        .map(|&s| Ok((s, encode_png(&render(master, s), false)?)))
        .collect::<AppResult<Vec<_>>>()?;
    Ok(crate::ico::encode(&entries))
}

// ---- premultiplied float helpers -------------------------------------------------------
// Resampling straight alpha lets the RGB of invisible (0,0,0,0) pixels bleed into visible
// edge pixels → dark halos around the rounded corners. Premultiplying first weights each
// sample's colour by its opacity, so transparent pixels contribute nothing.

fn to_premul(img: &RgbaImage) -> Rgba32FImage {
    let (w, h) = img.dimensions();
    Rgba32FImage::from_fn(w, h, |x, y| {
        let [r, g, b, a] = img.get_pixel(x, y).0.map(|v| v as f32 / 255.0);
        Rgba([r * a, g * a, b * a, a])
    })
}

fn from_premul(img: &Rgba32FImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    RgbaImage::from_fn(w, h, |x, y| {
        let [r, g, b, a] = img.get_pixel(x, y).0;
        // Divide by the *unclamped* alpha: with ringing, a=1.04 and c=0.82 still means
        // colour 0.79. Clamping alpha first would brighten every opaque edge pixel.
        let a8 = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
        if a8 == 0 {
            return Rgba([0, 0, 0, 0]);
        }
        let q = |c: f32| ((c / a).clamp(0.0, 1.0) * 255.0).round() as u8;
        Rgba([q(r), q(g), q(b), a8])
    })
}

/// `image`'s resampler clamps f32 channels to [0,1] *independently*, which destroys the
/// colour/alpha ratio wherever Lanczos overshoots. Resampling is linear with weights summing
/// to 1, so mapping v → 0.25 + 0.5·v before and inverting after is exact, and keeps
/// overshoot (Lanczos3 peaks ≈ ±10%) well inside the clamp range.
fn resize_premul(img: &Rgba32FImage, side: u32) -> Rgba32FImage {
    if img.dimensions() == (side, side) {
        return img.clone();
    }
    let mut shifted = img.clone();
    shifted.pixels_mut().for_each(|p| p.0.iter_mut().for_each(|v| *v = 0.25 + 0.5 * *v));
    let mut out = imageops::resize(&shifted, side, side, FilterType::Lanczos3);
    out.pixels_mut().for_each(|p| p.0.iter_mut().for_each(|v| *v = (*v - 0.25) * 2.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([200, 40, 90, 255]))
    }
    fn settings(shape: Shape, radius: f32, padding: f32) -> Settings {
        Settings { shape, radius, padding, sizes: ALLOWED_SIZES.to_vec() }
    }

    #[test]
    fn crops_to_centre_square() {
        let s = from_rgba(solid(300, 200)).unwrap();
        assert_eq!(s.img.dimensions(), (200, 200));
        assert!(s.warning().is_some());
    }

    #[test]
    fn caps_large_sources() {
        let s = from_rgba(solid(2000, 1500)).unwrap();
        assert_eq!(s.img.dimensions(), (MASTER_SIDE, MASTER_SIDE));
        assert!(s.warning().is_none());
    }

    #[test]
    fn rejects_tiny() {
        assert!(matches!(from_rgba(solid(400, 10)), Err(AppError::TooSmall { .. })));
    }

    #[test]
    fn circle_equals_rounded_at_50() {
        let src = from_rgba(solid(400, 400)).unwrap();
        let a = render(&compose(&src, &settings(Shape::Circle, 22.0, 0.0)), 64);
        let b = render(&compose(&src, &settings(Shape::Rounded, 50.0, 0.0)), 64);
        assert_eq!(a, b);
    }

    #[test]
    fn corners_transparent_every_size() {
        let src = from_rgba(solid(512, 512)).unwrap();
        let m = compose(&src, &settings(Shape::Rounded, 22.0, 0.0));
        for s in ALLOWED_SIZES {
            let img = render(&m, s);
            assert_eq!(img.get_pixel(0, 0)[3], 0, "size {s}");
            assert_eq!(img.get_pixel(s - 1, s - 1)[3], 0, "size {s}");
            assert_eq!(img.get_pixel(s / 2, s / 2)[3], 255, "size {s}");
        }
    }

    #[test]
    fn no_dark_fringe_on_edges() {
        // Premultiplied resampling: any partially transparent pixel keeps the source colour.
        let src = from_rgba(solid(512, 512)).unwrap();
        let img = render(&compose(&src, &settings(Shape::Circle, 0.0, 0.0)), 32);
        for p in img.pixels().filter(|p| p[3] > 8) {
            assert!((p[0] as i32 - 200).abs() <= 3, "fringe {:?}", p);
        }
    }

    #[test]
    fn none_shape_is_opaque_square() {
        let src = from_rgba(solid(256, 256)).unwrap();
        let img = render(&compose(&src, &settings(Shape::None, 0.0, 0.0)), 48);
        assert!(img.pixels().all(|p| p[3] == 255));
    }

    #[test]
    fn padding_leaves_transparent_margin() {
        let src = from_rgba(solid(256, 256)).unwrap();
        let img = render(&compose(&src, &settings(Shape::None, 0.0, 20.0)), 100);
        // 20% of 100 = 20px margin; stay a few px clear of the Lanczos ramp.
        assert_eq!(img.get_pixel(16, 50)[3], 0);
        assert_eq!(img.get_pixel(50, 16)[3], 0);
        assert_eq!(img.get_pixel(50, 50)[3], 255);
    }

    #[test]
    fn settings_validation() {
        let mut s = settings(Shape::Rounded, 99.0, -5.0);
        s.sizes = vec![512, 32, 16, 32];
        let v = s.validated().unwrap();
        assert_eq!((v.radius, v.padding), (50.0, 0.0));
        assert_eq!(v.sizes, vec![16, 32]);
        s.sizes = vec![1024];
        assert!(s.validated().is_err());
    }
}
