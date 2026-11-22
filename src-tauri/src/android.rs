//! Android adaptive-icon pack: `res/` folder (foreground / background / monochrome layers at the
//! five launcher densities, the adaptive XML, an optional colour resource and legacy icons) plus a
//! 512 px Play Store icon.
//!
//! Geometry: each layer is 108×108 dp. The launcher shows only the middle 72 dp (cut by its own
//! mask) and guarantees the 66 dp circle in the middle. "Scale" is relative to the 72 dp visible
//! area, so 100 % = full-bleed art that exactly fills what the user can see.

use std::path::Path;

use image::{imageops, Rgba, Rgba32FImage, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::mask::shape_coverage;
use crate::pipeline::{
    self, encode_png, frame_art, from_premul, normalize_hex, parse_hex_rgb, FitMode, Settings, Shape,
    Source,
};

/// Launcher densities and their 108 dp canvas side in px.
pub const DENSITIES: [(&str, u32); 5] =
    [("mdpi", 108), ("hdpi", 162), ("xhdpi", 216), ("xxhdpi", 324), ("xxxhdpi", 432)];
/// Legacy (pre-API 26) launcher icon sides, 48 dp per density.
pub const LEGACY_DENSITIES: [(&str, u32); 5] =
    [("mdpi", 48), ("hdpi", 72), ("xhdpi", 96), ("xxhdpi", 144), ("xxxhdpi", 192)];
pub const PLAY_STORE_SIDE: u32 = 512;
/// Side of the full 108 dp canvas returned by the preview command.
pub const PREVIEW_SIDE: u32 = 288;

const CANVAS_DP: f32 = 108.0;
const VISIBLE_DP: f32 = 72.0;
const VISIBLE_RADIUS_DP: f32 = 36.0;
/// Fraction of subject pixels allowed outside the 72 dp circle before warning.
const CLIP_WARN: f32 = 0.005;
/// Subject must span at least this much of the visible width.
const COVERAGE_WARN: f32 = 0.75;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AndroidBg {
    /// Flat edges → that colour; transparent edges → `bg_color`; varied edges → image layer.
    #[default]
    Auto,
    Color,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AndroidAnchor {
    #[default]
    Center,
    /// Art's bottom edge sits on the bottom of the visible 72 dp area.
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AndroidMono {
    #[default]
    Off,
    Auto,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AndroidSettings {
    pub bg_mode: AndroidBg,
    pub bg_color: String,
    /// Composite the foreground onto the background layer (fully opaque foreground PNGs).
    pub flatten: bool,
    /// Percent of the 72 dp visible area (60–130).
    pub scale: f32,
    pub anchor: AndroidAnchor,
    /// Offsets in dp (±18).
    pub offset_x: f32,
    pub offset_y: f32,
    pub mono: AndroidMono,
    /// Luminance difference (0–100) from the background that counts as "ink" for auto monochrome.
    pub mono_threshold: f32,
    pub strip_corners: bool,
    pub legacy: bool,
}

impl Default for AndroidSettings {
    fn default() -> Self {
        Self {
            bg_mode: AndroidBg::Auto,
            bg_color: "#ffffff".into(),
            flatten: false,
            scale: 100.0,
            anchor: AndroidAnchor::Center,
            offset_x: 0.0,
            offset_y: 0.0,
            mono: AndroidMono::Off,
            mono_threshold: 20.0,
            strip_corners: true,
            legacy: false,
        }
    }
}

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

impl AndroidSettings {
    pub fn validated(&self) -> AndroidSettings {
        AndroidSettings {
            bg_mode: self.bg_mode,
            bg_color: normalize_hex(&self.bg_color, "#ffffff"),
            flatten: self.flatten,
            scale: finite_or(self.scale, 100.0).clamp(60.0, 130.0),
            anchor: self.anchor,
            offset_x: finite_or(self.offset_x, 0.0).clamp(-18.0, 18.0),
            offset_y: finite_or(self.offset_y, 0.0).clamp(-18.0, 18.0),
            mono: self.mono,
            mono_threshold: finite_or(self.mono_threshold, 20.0).clamp(1.0, 100.0),
            strip_corners: self.strip_corners,
            legacy: self.legacy,
        }
    }
}

// ---- source analysis -------------------------------------------------------------------

/// Fraction of the centred square to crop from each side so baked-in transparent rounded
/// corners (an .ico, an exported app icon…) disappear; 0 when the source isn't such a tile.
pub fn detect_corner_inset(img: &RgbaImage) -> f32 {
    let (w, h) = img.dimensions();
    let m = w.min(h);
    if m < 16 {
        return 0.0;
    }
    let (x0, y0) = ((w - m) / 2, (h - m) / 2);
    let a = |x: u32, y: u32| img.get_pixel(x0 + x, y0 + y)[3];
    let last = m - 1;
    let corners_opaque = |k: u32| {
        [a(k, k), a(last - k, k), a(k, last - k), a(last - k, last - k)].iter().all(|&v| v >= 250)
    };
    if corners_opaque(0) {
        return 0.0;
    }
    // A masked tile touches every side in the middle; a padded logo on transparency doesn't.
    let (mid, e) = (m / 2, (m / 64).max(1));
    if [a(mid, e), a(mid, last - e), a(e, mid), a(last - e, mid), a(mid, mid)]
        .iter()
        .any(|&v| v < 250)
    {
        return 0.0;
    }
    // A circle needs 14.6 % (1 − 1/√2)/2; anything deeper isn't a corner mask.
    let max_k = ((m as f32) * 0.16).ceil() as u32;
    (1..=max_k)
        .find(|&k| corners_opaque(k))
        // One extra pixel so the antialiased rim never survives the crop.
        .map_or(0.0, |k| ((k + 1) as f32 / m as f32).min(0.16))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Transparent,
    Flat,
    Varied,
}

/// Classify the 1-px ring just inside the art square's border and return its median colour.
fn analyze_edges(art: &RgbaImage) -> (EdgeKind, [u8; 3]) {
    let (w, h) = art.dimensions();
    let d = if w > 8 && h > 8 { 1 } else { 0 };
    let (x1, y1) = (w - 1 - d, h - 1 - d);
    let mut ring: Vec<[u8; 4]> = Vec::with_capacity(2 * (w + h) as usize);
    for x in d..=x1 {
        ring.push(art.get_pixel(x, d).0);
        ring.push(art.get_pixel(x, y1).0);
    }
    for y in d + 1..y1 {
        ring.push(art.get_pixel(d, y).0);
        ring.push(art.get_pixel(x1, y).0);
    }
    let n = ring.len().max(1);
    let transparent = ring.iter().filter(|p| p[3] < 128).count();
    let opaque: Vec<[u8; 4]> = ring.iter().copied().filter(|p| p[3] >= 250).collect();
    let median = |c: usize, px: &[[u8; 4]]| {
        let mut v: Vec<u8> = px.iter().map(|p| p[c]).collect();
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(255)
    };
    let pool = if opaque.is_empty() { &ring } else { &opaque };
    let med = [median(0, pool), median(1, pool), median(2, pool)];
    if transparent * 2 > n {
        return (EdgeKind::Transparent, med);
    }
    let close = opaque
        .iter()
        .filter(|p| (0..3).all(|c| (p[c] as i16 - med[c] as i16).abs() <= 12))
        .count();
    let flat = opaque.len() * 100 >= n * 97 && close * 100 >= n * 97;
    (if flat { EdgeKind::Flat } else { EdgeKind::Varied }, med)
}

fn luminance(rgb: [f32; 3]) -> f32 {
    (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) * 100.0
}

fn rgb8_to_f(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| v as f32 / 255.0)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// White silhouette (premultiplied) from the art's alpha, optionally keyed on luminance distance
/// from `ref_lum` so a dark glyph on a light tile (or vice versa) becomes the ink.
fn silhouette(art: &Rgba32FImage, ref_lum: Option<f32>, threshold: f32) -> Rgba32FImage {
    let mut out = art.clone();
    for px in out.as_mut().chunks_exact_mut(4) {
        let a = px[3].clamp(0.0, 1.0);
        let mut ink = a;
        if a > 1e-4 {
            if let Some(l) = ref_lum {
                let lum = luminance([px[0] / a, px[1] / a, px[2] / a].map(|c| c.clamp(0.0, 1.0)));
                ink *= smoothstep(threshold * 0.5, threshold * 1.5, (lum - l).abs());
            }
        } else {
            ink = 0.0;
        }
        px.copy_from_slice(&[ink, ink, ink, ink]);
    }
    out
}

// ---- planning & layers -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum BgLayer {
    Color,
    Image,
}

/// Everything resolved once per export / preview; layers are then rendered per density straight
/// from the full-resolution source.
pub struct Plan<'a> {
    source: &'a Source,
    s: Settings,
    a: AndroidSettings,
    inset: f32,
    corners_detected: bool,
    edge: EdgeKind,
    sampled: [u8; 3],
    bg: BgLayer,
    bg_rgb: [u8; 3],
    mono: Option<MonoPlan<'a>>,
    warnings: Vec<String>,
}

enum MonoPlan<'a> {
    Auto { ref_lum: Option<f32> },
    Custom { src: &'a Source, ref_lum: Option<f32>, inset: f32 },
}

/// Settings used to frame a custom monochrome image: fit inside the art box, no transforms.
fn mono_frame_settings() -> Settings {
    Settings { fit_mode: FitMode::Contain, ..Settings::default() }
}

impl<'a> Plan<'a> {
    pub fn new(source: &'a Source, settings: &Settings, mono_src: Option<&'a Source>) -> Plan<'a> {
        let s = settings.clone();
        let a = s.android.validated();
        let detected = detect_corner_inset(&source.img);
        let inset = if a.strip_corners { detected } else { 0.0 };
        let mut warnings = Vec::new();
        if detected > 0.0 && !a.strip_corners {
            warnings.push(
                "The source has baked-in rounded corners — turn on “Strip rounded corners” or they will show inside the launcher mask."
                    .into(),
            );
        }

        let probe = from_premul(&frame_art(source, &s, 192, inset));
        let (edge, sampled) = analyze_edges(&probe);
        let user_rgb = hex_to_rgb8(&a.bg_color);
        let (bg, bg_rgb) = match a.bg_mode {
            AndroidBg::Color => (BgLayer::Color, user_rgb),
            AndroidBg::Image => (BgLayer::Image, user_rgb),
            AndroidBg::Auto => match edge {
                EdgeKind::Flat => (BgLayer::Color, sampled),
                EdgeKind::Transparent => (BgLayer::Color, user_rgb),
                EdgeKind::Varied => (BgLayer::Image, user_rgb),
            },
        };

        let auto_lum = match edge {
            EdgeKind::Transparent => None,
            _ if bg == BgLayer::Color => Some(luminance(rgb8_to_f(bg_rgb))),
            _ => Some(luminance(rgb8_to_f(sampled))),
        };
        let mono = match a.mono {
            AndroidMono::Off => None,
            AndroidMono::Auto => Some(MonoPlan::Auto { ref_lum: auto_lum }),
            AndroidMono::Custom => match mono_src {
                Some(src) => {
                    let ms = mono_frame_settings();
                    let m_inset = detect_corner_inset(&src.img);
                    let (k, c) = analyze_edges(&from_premul(&frame_art(src, &ms, 192, m_inset)));
                    let ref_lum = (k != EdgeKind::Transparent).then(|| luminance(rgb8_to_f(c)));
                    Some(MonoPlan::Custom { src, ref_lum, inset: m_inset })
                }
                None => {
                    warnings.push("No monochrome image chosen — using the auto silhouette.".into());
                    Some(MonoPlan::Auto { ref_lum: auto_lum })
                }
            },
        };

        Plan {
            source,
            s,
            a,
            inset,
            corners_detected: detected > 0.0,
            edge,
            sampled,
            bg,
            bg_rgb,
            mono,
            warnings,
        }
    }

    pub fn uses_color_resource(&self) -> bool {
        self.bg == BgLayer::Color
    }

    pub fn has_mono(&self) -> bool {
        self.mono.is_some()
    }

    pub fn bg_rgb(&self) -> [u8; 3] {
        self.bg_rgb
    }

    /// Art side and top-left position on an `n`-px canvas.
    fn placement(&self, n: u32) -> (u32, i64, i64) {
        let nf = n as f32;
        let unit = nf / CANVAS_DP;
        let art = (nf * (VISIBLE_DP / CANVAS_DP) * self.a.scale / 100.0).round().max(1.0);
        let x = (nf - art) * 0.5 + self.a.offset_x * unit;
        let y = match self.a.anchor {
            AndroidAnchor::Center => (nf - art) * 0.5,
            AndroidAnchor::Bottom => nf * (CANVAS_DP + VISIBLE_DP) / (2.0 * CANVAS_DP) - art,
        } + self.a.offset_y * unit;
        (art as u32, x.round() as i64, y.round() as i64)
    }

    pub fn layers(&self, n: u32) -> Layers {
        let (side, x0, y0) = self.placement(n);
        let art = frame_art(self.source, &self.s, side, self.inset);
        let mut fg = Rgba32FImage::new(n, n);
        imageops::replace(&mut fg, &art, x0, y0);

        let under = rgb8_to_f(self.bg_rgb);
        let bg = match self.bg {
            BgLayer::Color => Rgba32FImage::from_pixel(n, n, Rgba([under[0], under[1], under[2], 1.0])),
            BgLayer::Image => {
                // Edge-extend the framed art over the whole canvas, then flatten onto the
                // underlay colour so the layer is opaque even where the art is transparent.
                let last = side as i64 - 1;
                Rgba32FImage::from_fn(n, n, |x, y| {
                    let sx = (x as i64 - x0).clamp(0, last) as u32;
                    let sy = (y as i64 - y0).clamp(0, last) as u32;
                    let p = art.get_pixel(sx, sy).0;
                    let inv = 1.0 - p[3].clamp(0.0, 1.0);
                    Rgba([p[0] + under[0] * inv, p[1] + under[1] * inv, p[2] + under[2] * inv, 1.0])
                })
            }
        };

        let mono = self.mono.as_ref().map(|m| {
            let sil = match m {
                MonoPlan::Auto { ref_lum } => silhouette(&art, *ref_lum, self.a.mono_threshold),
                MonoPlan::Custom { src, ref_lum, inset } => silhouette(
                    &frame_art(src, &mono_frame_settings(), side, *inset),
                    *ref_lum,
                    self.a.mono_threshold,
                ),
            };
            let mut canvas = Rgba32FImage::new(n, n);
            imageops::replace(&mut canvas, &sil, x0, y0);
            canvas
        });

        Layers { fg, bg, mono, flatten: self.a.flatten }
    }

    /// Check clipping & coverage on a preview-sized render and describe the resolved plan.
    pub fn analyze(&self, layers: &Layers) -> Analysis {
        let n = layers.fg.width();
        let nf = n as f32;
        let unit = nf / CANVAS_DP;
        let r = VISIBLE_RADIUS_DP * unit;
        let c = nf * 0.5;
        let reference = match self.bg {
            BgLayer::Color => None,
            // The image layer *is* the art, so measure against the edge colour instead.
            BgLayer::Image => Some(rgb8_to_f(self.sampled)),
        };
        let (mut total, mut outside) = (0usize, 0usize);
        let (mut bx0, mut by0, mut bx1, mut by1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for (x, y, p) in layers.fg.enumerate_pixels() {
            let a = p[3].clamp(0.0, 1.0);
            if a < 0.1 {
                continue;
            }
            let bgp = match reference {
                Some(rgb) => rgb,
                None => {
                    let b = layers.bg.get_pixel(x, y).0;
                    [b[0], b[1], b[2]]
                }
            };
            let diff = (0..3)
                .map(|k| ((p[k] + bgp[k] * (1.0 - a)) - bgp[k]).abs())
                .fold(0.0f32, f32::max);
            if diff < 0.1 {
                continue;
            }
            total += 1;
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            if (dx * dx + dy * dy).sqrt() > r + 0.5 {
                outside += 1;
            }
            bx0 = bx0.min(x);
            by0 = by0.min(y);
            bx1 = bx1.max(x + 1);
            by1 = by1.max(y + 1);
        }

        let mut warnings = self.warnings.clone();
        let clipped = if total > 0 { outside as f32 / total as f32 } else { 0.0 };
        let (bbox, coverage) = if total > 0 {
            let span = (bx1 - bx0).max(by1 - by0) as f32;
            (
                Some([bx0 as f32 / unit, by0 as f32 / unit, bx1 as f32 / unit, by1 as f32 / unit]),
                span / (VISIBLE_DP * unit),
            )
        } else {
            (None, 0.0)
        };
        if clipped > CLIP_WARN {
            warnings.push(format!(
                "{:.1}% of the artwork lies outside the 72 dp circle and will be cut off by round masks — try “Safe zone”.",
                clipped * 100.0
            ));
        }
        if total > 0 && coverage < COVERAGE_WARN {
            warnings.push(format!(
                "The artwork spans only {:.0}% of the visible width and may look small — try “Fit visible” or a larger scale.",
                coverage * 100.0
            ));
        }

        Analysis {
            bg_mode: if self.bg == BgLayer::Color { "color" } else { "image" },
            bg_color: hex(self.bg_rgb),
            edge: self.edge,
            sampled_color: hex(self.sampled),
            clipped_pct: clipped * 100.0,
            coverage_pct: coverage * 100.0,
            subject_bbox_dp: bbox,
            corners_detected: self.corners_detected,
            corners_stripped: self.inset > 0.0,
            mono: self.mono.is_some(),
            warnings,
        }
    }
}

fn hex_to_rgb8(h: &str) -> [u8; 3] {
    parse_hex_rgb(h).map(|v| (v * 255.0).round() as u8)
}

pub struct Layers {
    /// Foreground art, transparent outside the art box (premultiplied, never flattened).
    pub fg: Rgba32FImage,
    /// Opaque background layer.
    pub bg: Rgba32FImage,
    pub mono: Option<Rgba32FImage>,
    flatten: bool,
}

impl Layers {
    /// Foreground over background — what a launcher shows before masking. Always opaque.
    pub fn composite(&self) -> Rgba32FImage {
        let mut out = self.bg.clone();
        for (o, f) in out.as_mut().chunks_exact_mut(4).zip(self.fg.as_raw().chunks_exact(4)) {
            let inv = 1.0 - f[3].clamp(0.0, 1.0);
            for k in 0..3 {
                o[k] = f[k] + o[k] * inv;
            }
            o[3] = 1.0;
        }
        out
    }

    /// The foreground PNG as exported: transparent, or composited when "flatten" is on.
    pub fn foreground_out(&self) -> RgbaImage {
        if self.flatten {
            from_premul(&self.composite())
        } else {
            from_premul(&self.fg)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub bg_mode: &'static str,
    pub bg_color: String,
    pub edge: EdgeKind,
    pub sampled_color: String,
    pub clipped_pct: f32,
    pub coverage_pct: f32,
    /// Bounding box of the subject in dp on the 108 dp canvas: [x0, y0, x1, y1].
    pub subject_bbox_dp: Option<[f32; 4]>,
    pub corners_detected: bool,
    pub corners_stripped: bool,
    pub mono: bool,
    pub warnings: Vec<String>,
}

/// Preview payload: `u32 json_len · json · u32 side · RGBA composite · u32 mono_side · [RGBA mono]`.
pub fn preview_payload(source: &Source, s: &Settings, mono_src: Option<&Source>) -> AppResult<Vec<u8>> {
    let plan = Plan::new(source, s, mono_src);
    let layers = plan.layers(PREVIEW_SIDE);
    let analysis = plan.analyze(&layers);
    let json = serde_json::to_vec(&analysis).map_err(|e| AppError::Internal(e.to_string()))?;
    let comp = from_premul(&layers.composite());
    let mono = layers.mono.as_ref().map(from_premul);
    let px = (PREVIEW_SIDE * PREVIEW_SIDE * 4) as usize;
    let mut buf = Vec::with_capacity(12 + json.len() + px * 2);
    buf.extend_from_slice(&(json.len() as u32).to_le_bytes());
    buf.extend_from_slice(&json);
    buf.extend_from_slice(&PREVIEW_SIDE.to_le_bytes());
    buf.extend_from_slice(comp.as_raw());
    match mono {
        Some(m) => {
            buf.extend_from_slice(&PREVIEW_SIDE.to_le_bytes());
            buf.extend_from_slice(m.as_raw());
        }
        None => buf.extend_from_slice(&0u32.to_le_bytes()),
    }
    Ok(buf)
}

// ---- legacy & store icons --------------------------------------------------------------

/// Pre-API-26 launcher icon: the visible 72 dp of the composite, masked, inside a 48 dp tile
/// with the standard 2 dp margin.
fn legacy_icon(plan: &Plan, side: u32, shape: Shape) -> RgbaImage {
    let inner = ((side as f32) * 44.0 / 48.0).round() as u32;
    let n = ((inner as f32) * CANVAS_DP / VISIBLE_DP).round() as u32;
    let comp = plan.layers(n).composite();
    let off = (n - inner) / 2;
    let mut visible = imageops::crop_imm(&comp, off, off, inner, inner).to_image();
    for (x, y, p) in visible.enumerate_pixels_mut() {
        let cov = shape_coverage(x, y, inner, shape, 12.0);
        p.0.iter_mut().for_each(|v| *v *= cov);
    }
    let mut out = Rgba32FImage::new(side, side);
    let m = ((side - inner) / 2) as i64;
    imageops::replace(&mut out, &visible, m, m);
    from_premul(&out)
}

/// 512×512 store icon: the visible 72 dp, full square, opaque, no rounding (Play applies it).
fn play_store_icon(plan: &Plan) -> RgbaImage {
    let n = ((PLAY_STORE_SIDE as f32) * CANVAS_DP / VISIBLE_DP).round() as u32;
    let comp = plan.layers(n).composite();
    let off = (n - PLAY_STORE_SIDE) / 2;
    let mut img = from_premul(&imageops::crop_imm(&comp, off, off, PLAY_STORE_SIDE, PLAY_STORE_SIDE).to_image());
    img.pixels_mut().for_each(|p| p[3] = 255);
    img
}

// ---- XML & README ----------------------------------------------------------------------

/// Resource names used for one launcher icon. `ic_launcher` / `ic_launcher_round` by default;
/// a project can point `android:icon` at any other mipmap name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResNames {
    pub base: String,
    /// `None` when the manifest has no separate round icon (or it is the same as `base`).
    pub round: Option<String>,
    /// Name of the `<color>` resource used for a solid background.
    pub color: String,
}

impl Default for ResNames {
    fn default() -> Self {
        ResNames::new("ic_launcher", Some("ic_launcher_round"))
    }
}

impl ResNames {
    pub fn new(base: &str, round: Option<&str>) -> ResNames {
        ResNames {
            base: base.to_string(),
            round: round.filter(|r| *r != base).map(str::to_string),
            color: format!("{base}_background"),
        }
    }
}

pub fn adaptive_xml(color_bg: bool, mono: bool) -> String {
    adaptive_xml_named(&ResNames::default(), color_bg, mono)
}

pub fn adaptive_xml_named(n: &ResNames, color_bg: bool, mono: bool) -> String {
    let base = &n.base;
    let bg = if color_bg { format!("@color/{}", n.color) } else { format!("@mipmap/{base}_background") };
    let mut s = String::from(
        "<adaptive-icon xmlns:android=\"http://schemas.android.com/apk/res/android\">\n",
    );
    s.push_str(&format!("    <background android:drawable=\"{bg}\" />\n"));
    s.push_str(&format!("    <foreground android:drawable=\"@mipmap/{base}_foreground\" />\n"));
    if mono {
        s.push_str(&format!("    <monochrome android:drawable=\"@mipmap/{base}_monochrome\" />\n"));
    }
    s.push_str("</adaptive-icon>\n");
    s
}

pub fn color_xml(rgb: [u8; 3]) -> String {
    color_xml_named("ic_launcher_background", rgb)
}

pub fn color_xml_named(name: &str, rgb: [u8; 3]) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n    <color name=\"{name}\">#FF{:02X}{:02X}{:02X}</color>\n</resources>\n",
        rgb[0], rgb[1], rgb[2]
    )
}

const README: &str = "Android launcher icons generated by Simulacrum
================================================

1. Copy the res/ folder into your module's app/src/main/ folder
   (merge it with the existing res/ and let it replace the ic_launcher files).
2. Keep these attributes on <application> in AndroidManifest.xml:
       android:icon=\"@mipmap/ic_launcher\"
       android:roundIcon=\"@mipmap/ic_launcher_round\"
3. Upload play_store_512.png as the app icon in the Google Play Console.
   It is a full square without transparency — Play rounds the corners itself.

Adaptive icon layers are 108x108 dp; launchers show the middle 72 dp and may
crop to any shape that contains the central 66 dp circle.
";

// ---- export ----------------------------------------------------------------------------

/// Where `write_res` sends its files. The plain sink writes and deletes directly; the project
/// exporter backs originals up first.
pub trait ResSink {
    fn write(&mut self, path: &Path, bytes: &[u8]) -> AppResult<()>;
    /// A file an earlier run of this exporter may have written but this run does not.
    fn remove_stale(&mut self, path: &Path) -> AppResult<()>;
    fn mkdir(&mut self, dir: &Path) -> AppResult<()> {
        mkdir(dir)
    }
}

struct PlainSink<'a>(&'a mut Vec<String>);

