//! Material model — PBR metallic-roughness plus the KHR_* extensions the
//! engine's `Cargo.toml` enables. Texture references store both the texture
//! index and the UV set so pipelines can pick the right TEXCOORD stream.
//!
//! The 80-byte `MaterialBlock` is what `MaterialFlattening` consumes — a
//! flat, GPU-friendly std140-compatible record that every material collapses
//! to regardless of which extensions are active.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

impl AlphaMode {
    pub fn from(am: gltf::material::AlphaMode) -> Self {
        match am {
            gltf::material::AlphaMode::Opaque => Self::Opaque,
            gltf::material::AlphaMode::Mask   => Self::Mask,
            gltf::material::AlphaMode::Blend  => Self::Blend,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TextureRef {
    pub texture: u32,
    pub tex_coord_set: u32,
    /// KHR_texture_transform: offset, rotation (radians), scale, optional override set.
    pub uv_offset:   [f32; 2],
    pub uv_rotation: f32,
    pub uv_scale:    [f32; 2],
}

impl TextureRef {
    pub fn identity(texture: u32, tex_coord_set: u32) -> Self {
        Self {
            texture,
            tex_coord_set,
            uv_offset: [0.0, 0.0],
            uv_rotation: 0.0,
            uv_scale: [1.0, 1.0],
        }
    }
}

#[derive(Debug, Clone)]
pub struct PbrMetallicRoughness {
    pub base_color_factor: [f32; 4],
    pub metallic_factor:   f32,
    pub roughness_factor:  f32,
    pub base_color_texture:         Option<TextureRef>,
    pub metallic_roughness_texture: Option<TextureRef>,
}

impl Default for PbrMetallicRoughness {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            metallic_factor:   1.0,
            roughness_factor:  1.0,
            base_color_texture: None,
            metallic_roughness_texture: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Transmission {
    pub factor:  f32,
    pub texture: Option<TextureRef>,
}

#[derive(Debug, Clone)]
pub struct Volume {
    pub thickness_factor:    f32,
    pub thickness_texture:   Option<TextureRef>,
    pub attenuation_distance:f32,
    pub attenuation_color:   [f32; 3],
}

impl Default for Volume {
    fn default() -> Self {
        Self {
            thickness_factor:    0.0,
            thickness_texture:   None,
            attenuation_distance:f32::INFINITY,
            attenuation_color:   [1.0, 1.0, 1.0],
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NormalTexture {
    pub texture: Option<TextureRef>,
    pub scale:   f32,
}

#[derive(Debug, Clone, Default)]
pub struct OcclusionTexture {
    pub texture:  Option<TextureRef>,
    pub strength: f32,
}

#[derive(Debug, Clone)]
pub struct Material {
    pub name:                Option<String>,
    pub pbr:                 PbrMetallicRoughness,
    pub normal:              NormalTexture,
    pub occlusion:           OcclusionTexture,
    pub emissive_factor:     [f32; 3],
    pub emissive_strength:   f32, // KHR_materials_emissive_strength
    pub emissive_texture:    Option<TextureRef>,
    pub alpha_mode:          AlphaMode,
    pub alpha_cutoff:        f32,
    pub double_sided:        bool,
    pub unlit:               bool,    // KHR_materials_unlit
    pub ior:                 f32,     // KHR_materials_ior, default 1.5
    pub transmission:        Transmission,
    pub volume:              Volume,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name:              None,
            pbr:               PbrMetallicRoughness::default(),
            normal:            NormalTexture { texture: None, scale: 1.0 },
            occlusion:         OcclusionTexture { texture: None, strength: 1.0 },
            emissive_factor:   [0.0, 0.0, 0.0],
            emissive_strength: 1.0,
            emissive_texture:  None,
            alpha_mode:        AlphaMode::Opaque,
            alpha_cutoff:      0.5,
            double_sided:      false,
            unlit:             false,
            ior:               1.5,
            transmission:      Transmission::default(),
            volume:            Volume::default(),
        }
    }
}

/// Flat, GPU-side material record. Lays out the parameters every material
/// collapses to once extensions are baked in, in std140-friendly order.
///
/// 5 × vec4 = 80 bytes per material — what `MaterialFlattening` writes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MaterialBlock {
    pub base_color_factor:   [f32; 4],
    pub emissive:            [f32; 4], // rgb = emissive_factor * strength, a = ior
    pub metallic_roughness:  [f32; 4], // metal, rough, normal_scale, occlusion_strength
    pub transmission_volume: [f32; 4], // transmission, thickness, attenuation_distance, alpha_cutoff
    pub flags:               [u32; 4], // [unlit, double_sided, alpha_mode, _reserved]
}

impl MaterialBlock {
    pub const BYTES: usize = core::mem::size_of::<Self>();

    pub fn from_material(m: &Material) -> Self {
        Self {
            base_color_factor: m.pbr.base_color_factor,
            emissive: [
                m.emissive_factor[0] * m.emissive_strength,
                m.emissive_factor[1] * m.emissive_strength,
                m.emissive_factor[2] * m.emissive_strength,
                m.ior,
            ],
            metallic_roughness: [
                m.pbr.metallic_factor,
                m.pbr.roughness_factor,
                m.normal.scale,
                m.occlusion.strength,
            ],
            transmission_volume: [
                m.transmission.factor,
                m.volume.thickness_factor,
                m.volume.attenuation_distance,
                m.alpha_cutoff,
            ],
            flags: [
                m.unlit as u32,
                m.double_sided as u32,
                m.alpha_mode as u32,
                0,
            ],
        }
    }
}
