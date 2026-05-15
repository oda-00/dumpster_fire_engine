//! `langc` library — exposes the codegen pipeline so benchmarks (and
//! `langcd`) can drive it without forking a subprocess.  Production users
//! continue to invoke `langc` as a CLI; the binary is a thin wrapper around
//! these modules.

pub mod codegen;
pub mod engine_api;
pub mod link;

/// Re-export so downstream benches and the daemon don't need a direct
/// inkwell dependency just to pick an optimisation level.
pub use inkwell::OptimizationLevel;
