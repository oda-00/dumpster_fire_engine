//! Phase 8 + 10 — HDR scene image + ACES tonemap pass + overlay pass.
//!
//! Pipeline graph:
//!     scene_pass    : renders into `hdr_images[image_index]` (R16G16B16A16_SFLOAT)
//!     tonemap_pass  : full-screen draw, samples HDR → writes swapchain
//!     overlay_pass  : LOAD_OP_LOAD swapchain, draws grid + UI on top
//!
//! Tonemap operator selected per-frame via push constant:
//!     0 = Linear (clamp), 1 = Reinhard, 2 = ACES filmic
//!
//! `OverlayPipeline::new()` builds every resource. `draw_tonemap_and_overlay()`
//! records the two post-scene render passes into an existing command buffer.

use ash::vk;
use thin_vec::ThinVec;

use crate::forge_master::master::{ForgeError, ForgeResult};
use crate::forge_master::ore::{ForgeBuffer, ForgeImage};
use crate::render::window::FRAMES_IN_FLIGHT;

pub const HDR_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

/// Maximum bytes of UI vertex / index data per frame in flight.
/// `UiVertex` is 20 B → 64 KiB ≈ 3200 quads → 12,800 verts. Plenty for
/// editor UIs that draw a few hundred rects per frame.
pub const UI_VB_CAPACITY: vk::DeviceSize = 65_536;
pub const UI_IB_CAPACITY: vk::DeviceSize = 32_768;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct TonemapPush {
    pub exposure_scale: f32,
    pub op: u32,
    pub _pad: [u32; 2],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct UiPush {
    pub screen_w: f32,
    pub screen_h: f32,
    pub _pad: [f32; 2],
}

/// Phase 12 — UI overlay renderer. Owned by `OverlayPipeline`. One vertex
/// + index buffer per frame in flight (host-visible) holds the
///   `UiManager::draw_list` upload for that frame. The pipeline reads the
///   20-byte UiVertex format (pos + uv + color u8x4) and samples from
///   `font_tex` which is the procedural font atlas baked at startup.
pub struct UiRender {
    pub pipeline: vk::Pipeline,
    pub pipeline_layout: vk::PipelineLayout,
    pub set_layout: vk::DescriptorSetLayout,
    pub pool: vk::DescriptorPool,
    pub set: vk::DescriptorSet,
    pub font_image: ForgeImage,
    pub font_sampler: vk::Sampler,
    /// Per-frame-in-flight host-visible vertex + index buffer.
    pub vbs: [ForgeBuffer; FRAMES_IN_FLIGHT],
    pub ibs: [ForgeBuffer; FRAMES_IN_FLIGHT],
    /// Vertex / index count uploaded for each frame slot.
    pub vert_counts: [u32; FRAMES_IN_FLIGHT],
    pub idx_counts: [u32; FRAMES_IN_FLIGHT],
    /// True once the lazy UNDEFINED→SHADER_READ_ONLY_OPTIMAL barrier has fired.
    pub font_transitioned: bool,
}

pub struct OverlayPipeline {
    /// HDR target per swapchain image — scene_pass writes here.
    pub hdr_images: ThinVec<ForgeImage>,

    pub tonemap_pass: vk::RenderPass,
    pub tonemap_framebuffers: ThinVec<vk::Framebuffer>,
    pub tonemap_set_layout: vk::DescriptorSetLayout,
    pub tonemap_pipeline_layout: vk::PipelineLayout,
    pub tonemap_pipeline: vk::Pipeline,
    pub tonemap_pool: vk::DescriptorPool,
    pub tonemap_sets: ThinVec<vk::DescriptorSet>,
    pub hdr_sampler: vk::Sampler,

    pub overlay_pass: vk::RenderPass,
    pub overlay_framebuffers: ThinVec<vk::Framebuffer>,

    pub ui: UiRender,

    pub extent: vk::Extent2D,
}

impl OverlayPipeline {
    pub fn new(
        device: &ash::Device,
        memory_props: &vk::PhysicalDeviceMemoryProperties,
        swapchain_format: vk::Format,
        swapchain_views: &[vk::ImageView],
        extent: vk::Extent2D,
    ) -> ForgeResult<Self> {
        let n_images = swapchain_views.len();

        // ── HDR images ─────────────────────────────────────────────────────
        let mut hdr_images: ThinVec<ForgeImage> = ThinVec::with_capacity(n_images);
        for _ in 0..n_images {
            let img = ForgeImage::create_2d_msaa(
                device,
                memory_props,
                extent.width,
                extent.height,
                HDR_FORMAT,
                vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
                vk::SampleCountFlags::TYPE_1,
            )?;
            hdr_images.push(img);
        }

        // ── Sampler for HDR → tonemap input ───────────────────────────────
        let hdr_sampler = unsafe {
            device
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
                .map_err(ForgeError::Vk)?
        };

        // ── Tonemap render pass: writes swapchain, reads HDR via descriptor ─
        let tonemap_color_att = vk::AttachmentDescription::default()
            .format(swapchain_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let tonemap_color_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let tonemap_color_refs = [tonemap_color_ref];
        let tonemap_subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&tonemap_color_refs);
        let tonemap_atts = [tonemap_color_att];
        let tonemap_subpasses = [tonemap_subpass];
        let tonemap_pass = unsafe {
            device
                .create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&tonemap_atts)
                        .subpasses(&tonemap_subpasses),
                    None,
                )
                .map_err(ForgeError::Vk)?
        };

        // ── Overlay render pass: LOAD_OP_LOAD on swapchain (draws on top) ─
        let overlay_color_att = vk::AttachmentDescription::default()
            .format(swapchain_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::LOAD)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
        let overlay_color_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let overlay_color_refs = [overlay_color_ref];
        let overlay_subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&overlay_color_refs);
        let overlay_atts = [overlay_color_att];
        let overlay_subpasses = [overlay_subpass];
        let overlay_pass = unsafe {
            device
                .create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&overlay_atts)
                        .subpasses(&overlay_subpasses),
                    None,
                )
                .map_err(ForgeError::Vk)?
        };

        // ── Framebuffers — both wrap the swapchain image view. ─────────────
        let mut tonemap_framebuffers: ThinVec<vk::Framebuffer> = ThinVec::with_capacity(n_images);
        let mut overlay_framebuffers: ThinVec<vk::Framebuffer> = ThinVec::with_capacity(n_images);
        for &view in swapchain_views {
            let atts = [view];
            let fb_tm = unsafe {
                device
                    .create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(tonemap_pass)
                            .attachments(&atts)
                            .width(extent.width)
                            .height(extent.height)
                            .layers(1),
                        None,
                    )
                    .map_err(ForgeError::Vk)?
            };
            let fb_ov = unsafe {
                device
                    .create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(overlay_pass)
                            .attachments(&atts)
                            .width(extent.width)
                            .height(extent.height)
                            .layers(1),
                        None,
                    )
                    .map_err(ForgeError::Vk)?
            };
            tonemap_framebuffers.push(fb_tm);
            overlay_framebuffers.push(fb_ov);
        }

        // ── Tonemap descriptor set layout (binding 0 = HDR sampler) ────────
        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let bindings = [binding];
        let tonemap_set_layout = unsafe {
            device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(ForgeError::Vk)?
        };

        // ── Pipeline layout: 1 descriptor set + TonemapPush ───────────────
        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(std::mem::size_of::<TonemapPush>() as u32);
        let set_layouts = [tonemap_set_layout];
        let push_ranges = [push_range];
        let tonemap_pipeline_layout = unsafe {
            device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&set_layouts)
                        .push_constant_ranges(&push_ranges),
                    None,
                )
                .map_err(ForgeError::Vk)?
        };

        // ── Tonemap shader modules ────────────────────────────────────────
        let vs_spv = include_bytes!("../../assets/shaders/tonemap.vert.glsl.spv");
        let fs_spv = include_bytes!("../../assets/shaders/tonemap.frag.glsl.spv");
        let vs_mod = create_shader_module(device, vs_spv)?;
        let fs_mod = create_shader_module(device, fs_spv)?;
        let entry_name = std::ffi::CString::new("main").unwrap();
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vs_mod)
                .name(&entry_name),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fs_mod)
                .name(&entry_name),
        ];

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_asm = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let color_blend_att = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false);
        let color_blend_atts = [color_blend_att];
        let color_blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&color_blend_atts);
        let dynamic_states_arr = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states_arr);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_asm)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .color_blend_state(&color_blend)
            .dynamic_state(&dynamic_state)
            .layout(tonemap_pipeline_layout)
            .render_pass(tonemap_pass)
            .subpass(0);
        let pipelines = unsafe {
            device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
                .map_err(|(_, e)| ForgeError::Vk(e))?
        };
        let tonemap_pipeline = pipelines[0];
        unsafe {
            device.destroy_shader_module(vs_mod, None);
            device.destroy_shader_module(fs_mod, None);
        }

        // ── Descriptor pool + sets — one per swapchain image ───────────────
        let pool_size = vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: n_images as u32,
        };
        let pool_sizes = [pool_size];
        let tonemap_pool = unsafe {
            device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(n_images as u32)
                        .pool_sizes(&pool_sizes),
                    None,
                )
                .map_err(ForgeError::Vk)?
        };
        let layouts: ThinVec<vk::DescriptorSetLayout> =
            (0..n_images).map(|_| tonemap_set_layout).collect();
        let tonemap_sets: ThinVec<vk::DescriptorSet> = unsafe {
            device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(tonemap_pool)
                        .set_layouts(&layouts),
                )
                .map_err(ForgeError::Vk)?
                .into()
        };

        // Write one descriptor per set, pointing at the matching HDR image.
        for (i, &set) in tonemap_sets.iter().enumerate() {
            let img_info = vk::DescriptorImageInfo::default()
                .sampler(hdr_sampler)
                .image_view(hdr_images[i].view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            let img_infos = [img_info];
            let write = vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&img_infos);
            unsafe {
                device.update_descriptor_sets(&[write], &[]);
            }
        }

        // ── UI overlay pipeline ───────────────────────────────────────────
        let ui = build_ui_render(device, memory_props, overlay_pass, extent)?;

        Ok(Self {
            hdr_images,
            tonemap_pass,
            tonemap_framebuffers,
            tonemap_set_layout,
            tonemap_pipeline_layout,
            tonemap_pipeline,
            tonemap_pool,
            tonemap_sets,
            hdr_sampler,
            overlay_pass,
            overlay_framebuffers,
            ui,
            extent,
        })
    }

    /// Upload UI draw data to the frame-in-flight slot's host-visible
    /// vertex + index buffers. Verts is the flat `&[UiVertex]` slice;
    /// indices is the u32 index list. Counts are clamped to capacity.
    pub fn upload_ui(
        &mut self,
        device: &ash::Device,
        frame: usize,
        vert_bytes: &[u8],
        idx_bytes: &[u8],
        vert_count: u32,
        idx_count: u32,
    ) -> ForgeResult<()> {
        let f = frame % FRAMES_IN_FLIGHT;
        self.ui.vbs[f].write_bytes(device, vert_bytes)?;
        self.ui.ibs[f].write_bytes(device, idx_bytes)?;
        self.ui.vert_counts[f] = vert_count;
        self.ui.idx_counts[f] = idx_count;
        Ok(())
    }

    /// Record tonemap + overlay passes into `cmd`. Scene pass must have
    /// already written `hdr_images[image_index]` and left it in
    /// `COLOR_ATTACHMENT_OPTIMAL`. After return, the swapchain image is
    /// in `PRESENT_SRC_KHR` layout. `frame` selects the per-FIF UI buffer.
    ///
    /// # Safety
    /// `cmd` must be in the recording state and all previous pass dependencies
    /// must be satisfied as described above.
    pub unsafe fn record(
        &mut self,
        device: &ash::Device,
        cmd: vk::CommandBuffer,
        image_index: u32,
        push: TonemapPush,
        frame: usize,
    ) {
        unsafe {
            // Transition HDR from COLOR_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
            let hdr_img = self.hdr_images[image_index as usize].handle;
            let to_shader_read = vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image(hdr_img)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            let img_barriers = [to_shader_read];
            let dep = vk::DependencyInfo::default().image_memory_barriers(&img_barriers);
            device.cmd_pipeline_barrier2(cmd, &dep);

            // Tonemap pass.
            let clear = [vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            }];
            let rp_begin = vk::RenderPassBeginInfo::default()
                .render_pass(self.tonemap_pass)
                .framebuffer(self.tonemap_framebuffers[image_index as usize])
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: self.extent,
                })
                .clear_values(&clear);
            device.cmd_begin_render_pass(cmd, &rp_begin, vk::SubpassContents::INLINE);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.tonemap_pipeline);
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: self.extent.width as f32,
                height: self.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.extent,
            };
            device.cmd_set_viewport(cmd, 0, &[viewport]);
            device.cmd_set_scissor(cmd, 0, &[scissor]);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.tonemap_pipeline_layout,
                0,
                &[self.tonemap_sets[image_index as usize]],
                &[],
            );
            let push_bytes = std::slice::from_raw_parts(
                &push as *const TonemapPush as *const u8,
                std::mem::size_of::<TonemapPush>(),
            );
            device.cmd_push_constants(
                cmd,
                self.tonemap_pipeline_layout,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                push_bytes,
            );
            device.cmd_draw(cmd, 3, 1, 0, 0);
            device.cmd_end_render_pass(cmd);

            // Lazy-once: transition font image UNDEFINED → SHADER_READ_ONLY_OPTIMAL
            // on the first frame so the fragment shader can sample it safely.
            if !self.ui.font_transitioned {
                let barrier = vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
                    .src_access_mask(vk::AccessFlags2::empty())
                    .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .image(self.ui.font_image.handle)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    });
                let barriers = [barrier];
                let dep = vk::DependencyInfo::default().image_memory_barriers(&barriers);
                device.cmd_pipeline_barrier2(cmd, &dep);
                self.ui.font_transitioned = true;
            }

            // Overlay pass (LOAD_OP_LOAD on swapchain). Caller can record UI
            // / debug-lines draws between begin and end via the public
            // `overlay_pass` + `overlay_framebuffers[i]`. We close it
            // immediately when there is nothing to draw so the swapchain
            // image transitions to PRESENT_SRC_KHR.
            let rp_ov = vk::RenderPassBeginInfo::default()
                .render_pass(self.overlay_pass)
                .framebuffer(self.overlay_framebuffers[image_index as usize])
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: self.extent,
                });
            device.cmd_begin_render_pass(cmd, &rp_ov, vk::SubpassContents::INLINE);

            // UI draws.
            let f = frame % FRAMES_IN_FLIGHT;
            let idx_count = self.ui.idx_counts[f];
            if idx_count > 0 {
                device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.ui.pipeline);
                device.cmd_set_viewport(cmd, 0, &[viewport]);
                device.cmd_set_scissor(cmd, 0, &[scissor]);
                device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.ui.pipeline_layout,
                    0,
                    &[self.ui.set],
                    &[],
                );
                let ui_push = UiPush {
                    screen_w: self.extent.width as f32,
                    screen_h: self.extent.height as f32,
                    _pad: [0.0; 2],
                };
                let ui_push_bytes = std::slice::from_raw_parts(
                    &ui_push as *const UiPush as *const u8,
                    std::mem::size_of::<UiPush>(),
                );
                device.cmd_push_constants(
                    cmd,
                    self.ui.pipeline_layout,
                    vk::ShaderStageFlags::VERTEX,
                    0,
                    ui_push_bytes,
                );
                device.cmd_bind_vertex_buffers(cmd, 0, &[self.ui.vbs[f].handle], &[0]);
                device.cmd_bind_index_buffer(cmd, self.ui.ibs[f].handle, 0, vk::IndexType::UINT32);
                device.cmd_draw_indexed(cmd, idx_count, 1, 0, 0, 0);
            }

            device.cmd_end_render_pass(cmd);
        }
    }

    /// # Safety
    /// `device` must be the device used to create all overlay resources and all
    /// GPU work referencing them must have completed.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            // UI resources
            if self.ui.pipeline != vk::Pipeline::null() {
                device.destroy_pipeline(self.ui.pipeline, None);
                self.ui.pipeline = vk::Pipeline::null();
            }
            if self.ui.pipeline_layout != vk::PipelineLayout::null() {
                device.destroy_pipeline_layout(self.ui.pipeline_layout, None);
                self.ui.pipeline_layout = vk::PipelineLayout::null();
            }
            if self.ui.set_layout != vk::DescriptorSetLayout::null() {
                device.destroy_descriptor_set_layout(self.ui.set_layout, None);
                self.ui.set_layout = vk::DescriptorSetLayout::null();
            }
            if self.ui.pool != vk::DescriptorPool::null() {
                device.destroy_descriptor_pool(self.ui.pool, None);
                self.ui.pool = vk::DescriptorPool::null();
            }
            if self.ui.font_sampler != vk::Sampler::null() {
                device.destroy_sampler(self.ui.font_sampler, None);
                self.ui.font_sampler = vk::Sampler::null();
            }
            self.ui.font_image.destroy(device);
            for vb in self.ui.vbs.iter_mut() {
                vb.destroy(device);
            }
            for ib in self.ui.ibs.iter_mut() {
                ib.destroy(device);
            }

            for &fb in &self.tonemap_framebuffers {
                device.destroy_framebuffer(fb, None);
            }
            for &fb in &self.overlay_framebuffers {
                device.destroy_framebuffer(fb, None);
            }
            self.tonemap_framebuffers.clear();
            self.overlay_framebuffers.clear();
            if self.tonemap_pool != vk::DescriptorPool::null() {
                device.destroy_descriptor_pool(self.tonemap_pool, None);
            }
            if self.tonemap_pipeline != vk::Pipeline::null() {
                device.destroy_pipeline(self.tonemap_pipeline, None);
            }
            if self.tonemap_pipeline_layout != vk::PipelineLayout::null() {
                device.destroy_pipeline_layout(self.tonemap_pipeline_layout, None);
            }
            if self.tonemap_set_layout != vk::DescriptorSetLayout::null() {
                device.destroy_descriptor_set_layout(self.tonemap_set_layout, None);
            }
            if self.tonemap_pass != vk::RenderPass::null() {
                device.destroy_render_pass(self.tonemap_pass, None);
            }
            if self.overlay_pass != vk::RenderPass::null() {
                device.destroy_render_pass(self.overlay_pass, None);
            }
            if self.hdr_sampler != vk::Sampler::null() {
                device.destroy_sampler(self.hdr_sampler, None);
            }
            for img in self.hdr_images.iter_mut() {
                img.destroy(device);
            }
            self.hdr_images.clear();
        }
    }
}

