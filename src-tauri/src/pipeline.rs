//! The one image pipeline used by preview *and* export, so what you see is what you write.
//!
//! load → decode (raster / SVG / ICO) → compose master (frame, backdrop, SDF mask, border,
//! shadow, badge at 1024², premultiplied f32) → render each size straight from the master
//! with Lanczos3 (+ optional small-size micro-sharpening or per-size overrides).

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops;
use image::{
    DynamicImage, ImageDecoder, ImageEncoder, ImageError, ImageFormat, ImageReader, Rgba,
    Rgba32FImage, RgbaImage,
};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::mask::{shape_coverage, shape_sdf_f};
use crate::resample::{self, par_rows};

/// Side of the masked master every output is resampled from. 4× the largest ICO entry and 2×
/// the 512 PNG — plenty of headroom for Lanczos while keeping a slider-drag re-render cheap.
pub const MASTER_SIDE: u32 = 1024;
/// Longest side cap for uncropped raw storage (`Source::raw`) so zoom/pan stays crisp and bounded.
pub const RAW_MAX_SIDE: u32 = 2048;
/// Below this the crop is rejected outright: even a 16px icon would be pure upscaling mush.
pub const MIN_SIDE: u32 = 16;
/// Below this we still work, but warn that the large ICO entries are upscaled.
pub const WARN_SIDE: u32 = 256;
pub const DEFAULT_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
pub const ALLOWED_SIZES: [u32; 11] = [16, 20, 24, 32, 40, 48, 60, 64, 96, 128, 256];
pub const PNG_EXPORT_SIDE: u32 = 512;
pub const PNG_1024_SIDE: u32 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    #[default]
    Rounded,
    Squircle,
    Circle,
    Hexagon,
    Octagon,
    Shield,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FitMode {
    #[default]
    Crop,
    Contain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BgMode {
    #[default]
    Transparent,
    Solid,
    Linear,
    Radial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BadgeMode {
    #[default]
    None,
    Dot,
    Ring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BadgeCorner {
    #[default]
    BottomRight,
    TopRight,
    BottomLeft,
    TopLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PackKind {
    #[default]
    Tauri,
    Web,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IcoEntryInfo {
    pub width: u32,
    pub height: u32,
    pub bpp: u16,
    pub is_png: bool,
    pub bytes: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub shape: Shape,
    /// Corner radius, percent of side (0–50).
    pub radius: f32,
    /// Transparent margin on *each* side, percent of side (0–30).
    pub padding: f32,
    pub sizes: Vec<u32>,

    // Framing & transforms
    pub fit_mode: FitMode,
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub rotation: u16,
    pub flip_h: bool,
    pub flip_v: bool,

    // Backdrop inside shape mask
    pub bg_mode: BgMode,
    pub bg_color1: String,
    pub bg_color2: String,
    pub bg_angle: f32,

    // SDF inner border
    pub border_width: f32,
    pub border_color: String,
    pub border_opacity: f32,

    // Soft outer drop shadow
    pub shadow: bool,
    pub shadow_blur: f32,
    pub shadow_opacity: f32,
    pub shadow_offset_y: f32,

    // Corner badge
    pub badge_mode: BadgeMode,
    pub badge_corner: BadgeCorner,
    pub badge_color: String,
    pub badge_size: f32,

    // Output tuning
    pub sharpen: f32,
    pub legacy_bmp: bool,

    /// Android adaptive-icon pack options (saved with presets).
    pub android: crate::android::AndroidSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shape: Shape::Rounded,
            radius: 22.0,
            padding: 0.0,
            sizes: DEFAULT_SIZES.to_vec(),
            fit_mode: FitMode::Crop,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            rotation: 0,
            flip_h: false,
            flip_v: false,
            bg_mode: BgMode::Transparent,
            bg_color1: "#1e293b".into(),
            bg_color2: "#0f172a".into(),
            bg_angle: 135.0,
            border_width: 0.0,
            border_color: "#ffffff".into(),
            border_opacity: 35.0,
            shadow: false,
            shadow_blur: 4.0,
            shadow_opacity: 45.0,
            shadow_offset_y: 2.5,
            badge_mode: BadgeMode::None,
            badge_corner: BadgeCorner::BottomRight,
            badge_color: "#22c55e".into(),
            badge_size: 20.0,
            sharpen: 0.0,
            legacy_bmp: false,
            android: crate::android::AndroidSettings::default(),
        }
    }
}

pub(crate) fn normalize_hex(raw: &str, fallback: &str) -> String {
    let s = raw.trim().trim_start_matches('#');
    if s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("#{}", s.to_ascii_lowercase())
    } else {
        fallback.to_string()
    }
}

pub(crate) fn parse_hex_rgb(hex: &str) -> [f32; 3] {
    let s = hex.trim().trim_start_matches('#');
    if s.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
            u8::from_str_radix(&s[4..6], 16),
        ) {
            return [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
        }
    }
    [0.0, 0.0, 0.0]
}

impl Settings {
    /// Never trust the webview: clamp/validate here so a tampered or stale localStorage value
    /// can't produce invalid geometry or a 0-entry / >256 ICO.
    pub fn validated(&self) -> AppResult<Settings> {
        let nums = [
            self.radius,
            self.padding,
            self.zoom,
            self.pan_x,
            self.pan_y,
            self.bg_angle,
            self.border_width,
            self.border_opacity,
            self.shadow_blur,
            self.shadow_opacity,
            self.shadow_offset_y,
            self.badge_size,
            self.sharpen,
            self.android.scale,
            self.android.offset_x,
            self.android.offset_y,
            self.android.mono_threshold,
        ];
        if nums.iter().any(|v| !v.is_finite()) {
            return Err(AppError::InvalidSettings("numeric settings must be finite numbers".into()));
        }
        let mut sizes: Vec<u32> =
            self.sizes.iter().copied().filter(|s| ALLOWED_SIZES.contains(s)).collect();
        sizes.sort_unstable();
        sizes.dedup();
        if sizes.is_empty() {
            return Err(AppError::InvalidSettings("select at least one icon size".into()));
        }
        let rotation = match self.rotation {
            90 | 180 | 270 => self.rotation,
            _ => 0,
        };
        Ok(Settings {
            shape: self.shape,
            radius: self.radius.clamp(0.0, 50.0),
            padding: self.padding.clamp(0.0, 30.0),
            sizes,
            fit_mode: self.fit_mode,
            zoom: self.zoom.clamp(0.5, 4.0),
            pan_x: self.pan_x.clamp(-100.0, 100.0),
            pan_y: self.pan_y.clamp(-100.0, 100.0),
            rotation,
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            bg_mode: self.bg_mode,
            bg_color1: normalize_hex(&self.bg_color1, "#1e293b"),
            bg_color2: normalize_hex(&self.bg_color2, "#0f172a"),
            bg_angle: self.bg_angle.rem_euclid(360.0),
            border_width: self.border_width.clamp(0.0, 10.0),
            border_color: normalize_hex(&self.border_color, "#ffffff"),
            border_opacity: self.border_opacity.clamp(0.0, 100.0),
            shadow: self.shadow,
            shadow_blur: self.shadow_blur.clamp(1.0, 12.0),
            shadow_opacity: self.shadow_opacity.clamp(0.0, 100.0),
            shadow_offset_y: self.shadow_offset_y.clamp(-8.0, 12.0),
            badge_mode: self.badge_mode,
            badge_corner: self.badge_corner,
            badge_color: normalize_hex(&self.badge_color, "#22c55e"),
            badge_size: self.badge_size.clamp(8.0, 35.0),
            sharpen: self.sharpen.clamp(0.0, 100.0),
            legacy_bmp: self.legacy_bmp,
            android: self.android.validated(),
        })
    }

    fn is_v1_fast_path(&self) -> bool {
        self.fit_mode == FitMode::Crop
            && (self.zoom - 1.0).abs() < 1e-4
            && self.pan_x.abs() < 1e-4
            && self.pan_y.abs() < 1e-4
            && self.rotation == 0
            && !self.flip_h
            && !self.flip_v
            && self.bg_mode == BgMode::Transparent
            && self.border_width <= 1e-4
            && !self.shadow
            && self.badge_mode == BadgeMode::None
    }
}

/// Decoded image source kept as straight-alpha u8 to keep memory compact for large batches.
pub struct Source {
    /// Center-cropped square up to `MASTER_SIDE`, used by the v1 fast path and thumbnails.
    pub img: RgbaImage,
    /// Uncropped image capped at `RAW_MAX_SIDE`, used by pan/zoom/contain framing.
    pub raw: RgbaImage,
    /// Original SVG/SVGZ bytes when loaded from vector artwork so zoom remains infinitely sharp.
    pub svg_bytes: Option<Vec<u8>>,
    pub orig_w: u32,
    pub orig_h: u32,
    pub is_svg: bool,
    pub ico_entries: Option<Vec<IcoEntryInfo>>,
}

impl Source {
    pub fn warning(&self) -> Option<String> {
        if self.is_svg {
            return None;
        }
        let side = self.orig_w.min(self.orig_h);
        (side < WARN_SIDE).then(|| {
            format!("Only {side}×{side} px after cropping — larger icon sizes will be upscaled and look soft.")
        })
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return true; // SVGZ gzip header
    }
    let prefix = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let trimmed = match std::str::from_utf8(&prefix[..prefix.len().min(4096)]) {
        Ok(s) => s.trim_start(),
        Err(_) => return false,
    };
    trimmed.starts_with("<svg")
        || trimmed.starts_with("<?xml")
        || (trimmed.starts_with("<!--") && trimmed.contains("<svg"))
}

fn demultiply_pixmap(pixmap: &resvg::tiny_skia::Pixmap) -> RgbaImage {
    let (w, h) = (pixmap.width(), pixmap.height());
    let mut img = RgbaImage::new(w, h);
    for (dst, src) in img.as_mut().chunks_exact_mut(4).zip(pixmap.pixels()) {
        let a = src.alpha();
        if a == 0 {
            dst.copy_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            dst.copy_from_slice(&[src.red(), src.green(), src.blue(), 255]);
        } else {
            let af = a as f32;
            let unpremul = |c: u8| ((c as f32 / af) * 255.0).round().clamp(0.0, 255.0) as u8;
            dst.copy_from_slice(&[unpremul(src.red()), unpremul(src.green()), unpremul(src.blue()), a]);
        }
    }
    img
}

fn load_svg(bytes: &[u8], label: &str) -> AppResult<Source> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(bytes, &opt)
        .map_err(|e| AppError::Decode { path: label.to_string(), detail: e.to_string() })?;
    let size = tree.size();
    let (sw, sh) = (size.width().max(1.0), size.height().max(1.0));
    let scale = (MASTER_SIDE as f32 / sw.min(sh)).min(RAW_MAX_SIDE as f32 / sw.max(sh));
    let rw = (sw * scale).round().clamp(MIN_SIDE as f32, RAW_MAX_SIDE as f32) as u32;
    let rh = (sh * scale).round().clamp(MIN_SIDE as f32, RAW_MAX_SIDE as f32) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(rw, rh)
        .ok_or_else(|| AppError::Decode { path: label.to_string(), detail: "invalid SVG size".into() })?;
    let transform = resvg::tiny_skia::Transform::from_scale(rw as f32 / sw, rh as f32 / sh);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let raw = demultiply_pixmap(&pixmap);
    let side = rw.min(rh);
    let (cx, cy) = ((rw - side) / 2, (rh - side) / 2);
    let square = imageops::crop_imm(&raw, cx, cy, side, side).to_image();
    let img = if side > MASTER_SIDE {
        from_premul(&resize_premul(&to_premul(&square), MASTER_SIDE))
    } else {
        square
    };

    Ok(Source {
        img,
        raw,
        svg_bytes: Some(bytes.to_vec()),
        orig_w: sw.round().max(1.0) as u32,
        orig_h: sh.round().max(1.0) as u32,
        is_svg: true,
        ico_entries: None,
    })
}

