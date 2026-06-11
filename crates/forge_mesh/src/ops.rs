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
    fn recalc_fixes_one_flipped_face() {
        let (pos, mut idx) = cube();
        // Flip face 0 so the raw indexed data is inconsistent (and would be
        // non-manifold if built directly).
        idx.swap(1, 2);
        let fixed = make_normals_consistent_indexed(&pos, &idx);
        let m = HalfEdgeMesh::build_from_indexed(&pos, &fixed)
            .expect("consistent winding builds a manifold");
        m.validate().expect("invariants hold");
        // Net signed volume must be positive (outward) for a closed cube.
        let mut vol = 0.0f64;
        for tri in fixed.chunks_exact(3) {
            let a = pos[tri[0] as usize];
            let b = pos[tri[1] as usize];
            let c = pos[tri[2] as usize];
            let cr = [
                b[1] as f64 * c[2] as f64 - b[2] as f64 * c[1] as f64,
                b[2] as f64 * c[0] as f64 - b[0] as f64 * c[2] as f64,
                b[0] as f64 * c[1] as f64 - b[1] as f64 * c[0] as f64,
            ];
            vol += a[0] as f64 * cr[0] + a[1] as f64 * cr[1] + a[2] as f64 * cr[2];
        }
        assert!(vol > 0.0, "recalculated normals face outward");
    }

    #[test]
    fn poke_face_adds_centroid_fan() {
        let (pos, idx) = cube();
        let (np, ni, sel) = poke_faces_indexed(&pos, &idx, &[0]);
        assert_eq!(np.len(), 8 + 1);
        assert_eq!(ni.len() / 3, 12 - 1 + 3);
        assert_eq!(sel.len(), 1);
        HalfEdgeMesh::build_from_indexed(&np, &ni).expect("manifold");
    }

    #[test]
    fn bevel_cube_vertex_stays_manifold() {
        // Bevel one cube corner: the corner vertex is replaced by a cap face,
        // so vertex count grows and the result is still a valid manifold.
        let (pos, idx) = cube();
        let (np, ni, sel) = bevel_vertices_indexed(&pos, &idx, &[6], 0.3);
        assert!(np.len() > pos.len(), "bevel adds chamfer vertices");
        assert!(!sel.is_empty(), "cap vertices selected");
        let m = HalfEdgeMesh::build_from_indexed(&np, &ni)
            .expect("beveled cube is a valid manifold");
        m.validate().expect("half-edge invariants hold after bevel");
    }

    #[test]
    fn smooth_rounds_subdivided_cube() {
        // Subdivide the cube twice, then Laplacian-smooth heavily: the closed
        // surface should shrink toward its centroid (umbrella smoothing has no
        // volume preservation), proving smoothing actually moves geometry.
        let (pos, idx) = cube();
        let (p1, i1, _) = subdivide_faces_indexed(&pos, &idx, &(0..(idx.len() / 3) as u32).collect::<Vec<_>>());
        let (p2, i2, _) = subdivide_faces_indexed(&p1, &i1, &(0..(i1.len() / 3) as u32).collect::<Vec<_>>());
        let m = HalfEdgeMesh::build_from_indexed(&p2, &i2).unwrap();
        let bbox = |ps: &[[f32; 3]]| {
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for p in ps {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            (hi[0] - lo[0]) * (hi[1] - lo[1]) * (hi[2] - lo[2])
        };
        let before = bbox(&m.pos);
        let sm = m.smoothed_positions(40, 0.5);
        let after = bbox(&sm);
        assert!(after < before * 0.9, "smoothing must shrink the closed cube ({before} -> {after})");
    }

    #[test]
    fn smooth_pulls_spike_toward_neighbors() {
        let (mut pos, idx) = cube();
        let m0 = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        let before = pos[6]; // +X+Y+Z corner
        // Spike vertex 6 far out; one smoothing step must pull it back inward.
        pos[6] = [5.0, 5.0, 5.0];
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        let sm = m.smoothed_positions(1, 0.5);
        let d_before = ((5.0 - before[0]).powi(2) * 3.0).sqrt();
        let d_after = ((sm[6][0] - before[0]).powi(2)
            + (sm[6][1] - before[1]).powi(2)
            + (sm[6][2] - before[2]).powi(2))
        .sqrt();
        assert!(d_after < d_before, "smoothing moves the spike back toward its neighbors");
        let _ = m0;
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

// ── Positional + repair operators ───────────────────────────────────────────

/// Make the winding of every face consistent (Blender *Recalculate Normals*,
/// Shift+N). Operates on raw indexed data — a half-edge mesh can't even hold
/// an inconsistent winding (it would be non-manifold), so this is the pass you
/// run *before* `build_from_indexed`. BFS-floods face adjacency from face 0,
/// flipping any neighbor that traverses a shared edge in the same direction as
/// the already-oriented face. Finally flips the whole mesh if its net signed
/// volume is negative, so a closed surface ends up facing **outward**.
pub fn make_normals_consistent_indexed(pos: &[[f32; 3]], idx: &[u32]) -> Vec<u32> {
    use std::collections::HashMap;
    let nf = idx.len() / 3;
    let mut out = idx.to_vec();

    // Map each undirected edge → the (face, directed a→b) uses of it.
    // After orientation, a shared edge must be used in opposite directions.
    let edge_key = |a: u32, b: u32| (a.min(b), a.max(b));
    // adjacency: edge → list of (face_index)
    let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (f, tri) in out.chunks_exact(3).enumerate() {
        for k in 0..3 {
            edge_faces
                .entry(edge_key(tri[k], tri[(k + 1) % 3]))
                .or_default()
                .push(f);
        }
    }

    // Does face f traverse the directed edge (a→b)?
    let traverses = |out: &[u32], f: usize, a: u32, b: u32| -> bool {
        let t = &out[f * 3..f * 3 + 3];
        for k in 0..3 {
            if t[k] == a && t[(k + 1) % 3] == b {
                return true;
            }
        }
        false
    };
    let flip = |out: &mut [u32], f: usize| out.swap(f * 3 + 1, f * 3 + 2);

    let mut visited = vec![false; nf];
    for seed in 0..nf {
        if visited[seed] {
            continue;
        }
        visited[seed] = true;
        let mut stack = vec![seed];
        while let Some(f) = stack.pop() {
            let tri = [out[f * 3], out[f * 3 + 1], out[f * 3 + 2]];
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                let Some(neigh) = edge_faces.get(&edge_key(a, b)) else {
                    continue;
                };
                for &g in neigh {
                    if g == f || visited[g] {
                        continue;
                    }
                    // Consistent ⇔ g traverses the shared edge as (b→a).
                    if traverses(&out, g, a, b) {
                        flip(&mut out, g); // same direction → flip
                    }
                    visited[g] = true;
                    stack.push(g);
                }
            }
        }
    }

    // Outward check via net signed volume of the tetrahedra (origin, tri).
    let mut vol = 0.0f64;
    for tri in out.chunks_exact(3) {
        let a = pos[tri[0] as usize];
        let b = pos[tri[1] as usize];
        let c = pos[tri[2] as usize];
        let cross = [
            b[1] as f64 * c[2] as f64 - b[2] as f64 * c[1] as f64,
            b[2] as f64 * c[0] as f64 - b[0] as f64 * c[2] as f64,
            b[0] as f64 * c[1] as f64 - b[1] as f64 * c[0] as f64,
        ];
        vol += a[0] as f64 * cross[0] + a[1] as f64 * cross[1] + a[2] as f64 * cross[2];
    }
    if vol < 0.0 {
        for f in 0..nf {
            flip(&mut out, f);
        }
    }
    out
}

/// Poke every face in `faces`: replace each triangle with a 3-triangle fan
/// around a new centroid vertex (Blender *Poke Faces*). Returns the rebuilt
/// indexed data and the new centroid vertex ids (selected afterward).
pub fn poke_faces_indexed(
    pos: &[[f32; 3]],
    idx: &[u32],
    faces: &[u32],
) -> (Vec<[f32; 3]>, Vec<u32>, Vec<u32>) {
    let mut in_region = vec![false; idx.len() / 3];
    for &fi in faces {
        if (fi as usize) < in_region.len() {
            in_region[fi as usize] = true;
        }
    }
    let mut new_pos = pos.to_vec();
    let mut new_idx: Vec<u32> = Vec::with_capacity(idx.len() + faces.len() * 6);
    let mut sel = Vec::new();
    for (fi, tri) in idx.chunks_exact(3).enumerate() {
        if !in_region[fi] {
            new_idx.extend_from_slice(tri);
            continue;
        }
        let [a, b, c] = [pos[tri[0] as usize], pos[tri[1] as usize], pos[tri[2] as usize]];
        new_pos.push([
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ]);
        let m = (new_pos.len() - 1) as u32;
        sel.push(m);
        new_idx.extend_from_slice(&[tri[0], tri[1], m, tri[1], tri[2], m, tri[2], tri[0], m]);
    }
    (new_pos, new_idx, sel)
}

impl HalfEdgeMesh {
    /// Laplacian (umbrella) smoothed positions: each vertex moves `factor`
    /// of the way toward the average of its one-ring neighbors, `iterations`
    /// times. Boundary vertices are pinned (Blender keeps the border stable).
    /// Returns a full position vector; the caller decides which vertices to
    /// actually write (so a selection-only smooth records a small delta).
    pub fn smoothed_positions(&self, iterations: u32, factor: f32) -> Vec<[f32; 3]> {
        let n = self.vertex_count();
        let mut cur: Vec<[f32; 3]> = self.pos.iter().copied().collect();
        // Pin boundary verts: any vertex with a boundary outgoing half-edge.
        let mut boundary = vec![false; n];
        for h in 0..self.half_edge_count() as u32 {
            if self.twin(h) == crate::INVALID {
                boundary[self.he_vert[h as usize] as usize] = true;
                let nx = self.he_next[h as usize];
                boundary[self.he_vert[nx as usize] as usize] = true;
            }
        }
        let mut next = cur.clone();
        for _ in 0..iterations {
            for v in 0..n {
                if boundary[v] {
                    next[v] = cur[v];
                    continue;
                }
                let ring = self.one_ring_vertices(v as u32);
                if ring.is_empty() {
                    next[v] = cur[v];
                    continue;
                }
                let mut avg = [0.0f32; 3];
                for &nb in &ring {
                    let p = cur[nb as usize];
                    avg[0] += p[0];
                    avg[1] += p[1];
                    avg[2] += p[2];
                }
                let inv = 1.0 / ring.len() as f32;
                let p = cur[v];
                next[v] = [
                    p[0] + factor * (avg[0] * inv - p[0]),
                    p[1] + factor * (avg[1] * inv - p[1]),
                    p[2] + factor * (avg[2] * inv - p[2]),
                ];
            }
            std::mem::swap(&mut cur, &mut next);
        }
        cur
    }

    /// Rebuild with consistent, outward-facing winding.
    pub fn with_normals_recalculated(&self) -> Result<HalfEdgeMesh, MeshError> {
        let (p, i) = self.to_indexed();
        let ni = make_normals_consistent_indexed(&p, &i);
        let pv: Vec<[f32; 3]> = p.into_iter().collect();
        HalfEdgeMesh::build_from_indexed(&pv, &ni)
    }

    /// Poke `faces` into centroid fans.
    pub fn with_faces_poked(
        &self,
        faces: &[u32],
    ) -> Result<(HalfEdgeMesh, Vec<u32>), MeshError> {
        let (p, i) = self.to_indexed();
        let (np, ni, sel) = poke_faces_indexed(&p, &i, faces);
        Ok((HalfEdgeMesh::build_from_indexed(&np, &ni)?, sel))
    }
}

/// Vertex bevel (chamfer): each selected vertex is replaced by a face that
/// cuts across its incident edges at fraction `t` (0..1). Each incident face
/// corner at a selected vertex is split into the two edge points; a cap face
/// closes the corner. Operates on the indexed mesh (Blender *Bevel Vertices*).
/// Returns the rebuilt geometry and the new cap vertex ids (selected after).
pub fn bevel_vertices_indexed(
    pos: &[[f32; 3]],
    idx: &[u32],
    verts: &[u32],
    t: f32,
) -> (Vec<[f32; 3]>, Vec<u32>, Vec<u32>) {
    use std::collections::HashMap;
    let t = t.clamp(0.05, 0.49);
    let mut selected = vec![false; pos.len()];
    for &v in verts {
        if (v as usize) < selected.len() {
            selected[v as usize] = true;
        }
    }
    let mut new_pos = pos.to_vec();
    let mut bp: HashMap<(u32, u32), u32> = HashMap::new();
    let mut get_bp = |v: u32, n: u32, np: &mut Vec<[f32; 3]>| -> u32 {
        *bp.entry((v, n)).or_insert_with(|| {
            let a = pos[v as usize];
            let b = pos[n as usize];
            np.push([
                a[0] + t * (b[0] - a[0]),
                a[1] + t * (b[1] - a[1]),
                a[2] + t * (b[2] - a[2]),
            ]);
            (np.len() - 1) as u32
        })
    };

    let mut new_idx: Vec<u32> = Vec::with_capacity(idx.len() * 2);
    // Per selected vertex, the directed cap edges (one per incident face).
    let mut cap_edges: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();

    for tri in idx.chunks_exact(3) {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        // (corner, prev, next) in CCW order: c->a->b, a->b->c, b->c->a.
        let corners = [(a, c, b), (b, a, c), (c, b, a)];
        let mut poly: Vec<u32> = Vec::with_capacity(6);
        for &(x, prev, next) in &corners {
            if selected[x as usize] {
                let p_prev = get_bp(x, prev, &mut new_pos);
                let p_next = get_bp(x, next, &mut new_pos);
                poly.push(p_prev);
                poly.push(p_next);
                cap_edges.entry(x).or_default().push((p_prev, p_next));
            } else {
                poly.push(x);
            }
        }
        // Fan-triangulate the (3..6)-gon, preserving winding.
        for i in 1..poly.len() - 1 {
            new_idx.extend_from_slice(&[poly[0], poly[i], poly[i + 1]]);
        }
    }

    // Cap faces: chain each selected vertex's directed edges into a cycle and
    // fan-triangulate. The edges (p_prev -> p_next) already wind consistently
    // with the corner (incoming from prev, outgoing to next), so the chained
    // ring is the cap boundary; fanning it caps the corner outward.
    let mut sel = Vec::new();
    for (_v, edges) in cap_edges {
        if edges.len() < 3 {
            continue;
        }
        // Build successor map: p_prev -> p_next, then walk the cycle.
        let mut succ: HashMap<u32, u32> = HashMap::new();
        for &(p0, p1) in &edges {
            succ.insert(p0, p1);
        }
        let start = edges[0].0;
        let mut ring: Vec<u32> = vec![start];
        let mut cur = succ[&start];
        let mut guard = 0;
        while cur != start && guard < edges.len() + 2 {
            ring.push(cur);
            cur = match succ.get(&cur) {
                Some(&n) => n,
                None => break,
            };
            guard += 1;
        }
        if ring.len() >= 3 {
            for i in 1..ring.len() - 1 {
                // Reverse winding so the cap faces away from the removed vertex.
                new_idx.extend_from_slice(&[ring[0], ring[i + 1], ring[i]]);
            }
            sel.extend_from_slice(&ring);
        }
    }
    (new_pos, new_idx, sel)
}

impl HalfEdgeMesh {
    /// Bevel (chamfer) the given vertices by fraction `t`.
    pub fn with_vertices_beveled(
        &self,
        verts: &[u32],
        t: f32,
    ) -> Result<(HalfEdgeMesh, Vec<u32>), MeshError> {
        let (p, i) = self.to_indexed();
        let (np, ni, sel) = bevel_vertices_indexed(&p, &i, verts, t);
        Ok((HalfEdgeMesh::build_from_indexed(&np, &ni)?, sel))
    }
}
