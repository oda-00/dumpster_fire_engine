//! Experiment 5 — Catmull-Clark face-point averaging (subdivision kernel).
//!
//! OpenSubdiv does Catmull-Clark splitting/averaging in a single compute kernel
//! (3 ms for 30k→L2 vs Maya's 100 ms — graphics.pixar.com). The CPU reference
//! kernel is just averaging; on SoA streams it vectorizes, and the same shape
//! ports directly to a GPU compute shader. This shows the averaging codegen.
//!
//! Build: rustc -O -C opt-level=3 -C target-cpu=x86-64-v3 --emit asm --crate-type=lib

// Quad face point = average of the 4 corner positions. SoA corner streams →
// packed adds + one reciprocal multiply.
#[no_mangle]
pub fn quad_face_points(
    ax: &[f32], ay: &[f32], az: &[f32],
    bx: &[f32], by: &[f32], bz: &[f32],
    cx: &[f32], cy: &[f32], cz: &[f32],
    dx: &[f32], dy: &[f32], dz: &[f32],
    fx: &mut [f32], fy: &mut [f32], fz: &mut [f32],
) {
    let n = fx.len();
    const Q: f32 = 0.25;
    for i in 0..n {
        fx[i] = (ax[i] + bx[i] + cx[i] + dx[i]) * Q;
        fy[i] = (ay[i] + by[i] + cy[i] + dy[i]) * Q;
        fz[i] = (az[i] + bz[i] + cz[i] + dz[i]) * Q;
    }
}

// Edge point = average of the 2 edge endpoints and the 2 adjacent face points
// (the Catmull-Clark edge rule). Again pure averaging over flat streams.
#[no_mangle]
pub fn edge_points(
    e0: &[f32], e1: &[f32], f0: &[f32], f1: &[f32], out: &mut [f32],
) {
    let n = out.len();
    for i in 0..n {
        out[i] = (e0[i] + e1[i] + f0[i] + f1[i]) * 0.25;
    }
}