fn load_ico(bytes: &[u8], label: &str) -> AppResult<Source> {
    let dir = crate::ico::parse(bytes)
        .map_err(|detail| AppError::Decode { path: label.to_string(), detail })?;
    let entries: Vec<IcoEntryInfo> = dir
        .iter()
        .map(|e| IcoEntryInfo {
            width: e.width,
            height: e.height,
            bpp: e.bpp,
            is_png: e.is_png,
            bytes: e.data.len(),
        })
        .collect();
    let best = dir
        .iter()
        .max_by_key(|e| (e.width * e.height, e.bpp))
        .ok_or_else(|| AppError::Decode { path: label.to_string(), detail: "empty ICO".into() })?;
    let rgba = match crate::ico::decode_entry(bytes, best) {
        Ok(img) => img,
        Err(_) => image::load_from_memory_with_format(bytes, ImageFormat::Ico)
            .map_err(|e| AppError::Decode { path: label.to_string(), detail: e.to_string() })?
            .into_rgba8(),
    };
    from_rgba_with_meta(rgba, None, false, Some(entries))
}

pub fn load_bytes(bytes: &[u8], label: &str) -> AppResult<Source> {
    if bytes.len() >= 6 && bytes[0..4] == [0, 0, 1, 0] {
        return load_ico(bytes, label);
    }
    if looks_like_svg(bytes) {
        return load_svg(bytes, label);
    }

    let format = match image::guess_format(bytes) {
        Ok(
            f @ (ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::WebP
            | ImageFormat::Bmp
            | ImageFormat::Tiff
            | ImageFormat::Gif),
        ) => f,
        _ => return Err(AppError::UnsupportedFormat { path: label.to_string() }),
    };

    let reader = ImageReader::with_format(Cursor::new(bytes), format);
    let map_err = |e: ImageError| match e {
        ImageError::Unsupported(_) => AppError::UnsupportedFormat { path: label.to_string() },
        ImageError::IoError(source) => AppError::Open { path: label.to_string(), source },
        other => AppError::Decode { path: label.to_string(), detail: other.to_string() },
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

pub fn load(path: &Path) -> AppResult<Source> {
    let p = path.display().to_string();
    let mut file = std::fs::File::open(path).map_err(|source| AppError::Open { path: p.clone(), source })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|source| AppError::Open { path: p.clone(), source })?;
    load_bytes(&bytes, &p)
}

fn from_rgba_with_meta(
    img: RgbaImage,
    svg_bytes: Option<Vec<u8>>,
    is_svg: bool,
    ico_entries: Option<Vec<IcoEntryInfo>>,
) -> AppResult<Source> {
    let (w, h) = img.dimensions();
    let side = w.min(h);
    if side < MIN_SIDE {
        return Err(AppError::TooSmall { w, h, min: MIN_SIDE });
    }
    let (x, y) = ((w - side) / 2, (h - side) / 2);
    let square = imageops::crop_imm(&img, x, y, side, side).to_image();
    let cropped = if side > MASTER_SIDE {
        from_premul(&resize_premul(&to_premul(&square), MASTER_SIDE))
    } else {
        square
    };

    let max_dim = w.max(h);
    let raw = if max_dim > RAW_MAX_SIDE {
        let scale = RAW_MAX_SIDE as f32 / max_dim as f32;
        let rw = ((w as f32) * scale).round().max(MIN_SIDE as f32) as u32;
        let rh = ((h as f32) * scale).round().max(MIN_SIDE as f32) as u32;
        from_premul(&resample::resize(&to_premul(&img), rw, rh))
    } else {
        img
    };

    Ok(Source {
        img: cropped,
        raw,
        svg_bytes,
        orig_w: w,
        orig_h: h,
        is_svg,
        ico_entries,
    })
}

/// Centre-crop to square and cap at `MASTER_SIDE`. Split out so tests can skip the filesystem.
pub fn from_rgba(img: RgbaImage) -> AppResult<Source> {
    from_rgba_with_meta(img, None, false, None)
}

/// The masked high-res image plus geometry needed to clip resampling ringing at the target size.
pub struct Master {
    img: Rgba32FImage,
    shape: Shape,
    radius: f32,
    art_lo: f32,
    art_hi: f32,
    clip_outside_art: bool,
    clip_outside_shape: bool,
    sharpen: f32,
    overrides: HashMap<u32, RgbaImage>,
}

pub(crate) fn transform_raw(raw: &RgbaImage, rotation: u16, flip_h: bool, flip_v: bool) -> RgbaImage {
    let mut out = match rotation {
        90 => imageops::rotate90(raw),
        180 => imageops::rotate180(raw),
        270 => imageops::rotate270(raw),
        _ => raw.clone(),
    };
    if flip_h {
        imageops::flip_horizontal_in_place(&mut out);
    }
    if flip_v {
        imageops::flip_vertical_in_place(&mut out);
    }
    out
}

fn render_svg_framed(
    svg_bytes: &[u8],
    inner: u32,
    s: &Settings,
    pan_dx: f32,
    pan_dy: f32,
) -> Option<Rgba32FImage> {
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &resvg::usvg::Options::default()).ok()?;
    let sz = tree.size();
    let (sw, sh) = (sz.width().max(1.0), sz.height().max(1.0));
    let (ew, eh) = if s.rotation == 90 || s.rotation == 270 { (sh, sw) } else { (sw, sh) };
    let inner_f = inner as f32;
    let base = match s.fit_mode {
        FitMode::Crop => inner_f / ew.min(eh),
        FitMode::Contain => inner_f / ew.max(eh),
    };
    let scale = base * s.zoom;
    let mut t = resvg::tiny_skia::Transform::from_translate(-sw * 0.5, -sh * 0.5);
    if s.rotation != 0 {
        t = t.post_rotate(s.rotation as f32);
    }
    t = t.post_scale(if s.flip_h { -scale } else { scale }, if s.flip_v { -scale } else { scale });
    t = t.post_translate(inner_f * 0.5 + pan_dx, inner_f * 0.5 + pan_dy);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(inner, inner)?;
    resvg::render(&tree, t, &mut pixmap.as_mut());
    // tiny_skia pixels are already premultiplied sRGB u8 RGBA.
    let mut fimg = Rgba32FImage::new(inner, inner);
    for (dst, src) in fimg.as_mut().chunks_exact_mut(4).zip(pixmap.pixels()) {
        dst[0] = src.red() as f32 / 255.0;
        dst[1] = src.green() as f32 / 255.0;
        dst[2] = src.blue() as f32 / 255.0;
        dst[3] = src.alpha() as f32 / 255.0;
    }
    Some(fimg)
}

