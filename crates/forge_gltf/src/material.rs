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

// ── KHR_materials_* extensions parsed from raw extension JSON ──────────────
//
// The base `gltf` crate exposes per-material extension data as a generic
// JSON map. These are the typed views we read from it.

#[derive(Debug, Clone, Default)]
pub struct Clearcoat {
    pub factor:               f32,
    pub roughness_factor:     f32,
    pub texture:              Option<TextureRef>,
    pub roughness_texture:    Option<TextureRef>,
    pub normal_texture:       Option<TextureRef>,
    pub normal_scale:         f32,
}

#[derive(Debug, Clone)]
pub struct Sheen {
    pub color_factor:      [f32; 3],
    pub color_texture:     Option<TextureRef>,
    pub roughness_factor:  f32,
    pub roughness_texture: Option<TextureRef>,
}

impl Default for Sheen {
    fn default() -> Self {
        Self {
            color_factor: [0.0, 0.0, 0.0],
            color_texture: None,
            roughness_factor: 0.0,
            roughness_texture: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Specular {
    pub factor:        f32,
    pub texture:       Option<TextureRef>,
    pub color_factor:  [f32; 3],
    pub color_texture: Option<TextureRef>,
}

impl Default for Specular {
    fn default() -> Self {
        Self {
            factor: 1.0,
            texture: None,
            color_factor: [1.0, 1.0, 1.0],
            color_texture: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Iridescence {
    pub factor:               f32,
    pub texture:              Option<TextureRef>,
    pub ior:                  f32,
    pub thickness_minimum:    f32,
    pub thickness_maximum:    f32,
    pub thickness_texture:    Option<TextureRef>,
}

impl Default for Iridescence {
    fn default() -> Self {
        Self {
            factor: 0.0,
            texture: None,
            ior: 1.3,
            thickness_minimum: 100.0,
            thickness_maximum: 400.0,
            thickness_texture: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Anisotropy {
    pub strength: f32,
    pub rotation: f32,
    pub texture:  Option<TextureRef>,
}

#[derive(Debug, Clone)]
pub struct DiffuseTransmission {
    pub factor:        f32,
    pub texture:       Option<TextureRef>,
    pub color_factor:  [f32; 3],
    pub color_texture: Option<TextureRef>,
}

impl Default for DiffuseTransmission {
    fn default() -> Self {
        Self {
            factor: 0.0,
            texture: None,
            color_factor: [1.0, 1.0, 1.0],
            color_texture: None,
        }
    }
}

/// KHR_materials_volume_scatter — non-standard scattering extension used by
/// `ScatteringSkull.glb` and similar subsurface assets.
#[derive(Debug, Clone, Default)]
pub struct VolumeScatter {
    pub scatter_color:    [f32; 3],
    pub scatter_distance: f32,
    pub anisotropy:       f32,
}

/// KHR_materials_pbrSpecularGlossiness — legacy (pre-2.0 ratification) but
/// still widely used. Fields per the extension spec §2.
#[derive(Debug, Clone)]
pub struct PbrSpecularGlossiness {
    pub diffuse_factor:               [f32; 4],
    pub diffuse_texture:              Option<TextureRef>,
    pub specular_factor:              [f32; 3],
    pub glossiness_factor:            f32,
    pub specular_glossiness_texture:  Option<TextureRef>,
}

impl Default for PbrSpecularGlossiness {
    fn default() -> Self {
        Self {
            diffuse_factor:              [1.0, 1.0, 1.0, 1.0],
            diffuse_texture:             None,
            specular_factor:             [1.0, 1.0, 1.0],
            glossiness_factor:           1.0,
            specular_glossiness_texture: None,
        }
    }
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
    // Extensions parsed from raw JSON (None when the material doesn't enable them).
    pub clearcoat:           Option<Clearcoat>,
    pub sheen:               Option<Sheen>,
    pub specular:            Option<Specular>,
    pub iridescence:         Option<Iridescence>,
    pub anisotropy:          Option<Anisotropy>,
    pub diffuse_transmission:Option<DiffuseTransmission>,
    /// KHR_materials_dispersion: a single scalar.
    pub dispersion:          f32,
    /// KHR_materials_volume_scatter (non-standard / experimental).
    pub volume_scatter:      Option<VolumeScatter>,
    /// KHR_materials_pbrSpecularGlossiness — legacy specular/glossiness workflow.
    pub pbr_specular_glossiness: Option<PbrSpecularGlossiness>,
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
            clearcoat:         None,
            sheen:             None,
            specular:          None,
            iridescence:       None,
            anisotropy:        None,
            diffuse_transmission: None,
            dispersion:               0.0,
            volume_scatter:           None,
            pbr_specular_glossiness:  None,
        }
    }
}

/// Flat, GPU-side material record. Lays out the parameters every material
/// collapses to once base extensions are baked in, in std140-friendly order.
///
/// 5 × vec4 = 80 bytes per material — what `MaterialFlattening` writes
/// for the base PBR + transmission/volume/unlit/IOR set.
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

/// Std140-friendly per-material extension block. 8 × vec4 = 128 bytes,
/// carrying the post-base KHR_materials_* extension parameters the engine's
/// shaders sample from. Always emitted alongside `MaterialBlock` — the
/// pipeline adapter writes the basic block array first then the extended
/// block array, so shaders read both with a single bound buffer pair.
///
/// Layout:
///   vec4 clearcoat   = factor | roughness | normal_scale | _
///   vec4 sheen_rgb_r = sheen_color.rgb | sheen_roughness
///   vec4 specular    = factor | color.r | color.g | color.b
///   vec4 irid        = factor | ior | thickness_min | thickness_max
///   vec4 anisotropy  = strength | rotation | dispersion | _
///   vec4 diff_trans  = factor | color.r | color.g | color.b
///   vec4 flags       = [clearcoat_on, sheen_on, specular_on, iridescence_on]
///   vec4 flags2      = [anisotropy_on, diffuse_transmission_on, _, _]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MaterialExtBlock {
    pub clearcoat:   [f32; 4],
    pub sheen:       [f32; 4],
    pub specular:    [f32; 4],
    pub iridescence: [f32; 4],
    pub anisotropy:  [f32; 4],
    pub diff_trans:  [f32; 4],
    pub flags:       [u32; 4],
    pub flags2:      [u32; 4],
}

impl MaterialExtBlock {
    pub const BYTES: usize = core::mem::size_of::<Self>();

    pub fn from_material(m: &Material) -> Self {
        let cc = m.clearcoat.clone().unwrap_or_default();
        let sh = m.sheen.clone().unwrap_or_default();
        let sp = m.specular.clone().unwrap_or_default();
        let ir = m.iridescence.clone().unwrap_or_default();
        let an = m.anisotropy.clone().unwrap_or_default();
        let dt = m.diffuse_transmission.clone().unwrap_or_default();
        Self {
            clearcoat: [
                cc.factor, cc.roughness_factor, cc.normal_scale, 0.0,
            ],
            sheen: [
                sh.color_factor[0], sh.color_factor[1], sh.color_factor[2], sh.roughness_factor,
            ],
            specular: [
                sp.factor, sp.color_factor[0], sp.color_factor[1], sp.color_factor[2],
            ],
            iridescence: [
                ir.factor, ir.ior, ir.thickness_minimum, ir.thickness_maximum,
            ],
            anisotropy: [
                an.strength, an.rotation, m.dispersion, 0.0,
            ],
            diff_trans: [
                dt.factor, dt.color_factor[0], dt.color_factor[1], dt.color_factor[2],
            ],
            flags: [
                m.clearcoat.is_some() as u32,
                m.sheen.is_some() as u32,
                m.specular.is_some() as u32,
                m.iridescence.is_some() as u32,
            ],
            flags2: [
                m.anisotropy.is_some() as u32,
                m.diffuse_transmission.is_some() as u32,
                0,
                0,
            ],
        }
    }
}

// ── Raw-JSON extension parser ──────────────────────────────────────────────
//
// The base `gltf` crate exposes per-material extension data as a generic
// JSON map. These helpers pluck typed views out of it for the
// KHR_materials_* extensions the crate doesn't model itself.

use serde_json::{Map, Value};

fn f32_field(o: &Map<String, Value>, key: &str, default: f32) -> f32 {
    o.get(key).and_then(|v| v.as_f64()).map(|f| f as f32).unwrap_or(default)
}

fn vec3_field(o: &Map<String, Value>, key: &str, default: [f32; 3]) -> [f32; 3] {
    let Some(arr) = o.get(key).and_then(|v| v.as_array()) else { return default; };
    let mut out = default;
    for (i, slot) in out.iter_mut().enumerate() {
        if let Some(v) = arr.get(i).and_then(|v| v.as_f64()) { *slot = v as f32; }
    }
    out
}

fn texture_ref_field(o: &Map<String, Value>, key: &str) -> Option<TextureRef> {
    let info = o.get(key)?.as_object()?;
    let texture = info.get("index")?.as_u64()? as u32;
    let tex_coord_set = info.get("texCoord").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let mut tr = TextureRef::identity(texture, tex_coord_set);
    if let Some(tx) = info.get("extensions")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("KHR_texture_transform"))
        .and_then(|v| v.as_object())
    {
        tr.uv_offset   = [
            f32_field_array(tx, "offset", 0, 0.0),
            f32_field_array(tx, "offset", 1, 0.0),
        ];
        tr.uv_rotation = f32_field(tx, "rotation", 0.0);
        tr.uv_scale    = [
            f32_field_array(tx, "scale", 0, 1.0),
            f32_field_array(tx, "scale", 1, 1.0),
        ];
        if let Some(ts) = tx.get("texCoord").and_then(|v| v.as_u64()) {
            tr.tex_coord_set = ts as u32;
        }
    }
    Some(tr)
}

fn f32_field_array(o: &Map<String, Value>, key: &str, i: usize, default: f32) -> f32 {
    o.get(key)
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(i))
        .and_then(|v| v.as_f64())
        .map(|f| f as f32)
        .unwrap_or(default)
}

pub fn parse_material_extensions(
    out:        &mut Material,
    extensions: &Map<String, Value>,
) {
    if let Some(o) = extensions.get("KHR_materials_clearcoat").and_then(|v| v.as_object()) {
        // Default factor is 0.0 per spec; default roughness 0.0.
        let mut cc = Clearcoat::default();
        cc.factor               = f32_field(o, "clearcoatFactor", 0.0);
        cc.roughness_factor     = f32_field(o, "clearcoatRoughnessFactor", 0.0);
        cc.texture              = texture_ref_field(o, "clearcoatTexture");
        cc.roughness_texture    = texture_ref_field(o, "clearcoatRoughnessTexture");
        cc.normal_texture       = texture_ref_field(o, "clearcoatNormalTexture");
        cc.normal_scale         = o.get("clearcoatNormalTexture")
            .and_then(|v| v.as_object())
            .and_then(|n| n.get("scale"))
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(1.0);
        out.clearcoat = Some(cc);
    }

    if let Some(o) = extensions.get("KHR_materials_sheen").and_then(|v| v.as_object()) {
        out.sheen = Some(Sheen {
            color_factor:      vec3_field(o, "sheenColorFactor", [0.0, 0.0, 0.0]),
            color_texture:     texture_ref_field(o, "sheenColorTexture"),
            roughness_factor:  f32_field(o, "sheenRoughnessFactor", 0.0),
            roughness_texture: texture_ref_field(o, "sheenRoughnessTexture"),
        });
    }

    if let Some(o) = extensions.get("KHR_materials_specular").and_then(|v| v.as_object()) {
        out.specular = Some(Specular {
            factor:        f32_field(o, "specularFactor", 1.0),
            texture:       texture_ref_field(o, "specularTexture"),
            color_factor:  vec3_field(o, "specularColorFactor", [1.0, 1.0, 1.0]),
            color_texture: texture_ref_field(o, "specularColorTexture"),
        });
    }

    if let Some(o) = extensions.get("KHR_materials_iridescence").and_then(|v| v.as_object()) {
        out.iridescence = Some(Iridescence {
            factor:            f32_field(o, "iridescenceFactor", 0.0),
            texture:           texture_ref_field(o, "iridescenceTexture"),
            ior:               f32_field(o, "iridescenceIor", 1.3),
            thickness_minimum: f32_field(o, "iridescenceThicknessMinimum", 100.0),
            thickness_maximum: f32_field(o, "iridescenceThicknessMaximum", 400.0),
            thickness_texture: texture_ref_field(o, "iridescenceThicknessTexture"),
        });
    }

    if let Some(o) = extensions.get("KHR_materials_anisotropy").and_then(|v| v.as_object()) {
        out.anisotropy = Some(Anisotropy {
            strength: f32_field(o, "anisotropyStrength", 0.0),
            rotation: f32_field(o, "anisotropyRotation", 0.0),
            texture:  texture_ref_field(o, "anisotropyTexture"),
        });
    }

    if let Some(o) = extensions.get("KHR_materials_diffuse_transmission").and_then(|v| v.as_object()) {
        out.diffuse_transmission = Some(DiffuseTransmission {
            factor:        f32_field(o, "diffuseTransmissionFactor", 0.0),
            texture:       texture_ref_field(o, "diffuseTransmissionTexture"),
            color_factor:  vec3_field(o, "diffuseTransmissionColorFactor", [1.0, 1.0, 1.0]),
            color_texture: texture_ref_field(o, "diffuseTransmissionColorTexture"),
        });
    }

    if let Some(o) = extensions.get("KHR_materials_dispersion").and_then(|v| v.as_object()) {
        out.dispersion = f32_field(o, "dispersion", 0.0);
    }

    // KHR_materials_volume_scatter (non-standard, but used by ScatteringSkull and others)
    if let Some(o) = extensions.get("KHR_materials_volume_scatter").and_then(|v| v.as_object()) {
        out.volume_scatter = Some(VolumeScatter {
            scatter_color:    vec3_field(o, "scatterColor", [1.0, 1.0, 1.0]),
            scatter_distance: f32_field(o, "scatterDistance", 0.0),
            anisotropy:       f32_field(o, "anisotropy", 0.0),
        });
    }

    // KHR_materials_pbrSpecularGlossiness
    if let Some(o) = extensions.get("KHR_materials_pbrSpecularGlossiness").and_then(|v| v.as_object()) {
        let mut sg = PbrSpecularGlossiness::default();
        if let Some(arr) = o.get("diffuseFactor").and_then(|v| v.as_array()) {
            for (i, slot) in sg.diffuse_factor.iter_mut().enumerate() {
                if let Some(v) = arr.get(i).and_then(|v| v.as_f64()) { *slot = v as f32; }
            }
        }
        sg.diffuse_texture              = texture_ref_field(o, "diffuseTexture");
        sg.specular_factor              = vec3_field(o, "specularFactor", [1.0, 1.0, 1.0]);
        sg.glossiness_factor            = f32_field(o, "glossinessFactor", 1.0);
        sg.specular_glossiness_texture  = texture_ref_field(o, "specularGlossinessTexture");
        out.pbr_specular_glossiness = Some(sg);
    }
}
