use std::sync::Arc;

use crate::forge_master::{ForgeMaster, ForgeResult};
use crate::resource_manager::manager::{Handle, Id};

use super::factory_master::{FactoryHandle, FactoryMaster, Proto};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct WindowTag;
pub type WindowHandle = Handle<WindowTag>;

pub struct WindowMarker;
pub type WindowId = Id<WindowMarker>;

// A render target descriptor + the Factories that drive its passes.
// The actual platform surface (winit / SDL) is layered on top of this once
// the swapchain story lands; for now Window is a logical owner of one
// FactoryMaster that builds against the renderer's shared ForgeMaster.
pub struct Window {
    pub id: WindowId,
    pub name: Arc<str>,
    pub width: u32,
    pub height: u32,
    pub factory_master: FactoryMaster,
}

impl Window {
    pub fn new(id: WindowId, name: impl Into<Arc<str>>, width: u32, height: u32) -> Self {
        Self {
            id,
            name: name.into(),
            width,
            height,
            factory_master: FactoryMaster::new(),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    pub fn build_factory(
        &mut self,
        proto: Proto,
        forge: &mut ForgeMaster,
    ) -> ForgeResult<FactoryHandle> {
        self.factory_master.build_from_proto(proto, forge)
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe { self.factory_master.destroy(device) };
    }
}
