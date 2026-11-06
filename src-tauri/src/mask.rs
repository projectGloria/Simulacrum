//! Shape masks. Rounded square and circle share one signed-distance function: a circle is
//! just a rounded square whose corner radius is half the side.

/// Coverage (0..=1) of pixel (px, py) for a rounded square filling an `n`×`n` canvas with
/// corner radius `r` (pixels). Evaluated at the pixel centre.
///
/// `d` is the signed distance to the shape edge (negative inside). `0.5 - d` maps a 1-px band
/// straddling the edge to a linear 0→1 ramp, which is a good analytic approximation of the
/// area of the pixel covered by the shape — i.e. anti-aliasing without supersampling cost.
#[inline]
pub fn rounded_square_coverage(px: u32, py: u32, n: u32, r: f32) -> f32 {
    let half = n as f32 / 2.0;
    let r = r.clamp(0.0, half);
    // Work in the first quadrant relative to the centre; the shape is symmetric.
    let x = (px as f32 + 0.5 - half).abs();
    let y = (py as f32 + 0.5 - half).abs();
    // Standard rounded-box SDF (Inigo Quilez): box half-extent shrunk by r, then inflated by r.
    let qx = x - (half - r);
    let qy = y - (half - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    let d = outside + inside - r;
    (0.5 - d).clamp(0.0, 1.0)
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
}
