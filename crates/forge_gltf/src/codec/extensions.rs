//! Supported extension registry.
//!
//! `extensionsRequired` validation at load time checks every required ext
//! against this set and returns `GltfError::UnsupportedExtension` for any
//! name not listed here.

/// All KHR_* and EXT_* extension names this crate handles.
pub const SUPPORTED: &[&str] = &[
    // Material extensions
    "KHR_materials_clearcoat",
    "KHR_materials_sheen",
    "KHR_materials_specular",
    "KHR_materials_iridescence",
    "KHR_materials_anisotropy",
    "KHR_materials_diffuse_transmission",
    "KHR_materials_dispersion",
    "KHR_materials_transmission",
    "KHR_materials_volume",
    "KHR_materials_ior",
    "KHR_materials_unlit",
    "KHR_materials_emissive_strength",
    "KHR_materials_pbrSpecularGlossiness",
    "KHR_materials_variants",
    // Texture extensions
    "KHR_texture_transform",
    "KHR_texture_basisu",
    "EXT_texture_webp",
    // Mesh extensions
    "KHR_draco_mesh_compression",
    "EXT_meshopt_compression",
    // Light extensions
    "KHR_lights_punctual",
    // Animation extensions
    "KHR_animation_pointer",
];

pub fn is_supported(name: &str) -> bool {
    SUPPORTED.contains(&name)
}
