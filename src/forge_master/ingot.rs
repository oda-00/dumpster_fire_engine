use ash::vk;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use thin_vec::ThinVec;

use super::master::ForgeResult;
use super::ore::{
    ForgeBuffer, ForgeImage, IngotSpec, OreKind, non_zero_size, storage_buffer_readback_barrier,
};

const INGOT_MAGIC: &[u8; 8] = b"DFINGOT\0";
const INGOT_VERSION: u32 = 1;

#[derive(Debug)]
pub enum IngotArtifact {
    Buffer {
        result: ForgeBuffer,
        readback: ForgeBuffer,
        bytes: ThinVec<u8>,
    },
    Image2d {
        result: ForgeImage,
        readback: ForgeBuffer,
        bytes: ThinVec<u8>,
        byte_size: vk::DeviceSize,
    },
}

#[derive(Debug)]
pub struct Ingot {
    pub kind: OreKind,
    pub artifact: IngotArtifact,
    pub save_path: Option<PathBuf>,
}

impl Ingot {
    pub fn create(
        kind: OreKind,
        spec: &IngotSpec,
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
    ) -> ForgeResult<Self> {
        match spec {
            IngotSpec::Buffer { size, save_path, extra_usage } => {
                let size = non_zero_size(*size);
                let usage = vk::BufferUsageFlags::STORAGE_BUFFER
                    | vk::BufferUsageFlags::TRANSFER_SRC
                    | *extra_usage;
                let result = ForgeBuffer::create(
                    device,
                    memory_properties,
                    size,
                    usage,
                    vk::MemoryPropertyFlags::DEVICE_LOCAL,
                )?;
                let readback = ForgeBuffer::create(
                    device,
                    memory_properties,
                    size,
                    vk::BufferUsageFlags::TRANSFER_DST,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                )?;
                Ok(Self {
                    kind,
                    artifact: IngotArtifact::Buffer {
                        result,
                        readback,
                        bytes: ThinVec::new(),
                    },
                    save_path: save_path.clone(),
                })
            }
            IngotSpec::Image2d {
                width,
                height,
                format,
                byte_size,
                save_path,
            } => {
                let byte_size = non_zero_size(*byte_size);
                let result = ForgeImage::create_2d(
                    device,
                    memory_properties,
                    *width,
                    *height,
                    *format,
                    vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC,
                    vk::MemoryPropertyFlags::DEVICE_LOCAL,
                )?;
                let readback = ForgeBuffer::create(
                    device,
                    memory_properties,
                    byte_size,
                    vk::BufferUsageFlags::TRANSFER_DST,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                )?;
                Ok(Self {
                    kind,
                    artifact: IngotArtifact::Image2d {
                        result,
                        readback,
                        bytes: ThinVec::new(),
                        byte_size,
                    },
                    save_path: save_path.clone(),
                })
            }
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match &self.artifact {
            IngotArtifact::Buffer { bytes, .. } => bytes,
            IngotArtifact::Image2d { bytes, .. } => bytes,
        }
    }

    pub fn result_buffer(&self) -> Option<&ForgeBuffer> {
        match &self.artifact {
            IngotArtifact::Buffer { result, .. } => Some(result),
            IngotArtifact::Image2d { .. } => None,
        }
    }

    pub fn result_image(&self) -> Option<&ForgeImage> {
        match &self.artifact {
            IngotArtifact::Buffer { .. } => None,
            IngotArtifact::Image2d { result, .. } => Some(result),
        }
    }

    pub unsafe fn record_prepare_for_compute(
        &self,
        device: &ash::Device,
        command_buffer: vk::CommandBuffer,
    ) {
        if let IngotArtifact::Image2d { result, .. } = &self.artifact {
            let barrier = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(result.handle)
                .subresource_range(color_subresource_range());

            unsafe {
                device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
            }
        }
    }

    pub unsafe fn record_readback(&self, device: &ash::Device, command_buffer: vk::CommandBuffer) {
        match &self.artifact {
            IngotArtifact::Buffer {
                result, readback, ..
            } => {
                let barrier = storage_buffer_readback_barrier(result.handle, result.size);
                unsafe {
                    device.cmd_pipeline_barrier(
                        command_buffer,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[barrier],
                        &[],
                    );
                }
                let region = [vk::BufferCopy::default()
                    .src_offset(0)
                    .dst_offset(0)
                    .size(result.size)];
                unsafe {
                    device.cmd_copy_buffer(command_buffer, result.handle, readback.handle, &region)
                };
            }
            IngotArtifact::Image2d {
                result, readback, ..
            } => {
                let barrier = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(result.handle)
                    .subresource_range(color_subresource_range());

                unsafe {
                    device.cmd_pipeline_barrier(
                        command_buffer,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier],
                    );
                }

                let region = [vk::BufferImageCopy::default()
                    .buffer_offset(0)
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(0)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                    .image_extent(result.extent)];

                unsafe {
                    device.cmd_copy_image_to_buffer(
                        command_buffer,
                        result.handle,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        readback.handle,
                        &region,
                    );
                }
            }
        }
    }

    pub fn finish_readback(&mut self, device: &ash::Device) -> ForgeResult<()> {
        match &mut self.artifact {
            IngotArtifact::Buffer {
                result,
                readback,
                bytes,
            } => {
                *bytes = readback.read_bytes(device, result.size)?;
            }
            IngotArtifact::Image2d {
                readback,
                bytes,
                byte_size,
                ..
            } => {
                *bytes = readback.read_bytes(device, *byte_size)?;
            }
        }

        if let Some(path) = &self.save_path {
            self.save(path)?;
        }
        Ok(())
    }

    pub fn save(&self, path: impl AsRef<Path>) -> ForgeResult<()> {
        let mut file = File::create(path)?;
        file.write_all(INGOT_MAGIC)?;
        file.write_all(&INGOT_VERSION.to_le_bytes())?;
        file.write_all(&(self.kind.index() as u32).to_le_bytes())?;

        match &self.artifact {
            IngotArtifact::Buffer { result, bytes, .. } => {
                file.write_all(&0u32.to_le_bytes())?;
                file.write_all(&result.size.to_le_bytes())?;
                file.write_all(&0u32.to_le_bytes())?;
                file.write_all(&0u32.to_le_bytes())?;
                file.write_all(&0i32.to_le_bytes())?;
                file.write_all(bytes)?;
            }
            IngotArtifact::Image2d {
                result,
                byte_size,
                bytes,
                ..
            } => {
                file.write_all(&1u32.to_le_bytes())?;
                file.write_all(&byte_size.to_le_bytes())?;
                file.write_all(&result.extent.width.to_le_bytes())?;
                file.write_all(&result.extent.height.to_le_bytes())?;
                file.write_all(&result.format.as_raw().to_le_bytes())?;
                file.write_all(bytes)?;
            }
        }

        Ok(())
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        match &mut self.artifact {
            IngotArtifact::Buffer {
                result, readback, ..
            } => unsafe {
                result.destroy(device);
                readback.destroy(device);
            },
            IngotArtifact::Image2d {
                result, readback, ..
            } => unsafe {
                result.destroy(device);
                readback.destroy(device);
            },
        }
    }
}

fn color_subresource_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1)
}
