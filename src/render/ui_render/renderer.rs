use ash::vk;
use std::mem;

use crate::render::overlay::OverlayPipeline;
use crate::render::ui_render::drawlist::DrawList;
use crate::render::ui_render::vertex::{RingBuffer, UiVertex};
use crate::render::vulkan::VulkanContext;

pub struct UIRenderer {
    vb: RingBuffer,
    ib: RingBuffer,
    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
    descriptor_set: vk::DescriptorSet,
}

impl UIRenderer {
    /// Construct by borrowing the pre-built pipeline and descriptor set from
    /// `OverlayPipeline::ui`, then creating independent ring buffers for
    /// vertex/index data (avoids aliasing with the overlay's per-frame slots).
    pub fn new(vulkan: &VulkanContext, overlay: &OverlayPipeline) -> Self {
        Self {
            vb: RingBuffer::new(&vulkan.device, &vulkan.memory_properties, 65_536),
            ib: RingBuffer::new(&vulkan.device, &vulkan.memory_properties, 32_768),
            pipeline: overlay.ui.pipeline,
            pipeline_layout: overlay.ui.pipeline_layout,
            descriptor_set: overlay.ui.set,
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

    /// Bind the UI pipeline, descriptor set, vertex + index buffers, then
    /// submit a single indexed draw call.
    #[inline]
    pub fn record_draw(
        &self,
        drawlist: &DrawList,
        cmd: vk::CommandBuffer,
        device: &ash::Device,
    ) {
        if drawlist.indices.is_empty() {
            return;
        }
        unsafe {
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                &[self.descriptor_set],
                &[],
            );
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
