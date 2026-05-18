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

/// KHR_animation_pointer — replaces a channel's `target.node` with a JSON
/// pointer like `/materials/0/pbrMetallicRoughness/baseColorFactor`. We
/// keep the original pointer string verbatim; the engine routes it to the
/// right uniform.
#[derive(Debug, Clone)]
pub struct AnimPointerChannel {
    pub pointer: String,
    pub sampler: u32,
}

#[derive(Debug, Clone)]
pub struct Animation {
    pub name:             Option<String>,
    pub samplers:         ThinVec<AnimSampler>,
    pub channels:         ThinVec<AnimChannel>,
    /// KHR_animation_pointer channels (sibling to `channels`).
    pub pointer_channels: ThinVec<AnimPointerChannel>,
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

/// Sample one element of a multi-scalar sampler. `stride` is the number of
/// scalars per keyframe (e.g. morph weights pack N scalars per frame); `k`
/// is the index within the keyframe. Used by morph-target weight tracks
/// and by KHR_animation_pointer scalar tracks.
pub fn sample_scalar(s: &AnimSampler, t: f32, stride: usize, k: usize) -> f32 {
    let SamplerOutput::Scalars(out) = &s.output else {
        // Caller passed a vec-shaped sampler; degrade by reading element k
        // of whatever vector form is available.
        return match &s.output {
            SamplerOutput::Vec3(_) => {
                let v = sample_vec3(s, t);
                v.get(k).copied().unwrap_or(0.0)
            }
            SamplerOutput::Vec4(_) => {
                let v = sample_quat(s, t);
                v.get(k).copied().unwrap_or(0.0)
            }
            _ => 0.0,
        };
    };
    if out.is_empty() || stride == 0 { return 0.0; }
    let frames = out.len() / stride;
    if frames == 0 { return 0.0; }

    // Slice `out` as one element per "keyframe-slot". `locate_segment`
    // operates on the input timeline (always one entry per keyframe).
    let (lo, hi, u) = locate_segment(&s.input, t);
    let pick = |frame: usize, slot: usize| -> f32 {
        let i = frame * stride + slot;
        out.get(i).copied().unwrap_or(0.0)
    };
    match s.interpolation {
        Interpolation::Step   => pick(lo, k),
        Interpolation::Linear => {
            let a = pick(lo, k);
            let b = pick(hi, k);
            a + (b - a) * u
        }
        Interpolation::CubicSpline => {
            // Layout is [in_tan_0, value_0, out_tan_0, in_tan_1, value_1, ...]
            // — three samples per keyframe, each of `stride` scalars.
            let dt = (s.input[hi] - s.input[lo]).max(1e-12);
            let p0 = pick(lo * 3 + 1, k);
            let p1 = pick(hi * 3 + 1, k);
            let m0 = pick(lo * 3 + 2, k);
            let m1 = pick(hi * 3, k);
            hermite(p0, m0 * dt, p1, m1 * dt, u)
        }
    }
}

/// Public spherical-linear-interpolation entry point — used by the pose
/// blender to mix the rest pose with a second animation track.
pub fn slerp_quat_pub(a: [f32; 4], b: [f32; 4], u: f32) -> [f32; 4] {
    slerp_quat(a, b, u)
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


#[cfg(test)]
mod tests {
    use super::*;

    /// Per spec §C.4 slerp must take the SHORT arc when `dot(a, b) < 0`
    /// — i.e. negate `b` before interpolating so the resulting quaternion
    /// sweeps less than 180°. Without this, animations flip 360° around.
    #[test]
    fn slerp_takes_short_path_when_dot_negative() {
        // a = identity quaternion. b = ~135° around X (positive form).
        let a = [0.0_f32, 0.0, 0.0, 1.0];
        let theta = 135.0_f32.to_radians();
        let b_pos = [(theta * 0.5).sin(), 0.0, 0.0, (theta * 0.5).cos()];
        let b_neg = [-b_pos[0], -b_pos[1], -b_pos[2], -b_pos[3]];

        // Mid-way through the slerp — the result should be the same
        // regardless of whether b is given as `b_pos` (positive form) or
        // `b_neg` (long-arc form). Equality is exact modulo sign because
        // q and -q represent the same rotation.
        let mid_pos = slerp_quat(a, b_pos, 0.5);
        let mid_neg = slerp_quat(a, b_neg, 0.5);
        let same_or_negated = mid_pos
            .iter()
            .zip(mid_neg.iter())
            .all(|(p, n)| (p - n).abs() < 1e-5)
        || mid_pos
            .iter()
            .zip(mid_neg.iter())
            .all(|(p, n)| (p + n).abs() < 1e-5);
        assert!(same_or_negated, "slerp short-path violated: pos={:?} neg={:?}", mid_pos, mid_neg);
    }

    /// CubicSpline rotation tangents that produce a non-unit quaternion
    /// post-hermite MUST be re-normalised — per spec §3.11 the rotation
    /// stream is normalised after interpolation so the downstream matrix
    /// composition stays orthogonal.
    #[test]
    fn cubicspline_quat_renormalises() {
        // Build a 2-keyframe sampler with deliberately large tangents so
        // the un-normalised hermite output is far from unit length.
        let sampler = AnimSampler {
            input: thin_vec::ThinVec::from(&[0.0_f32, 1.0][..]),
            output: SamplerOutput::Vec4(thin_vec::ThinVec::from(&[
                [0.0_f32, 0.0, 0.0, 0.0],  // in_tan_0
                [0.0, 0.0, 0.0, 1.0],      // value_0   (identity)
                [5.0, 5.0, 5.0, 5.0],      // out_tan_0 (huge)
                [-5.0, -5.0, -5.0, -5.0],  // in_tan_1  (huge)
                [1.0, 0.0, 0.0, 0.0],      // value_1   (180° around X)
                [0.0, 0.0, 0.0, 0.0],      // out_tan_1
            ][..])),
            interpolation: Interpolation::CubicSpline,
        };
        let q = sample_quat(&sampler, 0.5);
        let len2 = q[0]*q[0] + q[1]*q[1] + q[2]*q[2] + q[3]*q[3];
        assert!((len2 - 1.0).abs() < 1e-4, "post-CubicSpline quat not unit: |q|²={len2}");
    }
}
