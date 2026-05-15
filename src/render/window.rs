use std::sync::Arc;
use ash::vk;

use thin_vec::ThinVec;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use crate::forge_master::{ForgeError, ForgeMaster, ForgeResult, GraphicsForge, GraphicsMold};
use crate::resource_manager::manager::{Handle as ResourceHandle, Id};

use super::factory_master::{ComputeTag, FactoryHandle, FactoryMaster, GraphicsTag, Proto};

// ── Handle / Id types ──────────────────────────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct WindowTag;
pub type WindowHandle = ResourceHandle<WindowTag>;

pub struct WindowMarker;
pub type WindowId = Id<WindowMarker>;

// ── Graphics plumbing ───────────────────────────────────────────────────────

pub struct GraphicsState {
    // OS window (owned by caller's event loop)
    pub winit_window: Arc<winit::window::Window>,

    // surface / swapchain
    pub surface: vk::SurfaceKHR,
    pub surface_loader: ash::khr::surface::Instance,
    pub swapchain: vk::SwapchainKHR,
    pub swapchain_images: ThinVec<vk::Image>,
    pub swapchain_image_views: ThinVec<vk::ImageView>,
    pub swapchain_extent: vk::Extent2D,
    pub swapchain_format: vk::Format,
    pub swapchain_loader: ash::khr::swapchain::Device,

    // pipeline (compiled from GraphicsForge by new_with_surface)
    pub mold: GraphicsMold,
    pub framebuffers: ThinVec<vk::Framebuffer>,

    // command recording
    pub command_pool: vk::CommandPool,
    pub command_buffers: ThinVec<vk::CommandBuffer>,

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

    /// Fully initialised graphics window (winit + Vulkan).
    ///
    /// `forge` supplies the SPIR-V bytecode. `compile()` is called here once
    /// the swapchain format is known; the resulting `GraphicsMold` lives on
    /// `GraphicsState` and is destroyed by `Window::destroy`.
    pub fn new_with_surface(
        id: WindowId,
        name: impl Into<Arc<str>>,
        winit_window: Arc<winit::window::Window>,
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        _graphics_queue: vk::Queue,
        graphics_queue_family: u32,
        _memory_properties: &vk::PhysicalDeviceMemoryProperties,
        entry: &ash::Entry,
        forge: &GraphicsForge,
    ) -> ForgeResult<Self> {
        let title: Arc<str> = name.into();
        let size = winit_window.inner_size();
        let width = size.width;
        let height = size.height;

        // Surface via ash-window + raw-window-handle (from winit).
        let display_handle = winit_window
            .display_handle()
            .map_err(|e| ForgeError::Io(std::io::Error::other(e)))?
            .as_raw();
        let window_handle = winit_window
            .window_handle()
            .map_err(|e| ForgeError::Io(std::io::Error::other(e)))?
            .as_raw();
        let surface = unsafe {
            ash_window::create_surface(entry, instance, display_handle, window_handle, None)
                .map_err(ForgeError::Vk)?
        };

        // Swapchain
        let surface_loader = ash::khr::surface::Instance::new(entry, instance);
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
        let swapchain_format = formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_SRGB)
            .unwrap_or(&formats[0])
            .format;
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
            .map(|&img| unsafe {
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
            })
            .collect::<Result<ThinVec<_>, _>>()
            .map_err(ForgeError::Vk)?;

        // Compile the pipeline from the forge now that we know the format.
        let mold = forge.compile(device, swapchain_format, swapchain_extent)?;

