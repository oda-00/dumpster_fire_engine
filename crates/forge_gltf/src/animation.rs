//! Animation channels & samplers.
//!
//! Storage is keyframe-faithful: each sampler keeps its raw input (timestamps)
//! and output (per-component values) verbatim. CUBICSPLINE outputs include
//! the in/out tangents inline, three samples per keyframe — consumers can
//! pick them apart on demand.

use thin_vec::ThinVec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Interpolation { Step, Linear, CubicSpline }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AnimPath { Translation, Rotation, Scale, MorphWeights }

#[derive(Debug, Clone)]
pub enum SamplerOutput {
    /// 3 floats per keyframe (translation, scale).
    Vec3(ThinVec<[f32; 3]>),
    /// 4 floats per keyframe (rotation quaternion, xyzw).
    Vec4(ThinVec<[f32; 4]>),
    /// N floats per keyframe (morph-target weights; N = morph_target_count).
    Scalars(ThinVec<f32>),
}

#[derive(Debug, Clone)]
pub struct AnimSampler {
    pub interpolation: Interpolation,
    pub input:         ThinVec<f32>,   // keyframe times, seconds
    pub output:        SamplerOutput,
}

#[derive(Debug, Clone)]
pub struct AnimChannel {
    pub target_node: u32,
    pub target_path: AnimPath,
    pub sampler:     u32,
}

#[derive(Debug, Clone)]
pub struct Animation {
    pub name:     Option<String>,
    pub samplers: ThinVec<AnimSampler>,
    pub channels: ThinVec<AnimChannel>,
}

impl Animation {
    pub fn duration(&self) -> f32 {
        self.samplers
            .iter()
            .map(|s| s.input.last().copied().unwrap_or(0.0))
            .fold(0.0_f32, f32::max)
    }
}

// ── Sampler evaluation ──────────────────────────────────────────────────────
//
// Each `AnimSampler` is a piecewise function of time. The helpers below take
// a sampler + a time `t` and produce the interpolated output sample. STEP
// snaps to the previous keyframe; LINEAR (or SLERP for rotations) blends
// between neighbouring keyframes; CUBICSPLINE uses Hermite interpolation
// over (in-tangent, value, out-tangent) triplets.

fn locate_segment(times: &[f32], t: f32) -> (usize, usize, f32) {
    if times.is_empty() { return (0, 0, 0.0); }
    if t <= times[0]    { return (0, 0, 0.0); }
    let last = times.len() - 1;
    if t >= times[last] { return (last, last, 0.0); }
    let mut lo = 0;
    let mut hi = last;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if times[mid] <= t { lo = mid; } else { hi = mid; }
    }
    let span = (times[hi] - times[lo]).max(1e-12);
    (lo, hi, (t - times[lo]) / span)
}

fn lerp_vec3(a: [f32; 3], b: [f32; 3], u: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * u,
        a[1] + (b[1] - a[1]) * u,
        a[2] + (b[2] - a[2]) * u,
    ]
}

fn slerp_quat(a: [f32; 4], b: [f32; 4], u: f32) -> [f32; 4] {
    let mut b = b;
    let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if dot < 0.0 {
        b = [-b[0], -b[1], -b[2], -b[3]];
        dot = -dot;
    }
    // Fall back to lerp for nearly-parallel quaternions (avoids div by zero).
    if dot > 0.9995 {
        let mut out = [
            a[0] + (b[0] - a[0]) * u,
            a[1] + (b[1] - a[1]) * u,
            a[2] + (b[2] - a[2]) * u,
            a[3] + (b[3] - a[3]) * u,
        ];
        let len = (out[0]*out[0] + out[1]*out[1] + out[2]*out[2] + out[3]*out[3]).sqrt().max(1e-12);
        out[0] /= len; out[1] /= len; out[2] /= len; out[3] /= len;
        return out;
    }
    let theta_0 = dot.clamp(-1.0, 1.0).acos();
    let theta   = theta_0 * u;
    let sin_t0  = theta_0.sin().max(1e-12);
    let s1 = ((1.0 - u) * theta_0).sin() / sin_t0;
    let s2 = theta.sin() / sin_t0;
    [
        a[0] * s1 + b[0] * s2,
        a[1] * s1 + b[1] * s2,
        a[2] * s1 + b[2] * s2,
        a[3] * s1 + b[3] * s2,
    ]
}

fn hermite(p0: f32, m0: f32, p1: f32, m1: f32, u: f32) -> f32 {
    let u2 = u * u;
    let u3 = u2 * u;
    (2.0 * u3 - 3.0 * u2 + 1.0) * p0
        + (u3 - 2.0 * u2 + u) * m0
        + (-2.0 * u3 + 3.0 * u2) * p1
        + (u3 - u2) * m1
}

pub fn sample_vec3(s: &AnimSampler, t: f32) -> [f32; 3] {
    let SamplerOutput::Vec3(out) = &s.output else { return [0.0; 3]; };
    if out.is_empty() { return [0.0; 3]; }
    let (lo, hi, u) = locate_segment(&s.input, t);
    match s.interpolation {
        Interpolation::Step   => out[lo],
        Interpolation::Linear => lerp_vec3(out[lo], out[hi], u),
        Interpolation::CubicSpline => {
            // Output is [in_tan_0, value_0, out_tan_0, in_tan_1, value_1, ...]
            let p0 = out[lo * 3 + 1];
            let p1 = out[hi * 3 + 1];
            let m0 = out[lo * 3 + 2];
            let m1 = out[hi * 3];
            let dt = (s.input[hi] - s.input[lo]).max(1e-12);
            [
                hermite(p0[0], m0[0] * dt, p1[0], m1[0] * dt, u),
                hermite(p0[1], m0[1] * dt, p1[1], m1[1] * dt, u),
                hermite(p0[2], m0[2] * dt, p1[2], m1[2] * dt, u),
            ]
        }
    }
}

pub fn sample_quat(s: &AnimSampler, t: f32) -> [f32; 4] {
    let SamplerOutput::Vec4(out) = &s.output else { return [0.0, 0.0, 0.0, 1.0]; };
    if out.is_empty() { return [0.0, 0.0, 0.0, 1.0]; }
    let (lo, hi, u) = locate_segment(&s.input, t);
    match s.interpolation {
        Interpolation::Step   => out[lo],
        Interpolation::Linear => slerp_quat(out[lo], out[hi], u),
        Interpolation::CubicSpline => {
            let p0 = out[lo * 3 + 1];
            let p1 = out[hi * 3 + 1];
            let m0 = out[lo * 3 + 2];
            let m1 = out[hi * 3];
            let dt = (s.input[hi] - s.input[lo]).max(1e-12);
            let mut q = [
                hermite(p0[0], m0[0] * dt, p1[0], m1[0] * dt, u),
                hermite(p0[1], m0[1] * dt, p1[1], m1[1] * dt, u),
                hermite(p0[2], m0[2] * dt, p1[2], m1[2] * dt, u),
                hermite(p0[3], m0[3] * dt, p1[3], m1[3] * dt, u),
            ];
            let len = (q[0]*q[0] + q[1]*q[1] + q[2]*q[2] + q[3]*q[3]).sqrt().max(1e-12);
            q[0] /= len; q[1] /= len; q[2] /= len; q[3] /= len;
            q
        }
    }
}

