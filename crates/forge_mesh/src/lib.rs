//! `forge_mesh` — editable mesh topology core for the editor.
//!
//! A struct-of-arrays, **index-based half-edge** (EDITOR_research.md §4): adjacency
//! is `u32` indices into dense `ThinVec`s, not pointers, so traversal streams from
//! cache-resident arrays and the structure never fragments — the opposite of
//! Blender BMesh's pointer-linked lists (whose sculpt path spends ~30% of its time
//! fetching memory once fragmented). The half-edge arrays *are* the arena; stable
//! ids + free lists let edits reuse slots in place.
//!
//! Phase 0 covers: build from indexed triangles, adjacency queries, round-trip back
//! to indexed triangles, invariant validation, and (behind the `gltf` feature) a
//! bridge to/from `forge_gltf` primitives. Selection, operators, BVH, GPU kernels,
//! undo, and sculpt land in later phases.

pub mod bvh;
pub mod half_edge;
pub mod ops;
pub mod select;
pub mod transform;

#[cfg(feature = "gltf")]
pub mod bridge;

pub use half_edge::{HalfEdgeMesh, MeshError, INVALID};
