use std::ffi::CStr;
use std::sync::Arc;
use ash::vk;
use ash::vk::Handle;

use glam::{Affine3A, Mat4};
use thin_vec::ThinVec;
use crate::forge_master::{ForgeError, ForgeMaster, ForgeResult};
use crate::resource_manager::manager::{Handle as ResourceHandle, Id};

use super::factory_master::{FactoryHandle, FactoryMaster, Proto};

// ── Handle / Id types ──────────────────────────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct WindowTag;
pub type WindowHandle = ResourceHandle<WindowTag>;

pub struct WindowMarker;
pub type WindowId = Id<WindowMarker>;

// ── Graphics plumbing ───────────────────────────────────────────────────────

pub struct GraphicsState {
    // SDL2
    pub sdl: sdl2::Sdl,
    pub sdl_video: sdl2::VideoSubsystem,
    pub sdl_window: sdl2::video::Window,

    // surface / swapchain
    pub surface: vk::SurfaceKHR,
    pub surface_loader: ash::khr::surface::Instance,
    pub swapchain: vk::SwapchainKHR,
    pub swapchain_images: ThinVec<vk::Image>,
    pub swapchain_image_views: ThinVec<vk::ImageView>,
    pub swapchain_extent: vk::Extent2D,
    pub swapchain_format: vk::Format,
    pub swapchain_loader: ash::khr::swapchain::Device,

    // assembly line (graphics pipeline)
    pub render_pass: vk::RenderPass,
    pub assembly_line: vk::Pipeline,
    pub assembly_line_layout: vk::PipelineLayout,
    pub descriptor_pool: vk::DescriptorPool,
    pub descriptor_set: vk::DescriptorSet,

    // GPU buffers
    pub actor_ssbo: vk::Buffer,
    pub actor_ssbo_memory: vk::DeviceMemory,
    pub camera_ubo: vk::Buffer,
    pub camera_ubo_memory: vk::DeviceMemory,

    // command recording
    pub command_pool: vk::CommandPool,
    pub command_buffers: ThinVec<vk::CommandBuffer>,
    pub framebuffers: ThinVec<vk::Framebuffer>,

    // synchronisation
    pub image_available_semaphore: vk::Semaphore,
    pub render_finished_semaphore: vk::Semaphore,
    pub in_flight_fence: vk::Fence,
}

// ── Window ─────────────────────────────────────────────────────────────────

pub struct Window {
    pub id: WindowId,
    pub name: Arc<str>,
    pub width: u32,
    pub height: u32,
    pub factory_master: FactoryMaster,
    pub should_close: bool,
    pub graphics: Option<GraphicsState>,
}

impl Window {
    /// Logical window without a GPU surface.
    pub fn new(id: WindowId, name: impl Into<Arc<str>>, width: u32, height: u32) -> Self {
        Self {
            id,
            name: name.into(),
            width,
            height,
            factory_master: FactoryMaster::new(),
            should_close: false,
            graphics: None,
        }
    }

