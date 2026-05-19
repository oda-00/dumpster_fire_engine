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

use crate::animation::{
    AnimPath, Animation,
    sample_quat, sample_scalar, sample_vec3,
    sample_quat_hinted, sample_scalar_hinted, sample_vec3_hinted,
};
use crate::asset::GltfAsset;
use crate::material::{Material, TextureRef};
use crate::scene::{compose_trs, compose_world_matrices_from, mat4_mul};

#[derive(Debug, Clone)]
pub struct Pose {
    pub translation: ThinVec<[f32; 3]>,
    pub rotation:    ThinVec<[f32; 4]>,
    pub scale:       ThinVec<[f32; 3]>,
    pub local:       ThinVec<[f32; 16]>,
    pub world:       ThinVec<[f32; 16]>,
    /// Per-node morph-target weight overrides. Empty `ThinVec` = "use the
    /// owning mesh's default weights".
    pub morph_weights: ThinVec<ThinVec<f32>>,
    /// Per-channel monotonic-time hint into the channel's input timeline.
    /// Sized lazily inside `sample()` to `anim.channels.len()`; reused
    /// across frames so forward-running animation walks O(1) per channel
    /// instead of O(log N) binary search. See `locate_segment_hinted`.
    pub last_segments: ThinVec<u32>,
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
        let mut morph_weights = ThinVec::with_capacity(n);
        for node in &asset.nodes {
            translation.push(node.translation);
            rotation.push(node.rotation);
            scale.push(node.scale);
            local.push(node.local_matrix);
            morph_weights.push(node.weights.clone());
        }
        let world = match asset.primary_scene() {
            Some(scene) => compose_world_matrices_from(&scene.roots, &asset.nodes, &local),
            None        => (0..n).map(|_| crate::pipeline::IDENTITY_M4).collect(),
        };
        Self { translation, rotation, scale, local, world, morph_weights, last_segments: ThinVec::new() }
    }

    /// Sample one animation at time `t` seconds and refresh the pose. Times
    /// outside the animation's range clamp to the first/last keyframe.
    pub fn sample(&mut self, asset: &GltfAsset, anim: &Animation, t: f32) {
        // Resize the per-channel hint cache to match this animation. We
        // overwrite all hints if the cache size differs (anim swap or
        // first call); else reuse from last frame for O(1) forward step.
        if self.last_segments.len() != anim.channels.len() {
            self.last_segments.clear();
            self.last_segments.resize(anim.channels.len(), 0u32);
        }
        for (ch_i, ch) in anim.channels.iter().enumerate() {
            let sampler = &anim.samplers[ch.sampler as usize];
            let idx = ch.target_node as usize;
            if idx >= self.translation.len() { continue; }
            let hint = &mut self.last_segments[ch_i];
            match ch.target_path {
                AnimPath::Translation => self.translation[idx] = sample_vec3_hinted(sampler, t, hint),
                AnimPath::Rotation    => self.rotation[idx]    = sample_quat_hinted(sampler, t, hint),
                AnimPath::Scale       => self.scale[idx]       = sample_vec3_hinted(sampler, t, hint),
                AnimPath::MorphWeights => {
                    let stride = self.morph_weights[idx].len().max(1);
                    let mut out = ThinVec::with_capacity(stride);
                    for w_idx in 0..stride {
                        out.push(sample_scalar_hinted(sampler, t, stride, w_idx, hint));
                    }
                    self.morph_weights[idx] = out;
                }
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

    /// Blend two animations together at the same wall-clock time. `weight`
    /// is the contribution of `b`; `0.0` = pure `a`, `1.0` = pure `b`.
    /// Translations and scales lerp; rotations slerp; morph weights lerp.
    pub fn sample_blended(
        &mut self,
        asset: &GltfAsset,
        a: &Animation, b: &Animation,
        ta: f32, tb: f32,
        weight: f32,
    ) {
        // Sample `a` into the pose, then blend each channel from `b` on top.
        self.sample(asset, a, ta);
        let w = weight.clamp(0.0, 1.0);
        for ch in &b.channels {
            let sampler = &b.samplers[ch.sampler as usize];
            let idx = ch.target_node as usize;
            if idx >= self.translation.len() { continue; }
            match ch.target_path {
                AnimPath::Translation => {
                    let v = sample_vec3(sampler, tb);
                    self.translation[idx] = lerp3(self.translation[idx], v, w);
                }
                AnimPath::Rotation => {
                    let q = sample_quat(sampler, tb);
                    self.rotation[idx] = crate::animation::slerp_quat_pub(self.rotation[idx], q, w);
                }
                AnimPath::Scale => {
                    let v = sample_vec3(sampler, tb);
                    self.scale[idx] = lerp3(self.scale[idx], v, w);
                }
                AnimPath::MorphWeights => {
                    let stride = self.morph_weights[idx].len().max(1);
                    for k in 0..stride {
                        let s = sample_scalar(sampler, tb, stride, k);
                        if let Some(slot) = self.morph_weights[idx].get_mut(k) {
                            *slot = *slot + (s - *slot) * w;
                        }
                    }
                }
            }
            self.local[idx] = compose_trs(self.translation[idx], self.rotation[idx], self.scale[idx]);
        }
        if let Some(scene) = asset.primary_scene() {
            self.world = compose_world_matrices_from(&scene.roots, &asset.nodes, &self.local);
        }
    }

    /// Per-joint skinning matrices for one skin: each entry is
    /// `world(joint) × inverse_bind`. Mesh nodes that point to this skin
    /// upload the result as a storage buffer; the vertex shader indexes it
    /// by `JOINTS_0[v]` with weights `WEIGHTS_0[v]` to deform the mesh.
    pub fn skin_palette(&self, asset: &GltfAsset, skin_idx: u32) -> ThinVec<[f32; 16]> {
        let skin = &asset.skins[skin_idx as usize];
        let n = skin.joints.len();
        let mut out = ThinVec::with_capacity(n);
        for (i, &joint_node) in skin.joints.iter().enumerate() {
            let world = self.world
                .get(joint_node as usize)
                .copied()
                .unwrap_or(crate::pipeline::IDENTITY_M4);
            let ibm = skin
                .inverse_bind_matrices
                .get(i)
                .copied()
                .unwrap_or(crate::pipeline::IDENTITY_M4);
            out.push(mat4_mul(&world, &ibm));
        }
        out
    }

    /// Active morph weight vector for one mesh node. Falls back to the
    /// mesh's default weights when the node has no override.
    pub fn morph_weights_for_node(
        &self,
        asset: &GltfAsset,
        node_idx: u32,
    ) -> Option<ThinVec<f32>> {
        let node_idx = node_idx as usize;
        let node = asset.nodes.get(node_idx)?;
        let mesh_idx = node.mesh? as usize;
        let mesh = asset.meshes.get(mesh_idx)?;

        let node_w = self.morph_weights.get(node_idx);
        let pick: ThinVec<f32> = match node_w {
            Some(ws) if !ws.is_empty() => ws.clone(),
            _ => mesh.weights.clone(),
        };
        if pick.is_empty() { None } else { Some(pick) }
    }

    /// Evaluate every `KHR_animation_pointer` channel on `anim` at time `t`
    /// and write the sampled values into the given asset's
    /// materials / lights / nodes / cameras. Only paths the engine actually
    /// drives are recognised; unknown pointers are skipped silently.
    pub fn sample_pointer_animation(
        &self,
        asset: &mut GltfAsset,
        anim:  &Animation,
        t:     f32,
    ) {
        for pc in &anim.pointer_channels {
            apply_pointer(asset, &anim.samplers[pc.sampler as usize], &pc.pointer, t);
        }
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], u: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u]
}

// ── Morph target blending ──────────────────────────────────────────────────
//
// Each primitive carries one `MorphTarget` per registered morph; the active
// weight vector blends them additively into the rest position/normal/tangent
// streams. Output is a fresh per-vertex stream ready to upload.

#[derive(Debug, Clone)]
pub struct BlendedStreams {
    pub positions: ThinVec<[f32; 3]>,
    pub normals:   ThinVec<[f32; 3]>,
    pub tangents:  ThinVec<[f32; 3]>,
}

/// Apply morph deltas to a primitive's rest streams.
pub fn blend_morph_targets(
    prim:    &crate::mesh::Primitive,
    weights: &[f32],
) -> BlendedStreams {
    let n = prim.streams.positions.len();
    let mut positions: ThinVec<[f32; 3]> = prim.streams.positions.clone();
    let mut normals:   ThinVec<[f32; 3]> = prim.streams.normals.clone();
    let mut tangents:  ThinVec<[f32; 3]> = prim.streams.tangents
        .iter().map(|t| [t[0], t[1], t[2]]).collect();

    for (m_idx, m) in prim.morph_targets.iter().enumerate() {
        let Some(&w) = weights.get(m_idx) else { continue };
        if w == 0.0 { continue; }
        for i in 0..n {
            if let Some(d) = m.positions.get(i) {
                positions[i][0] += d[0] * w;
                positions[i][1] += d[1] * w;
                positions[i][2] += d[2] * w;
            }
            if let Some(d) = m.normals.get(i) {
                normals[i][0] += d[0] * w;
                normals[i][1] += d[1] * w;
                normals[i][2] += d[2] * w;
            }
            if let Some(d) = m.tangents.get(i) {
                tangents[i][0] += d[0] * w;
                tangents[i][1] += d[1] * w;
                tangents[i][2] += d[2] * w;
            }
        }
    }

    BlendedStreams { positions, normals, tangents }
}

// ── KHR_animation_pointer evaluation ───────────────────────────────────────
//
// Resolves a JSON-pointer string against the (mutable) asset. Each call
// samples the channel's sampler at the given time and writes the result
// into the appropriate field. Only the paths the engine actually consumes
// are wired; unknown pointers are no-ops so loaders never need to filter
// the channel list.

fn apply_pointer(
    asset:   &mut GltfAsset,
    sampler: &crate::animation::AnimSampler,
    pointer: &str,
    t:       f32,
) {
    let parts: Vec<&str> = pointer.trim_start_matches('/').split('/').collect();
    let stride = match &sampler.output {
        crate::animation::SamplerOutput::Vec3(_)    => 3,
        crate::animation::SamplerOutput::Vec4(_)    => 4,
        crate::animation::SamplerOutput::Scalars(_) => 1,
    };

    match parts.as_slice() {
        // /materials/{i}/pbrMetallicRoughness/baseColorFactor
        ["materials", i, "pbrMetallicRoughness", "baseColorFactor"] => {
            if let Some(m) = parse_idx(i).and_then(|i| asset.materials.get_mut(i)) {
                m.pbr.base_color_factor = read_vec4(sampler, t);
            }
        }
        ["materials", i, "pbrMetallicRoughness", "metallicFactor"] => {
            if let Some(m) = parse_idx(i).and_then(|i| asset.materials.get_mut(i)) {
                m.pbr.metallic_factor = read_scalar(sampler, t, stride);
            }
        }
        ["materials", i, "pbrMetallicRoughness", "roughnessFactor"] => {
            if let Some(m) = parse_idx(i).and_then(|i| asset.materials.get_mut(i)) {
                m.pbr.roughness_factor = read_scalar(sampler, t, stride);
            }
        }
        ["materials", i, "emissiveFactor"] => {
            if let Some(m) = parse_idx(i).and_then(|i| asset.materials.get_mut(i)) {
                let v = read_vec3(sampler, t);
                m.emissive_factor = v;
            }
        }
        ["materials", i, "alphaCutoff"] => {
            if let Some(m) = parse_idx(i).and_then(|i| asset.materials.get_mut(i)) {
                m.alpha_cutoff = read_scalar(sampler, t, stride);
            }
        }
        ["materials", i, "normalTexture", "scale"] => {
            if let Some(m) = parse_idx(i).and_then(|i| asset.materials.get_mut(i)) {
                m.normal.scale = read_scalar(sampler, t, stride);
            }
        }
        ["materials", i, "occlusionTexture", "strength"] => {
            if let Some(m) = parse_idx(i).and_then(|i| asset.materials.get_mut(i)) {
                m.occlusion.strength = read_scalar(sampler, t, stride);
            }
        }
        // /materials/{i}/extensions/{ext}/{...}
        ["materials", i, "extensions", ext, rest @ ..] => {
            if let Some(m) = parse_idx(i).and_then(|i| asset.materials.get_mut(i)) {
                apply_pointer_material_extension(m, ext, rest, sampler, t, stride);
            }
        }
        // /materials/{i}/{kind_texture}/extensions/KHR_texture_transform/{field}
        ["materials", i, tex_kind, "extensions", "KHR_texture_transform", field] => {
            if let Some(m) = parse_idx(i).and_then(|i| asset.materials.get_mut(i)) {
                let tex: Option<&mut TextureRef> = match *tex_kind {
                    "baseColorTexture"         => m.pbr.base_color_texture.as_mut(),
                    "metallicRoughnessTexture" => m.pbr.metallic_roughness_texture.as_mut(),
                    "emissiveTexture"          => m.emissive_texture.as_mut(),
                    "normalTexture"            => m.normal.texture.as_mut(),
                    "occlusionTexture"         => m.occlusion.texture.as_mut(),
                    _ => None,
                };
                if let Some(tr) = tex {
                    apply_texture_transform_field(tr, field, sampler, t, stride);
                }
            }
        }
        // /lights/{i}/intensity, color, range
        ["extensions", "KHR_lights_punctual", "lights", i, field] => {
            if let Some(l) = parse_idx(i).and_then(|i| asset.lights.get_mut(i)) {
                match *field {
                    "intensity" => l.intensity = read_scalar(sampler, t, stride),
                    "range"     => l.range     = read_scalar(sampler, t, stride),
                    "color"     => l.color     = read_vec3(sampler, t),
                    _ => {}
                }
            }
        }
        // /nodes/{i}/translation, rotation, scale (rare but spec-legal)
        ["nodes", i, field] => {
            if let Some(n) = parse_idx(i).and_then(|i| asset.nodes.get_mut(i)) {
                match *field {
                    "translation" => n.translation = read_vec3(sampler, t),
                    "rotation"    => n.rotation    = read_vec4(sampler, t),
                    "scale"       => n.scale       = read_vec3(sampler, t),
                    _ => {}
                }
            }
        }
        // /cameras/{i}/perspective/{yfov,znear,zfar,aspectRatio}
        ["cameras", i, "perspective", field] => {
            if let Some(c) = parse_idx(i).and_then(|i| asset.cameras.get_mut(i)) {
                if let crate::camera::Camera::Perspective {
                    aspect_ratio, y_fov, z_near, z_far, ..
                } = c {
                    let v = read_scalar(sampler, t, stride);
                    match *field {
                        "yfov"        => *y_fov = v,
                        "znear"       => *z_near = v,
                        "zfar"        => *z_far = Some(v),
                        "aspectRatio" => *aspect_ratio = Some(v),
                        _ => {}
                    }
                }
            }
        }
        _ => {} // unknown pointer — silently ignore
    }
}

fn apply_pointer_material_extension(
    m:       &mut Material,
    ext:     &str,
    rest:    &[&str],
    sampler: &crate::animation::AnimSampler,
    t:       f32,
    stride:  usize,
) {
    match (ext, rest) {
        ("KHR_materials_clearcoat", ["clearcoatFactor"]) => {
            if let Some(c) = m.clearcoat.as_mut() { c.factor = read_scalar(sampler, t, stride); }
        }
        ("KHR_materials_clearcoat", ["clearcoatRoughnessFactor"]) => {
            if let Some(c) = m.clearcoat.as_mut() { c.roughness_factor = read_scalar(sampler, t, stride); }
        }
        ("KHR_materials_sheen", ["sheenColorFactor"]) => {
            if let Some(s) = m.sheen.as_mut() { s.color_factor = read_vec3(sampler, t); }
        }
        ("KHR_materials_sheen", ["sheenRoughnessFactor"]) => {
            if let Some(s) = m.sheen.as_mut() { s.roughness_factor = read_scalar(sampler, t, stride); }
        }
        ("KHR_materials_specular", ["specularFactor"]) => {
            if let Some(s) = m.specular.as_mut() { s.factor = read_scalar(sampler, t, stride); }
        }
        ("KHR_materials_specular", ["specularColorFactor"]) => {
            if let Some(s) = m.specular.as_mut() { s.color_factor = read_vec3(sampler, t); }
        }
        ("KHR_materials_iridescence", ["iridescenceFactor"]) => {
            if let Some(i) = m.iridescence.as_mut() { i.factor = read_scalar(sampler, t, stride); }
        }
        ("KHR_materials_iridescence", ["iridescenceIor"]) => {
            if let Some(i) = m.iridescence.as_mut() { i.ior = read_scalar(sampler, t, stride); }
        }
        ("KHR_materials_anisotropy", ["anisotropyStrength"]) => {
            if let Some(a) = m.anisotropy.as_mut() { a.strength = read_scalar(sampler, t, stride); }
        }
        ("KHR_materials_anisotropy", ["anisotropyRotation"]) => {
            if let Some(a) = m.anisotropy.as_mut() { a.rotation = read_scalar(sampler, t, stride); }
        }
        ("KHR_materials_transmission", ["transmissionFactor"]) => {
            m.transmission.factor = read_scalar(sampler, t, stride);
        }
        ("KHR_materials_volume", ["thicknessFactor"]) => {
            m.volume.thickness_factor = read_scalar(sampler, t, stride);
        }
        ("KHR_materials_volume", ["attenuationDistance"]) => {
            m.volume.attenuation_distance = read_scalar(sampler, t, stride);
        }
        ("KHR_materials_volume", ["attenuationColor"]) => {
            m.volume.attenuation_color = read_vec3(sampler, t);
        }
        ("KHR_materials_ior", ["ior"]) => {
            m.ior = read_scalar(sampler, t, stride);
        }
        ("KHR_materials_emissive_strength", ["emissiveStrength"]) => {
            m.emissive_strength = read_scalar(sampler, t, stride);
        }
        ("KHR_materials_diffuse_transmission", ["diffuseTransmissionFactor"]) => {
            if let Some(d) = m.diffuse_transmission.as_mut() { d.factor = read_scalar(sampler, t, stride); }
        }
        ("KHR_materials_diffuse_transmission", ["diffuseTransmissionColorFactor"]) => {
            if let Some(d) = m.diffuse_transmission.as_mut() { d.color_factor = read_vec3(sampler, t); }
        }
        ("KHR_materials_dispersion", ["dispersion"]) => {
            m.dispersion = read_scalar(sampler, t, stride);
        }
        // /materials/{i}/extensions/{ext}/{tex}/extensions/KHR_texture_transform/{field}
        (_, [tex, "extensions", "KHR_texture_transform", field]) => {
            if let Some(tr) = pick_ext_texture(m, ext, tex) {
                apply_texture_transform_field(tr, field, sampler, t, stride);
            }
        }
        _ => {}
    }
}

fn pick_ext_texture<'a>(m: &'a mut Material, ext: &str, tex: &str) -> Option<&'a mut TextureRef> {
    match (ext, tex) {
        ("KHR_materials_clearcoat", "clearcoatTexture")             => m.clearcoat.as_mut()?.texture.as_mut(),
        ("KHR_materials_clearcoat", "clearcoatRoughnessTexture")    => m.clearcoat.as_mut()?.roughness_texture.as_mut(),
        ("KHR_materials_clearcoat", "clearcoatNormalTexture")       => m.clearcoat.as_mut()?.normal_texture.as_mut(),
        ("KHR_materials_sheen",     "sheenColorTexture")            => m.sheen.as_mut()?.color_texture.as_mut(),
        ("KHR_materials_sheen",     "sheenRoughnessTexture")        => m.sheen.as_mut()?.roughness_texture.as_mut(),
        ("KHR_materials_specular",  "specularTexture")              => m.specular.as_mut()?.texture.as_mut(),
        ("KHR_materials_specular",  "specularColorTexture")         => m.specular.as_mut()?.color_texture.as_mut(),
        ("KHR_materials_iridescence", "iridescenceTexture")         => m.iridescence.as_mut()?.texture.as_mut(),
        ("KHR_materials_iridescence", "iridescenceThicknessTexture")=> m.iridescence.as_mut()?.thickness_texture.as_mut(),
        ("KHR_materials_anisotropy", "anisotropyTexture")           => m.anisotropy.as_mut()?.texture.as_mut(),
        ("KHR_materials_transmission", "transmissionTexture")       => m.transmission.texture.as_mut(),
        ("KHR_materials_volume", "thicknessTexture")                => m.volume.thickness_texture.as_mut(),
        ("KHR_materials_diffuse_transmission", "diffuseTransmissionTexture")     => m.diffuse_transmission.as_mut()?.texture.as_mut(),
        ("KHR_materials_diffuse_transmission", "diffuseTransmissionColorTexture")=> m.diffuse_transmission.as_mut()?.color_texture.as_mut(),
        _ => None,
    }
}

