//! Experiment 3 — Face-normal computation (recomputed on every mesh edit).
//!
//! Normal recompute runs constantly during modeling. AoS vertex fetch (random
//! 12-byte strides via an index buffer) defeats vectorization; an SoA pass over
//! pre-gathered triangle corners is a flat FMA stream. Shows the cross-product
//! codegen and whether it vectorizes.
//!
//! Build: rustc -O -C opt-level=3 -C target-cpu=x86-64-v3 --emit asm --crate-type=lib

#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

// AoS + index buffer: gather 3 vertices per triangle from a [f32;3] array.
#[no_mangle]
pub fn face_normals_indexed(pos: &[[f32; 3]], idx: &[u32], out: &mut [[f32; 3]]) {
    for (t, n) in out.iter_mut().enumerate() {
        let a = pos[idx[t * 3] as usize];
        let b = pos[idx[t * 3 + 1] as usize];
        let c = pos[idx[t * 3 + 2] as usize];
        let e0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        *n = cross(e0, e1);
    }
}

// SoA: triangle corners already laid out as parallel streams (ax,ay,az,bx,...).
// One flat pass; the cross product becomes packed mul/sub across many faces.
pub struct TriSoA {
    pub ax: Vec<f32>, pub ay: Vec<f32>, pub az: Vec<f32>,
    pub bx: Vec<f32>, pub by: Vec<f32>, pub bz: Vec<f32>,
    pub cx: Vec<f32>, pub cy: Vec<f32>, pub cz: Vec<f32>,
}

#[no_mangle]
pub fn face_normals_soa(t: &TriSoA, nx: &mut [f32], ny: &mut [f32], nz: &mut [f32]) {
    let n = nx.len();
    for i in 0..n {
        let e0x = t.bx[i] - t.ax[i];
        let e0y = t.by[i] - t.ay[i];
        let e0z = t.bz[i] - t.az[i];
        let e1x = t.cx[i] - t.ax[i];
        let e1y = t.cy[i] - t.ay[i];
        let e1z = t.cz[i] - t.az[i];
        nx[i] = e0y * e1z - e0z * e1y;
        ny[i] = e0z * e1x - e0x * e1z;
        nz[i] = e0x * e1y - e0y * e1x;
    }
}