/// Frame the artwork into an `inner`×`inner` premultiplied square using the fit mode, zoom, pan,
/// rotation and flips from `s` (shape, padding and styling are ignored). `strip_inset` crops that
/// fraction off each side of the centred square first — used to cut away baked-in rounded corners.
pub(crate) fn frame_art(source: &Source, s: &Settings, inner: u32, strip_inset: f32) -> Rgba32FImage {
    let inner = inner.max(1);
    let inner_f = inner as f32;
    let pan_dx = (s.pan_x / 100.0) * inner_f * 0.5 * s.zoom.max(1.0);
    let pan_dy = (s.pan_y / 100.0) * inner_f * 0.5 * s.zoom.max(1.0);
    let strip = strip_inset.clamp(0.0, 0.45);

    if strip <= 1e-6 {
        if let Some(svg_bytes) = source.svg_bytes.as_deref() {
            return render_svg_framed(svg_bytes, inner, s, pan_dx, pan_dy)
                .unwrap_or_else(|| resize_premul(&to_premul(&source.img), inner));
        }
    }
    let mut oriented = transform_raw(&source.raw, s.rotation, s.flip_h, s.flip_v);
    if strip > 1e-6 {
        let (w, h) = oriented.dimensions();
        let m = w.min(h);
        let cut = ((m as f32) * strip).round() as u32;
        let side = m.saturating_sub(2 * cut).max(1);
        let (x0, y0) = ((w - m) / 2 + cut, (h - m) / 2 + cut);
        oriented = imageops::crop_imm(&oriented, x0, y0, side, side).to_image();
    }
    let (ow, oh) = (oriented.width() as f32, oriented.height() as f32);
    let base = match s.fit_mode {
        FitMode::Crop => inner_f / ow.min(oh),
        FitMode::Contain => inner_f / ow.max(oh),
    };
    let scale = base * s.zoom;
    let scaled_w = ow * scale;
    let scaled_h = oh * scale;
    let dst_x0 = inner_f * 0.5 + pan_dx - scaled_w * 0.5;
    let dst_y0 = inner_f * 0.5 + pan_dy - scaled_h * 0.5;
    let clamp_edges = dst_x0 <= 0.0
        && dst_y0 <= 0.0
        && (dst_x0 + scaled_w) >= inner_f
        && (dst_y0 + scaled_h) >= inner_f;
    resample::resize_mapped(&to_premul(&oriented), inner, inner, dst_x0, dst_y0, scale, clamp_edges)
}

