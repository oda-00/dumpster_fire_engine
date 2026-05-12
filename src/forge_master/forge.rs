use ash::vk;
use std::ffi::CStr;
use std::io::Cursor;

use crate::resource_manager::manager::{Handle, Id};

use super::ingot::{Ingot, IngotArtifact};
use super::master::{ForgeError, ForgeResult};
use super::ore::{OreKind, StagedOre};

pub const ORE_PRIMARY_BINDING: u32 = 0;
pub const ORE_SECONDARY_BINDING: u32 = 1;
pub const INGOT_BUFFER_BINDING: u32 = 2;
pub const INGOT_IMAGE_BINDING: u32 = 3;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ForgeTag;
pub type ForgeHandle = Handle<ForgeTag>;

pub struct ForgeMarker;
pub type ForgeId = Id<ForgeMarker>;

#[derive(Debug)]
pub struct Forge {
    pub id: ForgeId,
    pub kind: OreKind,
    mold: ForgeMold,
}

#[derive(Debug)]
struct ForgeMold {
    descriptor_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    compute: vk::Pipeline,
}

impl Forge {
    pub fn from_spirv_bytes(
        device: &ash::Device,
        id: ForgeId,
        kind: OreKind,
        spirv: &[u8],
    ) -> ForgeResult<Self> {
        let mut cursor = Cursor::new(spirv);
        let words = ash::util::read_spv(&mut cursor)?;
        Self::from_spirv_words(device, id, kind, &words)
    }

    pub fn from_spirv_words(
        device: &ash::Device,
        id: ForgeId,
        kind: OreKind,
        spirv: &[u32],
    ) -> ForgeResult<Self> {
        if spirv.is_empty() {
            return Err(ForgeError::EmptyShader { kind });
        }

        let bindings = [
            layout_binding(ORE_PRIMARY_BINDING, vk::DescriptorType::STORAGE_BUFFER),
            layout_binding(ORE_SECONDARY_BINDING, vk::DescriptorType::STORAGE_BUFFER),
            layout_binding(INGOT_BUFFER_BINDING, vk::DescriptorType::STORAGE_BUFFER),
            layout_binding(INGOT_IMAGE_BINDING, vk::DescriptorType::STORAGE_IMAGE),
        ];
        let descriptor_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let descriptor_layout =
            unsafe { device.create_descriptor_set_layout(&descriptor_info, None)? };

        let set_layouts = [descriptor_layout];
        let layout_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
        let layout = unsafe { device.create_pipeline_layout(&layout_info, None)? };

        let shader_info = vk::ShaderModuleCreateInfo::default().code(spirv);
        let shader = unsafe { device.create_shader_module(&shader_info, None)? };

        let entry =
            CStr::from_bytes_with_nul(b"main\0").expect("static shader entry is nul-terminated");
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(shader)
            .name(entry);
        let create_info = vk::ComputePipelineCreateInfo::default()
            .stage(stage)
            .layout(layout);

        let compute = unsafe {
            match device.create_compute_pipelines(vk::PipelineCache::null(), &[create_info], None) {
                Ok(mut created) => created.remove(0),
                Err((mut created, err)) => {
                    for pipeline in created.drain(..) {
                        if pipeline != vk::Pipeline::null() {
                            device.destroy_pipeline(pipeline, None);
                        }
                    }
                    device.destroy_shader_module(shader, None);
                    device.destroy_pipeline_layout(layout, None);
                    device.destroy_descriptor_set_layout(descriptor_layout, None);
                    return Err(ForgeError::Vk(err));
                }
            }
        };

        unsafe { device.destroy_shader_module(shader, None) };

        Ok(Self {
            id,
            kind,
            mold: ForgeMold {
                descriptor_layout,
                layout,
                compute,
            },
        })
    }

    pub fn descriptor_layout(&self) -> vk::DescriptorSetLayout {
        self.mold.descriptor_layout
    }

    pub unsafe fn record_dispatch(
        &self,
        device: &ash::Device,
        command_buffer: vk::CommandBuffer,
        descriptor_set: vk::DescriptorSet,
        workgroups: [u32; 3],
    ) {
        unsafe {
            device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.mold.compute,
            );
            device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.mold.layout,
                0,
                &[descriptor_set],
                &[],
            );
            device.cmd_dispatch(
                command_buffer,
                workgroups[0].max(1),
                workgroups[1].max(1),
                workgroups[2].max(1),
            );
        }
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.mold.compute != vk::Pipeline::null() {
                device.destroy_pipeline(self.mold.compute, None);
                self.mold.compute = vk::Pipeline::null();
            }
            if self.mold.layout != vk::PipelineLayout::null() {
                device.destroy_pipeline_layout(self.mold.layout, None);
                self.mold.layout = vk::PipelineLayout::null();
            }
            if self.mold.descriptor_layout != vk::DescriptorSetLayout::null() {
                device.destroy_descriptor_set_layout(self.mold.descriptor_layout, None);
                self.mold.descriptor_layout = vk::DescriptorSetLayout::null();
            }
        }
    }
}

pub fn write_forge_descriptors(
    device: &ash::Device,
    descriptor_set: vk::DescriptorSet,
    ore: &StagedOre,
    ingot: &Ingot,
) {
    let primary = [vk::DescriptorBufferInfo::default()
        .buffer(ore.primary.handle)
        .offset(0)
        .range(ore.primary.size)];
    let secondary = [vk::DescriptorBufferInfo::default()
        .buffer(ore.secondary.handle)
        .offset(0)
        .range(ore.secondary.size)];

    let mut writes = vec![
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(ORE_PRIMARY_BINDING)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&primary),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(ORE_SECONDARY_BINDING)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&secondary),
    ];

    match &ingot.artifact {
        IngotArtifact::Buffer { result, .. } => {
            let result_info = [vk::DescriptorBufferInfo::default()
                .buffer(result.handle)
                .offset(0)
                .range(result.size)];
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_set)
                    .dst_binding(INGOT_BUFFER_BINDING)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&result_info),
            );
            unsafe { device.update_descriptor_sets(&writes, &[]) };
        }
        IngotArtifact::Image2d { result, .. } => {
            let result_info = [vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::GENERAL)
                .image_view(result.view)];
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_set)
                    .dst_binding(INGOT_IMAGE_BINDING)
                    .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                    .image_info(&result_info),
            );
            unsafe { device.update_descriptor_sets(&writes, &[]) };
        }
    }
}

fn layout_binding(binding: u32, ty: vk::DescriptorType) -> vk::DescriptorSetLayoutBinding<'static> {
    vk::DescriptorSetLayoutBinding::default()
        .binding(binding)
        .descriptor_type(ty)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::COMPUTE)
}
