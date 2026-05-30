//! Normal recomputation — runs constantly during modeling. Per-face cross
//! products and area-weighted per-vertex accumulation over the SoA arrays (the
//! `docs/editor_research/asm/exp3_normals.rs` kernel), rayon-parallel on faces.
//! The same arithmetic is the GPU `RecomputeNormals` compute Ore in the
//! integration pass.

use rayon::prelude::*;

use crate::half_edge::HalfEdgeMesh;

#[inline]
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[inline]
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-12 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

impl HalfEdgeMesh {
    /// Un-normalized face normal (cross of two edges); magnitude = 2·area, so it
    /// doubles as the area weight for vertex-normal accumulation.
    #[inline]
    pub fn face_normal_raw(&self, f: u32) -> [f32; 3] {
        let [a, b, c] = self.face_verts(f);
        let (pa, pb, pc) = (
            self.pos[a as usize],
            self.pos[b as usize],
            self.pos[c as usize],
        );
        cross(sub(pb, pa), sub(pc, pa))
    }

    /// Normalized per-face normals (rayon-parallel over faces).
    pub fn face_normals(&self) -> Vec<[f32; 3]> {
        (0..self.face_count() as u32)
            .into_par_iter()
            .map(|f| normalize(self.face_normal_raw(f)))
            .collect()
    }

    /// Area-weighted per-vertex normals: accumulate each face's raw (area-scaled)
    /// normal into its corner vertices, then normalize.
    pub fn vertex_normals(&self) -> Vec<[f32; 3]> {
        let mut acc = vec![[0.0f32; 3]; self.vertex_count()];
        for f in 0..self.face_count() as u32 {
            let n = self.face_normal_raw(f);
            let [a, b, c] = self.face_verts(f);
            for v in [a, b, c] {
                let s = &mut acc[v as usize];
                s[0] += n[0];
                s[1] += n[1];
                s[2] += n[2];
            }
        }
        acc.par_iter().map(|&v| normalize(v)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_normals_point_up() {
        // CCW quad in the z=0 plane → +Z normals.
        let pos = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = vec![0, 1, 2, 0, 2, 3];
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        for n in m.face_normals() {
            assert!((n[2] - 1.0).abs() < 1e-5, "face normal {n:?}");
        }
        for n in m.vertex_normals() {
            assert!((n[2] - 1.0).abs() < 1e-5, "vertex normal {n:?}");
        }
    }

    #[test]
    fn cube_vertex_normals_point_outward() {
        let pos = vec![
            [-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0],
        ];
        let quads: [[u32; 4]; 6] = [
            [4, 5, 6, 7], [1, 0, 3, 2], [0, 4, 7, 3], [5, 1, 2, 6], [0, 1, 5, 4], [3, 7, 6, 2],
        ];
        let mut idx = Vec::new();
        for q in quads {
            idx.extend_from_slice(&[q[0], q[1], q[2], q[0], q[2], q[3]]);
        }
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        let vn = m.vertex_normals();
        // Each corner normal should point away from the cube center (dot with the
        // outward position direction > 0).
        for v in 0..8 {
            let p = pos[v];
            let n = vn[v];
            let d = p[0] * n[0] + p[1] * n[1] + p[2] * n[2];
            assert!(d > 0.0, "vertex {v} normal {n:?} not outward (dot {d})");
        }
    }
}