impl ResSink for PlainSink<'_> {
    fn write(&mut self, path: &Path, bytes: &[u8]) -> AppResult<()> {
        pipeline::write_atomic_file(path, bytes)?;
        self.0.push(path.display().to_string());
        Ok(())
    }
    fn remove_stale(&mut self, path: &Path) -> AppResult<()> {
        remove_stale(path)
    }
}

/// Remove a file this exporter wrote on an earlier run but not this time.
fn remove_stale(path: &Path) -> AppResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::write(path, e)),
    }
}

fn mkdir(p: &Path) -> AppResult<()> {
    std::fs::create_dir_all(p).map_err(|e| AppError::write(p, e))
}

/// The 512 px Play Store icon as PNG bytes.
pub fn play_store_png(plan: &Plan) -> AppResult<Vec<u8>> {
    encode_png(&play_store_icon(plan), false)
}

/// Write every launcher resource for `plan` into an Android `res/` folder.
pub fn write_res(plan: &Plan, res: &Path, names: &ResNames, sink: &mut dyn ResSink) -> AppResult<()> {
    let png = |img: &RgbaImage| encode_png(img, false);
    let base = &names.base;

    for (density, n) in DENSITIES {
        let dir = res.join(format!("mipmap-{density}"));
        sink.mkdir(&dir)?;
        let layers = plan.layers(n);
        sink.write(&dir.join(format!("{base}_foreground.png")), &png(&layers.foreground_out())?)?;
        let bg_path = dir.join(format!("{base}_background.png"));
        if plan.uses_color_resource() {
            sink.remove_stale(&bg_path)?;
        } else {
            sink.write(&bg_path, &png(&from_premul(&layers.bg))?)?;
        }
        let mono_path = dir.join(format!("{base}_monochrome.png"));
        match &layers.mono {
            Some(m) => sink.write(&mono_path, &png(&from_premul(m))?)?,
            None => sink.remove_stale(&mono_path)?,
        }
    }

    for (density, side) in LEGACY_DENSITIES {
        let dir = res.join(format!("mipmap-{density}"));
        let sq = dir.join(format!("{base}.png"));
        let round = names.round.as_ref().map(|r| dir.join(format!("{r}.png")));
        if plan.a.legacy {
            sink.write(&sq, &png(&legacy_icon(plan, side, Shape::Rounded))?)?;
            if let Some(round) = &round {
                sink.write(round, &png(&legacy_icon(plan, side, Shape::Circle))?)?;
            }
        } else {
            sink.remove_stale(&sq)?;
            if let Some(round) = &round {
                sink.remove_stale(round)?;
            }
        }
    }

    let values = res.join("values");
    let color_path = values.join(format!("{}.xml", names.color));
    if plan.uses_color_resource() {
        sink.mkdir(&values)?;
        sink.write(&color_path, color_xml_named(&names.color, plan.bg_rgb()).as_bytes())?;
    } else {
        sink.remove_stale(&color_path)?;
    }

    let anydpi = res.join("mipmap-anydpi-v26");
    sink.mkdir(&anydpi)?;
    let xml = adaptive_xml_named(names, plan.uses_color_resource(), plan.has_mono());
    sink.write(&anydpi.join(format!("{base}.xml")), xml.as_bytes())?;
    if let Some(r) = &names.round {
        sink.write(&anydpi.join(format!("{r}.xml")), xml.as_bytes())?;
    }
    Ok(())
}