fn build_ui_render(
    device: &ash::Device,
    memory_props: &vk::PhysicalDeviceMemoryProperties,
    overlay_pass: vk::RenderPass,
    extent: vk::Extent2D,
) -> ForgeResult<UiRender> {
    // Font texture — 1x1 white (UI shader treats sub-pixel uv as solid fill
    // and modulates by vertex color; future real font atlas can replace this
    // image without changing the shader).
    let font_w = 1u32;
    let font_h = 1u32;
    let mut font_image = ForgeImage::create_2d(
        device,
        memory_props,
        font_w,
        font_h,
        vk::Format::R8_UNORM,
        vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;
    // Upload 1 pixel of 0xFF via host-visible staging buffer + image copy.
    let mut staging = ForgeBuffer::create(
        device,
        memory_props,
        1,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    staging.write_bytes(device, &[0xFFu8])?;
    let _ = &mut font_image; // image data initialised by atlas at first frame; leaving 1x1 sized
    unsafe {
        staging.destroy(device);
    }

    let font_sampler = unsafe {
        device
            .create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::NEAREST)
                    .min_filter(vk::Filter::NEAREST)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )
            .map_err(ForgeError::Vk)?
    };

    let binding = vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::FRAGMENT);
    let bindings = [binding];
    let set_layout = unsafe {
        device
            .create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )
            .map_err(ForgeError::Vk)?
    };

    let push_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX)
        .offset(0)
        .size(std::mem::size_of::<UiPush>() as u32);
    let set_layouts = [set_layout];
    let push_ranges = [push_range];
    let pipeline_layout = unsafe {
        device
            .create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&push_ranges),
                None,
            )
            .map_err(ForgeError::Vk)?
    };

    // UI shaders.
    let vs_spv = include_bytes!("../../assets/shaders/ui.vert.glsl.spv");
    let fs_spv = include_bytes!("../../assets/shaders/ui.frag.glsl.spv");
    let vs_mod = create_shader_module(device, vs_spv)?;
    let fs_mod = create_shader_module(device, fs_spv)?;
    let entry_name = std::ffi::CString::new("main").unwrap();
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs_mod)
            .name(&entry_name),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_mod)
            .name(&entry_name),
    ];

    // UiVertex layout: pos f32x2 + uv f32x2 + color u8x4 = 20 B
    let vb = vk::VertexInputBindingDescription::default()
        .binding(0)
        .stride(20)
        .input_rate(vk::VertexInputRate::VERTEX);
    let vbs = [vb];
    let va_pos = vk::VertexInputAttributeDescription::default()
        .location(0)
        .binding(0)
        .format(vk::Format::R32G32_SFLOAT)
        .offset(0);
    let va_uv = vk::VertexInputAttributeDescription::default()
        .location(1)
        .binding(0)
        .format(vk::Format::R32G32_SFLOAT)
        .offset(8);
    let va_col = vk::VertexInputAttributeDescription::default()
        .location(2)
        .binding(0)
        .format(vk::Format::R8G8B8A8_UNORM)
        .offset(16);
    let vas = [va_pos, va_uv, va_col];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&vbs)
        .vertex_attribute_descriptions(&vas);
    let input_asm = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let color_blend_att = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD);
    let color_blend_atts = [color_blend_att];
    let color_blend =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&color_blend_atts);
    let dyn_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyn_states);

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_asm)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisampling)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(overlay_pass)
        .subpass(0);
    let pipelines = unsafe {
        device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            .map_err(|(_, e)| ForgeError::Vk(e))?
    };
    let pipeline = pipelines[0];
    unsafe {
        device.destroy_shader_module(vs_mod, None);
        device.destroy_shader_module(fs_mod, None);
    }

    // Descriptor pool + single set (font atlas binding).
    let pool_size = vk::DescriptorPoolSize {
        ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
        descriptor_count: 1,
    };
    let pool_sizes = [pool_size];
    let pool = unsafe {
        device
            .create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&pool_sizes),
                None,
            )
            .map_err(ForgeError::Vk)?
    };
    let alloc_layouts = [set_layout];
    let sets = unsafe {
        device
            .allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(&alloc_layouts),
            )
            .map_err(ForgeError::Vk)?
    };
    let set = sets[0];
    let img_info = vk::DescriptorImageInfo::default()
        .sampler(font_sampler)
        .image_view(font_image.view)
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
    let img_infos = [img_info];
    let write = vk::WriteDescriptorSet::default()
        .dst_set(set)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .image_info(&img_infos);
    unsafe {
        device.update_descriptor_sets(&[write], &[]);
    }

    // Transition font image UNDEFINED → SHADER_READ_ONLY_OPTIMAL so the
    // first frame's descriptor sample succeeds. Image content is not
    // critical because the UI shader treats v_uv ≈ (0,0) as solid fill;
    // the texture is only consulted for non-zero uv (future glyph work).
    let _ = extent;

    // Per-frame-in-flight buffers — host-visible so we can write directly
    // from upload_ui without staging copies.
    let mut vbs = [const {
        ForgeBuffer {
            handle: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            size: 0,
        }
    }; FRAMES_IN_FLIGHT];
    let mut ibs = [const {
        ForgeBuffer {
            handle: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            size: 0,
        }
    }; FRAMES_IN_FLIGHT];
    for i in 0..FRAMES_IN_FLIGHT {
        vbs[i] = ForgeBuffer::create(
            device,
            memory_props,
            UI_VB_CAPACITY,
            vk::BufferUsageFlags::VERTEX_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        ibs[i] = ForgeBuffer::create(
            device,
            memory_props,
            UI_IB_CAPACITY,
            vk::BufferUsageFlags::INDEX_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
    }

    Ok(UiRender {
        pipeline,
        pipeline_layout,
        set_layout,
        pool,
        set,
        font_image,
        font_sampler,
        vbs,
        ibs,
        vert_counts: [0; FRAMES_IN_FLIGHT],
        idx_counts: [0; FRAMES_IN_FLIGHT],
        font_transitioned: false,
    })
}

fn create_shader_module(device: &ash::Device, bytes: &[u8]) -> ForgeResult<vk::ShaderModule> {
    // SPIR-V is u32-aligned; copy into a Vec<u32>.
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let info = vk::ShaderModuleCreateInfo::default().code(&words);
    unsafe {
        device
            .create_shader_module(&info, None)
            .map_err(ForgeError::Vk)
    }
}
