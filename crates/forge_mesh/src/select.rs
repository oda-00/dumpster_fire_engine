//! Selection over the half-edge mesh, stored in the SoA flag layers.
//!
//! Blender's selection "gets really slow" past ~20M polys because it is
//! effectively single-threaded (blenderartists). Here the per-element predicates
//! are branchless and `rayon`-parallel over the flat flag arrays (the codegen
//! shape proven in `docs/editor_research/asm/exp4_select_upload.rs`).

use rayon::prelude::*;
use thin_vec::ThinVec;

use crate::half_edge::HalfEdgeMesh;

/// Flag bits in the `*flag` layers.
pub const SELECTED: u8 = 1 << 0;
pub const HIDDEN: u8 = 1 << 1;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SelectMode {
    Vertex,
    Edge,
    Face,
}

/// Branchless, `rayon`-parallel box select over a positions/flags pair. When
/// `additive`, selected elements are only added; otherwise the `SELECTED` bit is
/// set inside the box and cleared outside. Free function so the kernel can be
/// benchmarked independently of topology.
pub fn box_select_into(
    positions: &[[f32; 3]],
    flags: &mut [u8],
    lo: [f32; 3],
    hi: [f32; 3],
    additive: bool,
) {
    positions
        .par_iter()
        .zip(flags.par_iter_mut())
        .for_each(|(p, f)| {
            let inside = (p[0] >= lo[0])
                & (p[0] <= hi[0])
                & (p[1] >= lo[1])
                & (p[1] <= hi[1])
                & (p[2] >= lo[2])
                & (p[2] <= hi[2]);
            if inside {
                *f |= SELECTED;
            } else if !additive {
                *f &= !SELECTED;
            }
        });
}

/// Parallel count of elements with the `SELECTED` bit set.
pub fn count_selected(flags: &[u8]) -> usize {
    flags.par_iter().filter(|&&f| f & SELECTED != 0).count()
}

impl HalfEdgeMesh {
    #[inline]
    pub fn is_vertex_selected(&self, v: u32) -> bool {
        self.vflag[v as usize] & SELECTED != 0
    }

    #[inline]
    pub fn set_vertex_selected(&mut self, v: u32, on: bool) {
        if on {
            self.vflag[v as usize] |= SELECTED;
        } else {
            self.vflag[v as usize] &= !SELECTED;
        }
    }

    pub fn select_all_vertices(&mut self) {
        for f in self.vflag.iter_mut() {
            *f |= SELECTED;
        }
    }
    pub fn deselect_all_vertices(&mut self) {
        for f in self.vflag.iter_mut() {
            *f &= !SELECTED;
        }
    }
    pub fn invert_vertex_selection(&mut self) {
        for f in self.vflag.iter_mut() {
            *f ^= SELECTED;
        }
    }

    pub fn box_select_vertices(&mut self, lo: [f32; 3], hi: [f32; 3], additive: bool) {
        box_select_into(&self.pos, &mut self.vflag, lo, hi, additive);
    }

    pub fn count_selected_vertices(&self) -> usize {
        count_selected(&self.vflag)
    }

    pub fn selected_vertices(&self) -> ThinVec<u32> {
        (0..self.vertex_count() as u32)
            .filter(|&v| self.is_vertex_selected(v))
            .collect()
    }

    /// Add every vertex sharing an edge with a selected vertex. Scans half-edges
    /// (not the fan walk) so boundary/non-manifold neighborhoods are complete.
    pub fn grow_vertex_selection(&mut self) {
        let mut add: ThinVec<u32> = ThinVec::new();
        for h in 0..self.half_edge_count() as u32 {
            let a = self.he_vert[h as usize];
            let b = self.he_vert[self.he_next[h as usize] as usize];
            if self.is_vertex_selected(a) && !self.is_vertex_selected(b) {
                add.push(b);
            }
            if self.is_vertex_selected(b) && !self.is_vertex_selected(a) {
                add.push(a);
            }
        }
        for v in add {
            self.vflag[v as usize] |= SELECTED;
        }
    }

