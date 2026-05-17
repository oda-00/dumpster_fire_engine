//! Skinning data — joint nodes + inverse bind matrices.

use thin_vec::ThinVec;

#[derive(Debug, Clone)]
pub struct Skin {
    pub name:                  Option<String>,
    pub joints:                ThinVec<u32>,
    /// Column-major 4x4 per joint; same length as `joints`.
    pub inverse_bind_matrices: ThinVec<[f32; 16]>,
    pub skeleton_root:         Option<u32>,
}
