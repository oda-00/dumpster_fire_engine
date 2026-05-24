// These lints require extensive refactoring to address correctly; suppress
// project-wide until a dedicated cleanup pass.
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::module_inception)]
#![allow(clippy::large_enum_variant)]

pub mod forge_master;
pub mod render;
pub mod resource_manager;

// Re-export the engine's preferred collection so game code can stay on the
// engine-only dep policy (no direct `use thin_vec`).
pub use thin_vec::{ThinVec, thin_vec};