    /// Deselect any selected vertex that shares an edge with an unselected one.
    pub fn shrink_vertex_selection(&mut self) {
        let nv = self.vertex_count();
        let mut border = vec![false; nv];
        for h in 0..self.half_edge_count() as u32 {
            let a = self.he_vert[h as usize] as usize;
            let b = self.he_vert[self.he_next[h as usize] as usize] as usize;
            let sa = self.vflag[a] & SELECTED != 0;
            let sb = self.vflag[b] & SELECTED != 0;
            if sa && !sb {
                border[a] = true;
            }
            if sb && !sa {
                border[b] = true;
            }
        }
        for (v, &is_border) in border.iter().enumerate() {
            if is_border {
                self.vflag[v] &= !SELECTED;
            }
        }
    }

    /// Select the whole connected component containing `seed`. Union-find over
    /// all edges — robust for boundaries and non-manifold fans.
    pub fn select_linked(&mut self, seed: u32) {
        let nv = self.vertex_count();
        let mut parent: Vec<u32> = (0..nv as u32).collect();
        fn find(p: &mut [u32], mut x: u32) -> u32 {
            while p[x as usize] != x {
                p[x as usize] = p[p[x as usize] as usize]; // path halving
                x = p[x as usize];
            }
            x
        }
        for h in 0..self.half_edge_count() as u32 {
            let a = self.he_vert[h as usize];
            let b = self.he_vert[self.he_next[h as usize] as usize];
            let ra = find(&mut parent, a);
            let rb = find(&mut parent, b);
            if ra != rb {
                parent[ra as usize] = rb;
            }
        }
        let target = find(&mut parent, seed);
        for v in 0..nv as u32 {
            if find(&mut parent, v) == target {
                self.vflag[v as usize] |= SELECTED;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> HalfEdgeMesh {
        let pos = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = vec![0, 1, 2, 0, 2, 3];
        HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap()
    }

    #[test]
    fn box_select_picks_left_edge() {
        let mut m = quad();
        // Box covering x<=0.5 selects the two left verts (0 and 3).
        m.box_select_vertices([-0.1, -0.1, -0.1], [0.5, 1.1, 0.1], false);
        assert_eq!(m.count_selected_vertices(), 2);
        assert!(m.is_vertex_selected(0) && m.is_vertex_selected(3));
        assert!(!m.is_vertex_selected(1) && !m.is_vertex_selected(2));
    }

    #[test]
    fn all_invert_deselect() {
        let mut m = quad();
        m.select_all_vertices();
        assert_eq!(m.count_selected_vertices(), 4);
        m.invert_vertex_selection();
        assert_eq!(m.count_selected_vertices(), 0);
        m.select_all_vertices();
        m.deselect_all_vertices();
        assert_eq!(m.count_selected_vertices(), 0);
    }

    #[test]
    fn grow_expands_one_ring() {
        let mut m = quad();
        m.set_vertex_selected(0, true);
        m.grow_vertex_selection();
        // Vertex 0 connects to 1, 2, 3 in the triangulated quad → all selected.
        assert_eq!(m.count_selected_vertices(), 4);
    }

    #[test]
    fn select_linked_isolates_component() {
        // Two disconnected triangles in one mesh (verts 0-2 and 3-5).
        let pos = vec![
            [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0],
            [5.0, 0.0, 0.0], [6.0, 0.0, 0.0], [5.0, 1.0, 0.0],
        ];
        let idx = vec![0, 1, 2, 3, 4, 5];
        let mut m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        m.select_linked(0);
        assert_eq!(m.count_selected_vertices(), 3);
        assert!(m.is_vertex_selected(0) && m.is_vertex_selected(1) && m.is_vertex_selected(2));
        assert!(!m.is_vertex_selected(3));
    }
}
