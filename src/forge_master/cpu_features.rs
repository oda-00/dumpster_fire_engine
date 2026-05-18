//! Engine-side mirror of `forge_gltf::cpu` — duplicated rather than
//! cross-crate-imported so the engine's SIMD entry points (in
//! resource_manager::asset_manager::gltf_loader::pack_skin_attrs_*,
//! ::compute_asset_aabb_*) don't have to depend on forge_gltf for a
//! 6-bool struct.

pub use forge_gltf::cpu::{CpuFeatures, cpu_features};
