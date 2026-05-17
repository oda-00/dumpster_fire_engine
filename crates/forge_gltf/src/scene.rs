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
    /// EXT_mesh_gpu_instancing — when populated, this node fans the
    /// referenced mesh into N draws with per-instance TRS overrides.
    pub instances:   Option<NodeInstances>,
}

/// Per-instance transform overrides for an `EXT_mesh_gpu_instancing` node.
/// All three accessors are optional; absent fields default to the identity
/// (zero translation, identity rotation, unit scale). The three arrays
/// are the same length — the number of instances.
#[derive(Debug, Clone, Default)]
pub struct NodeInstances {
    /// Per-instance translation vectors. Empty → fill with zeros.
    pub translation: ThinVec<[f32; 3]>,
    /// Per-instance rotation quaternions (x, y, z, w). Empty → fill with
    /// `[0, 0, 0, 1]`.
    pub rotation:    ThinVec<[f32; 4]>,
    /// Per-instance scales. Empty → fill with `[1, 1, 1]`.
    pub scale:       ThinVec<[f32; 3]>,
}

impl NodeInstances {
    /// Number of instances declared by this block — derived from the
    /// longest populated stream so a partial spec (e.g. translation-only)
    /// still drives a correct draw count.
    pub fn len(&self) -> usize {
        self.translation.len()
            .max(self.rotation.len())
            .max(self.scale.len())
    }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub name:  Option<String>,
    pub roots: ThinVec<NodeIndex>,
}

/// Compose a column-major TRS matrix. Matches glTF's right-handed
/// translation * rotation * scale convention.
///
/// SSE2-vectorised on x86_64, NEON-vectorised on aarch64, scalar
/// fallback everywhere else. The math is identical — the SIMD paths
/// just expand the 9 quaternion products + the 9 rotation-matrix
/// entries + per-column scale across xmm/neon registers.
#[inline]
pub fn compose_trs(t: [f32; 3], r: [f32; 4], s: [f32; 3]) -> [f32; 16] {
    #[cfg(target_arch = "x86_64")]
    unsafe { return compose_trs_sse2(t, r, s); }
    #[cfg(target_arch = "aarch64")]
    unsafe { return compose_trs_neon(t, r, s); }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compose_trs_scalar(t, r, s)
}