fn apply_texture_transform_field(
    tr:      &mut TextureRef,
    field:   &str,
    sampler: &crate::animation::AnimSampler,
    t:       f32,
    stride:  usize,
) {
    match field {
        "offset"   => {
            let v = read_vec3(sampler, t); // vec2 stored as vec3 trailing zero
            tr.uv_offset = [v[0], v[1]];
        }
        "scale"    => {
            let v = read_vec3(sampler, t);
            tr.uv_scale = [v[0], v[1]];
        }
        "rotation" => tr.uv_rotation = read_scalar(sampler, t, stride),
        _ => {}
    }
}

fn parse_idx(s: &str) -> Option<usize> { s.parse().ok() }

fn read_scalar(s: &crate::animation::AnimSampler, t: f32, stride: usize) -> f32 {
    sample_scalar(s, t, stride, 0)
}

fn read_vec3(s: &crate::animation::AnimSampler, t: f32) -> [f32; 3] {
    match &s.output {
        crate::animation::SamplerOutput::Vec3(_) => sample_vec3(s, t),
        crate::animation::SamplerOutput::Vec4(_) => {
            let v = sample_quat(s, t);
            [v[0], v[1], v[2]]
        }
        crate::animation::SamplerOutput::Scalars(_) => {
            [sample_scalar(s, t, 3, 0), sample_scalar(s, t, 3, 1), sample_scalar(s, t, 3, 2)]
        }
    }
}

fn read_vec4(s: &crate::animation::AnimSampler, t: f32) -> [f32; 4] {
    match &s.output {
        crate::animation::SamplerOutput::Vec4(_) => sample_quat(s, t),
        crate::animation::SamplerOutput::Vec3(_) => {
            let v = sample_vec3(s, t);
            [v[0], v[1], v[2], 1.0]
        }
        crate::animation::SamplerOutput::Scalars(_) => [
            sample_scalar(s, t, 4, 0),
            sample_scalar(s, t, 4, 1),
            sample_scalar(s, t, 4, 2),
            sample_scalar(s, t, 4, 3),
        ],
    }
}

