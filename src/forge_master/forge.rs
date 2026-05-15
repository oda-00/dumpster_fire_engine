use ash::vk;
use std::ffi::CStr;
use std::io::Cursor;
use std::mem::size_of;

use crate::resource_manager::manager::{Handle, Id};

use super::ingot::{Ingot, IngotArtifact};
use super::master::{ForgeError, ForgeResult};
use thin_vec::ThinVec;

use super::ore::{ForgeVertex, GraphicsOreKind, OreKind, StagedOre};

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

// ── Graphics forge ─────────────────────────────────────────────────────────
//
// GraphicsForge is the rasterization analog of Forge: a `kind` (drives the
// descriptor-binding shape) plus owned vert/frag SPIR-V words. It does NOT
// own device-side pipeline objects — those live in `GraphicsMold`, produced
// by `compile()` once the target surface format + extent are known. Decoupling
// the bytecode from the mold lets one forge be re-compiled across resized or
// re-created swapchains without re-uploading the SPIR-V.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GraphicsForgeTag;
pub type GraphicsForgeHandle = Handle<GraphicsForgeTag>;

pub struct GraphicsForgeMarker;
pub type GraphicsForgeId = Id<GraphicsForgeMarker>;

#[derive(Debug)]
pub struct GraphicsForge {
    pub id: GraphicsForgeId,
    pub kind: GraphicsOreKind,
    vert_spirv: ThinVec<u32>,
    frag_spirv: ThinVec<u32>,
}

#[derive(Debug)]
pub struct GraphicsMold {
    pub render_pass: vk::RenderPass,
    pub descriptor_set_layout: vk::DescriptorSetLayout,
    pub pipeline_layout: vk::PipelineLayout,
    pub pipeline: vk::Pipeline,
}

impl GraphicsForge {
    pub fn from_spirv_bytes(
        id: GraphicsForgeId,
        kind: GraphicsOreKind,
        vert_spv: &[u8],
        frag_spv: &[u8],
    ) -> ForgeResult<Self> {
        let vert_words = ash::util::read_spv(&mut Cursor::new(vert_spv))?;
        let frag_words = ash::util::read_spv(&mut Cursor::new(frag_spv))?;
        Self::from_spirv_words(id, kind, &vert_words, &frag_words)
    }

    pub fn from_spirv_words(
        id: GraphicsForgeId,
        kind: GraphicsOreKind,
        vert: &[u32],
        frag: &[u32],
    ) -> ForgeResult<Self> {
        if vert.is_empty() || frag.is_empty() {
            return Err(ForgeError::EmptyShader {
                kind: OreKind::Graphics(kind),
            });
        }
        Ok(Self {
            id,
            kind,
            vert_spirv: vert.iter().copied().collect(),
            frag_spirv: frag.iter().copied().collect(),
        })
    }

