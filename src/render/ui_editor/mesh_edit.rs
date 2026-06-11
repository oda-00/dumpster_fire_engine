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

    // ── Topology operators (Blender-core, undoable via Snapshot) ──────────

    /// Faces whose three vertices are all selected — the operand region for
    /// face-driven operators, matching Blender's vertex→face selection flush.
    pub fn selected_faces(&self) -> Vec<u32> {
        self.mesh.faces_with_all_selected().into_iter().collect()
    }

    /// Run a topology-changing operator: snapshot for undo, swap the mesh in,
    /// select the operator's result geometry, refresh edges + BVH.
    fn apply_topology(
        &mut self,
        result: Result<(forge_mesh::HalfEdgeMesh, Vec<u32>), forge_mesh::MeshError>,
    ) -> bool {
        let Ok((mut new_mesh, sel)) = result else {
            return false;
        };
        new_mesh.deselect_all_vertices();
        for &v in &sel {
            new_mesh.set_vertex_selected(v, true);
        }
        let before = Box::new(std::mem::replace(&mut self.mesh, new_mesh));
        self.history.push(forge_mesh::history::Edit::Snapshot {
            before,
            after: Box::new(self.mesh.clone()),
        });
        self.rebuild_edges();
        self.refresh_bvh();
        true
    }

    /// Extrude the selected face region along its normal (Blender E).
    pub fn extrude_selected(&mut self, offset: f32) -> bool {
        let faces = self.selected_faces();
        if faces.is_empty() {
            return false;
        }
        self.apply_topology(self.mesh.with_faces_extruded(&faces, offset))
    }

    /// Inset each selected face individually (Blender I).
    pub fn inset_selected(&mut self, amount: f32) -> bool {
        let faces = self.selected_faces();
        if faces.is_empty() {
            return false;
        }
        self.apply_topology(self.mesh.with_faces_inset(&faces, amount))
    }

    /// Crack-free subdivision of the selected faces (whole mesh when nothing
    /// is selected — Blender's subdivide-with-everything-selected default).
    pub fn subdivide_selected(&mut self) -> bool {
        let mut faces = self.selected_faces();
        if faces.is_empty() {
            faces = (0..self.mesh.face_count() as u32).collect();
        }
        self.apply_topology(self.mesh.with_faces_subdivided(&faces))
    }

    /// Delete the selected faces (Blender X → Faces).
    pub fn delete_selected(&mut self) -> bool {
        let faces = self.selected_faces();
        if faces.is_empty() {
            return false;
        }
        self.apply_topology(
            self.mesh
                .with_faces_deleted(&faces)
                .map(|m| (m, Vec::new())),
        )
    }

    /// Merge the selected vertices at their centroid (Blender M → Center).
    pub fn merge_selected(&mut self) -> bool {
        if self.mesh.count_selected_vertices() < 2 {
            return false;
        }
        self.apply_topology(self.mesh.with_selected_merged())
    }

    /// Laplacian-smooth the selected vertices (whole mesh when none are
    /// selected). Positional only, so it records a cheap O(touched) delta
    /// rather than a topology snapshot.
    pub fn smooth_selected(&mut self, iterations: u32, factor: f32) -> bool {
        let smoothed = self.mesh.smoothed_positions(iterations, factor);
        let ids: thin_vec::ThinVec<u32> = if self.mesh.count_selected_vertices() > 0 {
            self.mesh.selected_vertices()
        } else {
            (0..self.mesh.vertex_count() as u32).collect()
        };
        if ids.is_empty() {
            return false;
        }
        let before: thin_vec::ThinVec<[f32; 3]> =
            ids.iter().map(|&v| self.mesh.pos[v as usize]).collect();
        for &v in &ids {
            self.mesh.pos[v as usize] = smoothed[v as usize];
        }
        let after: thin_vec::ThinVec<[f32; 3]> =
            ids.iter().map(|&v| self.mesh.pos[v as usize]).collect();
        self.history
            .push(forge_mesh::history::Edit::Positions { ids, before, after });
        self.refresh_bvh();
        true
    }

    /// Recalculate consistent, outward-facing normals (Blender Shift+N).
    pub fn recalc_normals(&mut self) -> bool {
        self.apply_topology(self.mesh.with_normals_recalculated().map(|m| (m, Vec::new())))
    }

    /// Poke the selected faces into centroid fans (whole mesh when none).
    pub fn poke_selected(&mut self) -> bool {
        let mut faces = self.selected_faces();
        if faces.is_empty() {
            faces = (0..self.mesh.face_count() as u32).collect();
        }
        self.apply_topology(self.mesh.with_faces_poked(&faces))
    }

    /// Weld coincident vertices across the whole mesh.
    pub fn weld(&mut self, eps: f32) -> bool {
        self.apply_topology(self.mesh.welded(eps).map(|m| (m, Vec::new())))
    }

    /// Flip the winding of the selected faces (all faces when none selected).
    pub fn flip_selected(&mut self) -> bool {
        let mut faces = self.selected_faces();
        if faces.is_empty() {
            faces = (0..self.mesh.face_count() as u32).collect();
        }
        self.apply_topology(self.mesh.with_faces_flipped(&faces).map(|m| (m, Vec::new())))
    }

    // ── Selection commands (Blender A / Ctrl+I / Ctrl+± / L) ─────────────

    pub fn select_all(&mut self) {
        self.mesh.select_all_vertices();
    }
    pub fn select_none(&mut self) {
        self.mesh.deselect_all_vertices();
    }
    pub fn invert_selection(&mut self) {
        self.mesh.invert_vertex_selection();
    }
    pub fn grow_selection(&mut self) {
        self.mesh.grow_vertex_selection();
    }
    pub fn shrink_selection(&mut self) {
        self.mesh.shrink_vertex_selection();
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Box-select vertices whose projected screen position falls inside the
    /// rectangle `[lo, hi]` (screen px). `additive` keeps the current
    /// selection (Shift+drag); otherwise it replaces it. `project` maps a
    /// world-space vertex to screen px, or `None` if behind the camera.
    /// Indexed (positions, normals, indices) for committing the edited mesh
    /// back to a rendered GPU asset. Normals are smooth per-vertex.
    pub fn to_renderable(&self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>) {
        let (pos, idx) = self.mesh.to_indexed();
        let normals = self.mesh.vertex_normals();
        (pos.into_iter().collect(), normals, idx.into_iter().collect())
    }

    pub fn box_select_screen<P: Fn([f32; 3]) -> Option<[f32; 2]>>(
        &mut self,
        lo: [f32; 2],
        hi: [f32; 2],
        additive: bool,
        project: P,
    ) -> usize {
        if !additive {
            self.mesh.deselect_all_vertices();
        }
        let mut hit = 0;
        for v in 0..self.mesh.vertex_count() {
            if let Some(s) = project(self.mesh.pos[v]) {
                if s[0] >= lo[0] && s[0] <= hi[0] && s[1] >= lo[1] && s[1] <= hi[1] {
                    self.mesh.set_vertex_selected(v as u32, true);
                    hit += 1;
                }
            }
        }
        hit
    }

    pub fn undo(&mut self) -> bool {
        let ok = self.history.undo(&mut self.mesh);
        if ok {
            self.rebuild_edges();
            self.refresh_bvh();
        }
        ok
    }
    pub fn redo(&mut self) -> bool {
        let ok = self.history.redo(&mut self.mesh);
        if ok {
            self.rebuild_edges();
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
