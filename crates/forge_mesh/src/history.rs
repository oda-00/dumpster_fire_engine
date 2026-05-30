//! Transaction-based undo/redo (Unreal ITF / Godot UndoRedo model).
//!
//! Attribute-only edits (transform, selection — the common case) record a
//! **delta**: just the touched element ids + their before/after values, so undo
//! is O(touched), matching the edit cost — not Blender's per-mode full snapshots.
//! Topology-changing ops (subdivide, delete) fall back to a `Snapshot` since the
//! array shapes change; those are rarer and already O(mesh).

use thin_vec::ThinVec;

use crate::half_edge::HalfEdgeMesh;

/// One reversible edit.
pub enum Edit {
    /// O(touched) vertex-position delta.
    Positions {
        ids: ThinVec<u32>,
        before: ThinVec<[f32; 3]>,
        after: ThinVec<[f32; 3]>,
    },
    /// O(touched) vertex-flag delta.
    VFlags {
        ids: ThinVec<u32>,
        before: ThinVec<u8>,
        after: ThinVec<u8>,
    },
    /// Whole-mesh snapshot for topology changes.
    Snapshot {
        before: Box<HalfEdgeMesh>,
        after: Box<HalfEdgeMesh>,
    },
}

impl Edit {
    fn apply_before(&self, m: &mut HalfEdgeMesh) {
        match self {
            Edit::Positions { ids, before, .. } => {
                for (i, &id) in ids.iter().enumerate() {
                    m.pos[id as usize] = before[i];
                }
            }
            Edit::VFlags { ids, before, .. } => {
                for (i, &id) in ids.iter().enumerate() {
                    m.vflag[id as usize] = before[i];
                }
            }
            Edit::Snapshot { before, .. } => *m = (**before).clone(),
        }
    }
    fn apply_after(&self, m: &mut HalfEdgeMesh) {
        match self {
            Edit::Positions { ids, after, .. } => {
                for (i, &id) in ids.iter().enumerate() {
                    m.pos[id as usize] = after[i];
                }
            }
            Edit::VFlags { ids, after, .. } => {
                for (i, &id) in ids.iter().enumerate() {
                    m.vflag[id as usize] = after[i];
                }
            }
            Edit::Snapshot { after, .. } => *m = (**after).clone(),
        }
    }
}

/// Undo/redo stacks with a bounded depth.
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    limit: usize,
}

impl History {
    pub fn new(limit: usize) -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit: limit.max(1) }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Record an edit. Clears the redo stack (new branch) and trims to `limit`.
    pub fn push(&mut self, e: Edit) {
        self.undo.push(e);
        self.redo.clear();
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    pub fn undo(&mut self, m: &mut HalfEdgeMesh) -> bool {
        if let Some(e) = self.undo.pop() {
            e.apply_before(m);
            self.redo.push(e);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, m: &mut HalfEdgeMesh) -> bool {
        if let Some(e) = self.redo.pop() {
            e.apply_after(m);
            self.undo.push(e);
            true
        } else {
            false
        }
    }

    /// Convenience: record the position delta for `ids`, given their `before`
    /// values; reads the current (after) values from `m`.
    pub fn record_positions(&mut self, m: &HalfEdgeMesh, ids: ThinVec<u32>, before: ThinVec<[f32; 3]>) {
        let after = ids.iter().map(|&i| m.pos[i as usize]).collect();
        self.push(Edit::Positions { ids, before, after });
    }

    /// Capture the current positions of `ids` (call before an edit).
    pub fn snapshot_positions(m: &HalfEdgeMesh, ids: &[u32]) -> ThinVec<[f32; 3]> {
        ids.iter().map(|&i| m.pos[i as usize]).collect()
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
    fn position_edit_undo_redo() {
        let mut m = quad();
        let mut h = History::new(64);
        let ids: ThinVec<u32> = ThinVec::from(vec![1u32, 2]);
        let before = History::snapshot_positions(&m, &ids);
        m.set_vertex_selected(1, true);
        m.set_vertex_selected(2, true);
        m.translate_selected_vertices([10.0, 0.0, 0.0]);
        h.record_positions(&m, ids, before);

        assert_eq!(m.pos[1], [11.0, 0.0, 0.0]);
        assert!(h.undo(&mut m));
        assert_eq!(m.pos[1], [1.0, 0.0, 0.0]); // restored
        assert_eq!(m.pos[2], [1.0, 1.0, 0.0]);
        assert!(h.redo(&mut m));
        assert_eq!(m.pos[1], [11.0, 0.0, 0.0]); // reapplied
        assert!(!h.can_redo());
    }

    #[test]
    fn snapshot_undo_restores_topology() {
        let mut m = quad();
        let mut h = History::new(64);
        let before = Box::new(m.clone());
        m = m.subdivided().unwrap();
        let after = Box::new(m.clone());
        h.push(Edit::Snapshot { before, after });
        assert_eq!(m.face_count(), 8); // quad (2 tris) → 8
        assert!(h.undo(&mut m));
        assert_eq!(m.face_count(), 2); // restored
        assert!(h.redo(&mut m));
        assert_eq!(m.face_count(), 8);
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut m = quad();
        let mut h = History::new(64);
        let ids: ThinVec<u32> = ThinVec::from(vec![0u32]);
        let before = History::snapshot_positions(&m, &ids);
        m.pos[0] = [9.0, 9.0, 9.0];
        h.record_positions(&m, ids.clone(), before);
        h.undo(&mut m);
        assert!(h.can_redo());
        // A fresh edit invalidates the redo branch.
        let before2 = History::snapshot_positions(&m, &ids);
        m.pos[0] = [1.0, 1.0, 1.0];
        h.record_positions(&m, ids, before2);
        assert!(!h.can_redo());
    }
}
