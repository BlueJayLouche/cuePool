//! Homography solve + 3×3 inverse for the corner-pin warp.
//!
//! Ported from `rustjay-projection/src/warp.rs`, reduced to what the
//! projection renderer needs: given the output quad an image should land in
//! ([`WarpCorners`], normalized output UV), produce the inverse-warp matrix
//! the shader uses to map each fragment back to unwarped canvas UV.

use cuepool_core::WarpCorners;

/// Compute a forward homography mapping `src_corners` to `dst_corners`.
/// Returns the 3×3 matrix as a flat, row-major `[f32; 9]`.
pub fn compute_forward_homography(
    src_corners: &[[f32; 2]; 4],
    dst_corners: &[[f32; 2]; 4],
) -> [f32; 9] {
    solve_homography(src_corners, dst_corners)
}

fn solve_homography(src: &[[f32; 2]; 4], dst: &[[f32; 2]; 4]) -> [f32; 9] {
    let mut a = [[0.0_f64; 8]; 8];
    let mut b = [0.0_f64; 8];

    for i in 0..4 {
        let (sx, sy) = (src[i][0] as f64, src[i][1] as f64);
        let (dx, dy) = (dst[i][0] as f64, dst[i][1] as f64);
        let row1 = i * 2;
        let row2 = i * 2 + 1;
        a[row1] = [sx, sy, 1.0, 0.0, 0.0, 0.0, -sx * dx, -sy * dx];
        b[row1] = dx;
        a[row2] = [0.0, 0.0, 0.0, sx, sy, 1.0, -sx * dy, -sy * dy];
        b[row2] = dy;
    }

    let h = gauss_solve_8x8(&mut a, &mut b);
    [
        h[0] as f32,
        h[1] as f32,
        h[2] as f32,
        h[3] as f32,
        h[4] as f32,
        h[5] as f32,
        h[6] as f32,
        h[7] as f32,
        1.0,
    ]
}

#[allow(clippy::needless_range_loop)]
fn gauss_solve_8x8(a: &mut [[f64; 8]; 8], b: &mut [f64; 8]) -> [f64; 8] {
    let n = 8;
    for col in 0..n {
        let mut max_row = col;
        let mut max_val = a[col][col].abs();
        for row in (col + 1)..n {
            if a[row][col].abs() > max_val {
                max_val = a[row][col].abs();
                max_row = row;
            }
        }
        a.swap(col, max_row);
        b.swap(col, max_row);

        let pivot = a[col][col];
        if pivot.abs() < 1e-12 {
            log::warn!("Degenerate homography: pivot near zero");
            return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        }

        for row in (col + 1)..n {
            let factor = a[row][col] / pivot;
            for k in col..n {
                a[row][k] -= factor * a[col][k];
            }
            b[row] -= factor * b[col];
        }
    }

    let mut x = [0.0_f64; 8];
    for col in (0..n).rev() {
        x[col] = b[col];
        for k in (col + 1)..n {
            x[col] -= a[col][k] * x[k];
        }
        x[col] /= a[col][col];
    }
    x
}

/// Multiply two row-major 3×3 matrices: `a · b` (applies `b` first).
pub fn mul_3x3(a: [f32; 9], b: [f32; 9]) -> [f32; 9] {
    let mut out = [0.0; 9];
    for (row, out_row) in out.chunks_exact_mut(3).enumerate() {
        for (col, cell) in out_row.iter_mut().enumerate() {
            *cell = (0..3).fold(0.0, |acc, k| acc + a[row * 3 + k] * b[k * 3 + col]);
        }
    }
    out
}

/// Apply a row-major 3×3 homography to a 2D point, with perspective divide.
pub fn apply_3x3(m: [f32; 9], p: [f32; 2]) -> [f32; 2] {
    let x = m[0] * p[0] + m[1] * p[1] + m[2];
    let y = m[3] * p[0] + m[4] * p[1] + m[5];
    let w = m[6] * p[0] + m[7] * p[1] + m[8];
    [x / w, y / w]
}

/// Invert a row-major 3×3. Singular input falls back to identity so a bad
/// calibration degrades to "no warp" instead of a black screen.
pub fn invert_3x3(m: [f32; 9]) -> [f32; 9] {
    let [a, b, c, d, e, f, g, h, i] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        log::warn!("Singular warp homography: falling back to identity");
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    let inv = 1.0 / det;
    [
        (e * i - f * h) * inv,
        (c * h - b * i) * inv,
        (b * f - c * e) * inv,
        (f * g - d * i) * inv,
        (a * i - c * g) * inv,
        (c * d - a * f) * inv,
        (d * h - e * g) * inv,
        (b * g - a * h) * inv,
        (a * e - b * d) * inv,
    ]
}

