//! The SoA index-based half-edge mesh.

use thin_vec::{thin_vec, ThinVec};

/// Sentinel for "no half-edge / boundary".
pub const INVALID: u32 = u32::MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeshError {
    /// Index count is not a multiple of 3 (Phase 0 is triangles-only).
    NonTriangle,
    /// A directed edge `a→b` occurred more than once — the mesh is non-manifold
    /// (an undirected edge shared by >2 faces, or inconsistent winding).
    NonManifold { a: u32, b: u32 },
    /// An invariant check failed.
    Invalid(&'static str),
}

/// Half-edge mesh in struct-of-arrays form. All `he_*` arrays are parallel and
/// indexed by half-edge id; `vert_*`/`face_*` are indexed by vertex/face id.
///
/// Half-edge winding: for triangle `t`, half-edges `3t, 3t+1, 3t+2` form the loop
/// via `he_next`, with `he_vert[h]` the *origin* vertex of `h` (so `h` is the
/// directed edge `he_vert[h] → he_vert[he_next[h]]`).
#[derive(Debug, Clone, Default)]
pub struct HalfEdgeMesh {
    pub he_next: ThinVec<u32>,
    pub he_twin: ThinVec<u32>,
    pub he_vert: ThinVec<u32>,
    pub he_face: ThinVec<u32>,
    pub vert_he: ThinVec<u32>,
    pub face_he: ThinVec<u32>,
    pub pos: ThinVec<[f32; 3]>,
    // Per-element flag layers (selection/visibility/etc.) — used from Phase 1 on.
    pub vflag: ThinVec<u8>,
    pub hflag: ThinVec<u8>,
    pub fflag: ThinVec<u8>,
    // Free lists for in-place slot reuse during editing (no compaction churn).
    pub free_he: ThinVec<u32>,
    pub free_vert: ThinVec<u32>,
    pub free_face: ThinVec<u32>,
}

impl HalfEdgeMesh {
    /// Build a half-edge mesh from a triangle position+index buffer.
    pub fn build_from_indexed(positions: &[[f32; 3]], indices: &[u32]) -> Result<Self, MeshError> {
        if indices.len() % 3 != 0 {
            return Err(MeshError::NonTriangle);
        }
        let nhe = indices.len();
        let nf = nhe / 3;
        let nv = positions.len();

        let mut m = HalfEdgeMesh {
            he_next: ThinVec::with_capacity(nhe),
            he_twin: thin_vec![INVALID; nhe],
            he_vert: ThinVec::with_capacity(nhe),
            he_face: ThinVec::with_capacity(nhe),
            vert_he: thin_vec![INVALID; nv],
            face_he: ThinVec::with_capacity(nf),
            pos: positions.iter().copied().collect(),
            vflag: thin_vec![0u8; nv],
            hflag: thin_vec![0u8; nhe],
            fflag: thin_vec![0u8; nf],
            free_he: ThinVec::new(),
            free_vert: ThinVec::new(),
            free_face: ThinVec::new(),
        };

        // Pass 1: per-triangle half-edges, next-cycle, origin vertex, owning face.
        for t in 0..nf {
            let base = (t * 3) as u32;
            for k in 0..3u32 {
                let h = (base + k) as usize;
                let v = indices[h];
                if v as usize >= nv {
                    return Err(MeshError::Invalid("index out of range"));
                }
                m.he_vert.push(v);
                m.he_face.push(t as u32);
                m.he_next.push(base + (k + 1) % 3);
                if m.vert_he[v as usize] == INVALID {
                    m.vert_he[v as usize] = h as u32;
                }
            }
            m.face_he.push(base);
        }

        // Pass 2: pair twins via a transient directed-edge map (build-time only).
        // A manifold mesh uses each directed edge `a→b` exactly once.
        let mut dir: std::collections::HashMap<(u32, u32), u32> =
            std::collections::HashMap::with_capacity(nhe);
        for h in 0..nhe as u32 {
            let a = m.he_vert[h as usize];
            let b = m.he_vert[m.he_next[h as usize] as usize];
            if dir.insert((a, b), h).is_some() {
                return Err(MeshError::NonManifold { a, b });
            }
        }
        for h in 0..nhe as u32 {
            let a = m.he_vert[h as usize];
            let b = m.he_vert[m.he_next[h as usize] as usize];
            if let Some(&twin) = dir.get(&(b, a)) {
                m.he_twin[h as usize] = twin;
            }
        }

        Ok(m)
    }

    #[inline]
    pub fn vertex_count(&self) -> usize {
        self.pos.len()
    }
    #[inline]
    pub fn face_count(&self) -> usize {
        self.face_he.len()
    }
    #[inline]
    pub fn half_edge_count(&self) -> usize {
        self.he_next.len()
    }

    /// Number of undirected edges (each counted once).
    pub fn edge_count(&self) -> usize {
        let mut c = 0usize;
        for h in 0..self.he_next.len() as u32 {
            let t = self.he_twin[h as usize];
            if t == INVALID || h < t {
                c += 1;
            }
        }
        c
    }

    #[inline]
    pub fn twin(&self, h: u32) -> u32 {
        self.he_twin[h as usize]
    }
    #[inline]
    pub fn is_boundary(&self, h: u32) -> bool {
        self.he_twin[h as usize] == INVALID
    }

    /// The three vertices of triangle face `f`, in winding order.
    pub fn face_verts(&self, f: u32) -> [u32; 3] {
        let h0 = self.face_he[f as usize];
        let h1 = self.he_next[h0 as usize];
        let h2 = self.he_next[h1 as usize];
        [
            self.he_vert[h0 as usize],
            self.he_vert[h1 as usize],
            self.he_vert[h2 as usize],
        ]
    }

    /// Visit each half-edge whose origin is vertex `v`, rotating around `v` via
    /// `next(twin(h))`. Stops at a boundary (one direction) or when it returns to
    /// the start. (See `docs/editor_research/asm/exp1_halfedge.rs` for the codegen
    /// rationale vs pointer-chasing.)
    pub fn for_each_outgoing<F: FnMut(u32)>(&self, v: u32, mut f: F) {
        let start = self.vert_he[v as usize];
        if start == INVALID {
            return;
        }
        let mut h = start;
        loop {
            f(h);
            let t = self.he_twin[h as usize];
            if t == INVALID {
                break;
            }
            h = self.he_next[t as usize];
            if h == start {
                break;
            }
        }
    }

    /// Neighbor vertices in the one-ring of `v` (destinations of outgoing edges).
    pub fn one_ring_vertices(&self, v: u32) -> ThinVec<u32> {
        let mut out = ThinVec::new();
        self.for_each_outgoing(v, |h| {
            out.push(self.he_vert[self.he_next[h as usize] as usize]);
        });
        out
    }

    /// Number of outgoing half-edges at `v` (its valence, for interior vertices).
    pub fn valence(&self, v: u32) -> u32 {
        let mut n = 0u32;
        self.for_each_outgoing(v, |_| n += 1);
        n
    }

    /// Round-trip: emit `(positions, triangle indices)`.
    pub fn to_indexed(&self) -> (ThinVec<[f32; 3]>, ThinVec<u32>) {
        let mut idx = ThinVec::with_capacity(self.face_he.len() * 3);
        for f in 0..self.face_he.len() as u32 {
            let [a, b, c] = self.face_verts(f);
            idx.push(a);
            idx.push(b);
            idx.push(c);
        }
        (self.pos.clone(), idx)
    }

    /// Check structural invariants. Returns `Ok(())` for a well-formed mesh.
    pub fn validate(&self) -> Result<(), MeshError> {
        let nhe = self.he_next.len();
        let nv = self.pos.len();
        let nf = self.face_he.len();
        if self.he_twin.len() != nhe || self.he_vert.len() != nhe || self.he_face.len() != nhe {
            return Err(MeshError::Invalid("half-edge arrays length mismatch"));
        }
        for h in 0..nhe {
            if self.he_vert[h] as usize >= nv {
                return Err(MeshError::Invalid("he_vert out of range"));
            }
            if self.he_next[h] as usize >= nhe {
                return Err(MeshError::Invalid("he_next out of range"));
            }
            if self.he_face[h] as usize >= nf {
                return Err(MeshError::Invalid("he_face out of range"));
            }
            let t = self.he_twin[h];
            if t != INVALID {
                if t as usize >= nhe {
                    return Err(MeshError::Invalid("he_twin out of range"));
                }
                if self.he_twin[t as usize] != h as u32 {
                    return Err(MeshError::Invalid("twin asymmetry"));
                }
            }
            // next must form a 3-cycle (triangle face).
            let n1 = self.he_next[h] as usize;
            let n2 = self.he_next[n1] as usize;
            let n3 = self.he_next[n2];
            if n3 != h as u32 {
                return Err(MeshError::Invalid("face is not a triangle cycle"));
            }
        }
        for f in 0..nf {
            if self.face_he[f] as usize >= nhe {
                return Err(MeshError::Invalid("face_he out of range"));
            }
        }
        for v in 0..nv {
            let hv = self.vert_he[v];
            if hv != INVALID && hv as usize >= nhe {
                return Err(MeshError::Invalid("vert_he out of range"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit cube: 8 corners, 12 triangles, consistent outward (CCW) winding.
    fn cube() -> (Vec<[f32; 3]>, Vec<u32>) {
        let pos = vec![
            [-1.0, -1.0, -1.0], // 0
            [1.0, -1.0, -1.0],  // 1
            [1.0, 1.0, -1.0],   // 2
            [-1.0, 1.0, -1.0],  // 3
            [-1.0, -1.0, 1.0],  // 4
            [1.0, -1.0, 1.0],   // 5
            [1.0, 1.0, 1.0],    // 6
            [-1.0, 1.0, 1.0],   // 7
        ];
        // Quads (CCW outward) → 2 tris each: (a,b,c),(a,c,d).
        let quads: [[u32; 4]; 6] = [
            [4, 5, 6, 7], // +z front
            [1, 0, 3, 2], // -z back
            [0, 4, 7, 3], // -x left
            [5, 1, 2, 6], // +x right
            [0, 1, 5, 4], // -y bottom
            [3, 7, 6, 2], // +y top
        ];
        let mut idx = Vec::new();
        for q in quads {
            idx.extend_from_slice(&[q[0], q[1], q[2], q[0], q[2], q[3]]);
        }
        (pos, idx)
    }

    #[test]
    fn cube_is_closed_manifold() {
        let (pos, idx) = cube();
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        assert_eq!(m.vertex_count(), 8);
        assert_eq!(m.face_count(), 12);
        assert_eq!(m.half_edge_count(), 36);
        // Closed: no boundary half-edges.
        assert!((0..36).all(|h| !m.is_boundary(h)), "cube should have no boundary");
        // Euler characteristic V - E + F == 2 for a closed genus-0 surface.
        assert_eq!(m.edge_count(), 18);
        assert_eq!(
            m.vertex_count() as i64 - m.edge_count() as i64 + m.face_count() as i64,
            2
        );
        m.validate().unwrap();
        // Every corner has neighbors and a sane valence (4 or 5 on a tri-cube).
        for v in 0..8u32 {
            let val = m.valence(v);
            assert!(val >= 3, "vertex {v} valence {val}");
        }
    }

    #[test]
    fn quad_has_boundary() {
        // Two triangles forming a quad in the z=0 plane.
        let pos = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = vec![0, 1, 2, 0, 2, 3];
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        m.validate().unwrap();
        // The 4 outer edges are boundary; the shared diagonal (0-2) is paired.
        let boundary = (0..m.half_edge_count() as u32).filter(|&h| m.is_boundary(h)).count();
        assert_eq!(boundary, 4);
        assert_eq!(m.edge_count(), 5); // 4 outer + 1 diagonal
    }

    #[test]
    fn round_trip_preserves_topology() {
        let (pos, idx) = cube();
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        let (rp, ri) = m.to_indexed();
        assert_eq!(rp.as_slice(), pos.as_slice());
        // Triangle sets equal (each triangle compared as a sorted triple).
        let norm = |v: &[u32]| {
            let mut tris: Vec<[u32; 3]> = v
                .chunks_exact(3)
                .map(|c| {
                    let mut t = [c[0], c[1], c[2]];
                    t.sort_unstable();
                    t
                })
                .collect();
            tris.sort_unstable();
            tris
        };
        assert_eq!(norm(&ri), norm(&idx));
    }

    #[test]
    fn non_manifold_is_rejected() {
        // Two triangles sharing the *same-direction* edge 0→1 → non-manifold.
        let pos = vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]];
        let idx = vec![0, 1, 2, 0, 1, 3];
        assert_eq!(
            HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap_err(),
            MeshError::NonManifold { a: 0, b: 1 }
        );
    }

    #[test]
    fn non_triangle_is_rejected() {
        let pos = vec![[0.0; 3], [1.0; 3]];
        let idx = vec![0, 1]; // not a multiple of 3
        assert_eq!(
            HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap_err(),
            MeshError::NonTriangle
        );
    }
}
