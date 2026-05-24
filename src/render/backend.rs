//! Backend abstraction — common types shared between the Vulkan and wgpu paths.

use crate::resource_manager::ui_manager::draw::UiVertex;
use crate::resource_manager::world_manager::World;

/// Which rendering backend is active for this process.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BackendKind {
    Vulkan,
    Wgpu,
}

/// Snapshot of one frame's UI geometry, ready to upload.
pub struct DrawListSnapshot<'a> {
    pub vertices: &'a [UiVertex],
    pub indices: &'a [u32],
}

/// Per-frame scene + timing data passed into a backend's draw call.
pub struct RenderSceneInput<'a> {
    pub world: &'a World,
    pub elapsed: f32,
}

/// Minimal rendering surface interface shared by Vulkan (`Window`) and
/// wgpu (`WgpuSurface`).
pub trait GpuSurface: Send {
    fn backend(&self) -> BackendKind;

    /// Draw one frame: upload `ui`, render the scene, present.
    /// Returns `Ok(Some(semaphore))` only on the Vulkan path (compute sync).
    fn draw_frame(
        &mut self,
        scene: &RenderSceneInput<'_>,
        ui: &DrawListSnapshot<'_>,
    ) -> crate::forge_master::master::ForgeResult<()>;

    fn resize(&mut self, width: u32, height: u32);
    fn has_ray_tracing(&self) -> bool {
        false
    }
}
