//! Animation evaluation — `Pose` is the per-frame state every pipeline reads.
//!
//! A `Pose` holds, for each node in the asset, the sampled TRS values plus
//! the resulting local 4x4 matrix. World matrices are computed on top of
//! that local set by walking the scene tree, so animated nodes ripple
//! through their descendants for free.
//!
//! The evaluator is allocation-light: callers re-use a single `Pose` across
//! frames and call `Pose::sample` to overwrite it in place.

use thin_vec::ThinVec;

use crate::animation::{AnimPath, Animation, sample_quat, sample_vec3};
use crate::asset::GltfAsset;
use crate::scene::{compose_trs, compose_world_matrices_from};

#[derive(Debug, Clone)]
pub struct Pose {
    pub translation: ThinVec<[f32; 3]>,
    pub rotation:    ThinVec<[f32; 4]>,
    pub scale:       ThinVec<[f32; 3]>,
    pub local:       ThinVec<[f32; 16]>,
    pub world:       ThinVec<[f32; 16]>,
}

impl Pose {
    /// Initialise a pose from the asset's static (rest) TRS values. Calling
    /// `sample` later overwrites the entries that have animation channels.
    pub fn rest(asset: &GltfAsset) -> Self {
        let n = asset.nodes.len();
        let mut translation = ThinVec::with_capacity(n);
        let mut rotation    = ThinVec::with_capacity(n);
        let mut scale       = ThinVec::with_capacity(n);
        let mut local       = ThinVec::with_capacity(n);
        for node in &asset.nodes {
            translation.push(node.translation);
            rotation.push(node.rotation);
            scale.push(node.scale);
            local.push(node.local_matrix);
        }
        let world = match asset.primary_scene() {
            Some(scene) => compose_world_matrices_from(&scene.roots, &asset.nodes, &local),
            None        => (0..n).map(|_| crate::pipeline::IDENTITY_M4).collect(),
        };
        Self { translation, rotation, scale, local, world }
    }

    /// Sample one animation at time `t` seconds and refresh the pose. Times
    /// outside the animation's range clamp to the first/last keyframe.
    pub fn sample(&mut self, asset: &GltfAsset, anim: &Animation, t: f32) {
        for ch in &anim.channels {
            let sampler = &anim.samplers[ch.sampler as usize];
            let idx = ch.target_node as usize;
            if idx >= self.translation.len() { continue; }
            match ch.target_path {
                AnimPath::Translation => self.translation[idx] = sample_vec3(sampler, t),
                AnimPath::Rotation    => self.rotation[idx]    = sample_quat(sampler, t),
                AnimPath::Scale       => self.scale[idx]       = sample_vec3(sampler, t),
                AnimPath::MorphWeights => { /* fed through `MorphTarget.weights` separately */ }
            }
            self.local[idx] = compose_trs(self.translation[idx], self.rotation[idx], self.scale[idx]);
        }
        if let Some(scene) = asset.primary_scene() {
            self.world = compose_world_matrices_from(&scene.roots, &asset.nodes, &self.local);
        }
    }

    /// Sample the asset's first animation, if any.
    pub fn sample_default(&mut self, asset: &GltfAsset, t: f32) -> bool {
        if let Some(anim) = asset.animations.first() {
            self.sample(asset, anim, t);
            true
        } else {
            false
        }
    }
}
