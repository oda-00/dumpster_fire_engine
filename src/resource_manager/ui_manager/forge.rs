//! Registers the UI graphics forge — exposes `register_ui_forge()` so the
//! engine bootstrap can wire `GraphicsOreKind::Ui` to the ui.vert/ui.frag
//! pipeline. The forge itself is built by the engine's factory_master; this
//! file just owns the shader-bytecode entry points so consumers can locate
//! them without reaching into ui_manager internals.

/// Vertex shader SPIR-V bytes. Compiled by build.rs from assets/shaders/ui.vert.glsl.
pub const UI_VERT_SPV: &[u8] = include_bytes!("../../../assets/shaders/ui.vert.glsl.spv");

/// Fragment shader SPIR-V bytes. Compiled by build.rs from assets/shaders/ui.frag.glsl.
pub const UI_FRAG_SPV: &[u8] = include_bytes!("../../../assets/shaders/ui.frag.glsl.spv");

/// Vertex layout for the UI pipeline.
/// Format: `R32G32_SFLOAT pos + R32G32_SFLOAT uv + R8G8B8A8_UNORM color = 20 bytes`.
pub const UI_VERTEX_STRIDE: u32 = 20;