        // Framebuffers — one per swapchain image, referencing mold.render_pass.
        let framebuffers: ThinVec<vk::Framebuffer> = swapchain_image_views
            .iter()
            .map(|&view| unsafe {
                device.create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(mold.render_pass)
                        .attachments(&[view])
                        .width(swapchain_extent.width)
                        .height(swapchain_extent.height)
                        .layers(1),
                    None,
                )
            })
            .collect::<Result<ThinVec<_>, _>>()
            .map_err(ForgeError::Vk)?;

        // Command pool + one buffer per swapchain image.
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

        // Synchronisation primitives.
        let image_available_semaphore = unsafe {
            device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?
        };
        let render_finished_semaphore = unsafe {
            device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
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
                winit_window,
                surface,
                surface_loader,
                swapchain,
                swapchain_images,
                swapchain_image_views,
                swapchain_extent,
                swapchain_format,
                swapchain_loader,
                mold,
                framebuffers,
                command_pool,
                command_buffers,
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

    pub fn build_compute_factory(
        &mut self,
        proto: Proto<ComputeTag>,
        forge: &mut ForgeMaster,
    ) -> ForgeResult<FactoryHandle> {
        self.factory_master.build_compute_proto(proto, forge)
    }

    pub fn build_graphics_factory(
        &mut self,
        proto: Proto<GraphicsTag>,
    ) -> FactoryHandle {
        self.factory_master.build_graphics_proto(proto)
    }

    /// Destroy all Vulkan resources (if any) and the factory master.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        if let Some(ref mut gfx) = self.graphics {
            unsafe {
                if gfx.in_flight_fence != vk::Fence::null() {
                    device.destroy_fence(gfx.in_flight_fence, None);
                    gfx.in_flight_fence = vk::Fence::null();
                }
                if gfx.render_finished_semaphore != vk::Semaphore::null() {
                    device.destroy_semaphore(gfx.render_finished_semaphore, None);
                    gfx.render_finished_semaphore = vk::Semaphore::null();
                }
                if gfx.image_available_semaphore != vk::Semaphore::null() {
                    device.destroy_semaphore(gfx.image_available_semaphore, None);
                    gfx.image_available_semaphore = vk::Semaphore::null();
                }
                if gfx.command_pool != vk::CommandPool::null() {
                    device.destroy_command_pool(gfx.command_pool, None);
                    gfx.command_pool = vk::CommandPool::null();
                }
                for fb in gfx.framebuffers.iter() {
                    if *fb != vk::Framebuffer::null() {
                        device.destroy_framebuffer(*fb, None);
                    }
                }
                gfx.framebuffers.clear();
                gfx.mold.destroy(device);
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
            }
        }
        unsafe { self.factory_master.destroy(device) };
    }

    /// Issue all graphics draw calls from every factory this window owns.
    /// One render pass per frame; factories and their calls are iterated in
    /// insertion order.
    pub unsafe fn draw_frame(
        &mut self,
        device: &ash::Device,
        queue: vk::Queue,
    ) -> ForgeResult<()> {
        let gfx = self
            .graphics
            .as_mut()
            .expect("draw_frame called on a headless window");

        // Wait for the previous frame to finish.
        unsafe {
            device.wait_for_fences(&[gfx.in_flight_fence], true, u64::MAX)
                .map_err(ForgeError::Vk)?;
            device.reset_fences(&[gfx.in_flight_fence])
                .map_err(ForgeError::Vk)?;
        }

        // Acquire swapchain image.
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

        // Collect draw calls from all factories.
        let calls: ThinVec<_> = self
            .factory_master
            .iter()
            .flat_map(|f| f.graphics_calls().iter().cloned())
            .collect();

        // Record.
        let clear_values = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.05, 0.05, 0.1, 1.0],
            },
        }];
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        let rp_begin = vk::RenderPassBeginInfo::default()
            .render_pass(gfx.mold.render_pass)
            .framebuffer(gfx.framebuffers[image_index as usize])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: gfx.swapchain_extent,
            })
            .clear_values(&clear_values);

        unsafe {
            device.begin_command_buffer(command_buffer, &begin_info)
                .map_err(ForgeError::Vk)?;
            device.cmd_begin_render_pass(command_buffer, &rp_begin, vk::SubpassContents::INLINE);
            device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                gfx.mold.pipeline,
            );

            for call in &calls {
                if let Some(mesh) = &call.mesh {
                    // Indexed draw: bind vertex + index buffers, push MVP,
                    // then issue cmd_draw_indexed.
                    device.cmd_bind_vertex_buffers(
                        command_buffer, 0,
                        &[mesh.vertex_buffer.handle], &[0],
                    );
                    device.cmd_bind_index_buffer(
                        command_buffer,
                        mesh.index_buffer.handle,
                        0,
                        vk::IndexType::UINT32,
                    );
                    // Upload column-major MVP as a push constant.
                    let mvp_bytes: &[u8] =
                        std::slice::from_raw_parts(call.mvp.as_ptr().cast(), 64);
                    device.cmd_push_constants(
                        command_buffer,
                        gfx.mold.pipeline_layout,
                        vk::ShaderStageFlags::VERTEX,
                        0,
                        mvp_bytes,
                    );
                    device.cmd_draw_indexed(
                        command_buffer,
                        mesh.index_count,
                        call.instance_count,
                        0, 0, 0,
                    );
                } else {
                    // Procedural draw (Ui / shader-generated vertices).
                    device.cmd_draw(
                        command_buffer,
                        call.vertex_count,
                        call.instance_count,
                        call.first_vertex,
                        call.first_instance,
                    );
                }
            }

            device.cmd_end_render_pass(command_buffer);
            device.end_command_buffer(command_buffer).map_err(ForgeError::Vk)?;
        }

        // Submit + present.
        let wait_semaphores  = [gfx.image_available_semaphore];
        let signal_semaphores = [gfx.render_finished_semaphore];
        let cmd_buffers = [command_buffer];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&cmd_buffers)
            .signal_semaphores(&signal_semaphores);

        let present_swapchains = [gfx.swapchain];
        let present_image_indices = [image_index];

        unsafe {
            device.queue_submit(queue, &[submit_info], gfx.in_flight_fence)
                .map_err(ForgeError::Vk)?;

            let present_info = vk::PresentInfoKHR::default()
                .wait_semaphores(&signal_semaphores)
                .swapchains(&present_swapchains)
                .image_indices(&present_image_indices);
            gfx.swapchain_loader
                .queue_present(queue, &present_info)
                .map_err(ForgeError::Vk)?;
        }
        Ok(())
    }
}

