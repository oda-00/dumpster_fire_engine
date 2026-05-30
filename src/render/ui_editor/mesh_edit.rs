//! In-editor mesh edit session: wraps `forge_mesh` into a stateful editing
//! context the editor drives — element selection (vertex/edge/face), BVH +
//! screen-space picking, gizmo-style transform with undo/redo, and a wireframe
//! edge list for the viewport overlay.

use forge_mesh::bvh::Bvh;
use forge_mesh::history::History;
use forge_mesh::{HalfEdgeMesh, INVALID};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ElementMode {
    Vertex,
    Edge,
    Face,
}

pub struct EditSession {
    pub mesh: HalfEdgeMesh,
    pub bvh: Bvh,
    pub history: History,
    pub mode: ElementMode,
    /// Unique undirected edges `(v0, v1)` — wireframe overlay + edge selection.
    edges: Vec<(u32, u32)>,
    /// Captured `(vertex, start_pos)` for the in-flight gizmo transform.
    transform_start: Vec<(u32, [f32; 3])>,
}

impl EditSession {
    pub fn from_indexed(pos: &[[f32; 3]], idx: &[u32]) -> Option<Self> {
        let mesh = HalfEdgeMesh::build_from_indexed(pos, idx).ok()?;
        let bvh = mesh.build_bvh();
        let mut s = Self {
            mesh,
            bvh,
            history: History::new(256),
            mode: ElementMode::Vertex,
            edges: Vec::new(),
            transform_start: Vec::new(),
        };
        s.rebuild_edges();
        Some(s)
    }

    /// A unit cube of the given half-extent (fallback when an actor has no
    /// editable source geometry).
    pub fn cube(half: f32) -> Self {
        let h = half;
        let pos = vec![
            [-h, -h, -h], [h, -h, -h], [h, h, -h], [-h, h, -h],
            [-h, -h, h], [h, -h, h], [h, h, h], [-h, h, h],
        ];
        let quads: [[u32; 4]; 6] = [
            [4, 5, 6, 7], [1, 0, 3, 2], [0, 4, 7, 3], [5, 1, 2, 6], [0, 1, 5, 4], [3, 7, 6, 2],
        ];
        let mut idx = Vec::new();
        for q in quads {
            idx.extend_from_slice(&[q[0], q[1], q[2], q[0], q[2], q[3]]);
        }
        Self::from_indexed(&pos, &idx).expect("unit cube is a valid manifold")
    }

    fn rebuild_edges(&mut self) {
        let mut e = Vec::new();
        for h in 0..self.mesh.half_edge_count() as u32 {
            let t = self.mesh.twin(h);
            if t == INVALID || h < t {
                let a = self.mesh.he_vert[h as usize];
                let b = self.mesh.he_vert[self.mesh.he_next[h as usize] as usize];
                e.push((a, b));
            }
        }
        self.edges = e;
    }

    fn refresh_bvh(&mut self) {
        self.bvh = self.mesh.build_bvh();
    }

