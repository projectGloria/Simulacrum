//! Separable Lanczos3 resampler for premultiplied f32 RGBA, multithreaded over rows.
//!
//! Why not `image::imageops::resize`? Measured on a 1024² master it runs at ~5 ns per filter
//! tap through its generic `Pixel` machinery, and a 1024→16 downscale has 384 taps per output
//! pixel — so a 16 px icon cost as much as a 256 px one (~30 ms each). It also clamps f32
//! channels to [0,1] independently, which forced a shift/unshift copy of the whole master on
//! every call. This version precomputes the weights once per axis, runs tight `[f32; 4]`
//! loops, never clamps (callers handle overshoot in `from_premul`), and splits rows across
//! cores.

use image::Rgba32FImage;

#[inline]
fn lanczos3(x: f32) -> f32 {
    let x = x.abs();
    if x < 1e-6 {
        1.0
    } else if x >= 3.0 {
        0.0
    } else {
        let px = std::f32::consts::PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    }
}

/// Contiguous source window + normalised weights for one output sample.
struct Taps {
    start: usize,
    w: Vec<f32>,
}

/// Same geometry as `image`'s resampler: sample centres at (o + 0.5)·ratio, and when
/// downscaling the kernel is stretched by the ratio so it acts as a proper low-pass filter.
fn taps(src: usize, dst: usize) -> Vec<Taps> {
    let ratio = src as f32 / dst as f32;
    let scale = ratio.max(1.0);
    let support = 3.0 * scale;
    (0..dst)
        .map(|o| {
            let centre = (o as f32 + 0.5) * ratio;
            let lo = (centre - support).floor().max(0.0) as usize;
            let hi = ((centre + support).ceil() as usize).min(src).max(lo + 1);
            let mut w: Vec<f32> = (lo..hi).map(|i| lanczos3((i as f32 + 0.5 - centre) / scale)).collect();
            // Normalise so flat regions stay exactly flat (weights are truncated at the edges).
            let sum: f32 = w.iter().sum();
            if sum.abs() > f32::EPSILON {
                w.iter_mut().for_each(|v| *v /= sum);
            }
            Taps { start: lo, w }
        })
        .collect()
}

/// Run `f(row_index, row)` over every `row_len`-sized row of `buf`, split across cores.
/// Scoped threads instead of a rayon dependency: spawn cost (~30 µs) is noise next to a pass.
pub(crate) fn par_rows(buf: &mut [f32], row_len: usize, f: impl Fn(usize, &mut [f32]) + Sync) {
    let rows = buf.len() / row_len;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(rows).max(1);
    // Tiny jobs (16 px icons) aren't worth waking threads for.
    if threads == 1 || buf.len() < 64 * 64 * 4 {
        buf.chunks_mut(row_len).enumerate().for_each(|(y, r)| f(y, r));
        return;
    }
    let per = rows.div_ceil(threads);
    let f = &f;
    std::thread::scope(|s| {
        for (ci, chunk) in buf.chunks_mut(per * row_len).enumerate() {
            s.spawn(move || chunk.chunks_mut(row_len).enumerate().for_each(|(j, r)| f(ci * per + j, r)));
        }
    });
}

/// Resize to `w`×`h`. Input and output are premultiplied; values may overshoot [0,1].
pub fn resize(img: &Rgba32FImage, w: u32, h: u32) -> Rgba32FImage {
    let (sw, sh) = (img.width() as usize, img.height() as usize);
    let (w, h) = (w as usize, h as usize);
    if (sw, sh) == (w, h) {
        return img.clone();
    }
    let src = img.as_raw();

    // Horizontal pass: sh rows of sw → sh rows of w.
    let hx = taps(sw, w);
    let mut tmp = vec![0f32; w * sh * 4];
    par_rows(&mut tmp, w * 4, |y, row| {
        let s = &src[y * sw * 4..(y + 1) * sw * 4];
        for (x, t) in hx.iter().enumerate() {
            let mut acc = [0f32; 4];
            let px = &s[t.start * 4..(t.start + t.w.len()) * 4];
            for (wt, p) in t.w.iter().zip(px.chunks_exact(4)) {
                for c in 0..4 {
                    acc[c] += wt * p[c];
                }
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&acc);
        }
    });

    // Vertical pass: each output row is a weighted sum of whole tmp rows — a straight axpy
    // over contiguous memory, which LLVM auto-vectorises.
    let vy = taps(sh, h);
    let mut out = vec![0f32; w * h * 4];
    par_rows(&mut out, w * 4, |y, row| {
        let t = &vy[y];
        for (k, wt) in t.w.iter().enumerate() {
            let s = &tmp[(t.start + k) * w * 4..(t.start + k + 1) * w * 4];
            for (o, v) in row.iter_mut().zip(s) {
                *o += wt * v;
            }
        }
    });
    Rgba32FImage::from_raw(w as u32, h as u32, out).expect("buffer sized to w*h*4")
}