fn eval_backdrop(s: &Settings, u: f32, v: f32, c1: [f32; 3], c2: [f32; 3]) -> [f32; 3] {
    match s.bg_mode {
        BgMode::Transparent => [0.0, 0.0, 0.0],
        BgMode::Solid => c1,
        BgMode::Linear => {
            let rad = s.bg_angle.to_radians();
            let dx = rad.sin();
            let dy = -rad.cos();
            let span = (dx.abs() + dy.abs()).max(1e-4);
            let t = ((u * dx + v * dy) / span + 0.5).clamp(0.0, 1.0);
            [
                c1[0] + (c2[0] - c1[0]) * t,
                c1[1] + (c2[1] - c1[1]) * t,
                c1[2] + (c2[2] - c1[2]) * t,
            ]
        }
        BgMode::Radial => {
            let t = ((u * u + v * v).sqrt() * 2.0).clamp(0.0, 1.0);
            [
                c1[0] + (c2[0] - c1[0]) * t,
                c1[1] + (c2[1] - c1[1]) * t,
                c1[2] + (c2[2] - c1[2]) * t,
            ]
        }
    }
}

fn apply_badge(canvas: &mut Rgba32FImage, s: &Settings) {
    if s.badge_mode == BadgeMode::None {
        return;
    }
    let n = canvas.width();
    let nf = n as f32;
    let br = nf * (s.badge_size / 100.0) * 0.5;
    let cut = (br * 0.28).max(nf * 0.022);
    let edge_inset = br + cut * 0.65 + nf * 0.025;
    let (bx, by) = match s.badge_corner {
        BadgeCorner::TopLeft => (edge_inset, edge_inset),
        BadgeCorner::TopRight => (nf - edge_inset, edge_inset),
        BadgeCorner::BottomLeft => (edge_inset, nf - edge_inset),
        BadgeCorner::BottomRight => (nf - edge_inset, nf - edge_inset),
    };
    let badge_rgb = parse_hex_rgb(&s.badge_color);
    let ring_thick = (br * 0.36).max(2.0);
    let max_r = br + cut + 1.5;

    par_rows(canvas, (n * 4) as usize, |y, row| {
        let dy = y as f32 + 0.5 - by;
        if dy.abs() > max_r {
            return;
        }
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            let dx = x as f32 + 0.5 - bx;
            if dx.abs() > max_r {
                continue;
            }
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > max_r {
                continue;
            }
            let cut_erase = (0.5 - (dist - (br + cut))).clamp(0.0, 1.0);
            if cut_erase > 0.0 {
                let keep = 1.0 - cut_erase;
                px.iter_mut().for_each(|v| *v *= keep);
            }
            let badge_cov = match s.badge_mode {
                BadgeMode::None => 0.0,
                BadgeMode::Dot => (0.5 - (dist - br)).clamp(0.0, 1.0),
                BadgeMode::Ring => {
                    let outer = 0.5 - (dist - br);
                    let inner = dist - (br - ring_thick) + 0.5;
                    outer.min(inner).clamp(0.0, 1.0)
                }
            };
            if badge_cov > 0.0 {
                let inv = 1.0 - badge_cov;
                for c in 0..3 {
                    px[c] = badge_rgb[c] * badge_cov + px[c] * inv;
                }
                px[3] = badge_cov + px[3] * inv;
            }
        }
    });
}