    /// Fully initialised graphics window (SDL2 + Vulkan).
    pub fn new_with_surface(
        id: WindowId,
        name: impl Into<Arc<str>>,
        width: u32,
        height: u32,
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        graphics_queue: vk::Queue,
        graphics_queue_family: u32,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        entry: &ash::Entry,
        
    ) -> ForgeResult<Self> {
        let title: Arc<str> = name.into();

        // SDL2
        let sdl = sdl2::init().map_err(|e| {
            ForgeError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
        })?;
        let sdl_video = sdl.video().map_err(|e| {
            ForgeError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
        })?;
        let sdl_window = sdl_video
            .window(&title, width, height)
            .position_centered()
            .vulkan()
            .build()
            .map_err(|e| {
                ForgeError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
            })?;

        // Surface via SDL2
       

        let surface_raw = unsafe {
             sdl_window
                 .vulkan_create_surface(instance.handle().as_raw() as _)
                  .map_err(|e| ForgeError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?
          };
         let surface = vk::SurfaceKHR::from_raw(surface_raw as u64);


        // Swapchain
        let surface_loader = ash::khr::surface::Instance::new(&entry, instance);
        let caps = unsafe {
            surface_loader
                .get_physical_device_surface_capabilities(physical_device, surface)
                .map_err(ForgeError::Vk)?
        };
        let formats = unsafe {
            surface_loader
                .get_physical_device_surface_formats(physical_device, surface)
                .map_err(ForgeError::Vk)?
        };
        let present_modes = unsafe {
            surface_loader
                .get_physical_device_surface_present_modes(physical_device, surface)
                .map_err(ForgeError::Vk)?
        };

        let swapchain_extent = vk::Extent2D {
            width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
            height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
        };
        let swapchain_format = formats[0].format;
        let present_mode = if present_modes.contains(&vk::PresentModeKHR::MAILBOX) {
            vk::PresentModeKHR::MAILBOX
        } else {
            vk::PresentModeKHR::FIFO
        };

        let swapchain_loader = ash::khr::swapchain::Device::new(instance, device);
        let swapchain = unsafe {
            swapchain_loader
                .create_swapchain(
                    &vk::SwapchainCreateInfoKHR::default()
                        .surface(surface)
                        .min_image_count(2)
                        .image_format(swapchain_format)
                        .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                        .image_extent(swapchain_extent)
                        .image_array_layers(1)
                        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                        .pre_transform(caps.current_transform)
                        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                        .present_mode(present_mode)
                        .clipped(true),
                    None,
                )
                .map_err(ForgeError::Vk)?
        };

         let swapchain_images: ThinVec<vk::Image> = unsafe {
                swapchain_loader
                    .get_swapchain_images(swapchain)
                    .map_err(ForgeError::Vk)?
                    .into()
            };

        let swapchain_image_views: ThinVec<vk::ImageView> = swapchain_images
            .iter()
            .map(|&img| {
                unsafe {
                    device.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(img)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(swapchain_format)
                            .subresource_range(vk::ImageSubresourceRange {
                                aspect_mask: vk::ImageAspectFlags::COLOR,
                                base_mip_level: 0,
                                level_count: 1,
                                base_array_layer: 0,
                                layer_count: 1,
                            }),
                        None,
                    )
                }
            })
            .collect::<Result<ThinVec<_>, _>>()
            .map_err(ForgeError::Vk)?;

        // Render pass
        let render_pass = unsafe {
            device.create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(&[vk::AttachmentDescription::default()
                        .format(swapchain_format)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .load_op(vk::AttachmentLoadOp::CLEAR)
                        .store_op(vk::AttachmentStoreOp::STORE)
                        .initial_layout(vk::ImageLayout::UNDEFINED)
                        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)])
                    .subpasses(&[vk::SubpassDescription::default()
                        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                        .color_attachments(&[vk::AttachmentReference::default()
                            .attachment(0)
                            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)])]),
                None,
            ).map_err(ForgeError::Vk)?
        };

        // Framebuffers
        let framebuffers: ThinVec<vk::Framebuffer> = swapchain_image_views
            .iter()
            .map(|&view| {
                unsafe {
                    device.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(render_pass)
                            .attachments(&[view])
                            .width(swapchain_extent.width)
                            .height(swapchain_extent.height)
                            .layers(1),
                        None,
                    )
                }
            })
            .collect::<Result<ThinVec<_>, _>>()
            .map_err(ForgeError::Vk)?;

        // Descriptor set layout
        let descriptor_set_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&[
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(0)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::VERTEX),
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(1)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::VERTEX),
                ]),
                None,
            ).map_err(ForgeError::Vk)?
        };

        let assembly_line_layout = unsafe {
            device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&[descriptor_set_layout]),
                None,
            ).map_err(ForgeError::Vk)?
        };

        // Shaders (embedded minimal valid SPIR‑V)
        let vert_spirv = MINIMAL_VERT_SPV;
        let frag_spirv = MINIMAL_FRAG_SPV;

        let vert_module = create_shader_module(device, vert_spirv)?;
        let frag_module = create_shader_module(device, frag_spirv)?;

        let entry_name = CStr::from_bytes_with_nul(b"main\0").unwrap();
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(entry_name),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(entry_name),
        ];

        // Graphics pipeline
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: swapchain_extent.width as f32,
            height: swapchain_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: swapchain_extent,
        };
        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);
        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::NONE);
        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let color_blend = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false);

        let color_attachments = [color_blend];
        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
                .attachments(&color_attachments);

        let assembly_line = unsafe {
            device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::GraphicsPipelineCreateInfo::default()
                        .stages(&stages)
                        .vertex_input_state(&vertex_input)
                        .input_assembly_state(&input_assembly)
                        .viewport_state(&viewport_state)
                        .rasterization_state(&rasterizer)
                        .multisample_state(&multisampling)
                        .color_blend_state(&color_blend_state)
                        .layout(assembly_line_layout)
                        .render_pass(render_pass)
                        .subpass(0)],
                    None,
                )
                .map_err(|(_, e)| ForgeError::Vk(e))?[0]
        };
        unsafe {
            device.destroy_shader_module(vert_module, None);
            device.destroy_shader_module(frag_module, None);
        }

        // Buffers (pre‑allocated for up to 10k actors)
        let max_actors = 10000u64;
        let ssbo_size = max_actors * 64; // 4x4 matrix
        let (actor_ssbo, actor_ssbo_memory) = create_buffer(
            device,
            memory_properties,
            ssbo_size,
            vk::BufferUsageFlags::STORAGE_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let (camera_ubo, camera_ubo_memory) = create_buffer(
            device,
            memory_properties,
            64,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;

        // Descriptor pool + set
        let descriptor_pool = unsafe {
            device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&[
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::UNIFORM_BUFFER,
                            descriptor_count: 1,
                        },
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::STORAGE_BUFFER,
                            descriptor_count: 1,
                        },
                    ]),
                None,
            ).map_err(ForgeError::Vk)?
        };
        let descriptor_set = unsafe {
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(descriptor_pool)
                    .set_layouts(&[descriptor_set_layout]),
            ).map_err(ForgeError::Vk)?[0]
        };

        let camera_info = vk::DescriptorBufferInfo::default()
            .buffer(camera_ubo)
            .offset(0)
            .range(64);
        let ssbo_info = vk::DescriptorBufferInfo::default()
            .buffer(actor_ssbo)
            .offset(0)
            .range(ssbo_size);
        unsafe {
            device.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(descriptor_set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .buffer_info(&[camera_info]),
                    vk::WriteDescriptorSet::default()
                        .dst_set(descriptor_set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(&[ssbo_info]),
                ],
                &[],
            );
        }
        unsafe { device.destroy_descriptor_set_layout(descriptor_set_layout, None) };

        // Command pool + buffers
        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(graphics_queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            ).map_err(ForgeError::Vk)?
        };
        let command_buffers: ThinVec<vk::CommandBuffer> = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(swapchain_images.len() as u32),
            ).map_err(ForgeError::Vk)?.into()
        };

        // Synchronisation
        let image_available_semaphore = unsafe {
            device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?
        };
        let render_finished_semaphore = unsafe {
            device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?
        };
        let in_flight_fence = unsafe {
            device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            ).map_err(ForgeError::Vk)?
        };

        Ok(Self {
            id,
            name: title,
            width,
            height,
            factory_master: FactoryMaster::new(),
            should_close: false,
            graphics: Some(GraphicsState {
                sdl,
                sdl_video,
                sdl_window,
                surface,
                surface_loader,
                swapchain,
                swapchain_images,
                swapchain_image_views,
                swapchain_extent,
                swapchain_format,
                swapchain_loader,
                render_pass,
                assembly_line,
                assembly_line_layout,
                descriptor_pool,
                descriptor_set,
                actor_ssbo,
                actor_ssbo_memory,
                camera_ubo,
                camera_ubo_memory,
                command_pool,
                command_buffers,
                framebuffers,
                image_available_semaphore,
                render_finished_semaphore,
                in_flight_fence,
            }),
        })
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

    /// Destroy all Vulkan resources (if any) and the factory master.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        if let Some(ref mut gfx) = self.graphics {
            if gfx.command_pool != vk::CommandPool::null() {
                device.destroy_command_pool(gfx.command_pool, None);
                gfx.command_pool = vk::CommandPool::null();
            }
            if gfx.assembly_line != vk::Pipeline::null() {
                device.destroy_pipeline(gfx.assembly_line, None);
                gfx.assembly_line = vk::Pipeline::null();
            }
            if gfx.assembly_line_layout != vk::PipelineLayout::null() {
                device.destroy_pipeline_layout(gfx.assembly_line_layout, None);
                gfx.assembly_line_layout = vk::PipelineLayout::null();
            }
            if gfx.render_pass != vk::RenderPass::null() {
                device.destroy_render_pass(gfx.render_pass, None);
                gfx.render_pass = vk::RenderPass::null();
            }
            if gfx.descriptor_pool != vk::DescriptorPool::null() {
                device.destroy_descriptor_pool(gfx.descriptor_pool, None);
                gfx.descriptor_pool = vk::DescriptorPool::null();
            }
            if gfx.actor_ssbo != vk::Buffer::null() {
                device.destroy_buffer(gfx.actor_ssbo, None);
                gfx.actor_ssbo = vk::Buffer::null();
            }
            if gfx.actor_ssbo_memory != vk::DeviceMemory::null() {
                device.free_memory(gfx.actor_ssbo_memory, None);
                gfx.actor_ssbo_memory = vk::DeviceMemory::null();
            }
            if gfx.camera_ubo != vk::Buffer::null() {
                device.destroy_buffer(gfx.camera_ubo, None);
                gfx.camera_ubo = vk::Buffer::null();
            }
            if gfx.camera_ubo_memory != vk::DeviceMemory::null() {
                device.free_memory(gfx.camera_ubo_memory, None);
                gfx.camera_ubo_memory = vk::DeviceMemory::null();
            }
            for fb in gfx.framebuffers.iter() {
                if *fb != vk::Framebuffer::null() {
                    device.destroy_framebuffer(*fb, None);
                }
            }
            gfx.framebuffers.clear();
            for iv in gfx.swapchain_image_views.iter() {
                if *iv != vk::ImageView::null() {
                    device.destroy_image_view(*iv, None);
                }
            }
            gfx.swapchain_image_views.clear();
            if gfx.swapchain != vk::SwapchainKHR::null() {
                gfx.swapchain_loader.destroy_swapchain(gfx.swapchain, None);
                gfx.swapchain = vk::SwapchainKHR::null();
            }
            if gfx.surface != vk::SurfaceKHR::null() {
                gfx.surface_loader.destroy_surface(gfx.surface, None);
                gfx.surface = vk::SurfaceKHR::null();
            }
            if gfx.image_available_semaphore != vk::Semaphore::null() {
                device.destroy_semaphore(gfx.image_available_semaphore, None);
                gfx.image_available_semaphore = vk::Semaphore::null();
            }
            if gfx.render_finished_semaphore != vk::Semaphore::null() {
                device.destroy_semaphore(gfx.render_finished_semaphore, None);
                gfx.render_finished_semaphore = vk::Semaphore::null();
            }
            if gfx.in_flight_fence != vk::Fence::null() {
                device.destroy_fence(gfx.in_flight_fence, None);
                gfx.in_flight_fence = vk::Fence::null();
            }
        }
        unsafe { self.factory_master.destroy(device) };
    }

    /// Draw one frame of instanced actors.
    pub unsafe fn draw_frame(
        &mut self,
        device: &ash::Device,
        queue: vk::Queue,
        transforms: &[Affine3A],
        camera_view_proj: Mat4,
    ) -> ForgeResult<()> {
        let gfx = self
            .graphics
            .as_mut()
            .expect("draw_frame called on a headless window");

        let instance_count = transforms.len() as u32;
        if instance_count == 0 {
            return Ok(());
        }

        // Wait for in‑flight fence
        unsafe {
            device
                .wait_for_fences(&[gfx.in_flight_fence], true, u64::MAX)
                .map_err(ForgeError::Vk)?;
            device
                .reset_fences(&[gfx.in_flight_fence])
                .map_err(ForgeError::Vk)?;
        }

        // Acquire next swapchain image
        let (image_index, _) = unsafe {
            gfx.swapchain_loader
                .acquire_next_image(
                    gfx.swapchain,
                    u64::MAX,
                    gfx.image_available_semaphore,
                    vk::Fence::null(),
                )
                .map_err(ForgeError::Vk)?
        };
        let command_buffer = gfx.command_buffers[image_index as usize];

        // Upload camera UBO
        let camera_bytes = unsafe {
            std::slice::from_raw_parts(
                &camera_view_proj as *const Mat4 as *const u8,
                64,
            )
        };
        unsafe {
            let ptr = device
                .map_memory(gfx.camera_ubo_memory, 0, 64, vk::MemoryMapFlags::empty())
                .map_err(ForgeError::Vk)?;
            std::ptr::copy_nonoverlapping(camera_bytes.as_ptr(), ptr.cast(), 64);
            device.unmap_memory(gfx.camera_ubo_memory);
        }

        // Upload actor transforms to SSBO
        let ssbo_bytes = unsafe {
            std::slice::from_raw_parts(
                transforms.as_ptr() as *const u8,
                instance_count as usize * 64,
            )
        };
        let ssbo_size = (instance_count as u64) * 64;
        unsafe {
            let ptr = device
                .map_memory(gfx.actor_ssbo_memory, 0, ssbo_size, vk::MemoryMapFlags::empty())
                .map_err(ForgeError::Vk)?;
            std::ptr::copy_nonoverlapping(ssbo_bytes.as_ptr(), ptr.cast(), ssbo_bytes.len());
            device.unmap_memory(gfx.actor_ssbo_memory);
        }

        // Record command buffer
        let clear_color = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.1, 0.1, 0.15, 1.0],
            },
        };
        let clear_colors = [clear_color];
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        let render_pass_begin = vk::RenderPassBeginInfo::default()
            .render_pass(gfx.render_pass)
            .framebuffer(gfx.framebuffers[image_index as usize])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: gfx.swapchain_extent,
            })
            .clear_values(&clear_colors);
        unsafe {
            device
                .begin_command_buffer(command_buffer, &begin_info)
                .map_err(ForgeError::Vk)?;
            device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_begin,
                vk::SubpassContents::INLINE,
            );
            device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                gfx.assembly_line,
            );
            device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                gfx.assembly_line_layout,
                0,
                &[gfx.descriptor_set],
                &[],
            );
            device.cmd_draw(command_buffer, 4, instance_count, 0, 0);
            device.cmd_end_render_pass(command_buffer);
            device.end_command_buffer(command_buffer).map_err(ForgeError::Vk)?;
        }

        // Submit + present
        let wait_semaphores = [gfx.image_available_semaphore];
        let signal_semaphores = [gfx.render_finished_semaphore];
        let cmd_buffers = [command_buffer];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&cmd_buffers)
            .signal_semaphores(&signal_semaphores);

        unsafe {
            device
                .queue_submit(queue, &[submit_info], gfx.in_flight_fence)
                .map_err(ForgeError::Vk)?;

            let present_wait_semaphores = [gfx.render_finished_semaphore];
            let present_swapchains = [gfx.swapchain];
            let present_image_indices = [image_index];
            let present_info = vk::PresentInfoKHR::default()
                .wait_semaphores(&present_wait_semaphores)
                .swapchains(&present_swapchains)
                .image_indices(&present_image_indices);

            gfx.swapchain_loader
                .queue_present(queue, &present_info)
                .map_err(ForgeError::Vk)?;
        }
        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn create_shader_module(device: &ash::Device, spirv: &[u32]) -> ForgeResult<vk::ShaderModule> {
    let create_info = vk::ShaderModuleCreateInfo::default().code(spirv);
    unsafe { device.create_shader_module(&create_info, None).map_err(ForgeError::Vk) }
}

