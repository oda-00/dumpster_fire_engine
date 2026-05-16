//! Cameras — perspective + orthographic.

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
    /// Reverse-Z perspective matrix (right-handed). Returns `None` for ortho.
    pub fn perspective_matrix(&self) -> Option<[f32; 16]> {
        if let Camera::Perspective { aspect_ratio, y_fov, z_near, z_far, .. } = self {
            let aspect = aspect_ratio.unwrap_or(1.0).max(0.0001);
            let f = 1.0 / (y_fov * 0.5).tan();
            let zn = *z_near;
            // Infinite-far when z_far is None.
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
        } else { None }
    }
}