/// Shader-ready inverse-warp matrix: three padded rows mapping fragment UV
/// back to unwarped UV. Identity corners yield the identity matrix.
pub fn warp_matrix_rows(corners: &WarpCorners) -> [[f32; 4]; 3] {
    let unit = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let forward = compute_forward_homography(&unit, &corners.0);
    let m = invert_3x3(forward);
    [
        [m[0], m[1], m[2], 0.0],
        [m[3], m[4], m[5], 0.0],
        [m[6], m[7], m[8], 0.0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_then_inverse_is_identity() {
        let unit = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let quad = [[0.2, 0.1], [0.95, 0.0], [1.0, 0.9], [0.05, 1.0]];
        let h = compute_forward_homography(&unit, &quad);
        let roundtrip = mul_3x3(invert_3x3(h), h);
        for p in [[0.0, 0.0], [0.37, 0.62], [1.0, 1.0]] {
            let got = apply_3x3(roundtrip, p);
            assert!(
                (got[0] - p[0]).abs() < 1e-4 && (got[1] - p[1]).abs() < 1e-4,
                "{p:?} round-tripped to {got:?}"
            );
        }
    }

    #[test]
    fn mul_composes_maps() {
        // mul(scale, translate) applies translate first, then scale:
        // (1, 1) → (3, 4) → (6, 8).
        let translate = [1.0, 0.0, 2.0, 0.0, 1.0, 3.0, 0.0, 0.0, 1.0];
        let scale = [2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0];
        let composed = mul_3x3(scale, translate);
        assert_eq!(apply_3x3(composed, [1.0, 1.0]), [6.0, 8.0]);
    }

    #[test]
    fn apply_does_perspective_divide() {
        // x' = x / (x + 1), y' = y / (x + 1).
        let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let got = apply_3x3(m, [3.0, 8.0]);
        assert!((got[0] - 0.75).abs() < 1e-6 && (got[1] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn identity_corners_give_identity_rows() {
        let rows = warp_matrix_rows(&WarpCorners::default());
        for (uv, expected) in [
            ([0.0, 0.0], [0.0, 0.0]),
            ([1.0, 0.0], [1.0, 0.0]),
            ([1.0, 1.0], [1.0, 1.0]),
            ([0.0, 1.0], [0.0, 1.0]),
            ([0.37, 0.62], [0.37, 0.62]),
        ] {
            let m = [
                rows[0][0], rows[0][1], rows[0][2], rows[1][0], rows[1][1], rows[1][2], rows[2][0],
                rows[2][1], rows[2][2],
            ];
            let got = apply_3x3(m, uv);
            assert!(
                (got[0] - expected[0]).abs() < 1e-5 && (got[1] - expected[1]).abs() < 1e-5,
                "{uv:?} mapped to {got:?}, expected {expected:?}"
            );
        }
    }

    /// The shader rows are the inverse warp: a fragment sitting where a corner
    /// was warped *to* must map back to that corner's unwarped UV.
    #[test]
    fn warped_corners_map_back_to_unwarped_uv() {
        let corners = WarpCorners([[0.1, 0.05], [0.9, 0.0], [1.0, 0.95], [0.0, 1.0]]);
        let rows = warp_matrix_rows(&corners);
        let m = [
            rows[0][0], rows[0][1], rows[0][2], rows[1][0], rows[1][1], rows[1][2], rows[2][0],
            rows[2][1], rows[2][2],
        ];
        let unit = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for (dst, src) in corners.0.iter().zip(unit.iter()) {
            let got = apply_3x3(m, *dst);
            assert!(
                (got[0] - src[0]).abs() < 1e-4 && (got[1] - src[1]).abs() < 1e-4,
                "warped corner {dst:?} mapped to {got:?}, expected {src:?}"
            );
        }
    }

    #[test]
    fn degenerate_corners_fall_back_to_identity() {
        let collapsed = WarpCorners([[0.5, 0.5]; 4]);
        let rows = warp_matrix_rows(&collapsed);
        let m = [
            rows[0][0], rows[0][1], rows[0][2], rows[1][0], rows[1][1], rows[1][2], rows[2][0],
            rows[2][1], rows[2][2],
        ];
        let got = apply_3x3(m, [0.25, 0.75]);
        assert!((got[0] - 0.25).abs() < 1e-5 && (got[1] - 0.75).abs() < 1e-5);
    }
}
