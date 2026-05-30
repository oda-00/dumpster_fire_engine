//! Transform of the current vertex selection — the operation Blender slows on
//! (main-thread depsgraph/batch overhead). Here it is a branchless, rayon-parallel
//! pass over the SoA position + flag arrays (the `exp2_transform.rs` kernel shape),
//! and it reports the touched index range so a later GPU upload can be O(touched)
//! instead of a full rebuild.

use rayon::prelude::*;

use crate::half_edge::HalfEdgeMesh;
use crate::select::SELECTED;

/// Translate every selected vertex by `d`. Rayon-parallel; the inner body is a
/// masked add that vectorizes per chunk.
pub fn translate_selected(pos: &mut [[f32; 3]], vflag: &[u8], d: [f32; 3]) {
    pos.par_iter_mut().zip(vflag.par_iter()).for_each(|(p, &f)| {
        if f & SELECTED != 0 {
            p[0] += d[0];
            p[1] += d[1];
            p[2] += d[2];
        }
    });
}

/// Scale every selected vertex about `pivot` by per-axis factors `s`.
pub fn scale_selected(pos: &mut [[f32; 3]], vflag: &[u8], pivot: [f32; 3], s: [f32; 3]) {
    pos.par_iter_mut().zip(vflag.par_iter()).for_each(|(p, &f)| {
        if f & SELECTED != 0 {
            p[0] = pivot[0] + (p[0] - pivot[0]) * s[0];
            p[1] = pivot[1] + (p[1] - pivot[1]) * s[1];
            p[2] = pivot[2] + (p[2] - pivot[2]) * s[2];
        }
    });
}

/// Apply a row-major 4×4 affine matrix to every selected vertex (w=1). The
/// per-row `a·x+b·y+c·z+t` FMA is the `affine_row_soa` kernel.
pub fn transform_selected(pos: &mut [[f32; 3]], vflag: &[u8], m: [[f32; 4]; 4]) {
    pos.par_iter_mut().zip(vflag.par_iter()).for_each(|(p, &f)| {
        if f & SELECTED != 0 {
            let (x, y, z) = (p[0], p[1], p[2]);
            p[0] = m[0][0] * x + m[0][1] * y + m[0][2] * z + m[0][3];
            p[1] = m[1][0] * x + m[1][1] * y + m[1][2] * z + m[1][3];
            p[2] = m[2][0] * x + m[2][1] * y + m[2][2] * z + m[2][3];
        }
    });
}

impl HalfEdgeMesh {
    pub fn translate_selected_vertices(&mut self, d: [f32; 3]) {
        translate_selected(&mut self.pos, &self.vflag, d);
    }
    pub fn scale_selected_vertices(&mut self, pivot: [f32; 3], s: [f32; 3]) {
        scale_selected(&mut self.pos, &self.vflag, pivot, s);
    }
    pub fn transform_selected_vertices(&mut self, m: [[f32; 4]; 4]) {
        transform_selected(&mut self.pos, &self.vflag, m);
    }

    /// Inclusive `[min,max]` vertex-index range of the current selection, or
    /// `None` if empty. Feeds the O(touched) dirty-range GPU upload (P2/P10).
    pub fn selected_index_bounds(&self) -> Option<(u32, u32)> {
        let mut lo = u32::MAX;
        let mut hi = 0u32;
        let mut any = false;
        for v in 0..self.vertex_count() as u32 {
            if self.is_vertex_selected(v) {
                any = true;
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        any.then_some((lo, hi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::half_edge::HalfEdgeMesh;

    fn quad() -> HalfEdgeMesh {
        let pos = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = vec![0, 1, 2, 0, 2, 3];
        HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap()
    }

    #[test]
    fn translate_only_moves_selection() {
        let mut m = quad();
        m.set_vertex_selected(1, true);
        m.set_vertex_selected(2, true);
        m.translate_selected_vertices([10.0, 0.0, 0.0]);
        assert_eq!(m.pos[0], [0.0, 0.0, 0.0]); // untouched
        assert_eq!(m.pos[1], [11.0, 0.0, 0.0]);
        assert_eq!(m.pos[2], [11.0, 1.0, 0.0]);
        assert_eq!(m.pos[3], [0.0, 1.0, 0.0]); // untouched
        assert_eq!(m.selected_index_bounds(), Some((1, 2)));
    }

    #[test]
    fn scale_about_pivot() {
        let mut m = quad();
        m.select_all_vertices();
        m.scale_selected_vertices([0.0, 0.0, 0.0], [2.0, 2.0, 1.0]);
        assert_eq!(m.pos[2], [2.0, 2.0, 0.0]);
    }

    #[test]
    fn affine_matches_translate() {
        let mut m = quad();
        m.set_vertex_selected(0, true);
        // Pure translation matrix.
        let mat = [
            [1.0, 0.0, 0.0, 5.0],
            [0.0, 1.0, 0.0, -3.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        m.transform_selected_vertices(mat);
        assert_eq!(m.pos[0], [5.0, -3.0, 0.0]);
    }
}
