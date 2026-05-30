//! Experiment 2 — Vertex transform (gizmo move/scale of a selection).
//!
//! Blender transforms edit-mode vertices on the main thread with depsgraph +
//! batch-rebuild overhead, so moving a selection on a high-poly mesh is slow
//! (devtalk.blender.org T57936). The kernel itself should be embarrassingly
//! parallel and vectorizable. This shows AoS `[f32;3]` vs SoA x/y/z codegen.
//!
//! Build: rustc -O -C opt-level=3 -C target-cpu=x86-64-v3 --emit asm --crate-type=lib

// AoS: translate a slice of [f32;3] positions by a delta.
#[no_mangle]
pub fn translate_aos(pos: &mut [[f32; 3]], d: [f32; 3]) {
    for p in pos {
        p[0] += d[0];
        p[1] += d[1];
        p[2] += d[2];
    }
}

// SoA: three flat f32 streams. The x stream (and y, z) is a contiguous run the
// compiler turns into wide FMA/add over many vertices at once.
pub struct PosSoA {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub z: Vec<f32>,
}

#[no_mangle]
pub fn translate_soa(p: &mut PosSoA, d: [f32; 3]) {
    for v in p.x.iter_mut() {
        *v += d[0];
    }
    for v in p.y.iter_mut() {
        *v += d[1];
    }
    for v in p.z.iter_mut() {
        *v += d[2];
    }
}

// SoA affine transform of one axis stream: out = a*x + b*y + c*z + t. This is
// the per-row kernel of a Mat4 * positions; pure FMA, fully vectorizable.
#[no_mangle]
pub fn affine_row_soa(x: &[f32], y: &[f32], z: &[f32], out: &mut [f32], a: f32, b: f32, c: f32, t: f32) {
    let n = out.len();
    for i in 0..n {
        out[i] = a * x[i] + b * y[i] + c * z[i] + t;
    }
}

// Transform only the *selected* vertices (gather by index list). Shows the cost
// of indexed (scatter/gather) access vs the contiguous streams above.
#[no_mangle]
pub fn translate_selected_soa(p: &mut PosSoA, sel: &[u32], d: [f32; 3]) {
    for &i in sel {
        let i = i as usize;
        p.x[i] += d[0];
        p.y[i] += d[1];
        p.z[i] += d[2];
    }
}