    /// Descriptor bindings for this forge's kind. The compute path uses
    /// `STORAGE_BUFFER` everywhere; here `ForwardLit` exposes a camera UBO at
    /// binding 0 (vertex stage) and an actor-transform SSBO at binding 1
    /// (vertex stage). `Ui` has no descriptors — the shader generates
    /// vertices from `gl_VertexIndex` alone (perfect for the hello-triangle
    /// path, which baked positions into the shader).
    pub fn descriptor_bindings(&self) -> ThinVec<vk::DescriptorSetLayoutBinding<'static>> {
        match self.kind {
            GraphicsOreKind::ForwardLit => {
                let mut v = ThinVec::with_capacity(2);
                v.push(
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(0)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::VERTEX),
                );
                v.push(
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(1)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::VERTEX),
                );
                v
            }
            GraphicsOreKind::Ui => ThinVec::new(),
        }
    }

    /// Build the device-side mold (render pass + pipeline) against a concrete
    /// surface format and viewport extent. Caller takes ownership of the
    /// returned `GraphicsMold` and must call `mold.destroy(device)` before
    /// device destruction. Re-callable: produce a fresh mold every swapchain
    /// re-create.
    pub fn compile(
        &self,
        device: &ash::Device,
        color_format: vk::Format,
        depth_format: vk::Format,
    ) -> ForgeResult<GraphicsMold> {
        // Render pass — color + depth attachments, clear → present.
        let attachments = [
            vk::AttachmentDescription::default()
                .format(color_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR),
            vk::AttachmentDescription::default()
                .format(depth_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
        ];
        let color_refs = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let depth_ref = vk::AttachmentReference::default()
            .attachment(1)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_refs)
            .depth_stencil_attachment(&depth_ref)];
        let dependencies = [vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            )];
        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);
        let render_pass = unsafe { device.create_render_pass(&render_pass_info, None)? };

        // Descriptor set layout (may be empty).
        let bindings = self.descriptor_bindings();
        let descriptor_set_layout_info =
            vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let descriptor_set_layout = match unsafe {
            device.create_descriptor_set_layout(&descriptor_set_layout_info, None)
        } {
            Ok(l) => l,
            Err(e) => {
                unsafe { device.destroy_render_pass(render_pass, None) };
                return Err(ForgeError::Vk(e));
            }
        };

        // Pipeline layout — ForwardLit exposes a mat4 push constant (MVP).
        let set_layouts = [descriptor_set_layout];
        let push_ranges: &[vk::PushConstantRange] = match self.kind {
            GraphicsOreKind::ForwardLit => &[vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::VERTEX)
                .offset(0)
                .size(64)], // sizeof(mat4)
            GraphicsOreKind::Ui => &[],
        };
        let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(push_ranges);
        let pipeline_layout = match unsafe {
            device.create_pipeline_layout(&pipeline_layout_info, None)
        } {
            Ok(l) => l,
            Err(e) => {
                unsafe {
                    device.destroy_descriptor_set_layout(descriptor_set_layout, None);
                    device.destroy_render_pass(render_pass, None);
                }
                return Err(ForgeError::Vk(e));
            }
        };

        // Shader modules — destroyed before returning regardless of outcome.
        let vert_info = vk::ShaderModuleCreateInfo::default().code(&self.vert_spirv);
        let vert_module = match unsafe { device.create_shader_module(&vert_info, None) } {
            Ok(m) => m,
            Err(e) => {
                unsafe {
                    device.destroy_pipeline_layout(pipeline_layout, None);
                    device.destroy_descriptor_set_layout(descriptor_set_layout, None);
                    device.destroy_render_pass(render_pass, None);
                }
                return Err(ForgeError::Vk(e));
            }
        };
        let frag_info = vk::ShaderModuleCreateInfo::default().code(&self.frag_spirv);
        let frag_module = match unsafe { device.create_shader_module(&frag_info, None) } {
            Ok(m) => m,
            Err(e) => {
                unsafe {
                    device.destroy_shader_module(vert_module, None);
                    device.destroy_pipeline_layout(pipeline_layout, None);
                    device.destroy_descriptor_set_layout(descriptor_set_layout, None);
                    device.destroy_render_pass(render_pass, None);
                }
                return Err(ForgeError::Vk(e));
            }
        };

        let entry = CStr::from_bytes_with_nul(b"main\0").expect("static entry is nul-terminated");
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(entry),
        ];

        // Vertex input — ForwardLit reads ForgeVertex from binding 0;
        // Ui generates verts from gl_VertexIndex (no buffer needed).
        let binding_descs: &[vk::VertexInputBindingDescription] = match self.kind {
            GraphicsOreKind::ForwardLit => &[vk::VertexInputBindingDescription::default()
                .binding(0)
                .stride(size_of::<ForgeVertex>() as u32) // 48 bytes
                .input_rate(vk::VertexInputRate::VERTEX)],
            GraphicsOreKind::Ui => &[],
        };
        // ForgeVertex layout: position[0..12], normal[12..24], tangent[24..40], uv[40..48]
        let attr_descs: &[vk::VertexInputAttributeDescription] = match self.kind {
            GraphicsOreKind::ForwardLit => &[
                vk::VertexInputAttributeDescription::default()
                    .location(0).binding(0)
                    .format(vk::Format::R32G32B32_SFLOAT).offset(0),
                vk::VertexInputAttributeDescription::default()
                    .location(1).binding(0)
                    .format(vk::Format::R32G32B32_SFLOAT).offset(12),
                vk::VertexInputAttributeDescription::default()
                    .location(2).binding(0)
                    .format(vk::Format::R32G32B32A32_SFLOAT).offset(24),
                vk::VertexInputAttributeDescription::default()
                    .location(3).binding(0)
                    .format(vk::Format::R32G32_SFLOAT).offset(40),
            ],
            GraphicsOreKind::Ui => &[],
        };
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(binding_descs)
            .vertex_attribute_descriptions(attr_descs);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        // Dynamic viewport + scissor — pipeline doesn't bake in extent,
        // so swapchain resize doesn't require pipeline rebuild.
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&dynamic_states);
        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE);
        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let color_blend_attachments = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false)];
        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .attachments(&color_blend_attachments);

        // Depth-stencil: enabled for ForwardLit, disabled for Ui.
        let depth_stencil = match self.kind {
            GraphicsOreKind::ForwardLit => vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(true)
                .depth_write_enable(true)
                .depth_compare_op(vk::CompareOp::LESS)
                .depth_bounds_test_enable(false)
                .stencil_test_enable(false),
            GraphicsOreKind::Ui => vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(false)
                .depth_write_enable(false)
                .stencil_test_enable(false),
        };

        let pipeline_create = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend_state)
            .dynamic_state(&dynamic_state)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);

        let pipeline = unsafe {
            match device.create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[pipeline_create],
                None,
            ) {
                Ok(mut created) => created.remove(0),
                Err((mut created, err)) => {
                    for p in created.drain(..) {
                        if p != vk::Pipeline::null() {
                            device.destroy_pipeline(p, None);
                        }
                    }
                    device.destroy_shader_module(frag_module, None);
                    device.destroy_shader_module(vert_module, None);
                    device.destroy_pipeline_layout(pipeline_layout, None);
                    device.destroy_descriptor_set_layout(descriptor_set_layout, None);
                    device.destroy_render_pass(render_pass, None);
                    return Err(ForgeError::Vk(err));
                }
            }
        };

        unsafe {
            device.destroy_shader_module(frag_module, None);
            device.destroy_shader_module(vert_module, None);
        }

        Ok(GraphicsMold {
            render_pass,
            descriptor_set_layout,
            pipeline_layout,
            pipeline,
        })
    }
}

impl GraphicsMold {
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.pipeline != vk::Pipeline::null() {
                device.destroy_pipeline(self.pipeline, None);
                self.pipeline = vk::Pipeline::null();
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                device.destroy_pipeline_layout(self.pipeline_layout, None);
                self.pipeline_layout = vk::PipelineLayout::null();
            }
            if self.descriptor_set_layout != vk::DescriptorSetLayout::null() {
                device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
                self.descriptor_set_layout = vk::DescriptorSetLayout::null();
            }
            if self.render_pass != vk::RenderPass::null() {
                device.destroy_render_pass(self.render_pass, None);
                self.render_pass = vk::RenderPass::null();
            }
        }
    }
}
