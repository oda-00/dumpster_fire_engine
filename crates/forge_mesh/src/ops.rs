//! Modeling operators. Phase 3 lands the foundational, fully-verifiable ops here
//! (subdivide, delete-faces); bevel/loop-cut/knife/bridge/extrude follow as
//! further commits in this module. Operators are correctness-first (they produce
//! new index buffers and rebuild + `validate()`); in-place O(touched) half-edge
//! surgery is the later optimization the SoA layout enables.

use std::collections::HashMap;

use thin_vec::ThinVec;

use crate::half_edge::{HalfEdgeMesh, MeshError};

/// 1→4 midpoint subdivision of a triangle index buffer. Each triangle becomes
/// four; every undirected edge gets one shared midpoint vertex (deduped).
pub fn subdivide_indexed(pos: &[[f32; 3]], idx: &[u32]) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut out_pos: Vec<[f32; 3]> = pos.to_vec();
    let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
    let mut midpoint = |a: u32, b: u32, out: &mut Vec<[f32; 3]>| -> u32 {
        let key = if a < b { (a, b) } else { (b, a) };
        *mid.entry(key).or_insert_with(|| {
            let pa = pos[a as usize];
            let pb = pos[b as usize];
            out.push([
                0.5 * (pa[0] + pb[0]),
                0.5 * (pa[1] + pb[1]),
                0.5 * (pa[2] + pb[2]),
            ]);
            (out.len() - 1) as u32
        })
    };

    let mut out_idx: Vec<u32> = Vec::with_capacity(idx.len() * 4);
    for t in idx.chunks_exact(3) {
        let (a, b, c) = (t[0], t[1], t[2]);
        let mab = midpoint(a, b, &mut out_pos);
        let mbc = midpoint(b, c, &mut out_pos);
        let mca = midpoint(c, a, &mut out_pos);
        out_idx.extend_from_slice(&[
            a, mab, mca, // corner a
            mab, b, mbc, // corner b
            mca, mbc, c, // corner c
            mab, mbc, mca, // center
        ]);
    }
    (out_pos, out_idx)
}

/// Weld vertices that fall in the same `eps`-grid cell into one, remap indices,
/// and drop triangles that become degenerate. Returns `(positions, indices)`.
pub fn weld_indexed(pos: &[[f32; 3]], idx: &[u32], eps: f32) -> (Vec<[f32; 3]>, Vec<u32>) {
    let inv = if eps > 0.0 { 1.0 / eps } else { f32::INFINITY };
    let cell = |p: [f32; 3]| -> (i64, i64, i64) {
        (
            (p[0] * inv).floor() as i64,
            (p[1] * inv).floor() as i64,
            (p[2] * inv).floor() as i64,
        )
    };
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut new_pos: Vec<[f32; 3]> = Vec::new();
    let mut remap = vec![0u32; pos.len()];
    for (i, &p) in pos.iter().enumerate() {
        let rep = *map.entry(cell(p)).or_insert_with(|| {
            new_pos.push(p);
            (new_pos.len() - 1) as u32
        });
        remap[i] = rep;
    }
    let mut new_idx: Vec<u32> = Vec::with_capacity(idx.len());
    for t in idx.chunks_exact(3) {
        let (a, b, c) = (
            remap[t[0] as usize],
            remap[t[1] as usize],
            remap[t[2] as usize],
        );
        if a != b && b != c && a != c {
            new_idx.extend_from_slice(&[a, b, c]); // drop degenerate (collapsed) tris
        }
    }
    (new_pos, new_idx)
}

/// Build a new index buffer dropping the listed faces (positions retained).
pub fn delete_faces_indexed(idx: &[u32], faces: &[u32]) -> Vec<u32> {
    let drop: std::collections::HashSet<u32> = faces.iter().copied().collect();
    let mut out = Vec::with_capacity(idx.len());
    for (f, t) in idx.chunks_exact(3).enumerate() {
        if !drop.contains(&(f as u32)) {
            out.extend_from_slice(t);
        }
    }
    out
}