fn create_buffer(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    size: u64,
    usage: vk::BufferUsageFlags,
    flags: vk::MemoryPropertyFlags,
) -> ForgeResult<(vk::Buffer, vk::DeviceMemory)> {
    let buf_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = unsafe { device.create_buffer(&buf_info, None).map_err(ForgeError::Vk)? };
    let mem_req = unsafe { device.get_buffer_memory_requirements(buffer) };
    let mem_type = find_memory_type(mem_props, mem_req.memory_type_bits, flags)
        .expect("No suitable memory type");
    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_req.size)
        .memory_type_index(mem_type);
    let memory = unsafe { device.allocate_memory(&alloc_info, None).map_err(ForgeError::Vk)? };
    unsafe { device.bind_buffer_memory(buffer, memory, 0).map_err(ForgeError::Vk)? };
    Ok((buffer, memory))
}

fn find_memory_type(
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    for i in 0..mem_props.memory_type_count {
        if (type_bits & (1 << i)) != 0
            && mem_props.memory_types[i as usize].property_flags.contains(flags)
        {
            return Some(i);
        }
    }
    None
}

// Minimal valid SPIR‑V shaders (pass‑through vertex + white fragment)
const MINIMAL_VERT_SPV: &[u32] = &[
    0x07230203, 0x00010000, 0x0008000b, 0x00000006,
    0x00000001, 0x00000000, 0x00020011, 0x00000001,
    0x0006000b, 0x00000001, 0x4c534c47, 0x6474732e,
    0x3035342e, 0x00000000, 0x0003000e, 0x00000000,
    0x00000001, 0x0007000f, 0x00000000, 0x00000004,
    0x6e69616d, 0x00000000, 0x00000001, 0x00000000,
    0x00030003, 0x00000002, 0x000001c2, 0x00040005,
    0x00000004, 0x6e69616d, 0x00000000, 0x00020013,
    0x00000002, 0x00030021, 0x00000003, 0x00000002,
    0x00050036, 0x00000002, 0x00000004, 0x00000000,
    0x00000003, 0x000200f8, 0x00000005, 0x000100fd,
    0x00010038,
];