    pub fn edges(&self) -> &[(u32, u32)] {
        &self.edges
    }
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.mesh.pos
    }
    pub fn vertex_count(&self) -> usize {
        self.mesh.vertex_count()
    }
    pub fn face_count(&self) -> usize {
        self.mesh.face_count()
    }
    pub fn face_verts(&self, f: u32) -> [u32; 3] {
        self.mesh.face_verts(f)
    }
    pub fn is_vertex_selected(&self, v: u32) -> bool {
        self.mesh.is_vertex_selected(v)
    }
    pub fn selected_vertex_count(&self) -> usize {
        self.mesh.count_selected_vertices()
    }

    pub fn pick_face(&self, origin: [f32; 3], dir: [f32; 3]) -> Option<u32> {
        self.bvh.raycast(origin, dir).map(|(_, f)| f)
    }

    /// Nearest vertex to `cursor` whose projection is within `max_px`.
    pub fn nearest_vertex<P: Fn([f32; 3]) -> Option<[f32; 2]>>(
        &self,
        cursor: [f32; 2],
        project: P,
        max_px: f32,
    ) -> Option<u32> {
        let mut best = None;
        let mut bd = max_px * max_px;
        for v in 0..self.mesh.vertex_count() as u32 {
            if let Some(s) = project(self.mesh.pos[v as usize]) {
                let dd = (s[0] - cursor[0]).powi(2) + (s[1] - cursor[1]).powi(2);
                if dd < bd {
                    bd = dd;
                    best = Some(v);
                }
            }
        }
        best
    }

    /// Nearest edge (index into `edges()`) to `cursor` within `max_px`.
    pub fn nearest_edge<P: Fn([f32; 3]) -> Option<[f32; 2]>>(
        &self,
        cursor: [f32; 2],
        project: P,
        max_px: f32,
    ) -> Option<usize> {
        let mut best = None;
        let mut bd = max_px;
        for (i, &(a, b)) in self.edges.iter().enumerate() {
            let (Some(pa), Some(pb)) = (
                project(self.mesh.pos[a as usize]),
                project(self.mesh.pos[b as usize]),
            ) else {
                continue;
            };
            let d = point_seg_dist(cursor, pa, pb);
            if d < bd {
                bd = d;
                best = Some(i);
            }
        }
        best
    }

    pub fn toggle_vertex(&mut self, v: u32) {
        let on = self.mesh.is_vertex_selected(v);
        self.mesh.set_vertex_selected(v, !on);
    }
    pub fn toggle_edge(&mut self, ei: usize) {
        let (a, b) = self.edges[ei];
        let on = self.mesh.is_vertex_selected(a) && self.mesh.is_vertex_selected(b);
        self.mesh.set_vertex_selected(a, !on);
        self.mesh.set_vertex_selected(b, !on);
    }
    pub fn toggle_face(&mut self, f: u32) {
        let [a, b, c] = self.mesh.face_verts(f);
        let on = self.mesh.is_vertex_selected(a)
            && self.mesh.is_vertex_selected(b)
            && self.mesh.is_vertex_selected(c);
        for v in [a, b, c] {
            self.mesh.set_vertex_selected(v, !on);
        }
    }
    pub fn clear_selection(&mut self) {
        self.mesh.deselect_all_vertices();
    }

    /// Centroid of the selected vertices (the gizmo origin in edit mode).
    pub fn selection_centroid(&self) -> Option<[f32; 3]> {
        let mut sum = [0.0f32; 3];
        let mut n = 0u32;
        for v in 0..self.mesh.vertex_count() as u32 {
            if self.mesh.is_vertex_selected(v) {
                let p = self.mesh.pos[v as usize];
                sum[0] += p[0];
                sum[1] += p[1];
                sum[2] += p[2];
                n += 1;
            }
        }
        (n > 0).then(|| [sum[0] / n as f32, sum[1] / n as f32, sum[2] / n as f32])
    }

    // ── gizmo-style transform (begin → update(delta) → commit) ───────────────

    pub fn begin_transform(&mut self) {
        self.transform_start = (0..self.mesh.vertex_count() as u32)
            .filter(|&v| self.mesh.is_vertex_selected(v))
            .map(|v| (v, self.mesh.pos[v as usize]))
            .collect();
    }

    /// Set selected vertices to `start + delta` (absolute from the drag start).
    pub fn update_transform(&mut self, delta: [f32; 3]) {
        for &(v, start) in &self.transform_start {
            self.mesh.pos[v as usize] = [start[0] + delta[0], start[1] + delta[1], start[2] + delta[2]];
        }
    }

    /// Record the completed transform as one undo delta and refresh the BVH.
    pub fn commit_transform(&mut self) {
        if self.transform_start.is_empty() {
            return;
        }
        let ids: thin_vec::ThinVec<u32> = self.transform_start.iter().map(|&(v, _)| v).collect();
        let before: thin_vec::ThinVec<[f32; 3]> =
            self.transform_start.iter().map(|&(_, p)| p).collect();
        let after: thin_vec::ThinVec<[f32; 3]> =
            ids.iter().map(|&v| self.mesh.pos[v as usize]).collect();
        self.history
            .push(forge_mesh::history::Edit::Positions { ids, before, after });
        self.transform_start.clear();
        self.refresh_bvh();
    }

    pub fn undo(&mut self) -> bool {
        let ok = self.history.undo(&mut self.mesh);
        if ok {
            self.refresh_bvh();
        }
        ok
    }
    pub fn redo(&mut self) -> bool {
        let ok = self.history.redo(&mut self.mesh);
        if ok {
            self.refresh_bvh();
        }
        ok
    }
}

#[inline]
fn point_seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-6 {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0);
    let cx = a[0] + t * dx;
    let cy = a[1] + t * dy;
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_session_has_12_edges() {
        let s = EditSession::cube(1.0);
        assert_eq!(s.vertex_count(), 8);
        assert_eq!(s.face_count(), 12);
        assert_eq!(s.edges().len(), 18); // triangulated cube
    }

    #[test]
    fn pick_face_with_ray() {
        let s = EditSession::cube(1.0);
        // Ray from far +z toward -z hits the +z face.
        assert!(s.pick_face([0.0, 0.0, 5.0], [0.0, 0.0, -1.0]).is_some());
        assert!(s.pick_face([5.0, 5.0, 5.0], [0.0, 0.0, -1.0]).is_none());
    }

    #[test]
    fn nearest_vertex_uses_projector() {
        let s = EditSession::cube(1.0);
        // Orthographic xy projection; cursor near corner (1,1,*) → that vertex.
        let project = |p: [f32; 3]| Some([p[0], p[1]]);
        let v = s.nearest_vertex([1.0, 1.0], project, 0.5).unwrap();
        let p = s.positions()[v as usize];
        assert_eq!([p[0], p[1]], [1.0, 1.0]);
    }

    #[test]
    fn transform_with_undo() {
        let mut s = EditSession::cube(1.0);
        s.toggle_vertex(6); // corner [1,1,1]
        s.begin_transform();
        s.update_transform([2.0, 0.0, 0.0]);
        s.commit_transform();
        assert_eq!(s.positions()[6], [3.0, 1.0, 1.0]);
        assert!(s.undo());
        assert_eq!(s.positions()[6], [1.0, 1.0, 1.0]);
        assert!(s.redo());
        assert_eq!(s.positions()[6], [3.0, 1.0, 1.0]);
    }

    #[test]
    fn toggle_face_selects_three_verts() {
        let mut s = EditSession::cube(1.0);
        s.toggle_face(0);
        assert_eq!(s.selected_vertex_count(), 3);
        s.toggle_face(0);
        assert_eq!(s.selected_vertex_count(), 0);
    }
}
