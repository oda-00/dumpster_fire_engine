//! Cameras — perspective and orthographic, with all glTF §3.10 matrix helpers.

#[derive(Debug, Clone)]
pub enum Camera {
    Perspective {
        name:         Option<String>,
        aspect_ratio: Option<f32>,
        y_fov:        f32,
        z_near:       f32,
        z_far:        Option<f32>,
    },
    Orthographic {
        name:    Option<String>,
        x_mag:   f32,
        y_mag:   f32,
        z_near:  f32,
        z_far:   f32,
    },
}

impl Camera {
    /// Reverse-Z perspective projection matrix (right-handed, column-major).
    /// Returns `None` for orthographic cameras.
    ///
    /// Implements spec §3.10.3.3 (finite) and §3.10.3.2 (infinite far, `z_far = None`).
    pub fn perspective_matrix(&self) -> Option<[f32; 16]> {
        let Camera::Perspective { aspect_ratio, y_fov, z_near, z_far, .. } = self else {
            return None;
        };
        let aspect = aspect_ratio.unwrap_or(1.0).max(1e-6);
        let f = 1.0 / (y_fov * 0.5).tan();
        let zn = *z_near;
        let (a, b) = match z_far {
            Some(zf) => (zf / (zn - zf), zn * zf / (zn - zf)),
            None     => (-1.0, -zn),
        };
        // Column-major:  [col0, col1, col2, col3]
        Some([
            f / aspect, 0.0, 0.0,  0.0,
            0.0,        f,   0.0,  0.0,
            0.0,        0.0, a,   -1.0,
            0.0,        0.0, b,    0.0,
        ])
    }

    /// Orthographic projection matrix per spec §3.10.3.4 (column-major).
    /// Returns `None` for perspective cameras.
    pub fn orthographic_matrix(&self) -> Option<[f32; 16]> {
        let Camera::Orthographic { x_mag, y_mag, z_near, z_far, .. } = self else {
            return None;
        };
        let xm = *x_mag;
        let ym = *y_mag;
        let zn = *z_near;
        let zf = *z_far;
        Some([
            1.0 / xm,  0.0,       0.0,              0.0,
            0.0,       1.0 / ym,  0.0,              0.0,
            0.0,       0.0,       2.0 / (zn - zf),  0.0,
            0.0,       0.0,      (zf + zn) / (zn - zf), 1.0,
        ])
    }

    /// Projection matrix — dispatches to the right formula based on variant.
    /// `viewport_aspect` overrides the camera's own aspect ratio for perspective
    /// cameras (spec §3.10.3 note: "If not supplied, the aspect ratio is from the viewport").
    pub fn projection_matrix(&self, viewport_aspect: Option<f32>) -> Option<[f32; 16]> {
        match self {
            Camera::Perspective { aspect_ratio, y_fov, z_near, z_far, .. } => {
                let aspect = viewport_aspect
                    .or(*aspect_ratio)
                    .unwrap_or(1.0)
                    .max(1e-6);
                let f = 1.0 / (y_fov * 0.5).tan();
                let zn = *z_near;
                let (a, b) = match z_far {
                    Some(zf) => (zf / (zn - zf), zn * zf / (zn - zf)),
                    None     => (-1.0, -zn),
                };
                Some([
                    f / aspect, 0.0, 0.0,  0.0,
                    0.0,        f,   0.0,  0.0,
                    0.0,        0.0, a,   -1.0,
                    0.0,        0.0, b,    0.0,
                ])
            }
            Camera::Orthographic { .. } => self.orthographic_matrix(),
        }
    }