/// Build the masked master using optional per-size overrides. Everything downstream is a single
/// Lanczos3 resample from this master — never a resample of a resample.
pub fn compose_with_overrides(
    source: &Source,
    s: &Settings,
    overrides: Option<&HashMap<u32, RgbaImage>>,
) -> Master {
    let n = MASTER_SIDE;
    let ov_map = overrides.cloned().unwrap_or_default();

    // Preserve exact v1 pipeline when no v2 framing/styling options are active.
    if s.is_v1_fast_path() && !source.is_svg {
        let inner =
            ((n as f32) * (1.0 - 2.0 * s.padding / 100.0)).round().clamp(1.0, n as f32) as u32;
        let art = resize_premul(&to_premul(&source.img), inner);
        let mut canvas = Rgba32FImage::new(n, n);
        let off = (n - inner) / 2;
        imageops::replace(&mut canvas, &art, off as i64, off as i64);

        if s.shape != Shape::None {
            let shape = s.shape;
            let radius = s.radius;
            par_rows(&mut canvas, (n * 4) as usize, |y, row| {
                for (x, px) in row.chunks_exact_mut(4).enumerate() {
                    let c = shape_coverage(x as u32, y as u32, n, shape, radius);
                    if c < 1.0 {
                        px.iter_mut().for_each(|v| *v *= c);
                    }
                }
            });
        }
        let side = n as f32;
        return Master {
            img: canvas,
            shape: s.shape,
            radius: s.radius,
            art_lo: off as f32 / side,
            art_hi: (off + inner) as f32 / side,
            clip_outside_art: true,
            clip_outside_shape: s.shape != Shape::None,
            sharpen: s.sharpen,
            overrides: ov_map,
        };
    }

    // Reserve outer margin for soft drop shadow so shadow blur never clips flat against the edge.
    let blur_px = n as f32 * (s.shadow_blur / 100.0);
    let off_y_px = n as f32 * (s.shadow_offset_y / 100.0);
    let shadow_margin = if s.shadow {
        (blur_px * 1.65 + off_y_px.abs()).min(n as f32 * 0.20).ceil() as u32
    } else {
        0
    };
    let shape_side = n - 2 * shadow_margin;
    let shape_off = shadow_margin;

    let inner = ((shape_side as f32) * (1.0 - 2.0 * s.padding / 100.0))
        .round()
        .clamp(1.0, shape_side as f32) as u32;
    let art_off = shape_off + (shape_side - inner) / 2;
    let art = frame_art(source, s, inner, 0.0);

    let mut canvas = Rgba32FImage::new(n, n);
    imageops::replace(&mut canvas, &art, art_off as i64, art_off as i64);

    let bg_c1 = parse_hex_rgb(&s.bg_color1);
    let bg_c2 = parse_hex_rgb(&s.bg_color2);
    let border_rgb = parse_hex_rgb(&s.border_color);
    let has_bg = s.bg_mode != BgMode::Transparent;
    let has_border = s.border_width > 1e-4 && s.border_opacity > 1e-4;
    let bw_px = shape_side as f32 * (s.border_width / 100.0);
    let border_op = s.border_opacity / 100.0;
    let shape_side_f = shape_side as f32;
    let shape_off_f = shape_off as f32;
    let shadow_op = s.shadow_opacity / 100.0;
    let shadow_spread = (blur_px * 1.6).max(1.0);

    par_rows(&mut canvas, (n * 4) as usize, |y, row| {
        let fy = y as f32 + 0.5 - shape_off_f;
        let sy_shadow = y as f32 + 0.5 - off_y_px - shape_off_f;
        let v = (fy / shape_side_f) - 0.5;
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            let fx = x as f32 + 0.5 - shape_off_f;
            let u = (fx / shape_side_f) - 0.5;

            let d = shape_sdf_f(fx, fy, shape_side_f, s.shape, s.radius);
            let cov = if s.shape == Shape::None && !s.shadow {
                1.0
            } else {
                (0.5 - d).clamp(0.0, 1.0)
            };

            let mut fg = [px[0], px[1], px[2], px[3]];
            if has_bg {
                let bg = eval_backdrop(s, u, v, bg_c1, bg_c2);
                let inv_a = 1.0 - fg[3].clamp(0.0, 1.0);
                for c in 0..3 {
                    fg[c] += bg[c] * inv_a;
                }
                fg[3] += inv_a;
            }

            if has_border {
                let inner_cov = (d + bw_px + 0.5).clamp(0.0, 1.0);
                if inner_cov > 0.0 {
                    let ab = inner_cov * border_op;
                    let inv = 1.0 - ab;
                    for c in 0..3 {
                        fg[c] = border_rgb[c] * ab + fg[c] * inv;
                    }
                    fg[3] = ab + fg[3] * inv;
                }
            }

            if cov < 1.0 {
                fg.iter_mut().for_each(|v| *v *= cov);
            }

            if s.shadow {
                let d_sh = shape_sdf_f(fx, sy_shadow, shape_side_f, s.shape, s.radius);
                let u_sh = (d_sh / shadow_spread).clamp(-1.0, 1.0);
                let t_sh = (0.5 - 0.5 * u_sh).clamp(0.0, 1.0);
                let sa = shadow_op * (t_sh * t_sh * (3.0 - 2.0 * t_sh));
                let inv_fg = 1.0 - fg[3].clamp(0.0, 1.0);
                // Shadow RGB is black (0,0,0), so only alpha accumulates underneath.
                fg[3] += sa * inv_fg;
            }

            px.copy_from_slice(&fg);
        }
    });

    if s.badge_mode != BadgeMode::None {
        apply_badge(&mut canvas, s);
    }

    let side = n as f32;
    let simple_art = !has_bg && !has_border && !s.shadow && s.badge_mode == BadgeMode::None
        && s.fit_mode == FitMode::Crop && (s.zoom - 1.0).abs() < 1e-4
        && s.pan_x.abs() < 1e-4 && s.pan_y.abs() < 1e-4;
    let clip_shape = !s.shadow && s.badge_mode == BadgeMode::None && s.shape != Shape::None;

    Master {
        img: canvas,
        shape: s.shape,
        radius: s.radius,
        art_lo: art_off as f32 / side,
        art_hi: (art_off + inner) as f32 / side,
        clip_outside_art: simple_art,
        clip_outside_shape: clip_shape,
        sharpen: s.sharpen,
        overrides: ov_map,
    }
}

