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

    /// Region-extrude `faces` along their average normal by `offset`.
    /// Returns the rebuilt mesh and the new cap vertex ids.
    pub fn with_faces_extruded(
        &self,
        faces: &[u32],
        offset: f32,
    ) -> Result<(HalfEdgeMesh, Vec<u32>), MeshError> {
        let (p, i) = self.to_indexed();
        let (np, ni, sel) = extrude_faces_indexed(&p, &i, faces, offset);
        Ok((HalfEdgeMesh::build_from_indexed(&np, &ni)?, sel))
    }

    /// Individually inset `faces` by absolute distance `amount`.
    pub fn with_faces_inset(
        &self,
        faces: &[u32],
        amount: f32,
    ) -> Result<(HalfEdgeMesh, Vec<u32>), MeshError> {
        let (p, i) = self.to_indexed();
        let (np, ni, sel) = inset_faces_indexed(&p, &i, faces, amount);
        Ok((HalfEdgeMesh::build_from_indexed(&np, &ni)?, sel))
    }

    /// Collapse the currently selected vertices to their centroid.
    pub fn with_selected_merged(&self) -> Result<(HalfEdgeMesh, Vec<u32>), MeshError> {
        let (p, i) = self.to_indexed();
        let sel: Vec<u32> = self.selected_vertices().into_iter().collect();
        let (np, ni, nsel) = merge_at_center_indexed(&p, &i, &sel);
        Ok((HalfEdgeMesh::build_from_indexed(&np, &ni)?, nsel))
    }

    /// Reverse the winding of `faces`.
    pub fn with_faces_flipped(&self, faces: &[u32]) -> Result<HalfEdgeMesh, MeshError> {
        let (p, i) = self.to_indexed();
        let ni = flip_faces_indexed(&i, faces);
        let pv: Vec<[f32; 3]> = p.into_iter().collect();
        HalfEdgeMesh::build_from_indexed(&pv, &ni)
    }

    /// Crack-free selective subdivision of `faces` (red-green conforming).
    pub fn with_faces_subdivided(
        &self,
        faces: &[u32],
    ) -> Result<(HalfEdgeMesh, Vec<u32>), MeshError> {
        let (p, i) = self.to_indexed();
        let (np, ni, sel) = subdivide_faces_indexed(&p, &i, faces);
        Ok((HalfEdgeMesh::build_from_indexed(&np, &ni)?, sel))
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

// ── Blender-core region operators ───────────────────────────────────────────
// All follow the same rebuild contract as `subdivided()`: operate on the
// indexed form, return (positions, indices, new_selection) where
// new_selection is the vertex ids (in the NEW mesh) that should be selected
// afterwards — matching Blender's behavior of selecting the result geometry.

/// Region-extrude the faces in `faces`. The region's faces become the moved
/// "cap" (offset along the area-weighted region normal by `offset`); side
/// quads stitch the old boundary ring to the new one. Interior shared edges
/// get no side walls — this is Blender's *Extrude Region*, not
/// per-face extrude.
pub fn extrude_faces_indexed(
    pos: &[[f32; 3]],
    idx: &[u32],
    faces: &[u32],
    offset: f32,
) -> (Vec<[f32; 3]>, Vec<u32>, Vec<u32>) {
    use std::collections::HashMap;
    let in_region = {
        let mut f = vec![false; idx.len() / 3];
        for &fi in faces {
            if (fi as usize) < f.len() {
                f[fi as usize] = true;
            }
        }
        f
    };

    // Area-weighted region normal.
    let mut n = [0.0f32; 3];
    for &fi in faces {
        let [a, b, c] = [
            pos[idx[fi as usize * 3] as usize],
            pos[idx[fi as usize * 3 + 1] as usize],
            pos[idx[fi as usize * 3 + 2] as usize],
        ];
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        n[0] += u[1] * v[2] - u[2] * v[1];
        n[1] += u[2] * v[0] - u[0] * v[2];
        n[2] += u[0] * v[1] - u[1] * v[0];
    }
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
    let d = [n[0] / len * offset, n[1] / len * offset, n[2] / len * offset];

    // Boundary directed edges: edges of region faces whose reversed twin is
    // NOT used by another region face.
    let mut region_dir_edges: HashMap<(u32, u32), ()> = HashMap::new();
    for &fi in faces {
        let f = fi as usize * 3;
        for k in 0..3 {
            let a = idx[f + k];
            let b = idx[f + (k + 1) % 3];
            region_dir_edges.insert((a, b), ());
        }
    }

    let mut new_pos: Vec<[f32; 3]> = pos.to_vec();
    let mut dup: HashMap<u32, u32> = HashMap::new();
    let mut prime = |v: u32, new_pos: &mut Vec<[f32; 3]>, dup: &mut HashMap<u32, u32>| -> u32 {
        *dup.entry(v).or_insert_with(|| {
            let p = pos[v as usize];
            new_pos.push([p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
            (new_pos.len() - 1) as u32
        })
    };

    let mut new_idx: Vec<u32> = Vec::with_capacity(idx.len() + faces.len() * 18);
    // Non-region faces unchanged; region faces rewritten to primed verts.
    for (fi, tri) in idx.chunks_exact(3).enumerate() {
        if in_region[fi] {
            let a = prime(tri[0], &mut new_pos, &mut dup);
            let b = prime(tri[1], &mut new_pos, &mut dup);
            let c = prime(tri[2], &mut new_pos, &mut dup);
            new_idx.extend_from_slice(&[a, b, c]);
        } else {
            new_idx.extend_from_slice(tri);
        }
    }
    // Side quads on boundary edges (directed edge (a,b) whose twin (b,a) is
    // not in the region).
    for (&(a, b), _) in region_dir_edges.iter() {
        if region_dir_edges.contains_key(&(b, a)) {
            continue; // interior edge
        }
        let (ap, bp) = (dup[&a], dup[&b]);
        // The side wall replaces the cap's old boundary edge, so it must keep
        // the cap's direction (a, b) to stay the manifold twin of the
        // surrounding face's (b, a): quad (a, b, b', a') as two triangles.
        new_idx.extend_from_slice(&[a, b, bp]);
        new_idx.extend_from_slice(&[a, bp, ap]);
    }
    let sel: Vec<u32> = dup.values().copied().collect();
    (new_pos, new_idx, sel)
}

/// Individually inset each face in `faces` by pulling its corners toward the
/// face centroid by absolute distance `amount` (clamped to the centroid).
pub fn inset_faces_indexed(
    pos: &[[f32; 3]],
    idx: &[u32],
    faces: &[u32],
    amount: f32,
) -> (Vec<[f32; 3]>, Vec<u32>, Vec<u32>) {
    let mut in_region = vec![false; idx.len() / 3];
    for &fi in faces {
        if (fi as usize) < in_region.len() {
            in_region[fi as usize] = true;
        }
    }
    let mut new_pos = pos.to_vec();
    let mut new_idx: Vec<u32> = Vec::with_capacity(idx.len() + faces.len() * 9);
    let mut sel: Vec<u32> = Vec::new();
    for (fi, tri) in idx.chunks_exact(3).enumerate() {
        if !in_region[fi] {
            new_idx.extend_from_slice(tri);
            continue;
        }
        let [pa, pb, pc] = [pos[tri[0] as usize], pos[tri[1] as usize], pos[tri[2] as usize]];
        let cen = [
            (pa[0] + pb[0] + pc[0]) / 3.0,
            (pa[1] + pb[1] + pc[1]) / 3.0,
            (pa[2] + pb[2] + pc[2]) / 3.0,
        ];
        let mut inner = [0u32; 3];
        for (k, &v) in tri.iter().enumerate() {
            let p = pos[v as usize];
            let to_c = [cen[0] - p[0], cen[1] - p[1], cen[2] - p[2]];
            let dist = (to_c[0] * to_c[0] + to_c[1] * to_c[1] + to_c[2] * to_c[2])
                .sqrt()
                .max(1e-12);
            let t = (amount / dist).min(0.95);
            new_pos.push([p[0] + to_c[0] * t, p[1] + to_c[1] * t, p[2] + to_c[2] * t]);
            inner[k] = (new_pos.len() - 1) as u32;
            sel.push(inner[k]);
        }
        // Inner face + three side quads.
        new_idx.extend_from_slice(&inner);
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            let (ai, bi) = (inner[k], inner[(k + 1) % 3]);
            new_idx.extend_from_slice(&[a, b, bi]);
            new_idx.extend_from_slice(&[a, bi, ai]);
        }
    }
    (new_pos, new_idx, sel)
}

/// Collapse all vertices in `verts` to their centroid; degenerate faces
/// (with a repeated vertex) are dropped. Returns the surviving-mesh indexed
/// data plus the new id of the merged vertex.
pub fn merge_at_center_indexed(
    pos: &[[f32; 3]],
    idx: &[u32],
    verts: &[u32],
) -> (Vec<[f32; 3]>, Vec<u32>, Vec<u32>) {
    if verts.is_empty() {
        return (pos.to_vec(), idx.to_vec(), Vec::new());
    }
    let mut selected = vec![false; pos.len()];
    let mut cen = [0.0f32; 3];
    for &v in verts {
        selected[v as usize] = true;
        let p = pos[v as usize];
        cen[0] += p[0];
        cen[1] += p[1];
        cen[2] += p[2];
    }
    let n = verts.len() as f32;
    cen = [cen[0] / n, cen[1] / n, cen[2] / n];

    // Remap: selected → one shared id (first slot), others compacted.
    let mut remap = vec![u32::MAX; pos.len()];
    let mut new_pos: Vec<[f32; 3]> = Vec::with_capacity(pos.len());
    new_pos.push(cen);
    let merged_id = 0u32;
    for (v, p) in pos.iter().enumerate() {
        if selected[v] {
            remap[v] = merged_id;
        } else {
            new_pos.push(*p);
            remap[v] = (new_pos.len() - 1) as u32;
        }
    }
    let mut new_idx: Vec<u32> = Vec::with_capacity(idx.len());
    for tri in idx.chunks_exact(3) {
        let (a, b, c) = (remap[tri[0] as usize], remap[tri[1] as usize], remap[tri[2] as usize]);
        if a != b && b != c && a != c {
            new_idx.extend_from_slice(&[a, b, c]);
        }
    }
    (new_pos, new_idx, vec![merged_id])
}

/// Reverse the winding (flip the normal) of every face in `faces`.
pub fn flip_faces_indexed(idx: &[u32], faces: &[u32]) -> Vec<u32> {
    let mut out = idx.to_vec();
    for &fi in faces {
        let f = fi as usize * 3;
        if f + 2 < out.len() {
            out.swap(f + 1, f + 2);
        }
    }
    out
}

/// Red-green selective subdivision: faces in `faces` are midpoint-split 1→4;
/// neighbors sharing a split edge are split 1→2 (one split edge) or 1→3 /
/// 1→4 (two / three split edges) so the mesh stays crack-free — no
/// T-junctions, the property Blender's selective subdivide preserves.
pub fn subdivide_faces_indexed(
    pos: &[[f32; 3]],
    idx: &[u32],
    faces: &[u32],
) -> (Vec<[f32; 3]>, Vec<u32>, Vec<u32>) {
    use std::collections::HashMap;
    let mut in_region = vec![false; idx.len() / 3];
    for &fi in faces {
        if (fi as usize) < in_region.len() {
            in_region[fi as usize] = true;
        }
    }
    let mut new_pos = pos.to_vec();
    let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
    let mut sel: Vec<u32> = Vec::new();
    // Pre-split every edge of every region face.
    {
        let mut midpoint = |a: u32, b: u32, new_pos: &mut Vec<[f32; 3]>| -> u32 {
            let key = (a.min(b), a.max(b));
            *mid.entry(key).or_insert_with(|| {
                let (pa, pb) = (pos[a as usize], pos[b as usize]);
                new_pos.push([
                    (pa[0] + pb[0]) * 0.5,
                    (pa[1] + pb[1]) * 0.5,
                    (pa[2] + pb[2]) * 0.5,
                ]);
                (new_pos.len() - 1) as u32
            })
        };
        for (fi, tri) in idx.chunks_exact(3).enumerate() {
            if in_region[fi] {
                for k in 0..3 {
                    let m = midpoint(tri[k], tri[(k + 1) % 3], &mut new_pos);
                    sel.push(m);
                }
            }
        }
    }
    let get_mid = |a: u32, b: u32| -> Option<u32> {
        mid.get(&(a.min(b), a.max(b))).copied()
    };

    let mut new_idx: Vec<u32> = Vec::with_capacity(idx.len() * 2);
    for (fi, tri) in idx.chunks_exact(3).enumerate() {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        let (mab, mbc, mca) = (get_mid(a, b), get_mid(b, c), get_mid(c, a));
        if in_region[fi] {
            // 1 → 4 (all three midpoints exist by construction).
            let (ab, bc, ca) = (mab.unwrap(), mbc.unwrap(), mca.unwrap());
            new_idx.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
            continue;
        }
        match (mab, mbc, mca) {
            (None, None, None) => new_idx.extend_from_slice(tri),
            // One split edge → 1→2.
            (Some(m), None, None) => new_idx.extend_from_slice(&[a, m, c, m, b, c]),
            (None, Some(m), None) => new_idx.extend_from_slice(&[b, m, a, m, c, a]),
            (None, None, Some(m)) => new_idx.extend_from_slice(&[c, m, b, m, a, b]),
            // Two split edges → 1→3 (fan from the shared vertex side).
            (Some(m1), Some(m2), None) => {
                new_idx.extend_from_slice(&[b, m2, m1, a, m1, m2, a, m2, c]);
            }
            (None, Some(m1), Some(m2)) => {
                new_idx.extend_from_slice(&[c, m2, m1, b, m1, m2, b, m2, a]);
            }
            (Some(m2), None, Some(m1)) => {
                new_idx.extend_from_slice(&[a, m2, m1, c, m1, m2, c, m2, b]);
            }
            // All three split (face fully surrounded) → 1→4.
            (Some(ab), Some(bc), Some(ca)) => {
                new_idx.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
            }
        }
    }
    (new_pos, new_idx, sel)
}

#[cfg(test)]
mod region_op_tests {
    use super::*;
    use crate::half_edge::HalfEdgeMesh;

    fn cube() -> (Vec<[f32; 3]>, Vec<u32>) {
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
        (pos, idx)
    }

    #[test]
    fn extrude_region_two_tris_one_quad_face() {
        // Extrude the +Z cube face (faces 0 and 1 share an interior edge):
        // 4 new verts, cap stays 2 tris, 4 boundary edges → 8 side tris.
        let (pos, idx) = cube();
        let (np, ni, sel) = extrude_faces_indexed(&pos, &idx, &[0, 1], 1.0);
        assert_eq!(np.len(), 8 + 4, "one shared quad ring duplicates 4 verts");
        assert_eq!(ni.len() / 3, 12 + 8, "12 original + 8 side tris");
        assert_eq!(sel.len(), 4);
        // Result must remain a valid manifold for the half-edge builder.
        let m = HalfEdgeMesh::build_from_indexed(&np, &ni).expect("manifold");
        assert_eq!(m.vertex_count(), 12);
        // Extruded cap moved +Z by ~1.
        for &v in &sel {
            assert!((np[v as usize][2] - 2.0).abs() < 1e-5);
        }
    }

    #[test]
    fn inset_face_keeps_manifold_and_selects_inner() {
        let (pos, idx) = cube();
        let (np, ni, sel) = inset_faces_indexed(&pos, &idx, &[0], 0.3);
        assert_eq!(np.len(), 8 + 3);
        assert_eq!(ni.len() / 3, 12 - 1 + 1 + 6, "face replaced + 6 side tris");
        assert_eq!(sel.len(), 3);
        HalfEdgeMesh::build_from_indexed(&np, &ni).expect("manifold");
    }

    #[test]
    fn merge_collapses_face_ring() {
        let (pos, idx) = cube();
        // Merge the 4 verts of the +Z face → cube becomes a pyramid-ish hull.
        let (np, ni, sel) = merge_at_center_indexed(&pos, &idx, &[4, 5, 6, 7]);
        assert_eq!(np.len(), 5);
        assert_eq!(sel, vec![0]);
        assert!((np[0][2] - 1.0).abs() < 1e-5, "centroid of +Z ring");
        // The two +Z cap faces degenerate away; side quads each lose one tri.
        assert_eq!(ni.len() / 3, 12 - 2 - 4);
        HalfEdgeMesh::build_from_indexed(&np, &ni).expect("manifold");
    }

    #[test]
    fn flip_reverses_winding() {
        let (pos, idx) = cube();
        let ni = flip_faces_indexed(&idx, &[0]);
        assert_eq!(ni[0], idx[0]);
        assert_eq!(ni[1], idx[2]);
        assert_eq!(ni[2], idx[1]);
        let _ = pos;
    }

    #[test]
    fn selective_subdivide_is_crack_free() {
        let (pos, idx) = cube();
        // Subdivide one face; every neighbor sharing a split edge must conform.
        let (np, ni, _sel) = subdivide_faces_indexed(&pos, &idx, &[0]);
        let m = HalfEdgeMesh::build_from_indexed(&np, &ni)
            .expect("crack-free manifold after selective subdivision");
        m.validate().expect("half-edge invariants hold");
        // Face 0 → 4 tris; its 3 split edges touch 3 neighbors → each 1→2.
        assert_eq!(ni.len() / 3, 12 - 1 + 4 - 3 + 6);
    }
}
