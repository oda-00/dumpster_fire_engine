use ash::vk;
use std::mem;

use crate::render::ui_render::drawlist::DrawList;
use crate::render::ui_render::vertex::{RingBuffer, UiVertex};

pub struct UIRenderer {
    vb: RingBuffer,
    ib: RingBuffer,
}

impl UIRenderer {
    pub fn new(device: &ash::Device, mem_props: &vk::PhysicalDeviceMemoryProperties) -> Self {
        Self {
            vb: RingBuffer::new(device, mem_props, 65_536),
            ib: RingBuffer::new(device, mem_props, 32_768),
        }
    }

    #[inline]
    pub fn upload(&mut self, drawlist: &DrawList, device: &ash::Device) {
        if drawlist.vertices.is_empty() {
            return;
        }

        let vb_bytes = unsafe {
            std::slice::from_raw_parts(
                drawlist.vertices.as_ptr() as *const u8,
                drawlist.vertices.len() * mem::size_of::<UiVertex>(),
            )
        };
        let ib_bytes = unsafe {
            std::slice::from_raw_parts(
                drawlist.indices.as_ptr() as *const u8,
                drawlist.indices.len() * mem::size_of::<u32>(),
            )
        };

        self.vb.upload(device, vb_bytes);
        self.ib.upload(device, ib_bytes);
    }

    #[inline]
    pub fn record_draw(
        &self,
        drawlist: &DrawList,
        pipeline_layout: vk::PipelineLayout,
        cmd: vk::CommandBuffer,
        device: &ash::Device,
    ) {
        if drawlist.indices.is_empty() {
            return;
        }
        unsafe {
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.vb.buffer()], &[0]);
            device.cmd_bind_index_buffer(cmd, self.ib.buffer(), 0, vk::IndexType::UINT32);
            device.cmd_draw_indexed(cmd, drawlist.indices.len() as u32, 1, 0, 0, 0);
        }
    }

    pub fn end_frame(&mut self, device: &ash::Device) {
        self.vb.end_frame(device);
        self.ib.end_frame(device);
    }
}
