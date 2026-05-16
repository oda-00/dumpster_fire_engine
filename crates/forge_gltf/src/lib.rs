//! Hand-rolled glTF façade.
//!
//! Wraps the `gltf` crate and lifts every dispatchable bit of data out of a
//! document into engine-neutral structs. Every loaded `GltfAsset` can then be
//! adapted into byte payloads tailored to a specific render pipeline through
//! the `pipeline` module — one adapter per compute/graphics pipeline kind the
//! engine ships with.
//!
//! The crate is a leaf — it knows nothing about Vulkan or about the engine's
//! `Ore`/`MeshOre` types. The engine bridges these adapters into its own
//! pipeline objects in `dumpster_fire_engine::resource_manager::asset_manager`.

pub mod animation;
pub mod asset;
pub mod builder;
pub mod camera;
pub mod error;
pub mod light;
pub mod material;
pub mod mesh;
pub mod pipeline;
pub mod pose;
pub mod scene;
pub mod skin;
pub mod texture;

pub use animation::*;
pub use asset::*;
pub use builder::*;
pub use camera::*;
pub use error::*;
pub use light::*;
pub use material::*;
pub use mesh::*;
pub use pipeline::*;
pub use pose::*;
pub use scene::*;
pub use skin::*;
pub use texture::*;