impl HalfEdgeMesh {
    /// Return a new mesh with every face midpoint-subdivided (1→4).
    pub fn subdivided(&self) -> Result<HalfEdgeMesh, MeshError> {
        let (p, i) = self.to_indexed();
        let (np, ni) = subdivide_indexed(&p, &i);
        HalfEdgeMesh::build_from_indexed(&np, &ni)
    }

    /// Return a new mesh with coincident vertices welded within `eps`.
    pub fn welded(&self, eps: f32) -> Result<HalfEdgeMesh, MeshError> {
        let (p, i) = self.to_indexed();
        let (np, ni) = weld_indexed(&p, &i, eps);
        HalfEdgeMesh::build_from_indexed(&np, &ni)
    }

    /// Return a new mesh with `faces` removed.
    pub fn with_faces_deleted(&self, faces: &[u32]) -> Result<HalfEdgeMesh, MeshError> {
        let (p, i) = self.to_indexed();
        let ni = delete_faces_indexed(&i, faces);
        let pv: Vec<[f32; 3]> = p.into_iter().collect();
        let iv: Vec<u32> = ni;
        HalfEdgeMesh::build_from_indexed(&pv, &iv)
    }

    /// Face ids all three of whose vertices are selected (for vertex→face ops).
    pub fn faces_with_all_selected(&self) -> ThinVec<u32> {
        (0..self.face_count() as u32)
            .filter(|&f| {
                let [a, b, c] = self.face_verts(f);
                self.is_vertex_selected(a) && self.is_vertex_selected(b) && self.is_vertex_selected(c)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> HalfEdgeMesh {
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
        HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap()
    }

    #[test]
    fn subdivide_one_triangle() {
        let pos = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = vec![0, 1, 2];
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        let s = m.subdivided().unwrap();
        assert_eq!(s.face_count(), 4); // 1 → 4
        assert_eq!(s.vertex_count(), 6); // 3 corners + 3 edge midpoints
        s.validate().unwrap();
    }

    #[test]
    fn subdivide_cube_preserves_euler() {
        let m = cube(); // V8 E18 F12
        let s = m.subdivided().unwrap();
        s.validate().unwrap();
        assert_eq!(s.vertex_count(), 8 + 18); // V + E new midpoints
        assert_eq!(s.face_count(), 12 * 4);
        assert_eq!(s.edge_count(), 2 * 18 + 3 * 12); // 2E + 3F
        assert_eq!(
            s.vertex_count() as i64 - s.edge_count() as i64 + s.face_count() as i64,
            2
        );
    }

    #[test]
    fn delete_one_face_of_quad() {
        let pos = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = vec![0, 1, 2, 0, 2, 3];
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        let d = m.with_faces_deleted(&[1]).unwrap();
        assert_eq!(d.face_count(), 1);
        d.validate().unwrap();
    }

    #[test]
    fn weld_merges_coincident_vertices() {
        // Two triangles sharing edge A–C, but with the shared corners duplicated
        // (6 verts, indices 0..5). Welding collapses to 4 unique verts, 2 tris.
        let a = [0.0, 0.0, 0.0];
        let b = [1.0, 0.0, 0.0];
        let c = [0.0, 1.0, 0.0];
        let d = [-1.0, 0.0, 0.0];
        let pos = vec![a, b, c, a, c, d]; // verts 3==0, 4==2 (coincident)
        let idx = vec![0, 1, 2, 3, 4, 5];
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        assert_eq!(m.vertex_count(), 6);
        let w = m.welded(1e-4).unwrap();
        assert_eq!(w.vertex_count(), 4);
        assert_eq!(w.face_count(), 2);
        w.validate().unwrap();
    }

    #[test]
    fn faces_with_all_selected_picks_full_triangles() {
        let pos = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = vec![0, 1, 2, 0, 2, 3];
        let mut m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        m.set_vertex_selected(0, true);
        m.set_vertex_selected(1, true);
        m.set_vertex_selected(2, true);
        // Only the first triangle (0,1,2) is fully selected.
        assert_eq!(m.faces_with_all_selected().as_slice(), &[0]);
    }
}
