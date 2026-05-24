use crate::render::ui_render::drawlist::DrawList;
use crate::render::ui_render::vertex::{RingBuffer, UiVertex};
use crate::render::vulkan::VulkanContext;
use ash::vk;

pub struct UIRenderer {
    ring: RingBuffer,
}

impl UIRenderer {
    pub fn new(device: &ash::Device, mem_props: &vk::PhysicalDeviceMemoryProperties) -> Self {
        let ring = RingBuffer::new(device, mem_props, 65536);
        Self { ring }
    }

    pub fn render(&mut self, drawlist: &DrawList, cmd: vk::CommandBuffer, device: &ash::Device) {
        if drawlist.vertices.is_empty() || drawlist.indices.is_empty() {
            return;
        }

        let vertex_data = unsafe {
            std::slice::from_raw_parts(
                drawlist.vertices.as_ptr() as *const u8,
                drawlist.vertices.len() * std::mem::size_of::<UiVertex>(),
            )
        };

        self.ring.upload(device, vertex_data);

        unsafe {
            device.cmd_draw_indexed(cmd, drawlist.indices.len() as u32, 1, 0, 0, 0);
        }
    }

    pub fn end_frame(&mut self, device: &ash::Device) {
        self.ring.end_frame(device);
    }
}