fn taps_mapped(
    src: usize,
    dst: usize,
    dst_origin: f32,
    scale: f32,
    transparent_outside: bool,
) -> Vec<Taps> {
    let ratio = 1.0 / scale.max(1e-4);
    let filter_scale = ratio.max(1.0);
    let support = 3.0 * filter_scale;
    (0..dst)
        .map(|o| {
            let centre = (o as f32 + 0.5 - dst_origin) * ratio;
            let i_lo = (centre - support).floor() as isize;
            let i_hi = (centre + support).ceil() as isize;
            if i_hi <= 0 || i_lo >= src as isize {
                return Taps { start: 0, w: Vec::new() };
            }
            let lo = i_lo.max(0) as usize;
            let hi = (i_hi as usize).min(src);
            if lo >= hi {
                return Taps { start: 0, w: Vec::new() };
            }
            let mut w: Vec<f32> =
                (lo..hi).map(|i| lanczos3((i as f32 + 0.5 - centre) / filter_scale)).collect();
            let sum: f32 = if transparent_outside {
                (i_lo..i_hi).map(|i| lanczos3((i as f32 + 0.5 - centre) / filter_scale)).sum()
            } else {
                w.iter().sum()
            };
            if sum.abs() > f32::EPSILON {
                w.iter_mut().for_each(|v| *v /= sum);
            }
            Taps { start: lo, w }
        })
        .collect()
}

/// Resize & position `img` onto an `out_w × out_h` canvas where source pixel `(0, 0)` lands at
/// `(dst_x0, dst_y0)` scaled by `scale`. Evaluates exact separable Lanczos3 without creating an
/// oversized intermediate image when zoomed in.
pub fn resize_mapped(
    img: &Rgba32FImage,
    out_w: u32,
    out_h: u32,
    dst_x0: f32,
    dst_y0: f32,
    scale: f32,
    clamp_edges: bool,
) -> Rgba32FImage {
    let (sw, sh) = (img.width() as usize, img.height() as usize);
    let (w, h) = (out_w as usize, out_h as usize);
    let hx = taps_mapped(sw, w, dst_x0, scale, !clamp_edges);
    let vy = taps_mapped(sh, h, dst_y0, scale, !clamp_edges);

    let y_min = vy.iter().filter(|t| !t.w.is_empty()).map(|t| t.start).min();
    let y_max = vy.iter().filter(|t| !t.w.is_empty()).map(|t| t.start + t.w.len()).max();
    let (Some(y_min), Some(y_max)) = (y_min, y_max) else {
        return Rgba32FImage::new(out_w, out_h);
    };

    let src = img.as_raw();
    let mut tmp = vec![0f32; w * (y_max - y_min) * 4];
    par_rows(&mut tmp, w * 4, |dy, row| {
        let y = y_min + dy;
        let s = &src[y * sw * 4..(y + 1) * sw * 4];
        for (x, t) in hx.iter().enumerate() {
            if t.w.is_empty() {
                continue;
            }
            let mut acc = [0f32; 4];
            let px = &s[t.start * 4..(t.start + t.w.len()) * 4];
            for (wt, p) in t.w.iter().zip(px.chunks_exact(4)) {
                for c in 0..4 {
                    acc[c] += wt * p[c];
                }
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&acc);
        }
    });

    let mut out = vec![0f32; w * h * 4];
    par_rows(&mut out, w * 4, |y, row| {
        let t = &vy[y];
        for (k, wt) in t.w.iter().enumerate() {
            let s = &tmp[(t.start + k - y_min) * w * 4..(t.start + k - y_min + 1) * w * 4];
            for (o, v) in row.iter_mut().zip(s) {
                *o += wt * v;
            }
        }
    });
    Rgba32FImage::from_raw(out_w, out_h, out).expect("buffer sized to w*h*4")
}

