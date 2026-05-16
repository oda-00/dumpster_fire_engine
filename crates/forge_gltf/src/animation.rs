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
