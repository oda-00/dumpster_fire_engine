use thin_vec::ThinVec;

use crate::forge_master::{ForgeMaster, ForgeResult};
use crate::resource_manager::manager::Arena;

use super::factory_master::{ComputeTag, FactoryHandle, GraphicsTag, Proto};
use super::window::{Window, WindowHandle, WindowId, WindowTag};

// Stable WindowId → live WindowHandle. Mirrors ForgeMaster::cache for forges
// and FactoryMaster::cache for factories.
#[derive(Clone, Copy)]
pub struct WindowCacheEntry {
    pub id: WindowId,
    pub handle: WindowHandle,
}

// Top of the render side. Owns the shared compute backbone (ForgeMaster) so
// every window's FactoryMaster refines against the same device/queue/pool.
// Windows live in an Arena so handles survive insert/remove with generational
// safety — same shape as Stage::actors.
pub struct Renderer {
    pub forge: ForgeMaster,
    windows: Arena<WindowTag, Window>,
    cache: ThinVec<WindowCacheEntry>,
}

impl Renderer {
    pub fn new(forge: ForgeMaster) -> Self {
        Self {
            forge,
            windows: Arena::new(),
            cache: ThinVec::new(),
        }
    }

    pub fn add_window(&mut self, window: Window) -> WindowHandle {
        let id = window.id;
        let handle = self.windows.insert(window);
        self.cache.push(WindowCacheEntry { id, handle });
        handle
    }

    pub fn window(&self, handle: WindowHandle) -> Option<&Window> {
        self.windows.get(handle)
    }

    pub fn window_mut(&mut self, handle: WindowHandle) -> Option<&mut Window> {
        self.windows.get_mut(handle)
    }

    pub fn handle_of(&self, id: WindowId) -> Option<WindowHandle> {
        self.cache.iter().find(|e| e.id == id).map(|e| e.handle)
    }

    pub fn window_by_id(&self, id: WindowId) -> Option<&Window> {
        self.handle_of(id).and_then(|h| self.windows.get(h))
    }

    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.values()
    }

    pub fn windows_mut(&mut self) -> impl Iterator<Item = &mut Window> {
        self.windows.values_mut()
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.len() == 0
    }

    pub fn remove_window(&mut self, handle: WindowHandle) -> Option<Window> {
        let mut window = self.windows.remove(handle)?;
        unsafe { window.destroy(&self.forge.device) };
        self.cache.retain(|e| e.handle != handle);
        Some(window)
    }

    pub fn build_compute_factory(
        &mut self,
        window_h: WindowHandle,
        proto:    Proto<ComputeTag>,
    ) -> ForgeResult<FactoryHandle> {
        let device = self.forge.device.clone();
        let window = self
            .windows
            .get_mut(window_h)
            .expect("window handle is stale or was never valid");
        window.build_compute_factory(proto, &mut self.forge, &device)
    }

    pub fn build_graphics_factory(
        &mut self,
        window_h: WindowHandle,
        proto:    Proto<GraphicsTag>,
    ) -> FactoryHandle {
        let device = self.forge.device.clone();
        let window = self
            .windows
            .get_mut(window_h)
            .expect("window handle is stale or was never valid");
        window.build_graphics_factory(proto, &device)
    }

    /// Wait on the most-recently-submitted frame fence for `window_h`.
    /// Use this in per-frame setup code to guarantee the previous frame's
    /// resources are no longer in flight before recycling/destroying them.
    /// Replaces `device_wait_idle` for single-window apps.
    pub fn wait_for_last_submission(&self, window_h: WindowHandle) -> ForgeResult<()> {
        let device = self.forge.device.clone();
        let window = self.windows.get(window_h)
            .expect("window handle is stale or was never valid");
        window.wait_for_last_submission(&device)
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // Frames hold Vulkan buffers/images; tear them down before the
        // ForgeMaster's Drop releases the device.
        let device = self.forge.device.clone();
        for window in self.windows.values_mut() {
            unsafe { window.destroy(&device) };
        }
        self.cache.clear();
    }
}