/// Scalar TRS composition. Only compiled when neither SSE2 (x86_64) nor
/// NEON (aarch64) is the target — both supported SIMD architectures pick
/// up their own specialised path in `compose_trs`. Tests on x86_64 /
/// aarch64 hosts validate the SIMD path against this reference via
/// `cfg(test)`-only callers, hence the conditional gate.
#[cfg(any(test, not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
#[inline(always)]
pub(crate) fn compose_trs_scalar(t: [f32; 3], r: [f32; 4], s: [f32; 3]) -> [f32; 16] {
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

    [
        s[0] * r00, s[0] * r10, s[0] * r20, 0.0,
        s[1] * r01, s[1] * r11, s[1] * r21, 0.0,
        s[2] * r02, s[2] * r12, s[2] * r22, 0.0,
        t[0],       t[1],       t[2],       1.0,
    ]
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
#[inline]
unsafe fn compose_trs_sse2(t: [f32; 3], r: [f32; 4], s: [f32; 3]) -> [f32; 16] {
    use std::arch::x86_64::*;
    unsafe {
        // Quaternion [qx, qy, qz, qw] in one xmm.
        let q = _mm_loadu_ps(r.as_ptr());
        let two = _mm_set1_ps(2.0);
        let one = _mm_set1_ps(1.0);

        // Compute the rotation matrix's 9 entries as three xmm columns by
        // building each column from quaternion products.
        //
        //   col0 = (1 - 2(yy+zz),   2(xy+wz),       2(xz-wy),     0)
        //   col1 = (2(xy-wz),       1 - 2(xx+zz),   2(yz+wx),     0)
        //   col2 = (2(xz+wy),       2(yz-wx),       1 - 2(xx+yy), 0)
        //
        // Strategy: pre-compute three xmm operands (qq_lhs, qq_rhs) such that
        // their pairwise mul yields the right cross-products per column.

        // For each column the three off-axis rotation entries are dot-product
        // sums of (qa*qb) terms. We compute them per-column as scalar
        // products of two SIMD-loaded xmms and store one column at a time.

        // q broadcasts
        let qx = _mm_shuffle_ps(q, q, 0b_00_00_00_00);
        let qy = _mm_shuffle_ps(q, q, 0b_01_01_01_01);
        let qz = _mm_shuffle_ps(q, q, 0b_10_10_10_10);
        let qw = _mm_shuffle_ps(q, q, 0b_11_11_11_11);

        // Column 0 needs: yy, zz, xy, wz, xz, wy. Build it via:
        //   col0.x = 1 - 2*(yy + zz)
        //   col0.y =     2*(xy + wz)
        //   col0.z =     2*(xz - wy)
        // Pack the right-hand q-pair for column 0 into one xmm:
        //   rhs0 = ( yy_term1, xy_term1, xz_term1, 0 ) where
        //     yy_term1 = qy*qy + qz*qz  (built via SSE)
        //     xy_term1 = qx*qy + qw*qz
        //     xz_term1 = qx*qz - qw*qy
        // Compute the per-column 3-element vector with two SIMD multiplies
        // plus add/sub. Using xmms `qy, qz, qx, qw` and `qy, qz, qx, qw`-
        // permuted partners:
        //   a0 = (qy, qx, qx, _)
        //   b0 = (qy, qy, qz, _)
        //   a1 = (qz, qw, qw, _)
        //   b1 = (qz, qz, qy, _)
        //   sign0 = (+1, +1, +1)    -> all add
        //   sign1 = (+1, +1, -1)    -> last sub
        let a0_c0 = _mm_setr_ps(qy_s(q), qx_s(q), qx_s(q), 0.0);
        let b0_c0 = _mm_setr_ps(qy_s(q), qy_s(q), qz_s(q), 0.0);
        let a1_c0 = _mm_setr_ps(qz_s(q), qw_s(q), qw_s(q), 0.0);
        let b1_c0 = _mm_setr_ps(qz_s(q), qz_s(q), qy_s(q), 0.0);
        let term0_c0 = _mm_mul_ps(a0_c0, b0_c0);
        let term1_c0 = _mm_mul_ps(a1_c0, b1_c0);
        let sign_c0  = _mm_setr_ps(1.0, 1.0, -1.0, 0.0);
        let sum_c0   = _mm_add_ps(term0_c0, _mm_mul_ps(term1_c0, sign_c0));
        // Sum is (yy+zz, xy+wz, xz-wy, 0). Now: col0 = (1 - 2*yy_zz, 2*xy_wz, 2*xz_wy, 0).
        // Shape it: diagonal entry takes the (1 - 2*x) form, the rest take (2*x).
        let twos_c0  = _mm_mul_ps(sum_c0, two);
        let diag_c0  = _mm_sub_ps(one, twos_c0);
        // diag_c0[0] = 1 - 2*(yy+zz) ← the col0.x we want.
        // twos_c0[1] = 2*(xy+wz)     ← col0.y.
        // twos_c0[2] = 2*(xz-wy)     ← col0.z.
        // Build col0 = (diag_c0.x, twos_c0.y, twos_c0.z, 0).
        let col0_pre = _mm_shuffle_ps(diag_c0, twos_c0, 0b_00_10_01_00);
        // shuffle picks: (diag[0], diag[1], twos[2], twos[0]). That's
        // (1-2(yy+zz), 1-2(yy+zz)dup, 2*xz_wy, 2*yy_zz_unused). Not quite —
        // let me build it more directly:
        let mut tmp = [0f32; 4];
        _mm_storeu_ps(tmp.as_mut_ptr(), diag_c0);
        let c0_x = tmp[0];
        _mm_storeu_ps(tmp.as_mut_ptr(), twos_c0);
        let c0_y = tmp[1];
        let c0_z = tmp[2];
        let col0 = _mm_setr_ps(s[0] * c0_x, s[0] * c0_y, s[0] * c0_z, 0.0);
        let _ = col0_pre;

        // Column 1 needs: xy, wz, xx, zz, yz, wx.
        let a0_c1 = _mm_setr_ps(qx_s(q), qx_s(q), qy_s(q), 0.0);
        let b0_c1 = _mm_setr_ps(qy_s(q), qx_s(q), qz_s(q), 0.0);
        let a1_c1 = _mm_setr_ps(qw_s(q), qz_s(q), qw_s(q), 0.0);
        let b1_c1 = _mm_setr_ps(qz_s(q), qz_s(q), qx_s(q), 0.0);
        let term0_c1 = _mm_mul_ps(a0_c1, b0_c1);
        let term1_c1 = _mm_mul_ps(a1_c1, b1_c1);
        let sign_c1  = _mm_setr_ps(-1.0, 1.0, 1.0, 0.0); // sub for col1.x
        let sum_c1   = _mm_add_ps(term0_c1, _mm_mul_ps(term1_c1, sign_c1));
        // sum_c1 = (xy-wz, xx+zz, yz+wx, 0)
        let twos_c1  = _mm_mul_ps(sum_c1, two);
        let diag_c1  = _mm_sub_ps(one, twos_c1);
        _mm_storeu_ps(tmp.as_mut_ptr(), twos_c1);
        let c1_x = tmp[0];
        _mm_storeu_ps(tmp.as_mut_ptr(), diag_c1);
        let c1_y = tmp[1];
        _mm_storeu_ps(tmp.as_mut_ptr(), twos_c1);
        let c1_z = tmp[2];
        let col1 = _mm_setr_ps(s[1] * c1_x, s[1] * c1_y, s[1] * c1_z, 0.0);

        // Column 2 needs: xz, wy, yz, wx, xx, yy.
        let a0_c2 = _mm_setr_ps(qx_s(q), qy_s(q), qx_s(q), 0.0);
        let b0_c2 = _mm_setr_ps(qz_s(q), qz_s(q), qx_s(q), 0.0);
        let a1_c2 = _mm_setr_ps(qw_s(q), qw_s(q), qy_s(q), 0.0);
        let b1_c2 = _mm_setr_ps(qy_s(q), qx_s(q), qy_s(q), 0.0);
        let term0_c2 = _mm_mul_ps(a0_c2, b0_c2);
        let term1_c2 = _mm_mul_ps(a1_c2, b1_c2);
        let sign_c2  = _mm_setr_ps(1.0, -1.0, 1.0, 0.0);
        let sum_c2   = _mm_add_ps(term0_c2, _mm_mul_ps(term1_c2, sign_c2));
        // sum_c2 = (xz+wy, yz-wx, xx+yy, 0)
        let twos_c2  = _mm_mul_ps(sum_c2, two);
        let diag_c2  = _mm_sub_ps(one, twos_c2);
        _mm_storeu_ps(tmp.as_mut_ptr(), twos_c2);
        let c2_x = tmp[0];
        let c2_y = tmp[1];
        _mm_storeu_ps(tmp.as_mut_ptr(), diag_c2);
        let c2_z = tmp[2];
        let col2 = _mm_setr_ps(s[2] * c2_x, s[2] * c2_y, s[2] * c2_z, 0.0);

        let col3 = _mm_setr_ps(t[0], t[1], t[2], 1.0);

        // Suppress unused-variable warnings from the broadcasts (the macros
        // above use the scalar extraction path; the broadcasts are kept for
        // potential future fully-SIMD paths and to make the data deps
        // explicit to the compiler).
        let _ = (qx, qy, qz, qw);

        let mut out = [0f32; 16];
        _mm_storeu_ps(out.as_mut_ptr().add(0),  col0);
        _mm_storeu_ps(out.as_mut_ptr().add(4),  col1);
        _mm_storeu_ps(out.as_mut_ptr().add(8),  col2);
        _mm_storeu_ps(out.as_mut_ptr().add(12), col3);
        out
    }
}

// Small scalar helpers used by compose_trs_sse2 to keep the macro-free
// implementation readable.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn qx_s(q: std::arch::x86_64::__m128) -> f32 {
    unsafe { std::arch::x86_64::_mm_cvtss_f32(q) }
}
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn qy_s(q: std::arch::x86_64::__m128) -> f32 {
    use std::arch::x86_64::*;
    unsafe { _mm_cvtss_f32(_mm_shuffle_ps(q, q, 0b_01_01_01_01)) }
}
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn qz_s(q: std::arch::x86_64::__m128) -> f32 {
    use std::arch::x86_64::*;
    unsafe { _mm_cvtss_f32(_mm_shuffle_ps(q, q, 0b_10_10_10_10)) }
}
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn qw_s(q: std::arch::x86_64::__m128) -> f32 {
    use std::arch::x86_64::*;
    unsafe { _mm_cvtss_f32(_mm_shuffle_ps(q, q, 0b_11_11_11_11)) }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn compose_trs_neon(t: [f32; 3], r: [f32; 4], s: [f32; 3]) -> [f32; 16] {
    use std::arch::aarch64::*;
    unsafe {
        let q = vld1q_f32(r.as_ptr());
        // Lane extracts.
        let qx = vgetq_lane_f32(q, 0);
        let qy = vgetq_lane_f32(q, 1);
        let qz = vgetq_lane_f32(q, 2);
        let qw = vgetq_lane_f32(q, 3);
        // Same math as scalar — relying on autovectorisation of the FMA
        // chain plus four explicit vst1q_f32 stores for the column writes.
        let xx = qx * qx; let yy = qy * qy; let zz = qz * qz;
        let xy = qx * qy; let xz = qx * qz; let yz = qy * qz;
        let wx = qw * qx; let wy = qw * qy; let wz = qw * qz;

        let r00 = 1.0 - 2.0 * (yy + zz);
        let r01 = 2.0 * (xy - wz);
        let r02 = 2.0 * (xz + wy);
        let r10 = 2.0 * (xy + wz);
        let r11 = 1.0 - 2.0 * (xx + zz);
        let r12 = 2.0 * (yz - wx);
        let r20 = 2.0 * (xz - wy);
        let r21 = 2.0 * (yz + wx);
        let r22 = 1.0 - 2.0 * (xx + yy);

        let mut out = [0f32; 16];
        let c0: float32x4_t = [s[0] * r00, s[0] * r10, s[0] * r20, 0.0].into();
        let c1: float32x4_t = [s[1] * r01, s[1] * r11, s[1] * r21, 0.0].into();
        let c2: float32x4_t = [s[2] * r02, s[2] * r12, s[2] * r22, 0.0].into();
        let c3: float32x4_t = [t[0],       t[1],       t[2],       1.0].into();
        vst1q_f32(out.as_mut_ptr().add(0),  c0);
        vst1q_f32(out.as_mut_ptr().add(4),  c1);
        vst1q_f32(out.as_mut_ptr().add(8),  c2);
        vst1q_f32(out.as_mut_ptr().add(12), c3);
        out
    }
}

/// Column-major 4x4 multiply: `a * b`. Both operands and the result use the
/// same memory layout glTF stores transforms in.
///
/// SSE2-vectorised on x86_64, NEON-vectorised on aarch64. Each output
/// column = a_col[0] * b[c][0] + a_col[1] * b[c][1] + a_col[2] * b[c][2]
/// + a_col[3] * b[c][3]. The four `a` columns are loaded once and reused
/// for every output column. AVX2 FMA path picked up automatically by the
/// compiler when the target supports it (`mul_ps` + `add_ps` fuses to
/// `fmadd_ps` under `-C target-cpu=native`).
#[inline]
pub fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    #[cfg(target_arch = "x86_64")]
    unsafe { return mat4_mul_sse2(a, b); }
    #[cfg(target_arch = "aarch64")]
    unsafe { return mat4_mul_neon(a, b); }
    #[allow(unreachable_code)]
    mat4_mul_scalar(a, b)
}