/// Resolution-adaptive premultiplied unsharp mask. Neighborhood averages are weighted by
/// sample opacity so transparent borders never darken or bleed into opaque edges.
pub fn sharpen_premul(img: &Rgba32FImage, strength: f32) -> Rgba32FImage {
    let strength = strength.clamp(0.0, 1.0);
    if strength <= 1e-4 {
        return img.clone();
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    let src = img.as_raw();
    let mut out = src.clone();

    for y in 0..h {
        let y0 = y.saturating_sub(1);
        let y1 = (y + 1).min(h - 1);
        for x in 0..w {
            let idx = (y * w + x) * 4;
            let a = src[idx + 3];
            if a <= 1e-3 {
                continue;
            }
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(w - 1);

            let mut sum_rgb = [0f32; 3];
            let mut sum_a = 0f32;
            for ny in y0..=y1 {
                for nx in x0..=x1 {
                    let ni = (ny * w + nx) * 4;
                    let na = src[ni + 3].max(0.0);
                    if na > 0.0 {
                        sum_a += na;
                        for c in 0..3 {
                            sum_rgb[c] += src[ni + c];
                        }
                    }
                }
            }
            if sum_a <= 1e-4 {
                continue;
            }
            let max_c = a.max(0.0);
            for c in 0..3 {
                let avg_premul = (sum_rgb[c] / sum_a) * a;
                let boosted = src[idx + c] + strength * 0.65 * (src[idx + c] - avg_premul);
                out[idx + c] = boosted.clamp(0.0, max_c);
            }
        }
    }
    Rgba32FImage::from_raw(w as u32, h as u32, out).expect("buffer sized to w*h*4")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn flat_stays_flat() {
        let img = Rgba32FImage::from_pixel(300, 300, Rgba([0.2, 0.4, 0.6, 1.0]));
        for side in [16, 48, 256, 512] {
            let out = resize(&img, side, side);
            for p in out.pixels() {
                for (a, b) in p.0.iter().zip([0.2, 0.4, 0.6, 1.0]) {
                    assert!((a - b).abs() < 1e-4, "{side}: {:?}", p);
                }
            }
        }
    }

    #[test]
    fn weights_sum_to_one() {
        for (s, d) in [(1024, 16), (1024, 512), (200, 1024), (7, 3)] {
            for t in taps(s, d) {
                assert!((t.w.iter().sum::<f32>() - 1.0).abs() < 1e-4);
                assert!(t.start + t.w.len() <= s);
            }
        }
    }

    #[test]
    fn sharpen_preserves_flat_and_boosts_step_edge() {
        let flat = Rgba32FImage::from_pixel(16, 16, Rgba([0.3, 0.5, 0.7, 1.0]));
        let flat_out = sharpen_premul(&flat, 1.0);
        for p in flat_out.pixels() {
            for (a, b) in p.0.iter().zip([0.3, 0.5, 0.7, 1.0]) {
                assert!((a - b).abs() < 1e-5);
            }
        }

        let step = Rgba32FImage::from_fn(16, 16, |x, _| {
            if x < 8 {
                Rgba([0.2, 0.2, 0.2, 1.0])
            } else {
                Rgba([0.8, 0.8, 0.8, 1.0])
            }
        });
        let sharp = sharpen_premul(&step, 1.0);
        assert!(sharp.get_pixel(7, 8)[0] < 0.2);
        assert!(sharp.get_pixel(8, 8)[0] > 0.8);
        for p in sharp.pixels() {
            let a = p[3];
            assert!(p[0] >= 0.0 && p[0] <= a);
            assert!(p[1] >= 0.0 && p[1] <= a);
            assert!(p[2] >= 0.0 && p[2] <= a);
        }
    }
}
