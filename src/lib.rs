pub mod forge_master;
pub mod render;
pub mod resource_manager;

// Re-export the engine's preferred collection so game code can stay on the
// engine-only dep policy (no direct `use thin_vec`).
pub use thin_vec::{ThinVec, thin_vec};