pub fn compose(source: &Source, s: &Settings) -> Master {
    compose_with_overrides(source, s, None)
}

pub fn render(master: &Master, size: u32) -> RgbaImage {
    if let Some(custom) = master.overrides.get(&size) {
        return custom.clone();
    }

    let mut fimg = resize_premul(&master.img, size);
    if master.sharpen > 1e-4 && size <= 64 {
        let strength = (master.sharpen / 100.0) * ((64.0 - size as f32) / 48.0).clamp(0.2, 1.0);
        fimg = resample::sharpen_premul(&fimg, strength);
    }
    let mut out = from_premul(&fimg);

    if master.clip_outside_shape || master.clip_outside_art {
        // Lanczos3's positive side lobes leak a few alpha units (seen: α=3 in a 24px corner) into
        // pixels that lie entirely outside the shape. Zero exactly those while preserving AA edges.
        let n = size as f32;
        let (lo, hi) = (master.art_lo * n, master.art_hi * n);
        for (x, y, px) in out.enumerate_pixels_mut() {
            let outside_art = master.clip_outside_art
                && ((x + 1) as f32 <= lo || x as f32 >= hi || (y + 1) as f32 <= lo || y as f32 >= hi);
            let outside_shape = master.clip_outside_shape
                && shape_coverage(x, y, size, master.shape, master.radius) == 0.0;
            if outside_shape || outside_art {
                *px = Rgba([0, 0, 0, 0]);
            }
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

/// Encode every selected size (PNG, or 32-bit DIB BMP for ≤48 px when `legacy_bmp`) into an ICO.
pub fn build_ico_with_options(master: &Master, sizes: &[u32], legacy_bmp: bool) -> AppResult<Vec<u8>> {
    let entries = sizes
        .iter()
        .filter(|&&s| s <= 256) // ICO's width byte can't express more; 256 is stored as 0.
        .map(|&s| {
            let img = render(master, s);
            let payload = if legacy_bmp && s <= 48 {
                crate::ico::encode_bmp_dib(&img)
            } else {
                encode_png(&img, false)?
            };
            Ok((s, payload))
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(crate::ico::encode(&entries))
}

/// Encode every selected size as PNG and pack them into an ICO.
pub fn build_ico(master: &Master, sizes: &[u32]) -> AppResult<Vec<u8>> {
    build_ico_with_options(master, sizes, false)
}

pub(crate) fn write_atomic_file(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let tmp = path.with_extension("simulacrum-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| AppError::write(path, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::write(path, e)
    })
}

/// Write a complete Tauri or Web/PWA developer icon bundle into `out_dir`.
pub fn export_pack(master: &Master, out_dir: &Path, pack: PackKind) -> AppResult<Vec<String>> {
    if !out_dir.is_dir() {
        return Err(AppError::Write {
            path: out_dir.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "folder does not exist"),
        });
    }
    let mut written = Vec::new();
    let mut put = |name: &str, bytes: Vec<u8>| -> AppResult<()> {
        let p = out_dir.join(name);
        write_atomic_file(&p, &bytes)?;
        written.push(p.display().to_string());
        Ok(())
    };

    match pack {
        PackKind::Tauri => {
            put("icon.ico", build_ico(master, &[16, 24, 32, 48, 64, 256])?)?;
            put("icon.icns", crate::icns::encode(master)?)?;
            put("32x32.png", encode_png(&render(master, 32), false)?)?;
            put("128x128.png", encode_png(&render(master, 128), false)?)?;
            put("128x128@2x.png", encode_png(&render(master, 256), false)?)?;
            put("icon.png", encode_png(&render(master, 512), false)?)?;
        }
        PackKind::Web => {
            put("favicon.ico", build_ico(master, &[16, 32, 48])?)?;
            put("favicon-16x16.png", encode_png(&render(master, 16), false)?)?;
            put("favicon-32x32.png", encode_png(&render(master, 32), false)?)?;
            put("apple-touch-icon.png", encode_png(&render(master, 180), false)?)?;
            put("android-chrome-192x192.png", encode_png(&render(master, 192), false)?)?;
            put("android-chrome-512x512.png", encode_png(&render(master, 512), false)?)?;
            let manifest = r##"{
  "name": "",
  "short_name": "",
  "icons": [
    { "src": "/android-chrome-192x192.png", "sizes": "192x192", "type": "image/png" },
    { "src": "/android-chrome-512x512.png", "sizes": "512x512", "type": "image/png" }
  ],
  "theme_color": "#ffffff",
  "background_color": "#ffffff",
  "display": "standalone"
}
"##;
            put("site.webmanifest", manifest.as_bytes().to_vec())?;
        }
    }
    Ok(written)
}

// ---- premultiplied float helpers -------------------------------------------------------
// Resampling straight alpha lets the RGB of invisible (0,0,0,0) pixels bleed into visible
// edge pixels → dark halos around the rounded corners. Premultiplying first weights each
// sample's colour by its opacity, so transparent pixels contribute nothing.

pub(crate) fn to_premul(img: &RgbaImage) -> Rgba32FImage {
    let (w, h) = img.dimensions();
    Rgba32FImage::from_fn(w, h, |x, y| {
        let [r, g, b, a] = img.get_pixel(x, y).0.map(|v| v as f32 / 255.0);
        Rgba([r * a, g * a, b * a, a])
    })
}

pub(crate) fn from_premul(img: &Rgba32FImage) -> RgbaImage {
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

/// Single entry point for every resample in the pipeline. `resample::resize` doesn't clamp,
/// so Lanczos overshoot survives intact until `from_premul` divides by the unclamped alpha.
pub(crate) fn resize_premul(img: &Rgba32FImage, side: u32) -> Rgba32FImage {
    resample::resize(img, side, side)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([200, 40, 90, 255]))
    }
    fn settings(shape: Shape, radius: f32, padding: f32) -> Settings {
        Settings { shape, radius, padding, sizes: DEFAULT_SIZES.to_vec(), ..Settings::default() }
    }

    #[test]
    fn crops_to_centre_square() {
        let s = from_rgba(solid(300, 200)).unwrap();
        assert_eq!(s.img.dimensions(), (200, 200));
        assert_eq!(s.raw.dimensions(), (300, 200));
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
        for s in DEFAULT_SIZES {
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
        s.sizes = vec![512, 32, 16, 32, 20];
        s.zoom = 10.0;
        s.rotation = 45;
        let v = s.validated().unwrap();
        assert_eq!((v.radius, v.padding, v.zoom, v.rotation), (50.0, 0.0, 4.0, 0));
        assert_eq!(v.sizes, vec![16, 20, 32]);
        s.sizes = vec![1024];
        assert!(s.validated().is_err());
    }

    #[test]
    fn loads_svg_and_renders_clean_squircle() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
            <rect x="8" y="8" width="48" height="48" fill="#3b82f6"/>
        </svg>"##;
        let src = load_bytes(svg, "test.svg").unwrap();
        assert!(src.is_svg);
        assert!(src.warning().is_none());
        let mut s = settings(Shape::Squircle, 28.0, 0.0);
        s.bg_mode = BgMode::Solid;
        s.bg_color1 = "#111827".into();
        let img = render(&compose(&src, &s), 64);
        assert_eq!(img.get_pixel(0, 0)[3], 0);
        assert_eq!(img.get_pixel(32, 32)[3], 255);
    }

    #[test]
    fn contain_mode_letterboxes_non_square() {
        let src = from_rgba(solid(400, 200)).unwrap();
        let mut s = settings(Shape::None, 0.0, 0.0);
        s.fit_mode = FitMode::Contain;
        let img = render(&compose(&src, &s), 64);
        // Top and bottom strips (16px each) should be transparent; middle row opaque.
        assert_eq!(img.get_pixel(32, 4)[3], 0);
        assert_eq!(img.get_pixel(32, 59)[3], 0);
        assert_eq!(img.get_pixel(32, 32)[3], 255);
    }

    #[test]
    fn shadow_border_and_badge_render_cleanly() {
        let src = from_rgba(solid(256, 256)).unwrap();
        let mut s = settings(Shape::Rounded, 22.0, 8.0);
        s.bg_mode = BgMode::Linear;
        s.bg_color1 = "#ff0000".into();
        s.bg_color2 = "#0000ff".into();
        s.border_width = 4.0;
        s.border_opacity = 80.0;
        s.shadow = true;
        s.badge_mode = BadgeMode::Dot;
        s.badge_corner = BadgeCorner::TopRight;
        s.sharpen = 50.0;
        let m = compose(&src, &s);
        let img = render(&m, 48);
        assert_eq!(img.dimensions(), (48, 48));
        assert_eq!(img.get_pixel(0, 0)[3], 0);
    }

    #[test]
    fn per_size_override_replaces_only_target_size() {
        let src = from_rgba(solid(256, 256)).unwrap();
        let s = settings(Shape::Circle, 22.0, 0.0);
        let mut ov = HashMap::new();
        ov.insert(16, RgbaImage::from_pixel(16, 16, Rgba([10, 20, 30, 255])));
        let m = compose_with_overrides(&src, &s, Some(&ov));
        assert_eq!(render(&m, 16).get_pixel(0, 0), &Rgba([10, 20, 30, 255]));
        assert_eq!(render(&m, 32).get_pixel(0, 0)[3], 0);
    }

    #[test]
    fn build_ico_hybrid_bmp_and_export_packs() {
        let src = from_rgba(solid(256, 256)).unwrap();
        let m = compose(&src, &settings(Shape::Rounded, 22.0, 0.0));
        let ico_bytes = build_ico_with_options(&m, &[16, 32, 256], true).unwrap();
        let dir = crate::ico::parse(&ico_bytes).unwrap();
        assert_eq!(dir.len(), 3);
        assert!(!dir[0].is_png);
        assert!(!dir[1].is_png);
        assert!(dir[2].is_png);

        let dir_path = std::env::temp_dir().join(format!("simulacrum-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir_path);
        std::fs::create_dir_all(&dir_path).unwrap();
        let tauri_files = export_pack(&m, &dir_path, PackKind::Tauri).unwrap();
        assert_eq!(tauri_files.len(), 6);
        assert!(dir_path.join("icon.icns").exists());
        let web_files = export_pack(&m, &dir_path, PackKind::Web).unwrap();
        assert_eq!(web_files.len(), 7);
        assert!(dir_path.join("site.webmanifest").exists());
        let _ = std::fs::remove_dir_all(&dir_path);
    }
}