    /// View matrix from a node's world transform.
    ///
    /// Per spec §3.10.2: the camera is at the node's position with the node's
    /// orientation. Scale is stripped per the spec note (camera nodes must not
    /// have non-uniform scale, but we normalize to be safe).
    ///
    /// Returns the inverse of the rotation-only part of the world matrix
    /// (which equals its transpose for a pure rotation).
    pub fn view_matrix(node_world: &[f32; 16]) -> [f32; 16] {
        // Extract columns (column-major storage):
        //   col 0 = [m0, m1, m2, m3]   (right)
        //   col 1 = [m4, m5, m6, m7]   (up)
        //   col 2 = [m8, m9, m10, m11] (back/forward)
        //   col 3 = [m12, m13, m14, m15] (translation)
        let m = node_world;

        // Lengths of the basis vectors (strip scale)
        let lx = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt().max(1e-12);
        let ly = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt().max(1e-12);
        let lz = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt().max(1e-12);

        let rx = m[0] / lx;  let ry = m[1] / lx;  let rz = m[2] / lx;
        let ux = m[4] / ly;  let uy = m[5] / ly;  let uz = m[6] / ly;
        let bx = m[8] / lz;  let by = m[9] / lz;  let bz = m[10] / lz;

        let tx = m[12];  let ty = m[13];  let tz = m[14];

        // View matrix = R^T * T^{-1}
        // Dot products with the negated translation:
        let dx = -(rx*tx + ry*ty + rz*tz);
        let dy = -(ux*tx + uy*ty + uz*tz);
        let dz = -(bx*tx + by*ty + bz*tz);

        [
            rx, ux, bx, 0.0,
            ry, uy, by, 0.0,
            rz, uz, bz, 0.0,
            dx, dy, dz, 1.0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per spec §3.10.3.4 the orthographic projection matrix for
    /// (x_mag = 2, y_mag = 1.5, z_near = 1, z_far = 100) lays out as:
    ///   [ 1/2     0       0           0
    ///     0       1/1.5   0           0
    ///     0       0       2/(zn-zf)   0
    ///     0       0       (zn+zf)/(zn-zf)  1 ]
    /// Column-major (each column listed top-to-bottom in the 16-array).
    #[test]
    fn orthographic_projection_matrix_matches_spec_3_10_3_4() {
        let cam = Camera::Orthographic {
            name: None,
            x_mag: 2.0, y_mag: 1.5,
            z_near: 1.0, z_far: 100.0,
        };
        let m = cam.orthographic_matrix().expect("ortho matrix");
        let expected = [
            1.0 / 2.0, 0.0,        0.0,                       0.0,
            0.0,       1.0 / 1.5,  0.0,                       0.0,
            0.0,       0.0,        2.0 / (1.0 - 100.0),       0.0,
            0.0,       0.0,        (100.0 + 1.0) / (1.0 - 100.0), 1.0,
        ];
        for i in 0..16 {
            assert!((m[i] - expected[i]).abs() < 1e-5,
                "lane {i}: got {} expected {}", m[i], expected[i]);
        }
    }

    /// Per spec §3.10.2 the view matrix is built from the camera node's
    /// world transform with scale STRIPPED. A non-uniform-scale input
    /// must still produce a rotation-only result.
    #[test]
    fn view_matrix_strips_scale_per_spec_3_10_2() {
        // World matrix = T(5, 0, 0) * S(2, 3, 4) — column-major.
        // (translation last column; non-uniform scale on the diagonal of
        // the rotation block.)
        let world = [
            2.0, 0.0, 0.0, 0.0,
            0.0, 3.0, 0.0, 0.0,
            0.0, 0.0, 4.0, 0.0,
            5.0, 0.0, 0.0, 1.0,
        ];
        let view = Camera::view_matrix(&world);
        // Basis vectors of the view's upper-3x3 (transpose of the
        // scale-stripped rotation) must be unit-length and orthogonal —
        // i.e. each column has |v| ≈ 1.
        for col in 0..3 {
            let x = view[col * 4 + 0];
            let y = view[col * 4 + 1];
            let z = view[col * 4 + 2];
            let len = (x*x + y*y + z*z).sqrt();
            assert!((len - 1.0).abs() < 1e-5,
                "view column {col} length {len} (should be 1.0 — scale not stripped)");
        }
        // Translation column applies the world's -t expressed in the
        // stripped basis. Translation is [-5, 0, 0, 1] (identity rotation
        // ⇒ view * world_translation = -world_translation).
        assert!((view[12] - (-5.0)).abs() < 1e-5);
        assert!((view[13] -   0.0).abs() < 1e-5);
        assert!((view[14] -   0.0).abs() < 1e-5);
        assert!((view[15] -   1.0).abs() < 1e-5);
    }
}
