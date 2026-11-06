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
}