const MINIMAL_FRAG_SPV: &[u32] = &[
    0x07230203, 0x00010000, 0x0008000b, 0x0000000c,
    0x00000001, 0x00000000, 0x00020011, 0x00000001,
    0x0006000b, 0x00000001, 0x4c534c47, 0x6474732e,
    0x3035342e, 0x00000000, 0x0003000e, 0x00000000,
    0x00000001, 0x0007000f, 0x00000004, 0x00000004,
    0x6e69616d, 0x00000000, 0x00000009, 0x00000003,
    0x00030003, 0x00000002, 0x000001c2, 0x00040005,
    0x00000004, 0x6e69616d, 0x00000000, 0x00050005,
    0x00000009, 0x4374756f, 0x726f6c6f, 0x00000000,
    0x00040047, 0x00000009, 0x0000001e, 0x00000000,
    0x00020013, 0x00000002, 0x00030021, 0x00000003,
    0x00000002, 0x00030016, 0x00000006, 0x00000020,
    0x00040017, 0x00000007, 0x00000006, 0x00000004,
    0x00040020, 0x00000008, 0x00000003, 0x00000007,
    0x0004003b, 0x00000008, 0x00000009, 0x00000003,
    0x00050036, 0x00000002, 0x00000004, 0x00000000,
    0x00000003, 0x000200f8, 0x00000005, 0x0004003d,
    0x00000007, 0x0000000a, 0x00000009, 0x0003003e,
    0x00000009, 0x0000000a, 0x000100fd, 0x00010038,
];