/// Write `<out_dir>/android/` and return every file written.
pub fn export(source: &Source, s: &Settings, mono_src: Option<&Source>, out_dir: &Path) -> AppResult<Vec<String>> {
    if !out_dir.is_dir() {
        return Err(AppError::Write {
            path: out_dir.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "folder does not exist"),
        });
    }
    let plan = Plan::new(source, s, mono_src);
    let root = out_dir.join("android");
    let mut written = Vec::new();
    let mut sink = PlainSink(&mut written);
    write_res(&plan, &root.join("res"), &ResNames::default(), &mut sink)?;
    sink.write(&root.join("play_store_512.png"), &play_store_png(&plan)?)?;
    sink.write(&root.join("README.txt"), README.as_bytes())?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(w: u32, h: u32, rgb: [u8; 3]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([rgb[0], rgb[1], rgb[2], 255]))
    }

    #[test]
    fn detects_rounded_corners_but_not_padded_logos() {
        let src = pipeline::from_rgba({
            let mut img = tile(256, 256, [200, 30, 30]);
            for (x, y, p) in img.enumerate_pixels_mut() {
                if shape_coverage(x, y, 256, Shape::Rounded, 22.0) < 0.5 {
                    p[3] = 0;
                }
            }
            img
        })
        .unwrap();
        let inset = detect_corner_inset(&src.img);
        assert!(inset > 0.0 && inset < 0.16, "inset {inset}");

        let mut logo = RgbaImage::new(256, 256);
        for (x, y, p) in logo.enumerate_pixels_mut() {
            if (64..192).contains(&x) && (64..192).contains(&y) {
                *p = Rgba([0, 0, 0, 255]);
            }
        }
        assert_eq!(detect_corner_inset(&logo), 0.0);
        assert_eq!(detect_corner_inset(&tile(256, 256, [1, 2, 3])), 0.0);
    }

    #[test]
    fn edge_classification() {
        assert_eq!(analyze_edges(&tile(64, 64, [10, 20, 30])), (EdgeKind::Flat, [10, 20, 30]));
        assert_eq!(analyze_edges(&RgbaImage::new(64, 64)).0, EdgeKind::Transparent);
        let grad = RgbaImage::from_fn(64, 64, |x, _| Rgba([(x * 4) as u8, 0, 0, 255]));
        assert_eq!(analyze_edges(&grad).0, EdgeKind::Varied);
    }
}
