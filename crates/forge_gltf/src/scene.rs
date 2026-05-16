//! Scene + node hierarchy with composed world transforms.
//!
//! Each `Node` keeps the local TRS (or matrix) form it came in with plus a
//! pre-composed local 4x4 (column-major) for fast traversal. World matrices
//! are computed once in `compose_world_matrices` so every pipeline adapter
//! gets stable instance transforms without re-walking the tree.

use thin_vec::ThinVec;

pub type NodeIndex = u32;
pub type SceneIndex = u32;

/// One node in the scene tree. Indices into `GltfAsset.{meshes, cameras, ...}`
/// are stored as `Option<u32>`; root nodes have `parent: None`.
#[derive(Debug, Clone)]
pub struct Node {
    pub name:        Option<String>,
    pub parent:      Option<NodeIndex>,
    pub children:    ThinVec<NodeIndex>,
    /// Local TRS, decomposed. Translation, rotation (xyzw quaternion), scale.
    pub translation: [f32; 3],
    pub rotation:    [f32; 4],
    pub scale:       [f32; 3],
    /// Local matrix already composed from TRS (column-major, 16 floats).
    pub local_matrix:[f32; 16],
    pub mesh:        Option<u32>,
    pub camera:      Option<u32>,
    pub skin:        Option<u32>,
    pub light:       Option<u32>,
    /// Indices into morph-target weights of the referenced mesh (if any).
    pub weights:     ThinVec<f32>,
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub name:  Option<String>,
    pub roots: ThinVec<NodeIndex>,
}

/// Compose a column-major TRS matrix. Matches glTF's right-handed
/// translation * rotation * scale convention.
pub fn compose_trs(t: [f32; 3], r: [f32; 4], s: [f32; 3]) -> [f32; 16] {
    let [qx, qy, qz, qw] = r;
    let xx = qx * qx;
    let yy = qy * qy;
    let zz = qz * qz;
    let xy = qx * qy;
    let xz = qx * qz;
    let yz = qy * qz;
    let wx = qw * qx;
    let wy = qw * qy;
    let wz = qw * qz;

    let r00 = 1.0 - 2.0 * (yy + zz);
    let r01 = 2.0 * (xy - wz);
    let r02 = 2.0 * (xz + wy);
    let r10 = 2.0 * (xy + wz);
    let r11 = 1.0 - 2.0 * (xx + zz);
    let r12 = 2.0 * (yz - wx);
    let r20 = 2.0 * (xz - wy);
    let r21 = 2.0 * (yz + wx);
    let r22 = 1.0 - 2.0 * (xx + yy);

    // Column-major: column 0 = (sx*r00, sx*r10, sx*r20, 0), etc.
    [
        s[0] * r00, s[0] * r10, s[0] * r20, 0.0,
        s[1] * r01, s[1] * r11, s[1] * r21, 0.0,
        s[2] * r02, s[2] * r12, s[2] * r22, 0.0,
        t[0],       t[1],       t[2],       1.0,
    ]
}

/// Column-major 4x4 multiply: `a * b`. Both operands and the result use the
/// same memory layout glTF stores transforms in.
pub fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0_f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            let mut acc = 0.0;
            for k in 0..4 {
                acc += a[k * 4 + row] * b[col * 4 + k];
            }
            out[col * 4 + row] = acc;
        }
    }
    out
}

/// Walks the node tree from the given scene's roots, returning a parallel
/// vector of world matrices indexed by node index. Nodes not visited by the
/// scene keep an identity entry — convenient for pipelines that index all
/// nodes regardless of scene membership.
pub fn compose_world_matrices(
    scene_roots: &[NodeIndex],
    nodes:       &[Node],
) -> ThinVec<[f32; 16]> {
    let mut out: ThinVec<[f32; 16]> = (0..nodes.len())
        .map(|_| crate::pipeline::IDENTITY_M4)
        .collect();

    let mut stack: ThinVec<(NodeIndex, [f32; 16])> = ThinVec::new();
    for &root in scene_roots {
        stack.push((root, crate::pipeline::IDENTITY_M4));
    }
    while let Some((idx, parent_world)) = stack.pop() {
        let node = &nodes[idx as usize];
        let world = mat4_mul(&parent_world, &node.local_matrix);
        out[idx as usize] = world;
        for &child in &node.children {
            stack.push((child, world));
        }
    }
    out
}
