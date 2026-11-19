//! Shape masks evaluated via analytic signed-distance functions (SDF).
//!
//! `d` is the signed distance in pixels to the shape boundary (negative inside). `0.5 - d`
//! maps a 1-px band straddling the edge to a linear 0→1 ramp, giving crisp anti-aliasing
//! without supersampling cost.

use crate::pipeline::Shape;

#[inline]
fn rounded_square_sdf_q(x: f32, y: f32, half: f32, r: f32) -> f32 {
    let r = r.clamp(0.0, half);
    let qx = x - (half - r);
    let qy = y - (half - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - r
}

#[inline]
fn squircle_sdf_q(x: f32, y: f32, half: f32, r: f32) -> f32 {
    let r = r.clamp(0.0, half);
    let qx = x - (half - r);
    let qy = y - (half - r);
    if qx > 0.0 && qy > 0.0 && r > 0.5 {
        let u = qx / r;
        let v = qy / r;
        let s = u.max(v);
        // Scale-invariant gradient normalization prevents underflow near (qx, qy) = (0, 0).
        let uh = u / s;
        let vh = v / s;
        let mh = (uh.powi(4) + vh.powi(4)).powf(0.25);
        let m = s * mh;
        let g = (uh.powi(6) + vh.powi(6)).sqrt() / mh.powi(3);
        (m - 1.0) * r / g
    } else {
        qx.max(qy) - r
    }
}

#[inline]
fn hexagon_sdf_q(x: f32, y: f32, half: f32, radius_pct: f32) -> f32 {
    // Pointy-top regular hexagon: swap (x, y) into flat-top hexagon SDF.
    let r_corner = (half * (radius_pct / 100.0) * 0.4).clamp(0.0, half * 0.35);
    let a = (half - r_corner) * 0.8660254;
    let kx = -0.866025404_f32;
    let ky = 0.5_f32;
    let kz = 0.577350269_f32;
    let mut px = y;
    let mut py = x;
    let refl = 2.0 * (kx * px + ky * py).min(0.0);
    px -= refl * kx;
    py -= refl * ky;
    px -= px.clamp(-kz * a, kz * a);
    py -= a;
    let d_hex = (px * px + py * py).sqrt() * py.signum();
    d_hex - r_corner
}

#[inline]
fn octagon_sdf_q(x: f32, y: f32, half: f32, radius_pct: f32) -> f32 {
    let r_corner = (half * (radius_pct / 100.0) * 0.4).clamp(0.0, half * 0.35);
    let a = half - r_corner;
    let kx = -0.9238795325_f32;
    let ky = 0.3826834323_f32;
    let kz = 0.4142135623_f32;
    let mut px = x;
    let mut py = y;
    let r1 = 2.0 * (kx * px + ky * py).min(0.0);
    px -= r1 * kx;
    py -= r1 * ky;
    let r2 = 2.0 * (-kx * px + ky * py).min(0.0);
    px -= r2 * (-kx);
    py -= r2 * ky;
    px -= px.clamp(-kz * a, kz * a);
    py -= a;
    let d_oct = (px * px + py * py).sqrt() * py.signum();
    d_oct - r_corner
}

#[inline]
fn shield_sdf_f(fx: f32, fy: f32, side: f32, radius_pct: f32) -> f32 {
    let half = side / 2.0;
    let sx = (fx - half).abs() / half;
    let sy = (fy - half) / half;

    // Gentle arched top passing through (0, -0.92) and (±0.86, -0.76).
    let d_top = (sx * sx + (sy - 1.47).powi(2)).sqrt() - 2.39;

    // Vertical sides above y = -0.10, joining C1-tangentially to curved lower taper to (0, 0.92).
    let d_right = if sy < -0.10 {
        sx - 0.86
    } else {
        ((sx + 0.17488).powi(2) + (sy + 0.10).powi(2)).sqrt() - 1.03488
    };

    let r_c = (radius_pct / 100.0 * 0.35).clamp(0.02, 0.15);
    let d1 = d_top + r_c;
    let d2 = d_right + r_c;
    let d_base = if d1 > 0.0 && d2 > 0.0 { (d1 * d1 + d2 * d2).sqrt() } else { d1.max(d2) };
    (d_base - r_c) * half
}

/// Coverage (0..=1) of pixel (px, py) for a rounded square filling an `n`×`n` canvas with
/// corner radius `r` (pixels). Evaluated at the pixel centre.
#[inline]
pub fn rounded_square_coverage(px: u32, py: u32, n: u32, r: f32) -> f32 {
    let half = n as f32 / 2.0;
    let x = (px as f32 + 0.5 - half).abs();
    let y = (py as f32 + 0.5 - half).abs();
    let d = rounded_square_sdf_q(x, y, half, r);
    (0.5 - d).clamp(0.0, 1.0)
}

/// Signed distance in pixels (negative inside, positive outside) from arbitrary point `(fx, fy)`
/// to the boundary of `shape` occupying `[0..side] × [0..side]`.
#[inline]
pub fn shape_sdf_f(fx: f32, fy: f32, side: f32, shape: Shape, radius_pct: f32) -> f32 {
    let half = side / 2.0;
    let x = (fx - half).abs();
    let y = (fy - half).abs();
    match shape {
        Shape::Rounded => rounded_square_sdf_q(x, y, half, side * (radius_pct / 100.0)),
        Shape::Squircle => squircle_sdf_q(x, y, half, side * (radius_pct / 100.0)),
        Shape::Circle => rounded_square_sdf_q(x, y, half, half),
        Shape::Hexagon => hexagon_sdf_q(x, y, half, radius_pct),
        Shape::Octagon => octagon_sdf_q(x, y, half, radius_pct),
        Shape::Shield => shield_sdf_f(fx, fy, side, radius_pct),
        Shape::None => x.max(y) - half,
    }
}

/// Signed distance in pixels (negative inside, positive outside) from pixel `(px, py)` centre
/// to the boundary of `shape` on an `n`×`n` canvas.
#[inline]
pub fn shape_sdf(px: u32, py: u32, n: u32, shape: Shape, radius_pct: f32) -> f32 {
    shape_sdf_f(px as f32 + 0.5, py as f32 + 0.5, n as f32, shape, radius_pct)
}

/// Analytic anti-aliased coverage (0..=1) of pixel `(px, py)` for `shape` on an `n`×`n` canvas.
#[inline]
pub fn shape_coverage(px: u32, py: u32, n: u32, shape: Shape, radius_pct: f32) -> f32 {
    match shape {
        Shape::None => 1.0,
        _ => (0.5 - shape_sdf(px, py, n, shape, radius_pct)).clamp(0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_transparent_centre_opaque() {
        let n = 512;
        let r = n as f32 * 0.22;
        assert_eq!(rounded_square_coverage(0, 0, n, r), 0.0);
        assert_eq!(rounded_square_coverage(n - 1, n - 1, n, r), 0.0);
        assert_eq!(rounded_square_coverage(n / 2, n / 2, n, r), 1.0);
        // Middle of the top edge is inside the straight segment → fully covered.
        assert_eq!(rounded_square_coverage(n / 2, 0, n, r), 1.0);
    }

    #[test]
    fn edge_is_antialiased() {
        // Along the 45° diagonal of a circle there must be at least one partial pixel.
        let n = 256;
        let partial = (0..n)
            .map(|i| rounded_square_coverage(i, i, n, n as f32 / 2.0))
            .filter(|c| *c > 0.0 && *c < 1.0)
            .count();
        assert!(partial >= 1);
    }

    #[test]
    fn zero_radius_is_full_square() {
        let n = 64;
        for y in 0..n {
            for x in 0..n {
                assert_eq!(rounded_square_coverage(x, y, n, 0.0), 1.0);
            }
        }
    }

    #[test]
    fn new_shapes_have_transparent_corners_opaque_centre_and_aa_edges() {
        let n = 256;
        for shape in [Shape::Squircle, Shape::Hexagon, Shape::Octagon, Shape::Shield] {
            assert_eq!(shape_coverage(0, 0, n, shape, 22.0), 0.0, "{shape:?} top-left");
            assert_eq!(shape_coverage(n - 1, n - 1, n, shape, 22.0), 0.0, "{shape:?} bottom-right");
            assert_eq!(shape_coverage(n / 2, n / 2, n, shape, 22.0), 1.0, "{shape:?} centre");

            let partial = (0..n)
                .flat_map(|y| (0..n).map(move |x| (x, y)))
                .map(|(x, y)| shape_coverage(x, y, n, shape, 22.0))
                .filter(|&c| c > 0.0 && c < 1.0)
                .count();
            assert!(partial >= 4, "{shape:?} should have antialiased edges, got {partial}");
        }
    }

    #[test]
    fn none_shape_is_always_full_coverage() {
        let n = 64;
        assert_eq!(shape_coverage(0, 0, n, Shape::None, 22.0), 1.0);
        assert!(shape_sdf(0, 0, n, Shape::None, 22.0) < 0.0);
    }
}