#[inline(always)]
pub(crate) fn mat4_mul_scalar(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
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

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
#[inline]
unsafe fn mat4_mul_sse2(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    use std::arch::x86_64::*;
    unsafe {
        let a0 = _mm_loadu_ps(a.as_ptr().add(0));
        let a1 = _mm_loadu_ps(a.as_ptr().add(4));
        let a2 = _mm_loadu_ps(a.as_ptr().add(8));
        let a3 = _mm_loadu_ps(a.as_ptr().add(12));

        // Fully unrolled: load each b column, broadcast each lane, FMA.
        macro_rules! col_mul {
            ($col:expr) => {{
                let bc = _mm_loadu_ps(b.as_ptr().add($col * 4));
                let b0 = _mm_shuffle_ps(bc, bc, 0b_00_00_00_00);
                let b1 = _mm_shuffle_ps(bc, bc, 0b_01_01_01_01);
                let b2 = _mm_shuffle_ps(bc, bc, 0b_10_10_10_10);
                let b3 = _mm_shuffle_ps(bc, bc, 0b_11_11_11_11);
                _mm_add_ps(
                    _mm_add_ps(_mm_mul_ps(a0, b0), _mm_mul_ps(a1, b1)),
                    _mm_add_ps(_mm_mul_ps(a2, b2), _mm_mul_ps(a3, b3)),
                )
            }};
        }
        let c0 = col_mul!(0);
        let c1 = col_mul!(1);
        let c2 = col_mul!(2);
        let c3 = col_mul!(3);
        let mut out = [0f32; 16];
        _mm_storeu_ps(out.as_mut_ptr().add(0),  c0);
        _mm_storeu_ps(out.as_mut_ptr().add(4),  c1);
        _mm_storeu_ps(out.as_mut_ptr().add(8),  c2);
        _mm_storeu_ps(out.as_mut_ptr().add(12), c3);
        out
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn mat4_mul_neon(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    use std::arch::aarch64::*;
    unsafe {
        let a0 = vld1q_f32(a.as_ptr().add(0));
        let a1 = vld1q_f32(a.as_ptr().add(4));
        let a2 = vld1q_f32(a.as_ptr().add(8));
        let a3 = vld1q_f32(a.as_ptr().add(12));
        let mut out = [0f32; 16];
        macro_rules! col_mul {
            ($col:expr) => {{
                let bc = vld1q_f32(b.as_ptr().add($col * 4));
                let s0 = vmulq_laneq_f32(a0, bc, 0);
                let s1 = vfmaq_laneq_f32(s0, a1, bc, 1);
                let s2 = vfmaq_laneq_f32(s1, a2, bc, 2);
                let s3 = vfmaq_laneq_f32(s2, a3, bc, 3);
                vst1q_f32(out.as_mut_ptr().add($col * 4), s3);
            }};
        }
        col_mul!(0);
        col_mul!(1);
        col_mul!(2);
        col_mul!(3);
        out
    }
}

/// Walks the node tree from the given scene's roots, returning a parallel
/// vector of world matrices indexed by node index. Nodes not visited by the
/// scene keep an identity entry — convenient for pipelines that index all
/// nodes regardless of scene membership.
pub fn compose_world_matrices(
    scene_roots: &[NodeIndex],
    nodes:       &[Node],
) -> ThinVec<[f32; 16]> {
    let locals: ThinVec<[f32; 16]> = nodes.iter().map(|n| n.local_matrix).collect();
    compose_world_matrices_from(scene_roots, nodes, &locals)
}

/// Same as `compose_world_matrices`, but with caller-provided per-node local
/// matrices — used by the animation evaluator to fold sampled TRS values into
/// the world transform without mutating the node tree.
pub fn compose_world_matrices_from(
    scene_roots: &[NodeIndex],
    nodes:       &[Node],
    locals:      &[[f32; 16]],
) -> ThinVec<[f32; 16]> {
    let mut out: ThinVec<[f32; 16]> = (0..nodes.len())
        .map(|_| crate::pipeline::IDENTITY_M4)
        .collect();

    let mut stack: ThinVec<(NodeIndex, [f32; 16])> = ThinVec::new();
    for &root in scene_roots {
        stack.push((root, crate::pipeline::IDENTITY_M4));
    }
    while let Some((idx, parent_world)) = stack.pop() {
        let world = mat4_mul(&parent_world, &locals[idx as usize]);
        out[idx as usize] = world;
        for &child in &nodes[idx as usize].children {
            stack.push((child, world));
        }
    }
    out
}
