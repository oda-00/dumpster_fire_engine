use std::sync::Arc;
use ash::vk;

use thin_vec::ThinVec;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use crate::forge_master::{ForgeError, ForgeMaster, ForgeResult, ForgeImage, GraphicsForge, GraphicsMold};
use crate::forge_master::ore::GraphicsOreKind;
use crate::resource_manager::manager::{Handle as ResourceHandle, Id};

use super::factory_master::{ComputeTag, FactoryHandle, FactoryMaster, GraphicsTag, Proto};

// ── Handle / Id types ──────────────────────────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct WindowTag;
pub type WindowHandle = ResourceHandle<WindowTag>;

pub struct WindowMarker;
pub type WindowId = Id<WindowMarker>;

// ── Best-practice constants ────────────────────────────────────────────────

/// Number of frames the CPU may keep "in flight" ahead of the GPU.
/// 2 = double-buffer the command recording — CPU records frame N+1 while
/// the GPU is still consuming frame N. Higher (3) reduces stutter on input
/// spikes but adds a frame of latency; 2 is the standard tradeoff.
/// Number of frames the CPU is allowed to record ahead of the GPU. Three
/// matches the MAILBOX-mode swapchain image count (caps.min_image_count
/// + 1 = typically 3) so we never CPU-stall waiting for an image while
/// also providing headroom for the GPU to be one frame behind. Two would
/// be the conservative pick but adds CPU-side fence waits on input-light
/// frames; four+ buys little and burns more host-visible UBO / staging
/// memory. Three is the standard Vulkan recommendation for real-time
/// rendering with triple-buffered presents.
pub const FRAMES_IN_FLIGHT: usize = 3;

// ── Graphics plumbing ───────────────────────────────────────────────────────

pub struct GraphicsState {
    // OS window (owned by caller's event loop)
    pub winit_window: Arc<winit::window::Window>,

    // Recreation-time state (needed by recreate_swapchain).
    pub physical_device: vk::PhysicalDevice,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub depth_format: vk::Format,

    // surface / swapchain
    pub surface: vk::SurfaceKHR,
    pub surface_loader: ash::khr::surface::Instance,
    pub swapchain: vk::SwapchainKHR,
    pub swapchain_images: ThinVec<vk::Image>,
    pub swapchain_image_views: ThinVec<vk::ImageView>,
    pub swapchain_extent: vk::Extent2D,
    pub swapchain_format: vk::Format,
    pub swapchain_loader: ash::khr::swapchain::Device,

    // One depth image per swapchain image. Per-image (not per-frame) so we
    // never race a depth write against a draw still reading from the
    // previous frame's same image-index. Depth-stencil format + MSAA
    // sample count matches the render pass's depth attachment.
    pub depth_images: ThinVec<ForgeImage>,

    /// Per-image MSAA colour attachment. Empty when `msaa_samples == TYPE_1`
    /// (lavapipe / D3D12 software path); populated otherwise so the
    /// rasterizer writes into a multi-sample image that gets resolved to
    /// the swapchain image at end-of-subpass.
    pub msaa_color_images: ThinVec<ForgeImage>,

    /// Render-pass sample count this swapchain was built against. The
    /// pipeline's `rasterization_samples` and every framebuffer attachment
    /// have to match.
    pub msaa_samples: vk::SampleCountFlags,

    // pipeline + framebuffers
    pub mold: GraphicsMold,
    /// Optional second pipeline for SkinnedForwardLit draws — compiled from
    /// a separate `GraphicsForge` via `Window::attach_skinned_forge` after
    /// construction. Stays `None` until the caller opts in.
    pub skinned_mold: Option<GraphicsMold>,
    pub framebuffers: ThinVec<vk::Framebuffer>,

    // command recording — FRAMES_IN_FLIGHT primary buffers
    pub command_pool: vk::CommandPool,
    pub command_buffers: [vk::CommandBuffer; FRAMES_IN_FLIGHT],

    // Per-frame-in-flight sync: image_available + in_flight (CPU-GPU fence).
    pub image_available_semaphores: [vk::Semaphore; FRAMES_IN_FLIGHT],
    pub in_flight_fences: [vk::Fence; FRAMES_IN_FLIGHT],
    // Per-swapchain-image sync: render_finished. Indexed by image_index
    // because present uses it; reusing one render_finished per frame slot
    // would race when the swapchain has more images than frames in flight.
    pub render_finished_semaphores: ThinVec<vk::Semaphore>,

    pub current_frame: usize,
    pub needs_resize: bool,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_surface(
        id: WindowId,
        name: impl Into<Arc<str>>,
        winit_window: Arc<winit::window::Window>,
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        _graphics_queue: vk::Queue,
        graphics_queue_family: u32,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        depth_format: vk::Format,
        msaa_samples: vk::SampleCountFlags,
        entry: &ash::Entry,
        forge: &GraphicsForge,
    ) -> ForgeResult<Self> {
        let title: Arc<str> = name.into();
        let size = winit_window.inner_size();
        let width = size.width;
        let height = size.height;

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

        let surface_loader = ash::khr::surface::Instance::new(entry, instance);
        let swapchain_loader = ash::khr::swapchain::Device::new(instance, device);

        // Pick the swapchain format up front; depth_format already chosen by VulkanContext.
        let formats = unsafe {
            surface_loader
                .get_physical_device_surface_formats(physical_device, surface)
                .map_err(ForgeError::Vk)?
        };
        let swapchain_format = formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_SRGB)
            .unwrap_or(&formats[0])
            .format;

        // Compile the pipeline. Uses dynamic viewport/scissor so we don't
        // need to recompile on resize.
        let mold = forge.compile(device, swapchain_format, depth_format, msaa_samples)?;

        // Build the swapchain + per-image resources via the shared helper.
        let (
            swapchain,
            swapchain_images,
            swapchain_image_views,
            swapchain_extent,
            depth_images,
            msaa_color_images,
            framebuffers,
            render_finished_semaphores,
        ) = create_swapchain_resources(
            instance,
            physical_device,
            device,
            &surface_loader,
            &swapchain_loader,
            surface,
            memory_properties,
            depth_format,
            swapchain_format,
            msaa_samples,
            mold.render_pass,
            width,
            height,
            vk::SwapchainKHR::null(),
        )?;

        // Command pool + FRAMES_IN_FLIGHT command buffers.
        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(graphics_queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            ).map_err(ForgeError::Vk)?
        };
        let cb_vec = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(FRAMES_IN_FLIGHT as u32),
            ).map_err(ForgeError::Vk)?
        };
        let mut command_buffers = [vk::CommandBuffer::null(); FRAMES_IN_FLIGHT];
        for (i, cb) in cb_vec.into_iter().enumerate() {
            command_buffers[i] = cb;
        }

        // Per-frame sync primitives.
        let mut image_available_semaphores = [vk::Semaphore::null(); FRAMES_IN_FLIGHT];
        let mut in_flight_fences = [vk::Fence::null(); FRAMES_IN_FLIGHT];
        for i in 0..FRAMES_IN_FLIGHT {
            image_available_semaphores[i] = unsafe {
                device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                    .map_err(ForgeError::Vk)?
            };
            in_flight_fences[i] = unsafe {
                device.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                ).map_err(ForgeError::Vk)?
            };
        }

        Ok(Self {
            id,
            name: title,
            width,
            height,
            factory_master: FactoryMaster::new(),
            should_close: false,
            graphics: Some(GraphicsState {
                winit_window,
                physical_device,
                memory_properties: *memory_properties,
                depth_format,
                surface,
                surface_loader,
                swapchain,
                swapchain_images,
                swapchain_image_views,
                swapchain_extent,
                swapchain_format,
                swapchain_loader,
                depth_images,
                msaa_color_images,
                msaa_samples,
                mold,
                skinned_mold: None,
                framebuffers,
                command_pool,
                command_buffers,
                image_available_semaphores,
                in_flight_fences,
                render_finished_semaphores,
                current_frame: 0,
                needs_resize: false,
            }),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        if let Some(gfx) = &mut self.graphics {
            gfx.needs_resize = true;
        }
    }

    /// Compile a second graphics forge (typically `SkinnedForwardLit`) into
    /// its own `GraphicsMold` and store it on this Window. Call once after
    /// `Window::new_with_surface` and before issuing any skinned draws.
    pub fn attach_skinned_forge(
        &mut self,
        device:    &ash::Device,
        forge:     &GraphicsForge,
    ) -> ForgeResult<()> {
        let gfx = self.graphics.as_mut()
            .expect("attach_skinned_forge requires a graphics window");
        let mold = forge.compile(device, gfx.swapchain_format, gfx.depth_format, gfx.msaa_samples)?;
        gfx.skinned_mold = Some(mold);
        Ok(())
    }

    /// Wait on the fence belonging to the most recently submitted frame.
    /// After this returns, the GPU has finished that submission and any
    /// resources it referenced (descriptor sets, compute output buffers,
    /// vertex buffers) are safe to destroy or recycle. Substantially
    /// lighter than `device_wait_idle` because it only blocks on the
    /// single relevant fence rather than every queue.
    ///
    /// Returns `Ok(())` immediately on the very first frame (the fences
    /// are created in the signalled state).
    pub fn wait_for_last_submission(&self, device: &ash::Device) -> ForgeResult<()> {
        let Some(gfx) = self.graphics.as_ref() else { return Ok(()); };
        let prev = (gfx.current_frame + FRAMES_IN_FLIGHT - 1) % FRAMES_IN_FLIGHT;
        let fence = gfx.in_flight_fences[prev];
        if fence == vk::Fence::null() { return Ok(()); }
        unsafe { device.wait_for_fences(&[fence], true, u64::MAX) }
            .map_err(ForgeError::Vk)
    }

    pub fn build_compute_factory(
        &mut self,
        proto:  Proto<ComputeTag>,
        forge:  &mut ForgeMaster,
        device: &ash::Device,
    ) -> ForgeResult<FactoryHandle> {
        self.factory_master.build_compute_proto(proto, forge, device)
    }

    pub fn build_graphics_factory(
        &mut self,
        proto:  Proto<GraphicsTag>,
        device: &ash::Device,
    ) -> FactoryHandle {
        self.factory_master.build_graphics_proto(proto, device)
    }

    /// Destroy all Vulkan resources (if any) and the factory master.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        if let Some(ref mut gfx) = self.graphics {
            unsafe {
                for f in gfx.in_flight_fences.iter_mut() {
                    if *f != vk::Fence::null() {
                        device.destroy_fence(*f, None);
                        *f = vk::Fence::null();
                    }
                }
                for s in gfx.image_available_semaphores.iter_mut() {
                    if *s != vk::Semaphore::null() {
                        device.destroy_semaphore(*s, None);
                        *s = vk::Semaphore::null();
                    }
                }
                for s in gfx.render_finished_semaphores.drain(..) {
                    if s != vk::Semaphore::null() {
                        device.destroy_semaphore(s, None);
                    }
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
                if let Some(m) = gfx.skinned_mold.as_mut() {
                    m.destroy(device);
                }
                for img in gfx.depth_images.iter_mut() {
                    img.destroy(device);
                }
                gfx.depth_images.clear();
                for img in gfx.msaa_color_images.iter_mut() {
                    img.destroy(device);
                }
                gfx.msaa_color_images.clear();
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

    /// Recreate the swapchain, framebuffers, depth images, and per-image
    /// semaphores at the current window size. Caller must guarantee the GPU
    /// is idle on this device — we call `device_wait_idle` ourselves.
    pub fn recreate_swapchain(&mut self, instance: &ash::Instance, device: &ash::Device) -> ForgeResult<()> {
        let Some(gfx) = self.graphics.as_mut() else { return Ok(()) };

        // If the window is minimised, defer recreation until non-zero size.
        let size = gfx.winit_window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }

        unsafe { device.device_wait_idle().map_err(ForgeError::Vk)? };

        // Tear down the old per-image resources (keep mold + command pool).
        unsafe {
            for s in gfx.render_finished_semaphores.drain(..) {
                if s != vk::Semaphore::null() {
                    device.destroy_semaphore(s, None);
                }
            }
            for fb in gfx.framebuffers.iter() {
                if *fb != vk::Framebuffer::null() {
                    device.destroy_framebuffer(*fb, None);
                }
            }
            gfx.framebuffers.clear();
            for img in gfx.depth_images.iter_mut() {
                img.destroy(device);
            }
            gfx.depth_images.clear();
            for img in gfx.msaa_color_images.iter_mut() {
                img.destroy(device);
            }
            gfx.msaa_color_images.clear();
            for iv in gfx.swapchain_image_views.iter() {
                if *iv != vk::ImageView::null() {
                    device.destroy_image_view(*iv, None);
                }
            }
            gfx.swapchain_image_views.clear();
        }

        // Rebuild via the shared helper (oldSwapchain = current handle for
        // graceful handoff). The helper destroys the old swapchain on success.
        let old_swapchain = gfx.swapchain;
        let (
            swapchain,
            swapchain_images,
            swapchain_image_views,
            swapchain_extent,
            depth_images,
            msaa_color_images,
            framebuffers,
            render_finished_semaphores,
        ) = create_swapchain_resources(
            instance,
            gfx.physical_device,
            device,
            &gfx.surface_loader,
            &gfx.swapchain_loader,
            gfx.surface,
            &gfx.memory_properties,
            gfx.depth_format,
            gfx.swapchain_format,
            gfx.msaa_samples,
            gfx.mold.render_pass,
            size.width,
            size.height,
            old_swapchain,
        )?;

        // Destroy the now-replaced swapchain.
        unsafe {
            if old_swapchain != vk::SwapchainKHR::null() {
                gfx.swapchain_loader.destroy_swapchain(old_swapchain, None);
            }
        }

        gfx.swapchain = swapchain;
        gfx.swapchain_images = swapchain_images;
        gfx.swapchain_image_views = swapchain_image_views;
        gfx.swapchain_extent = swapchain_extent;
        gfx.depth_images = depth_images;
        gfx.msaa_color_images = msaa_color_images;
        gfx.framebuffers = framebuffers;
        gfx.render_finished_semaphores = render_finished_semaphores;
        gfx.needs_resize = false;

        Ok(())
    }

    /// Issue all graphics draw calls from every factory this window owns.
    /// Handles in-flight frame pacing, swapchain out-of-date, and dynamic
    /// viewport/scissor.
    pub unsafe fn draw_frame(
        &mut self,
        instance: &ash::Instance,
        device: &ash::Device,
        queue: vk::Queue,
    ) -> ForgeResult<()> {
        unsafe { self.draw_frame_with_compute_wait(instance, device, queue, &[]) }
    }

    /// Draw one frame, waiting on `compute_wait` semaphores before the
    /// graphics submission's vertex stages execute. Used by callers that
    /// dispatch compute work asynchronously (via
    /// `ForgeMaster::refine_batch_async`) — the returned semaphore from
    /// that call must be passed here so the graphics queue blocks
    /// per-stage instead of the CPU blocking on a fence.
    pub unsafe fn draw_frame_with_compute_wait(
        &mut self,
        instance: &ash::Instance,
        device: &ash::Device,
        queue: vk::Queue,
        compute_wait: &[vk::Semaphore],
    ) -> ForgeResult<()> {
        // Resize check up front — covers explicit resize() calls.
        if self.graphics.as_ref().is_some_and(|g| g.needs_resize) {
            self.recreate_swapchain(instance, device)?;
        }
        let gfx = self
            .graphics
            .as_mut()
            .expect("draw_frame called on a headless window");

        // Skip if the window has zero area (minimised).
        if gfx.swapchain_extent.width == 0 || gfx.swapchain_extent.height == 0 {
            return Ok(());
        }

        let frame = gfx.current_frame;
        let img_avail = gfx.image_available_semaphores[frame];
        let in_flight = gfx.in_flight_fences[frame];

        // Wait for this slot's previous submission to complete.
        unsafe {
            device.wait_for_fences(&[in_flight], true, u64::MAX)
                .map_err(ForgeError::Vk)?;
        }

        // Acquire next image; handle OUT_OF_DATE / SUBOPTIMAL.
        let acquire = unsafe {
            gfx.swapchain_loader.acquire_next_image(
                gfx.swapchain,
                u64::MAX,
                img_avail,
                vk::Fence::null(),
            )
        };
        let (image_index, _suboptimal) = match acquire {
            Ok(t) => t,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                gfx.needs_resize = true;
                return Ok(());
            }
            Err(e) => return Err(ForgeError::Vk(e)),
        };

        // Now safe to reset the fence — we will submit.
        unsafe {
            device.reset_fences(&[in_flight]).map_err(ForgeError::Vk)?;
        }

        let command_buffer = gfx.command_buffers[frame];

        // Collect draw calls from all factories.
        let calls: ThinVec<_> = self
            .factory_master
            .iter()
            .flat_map(|f| f.graphics_calls().iter().cloned())
            .collect();

        // Record.
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.05, 0.05, 0.1, 1.0],
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];
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
            device.reset_command_buffer(command_buffer, vk::CommandBufferResetFlags::empty())
                .map_err(ForgeError::Vk)?;
            device.begin_command_buffer(command_buffer, &begin_info)
                .map_err(ForgeError::Vk)?;

            // Compute → graphics memory dependency. Same-queue submission
            // ordering guarantees the compute dispatch FINISHES before this
            // draw starts, but does NOT guarantee the compute writes are
            // visible to the vertex stage's reads — that needs an explicit
            // memory barrier. Sync2 per-stage masks let us scope this
            // precisely to VERTEX_ATTRIBUTE_INPUT (for MorphBlend, read via
            // vertex input) + VERTEX_SHADER (for SkinPalette, read as SSBO)
            // instead of a coarse VERTEX_INPUT | VERTEX_SHADER.
            let mem_barrier2 = vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .src_access_mask(vk::AccessFlags2::SHADER_STORAGE_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT
                    | vk::PipelineStageFlags2::VERTEX_SHADER)
                .dst_access_mask(vk::AccessFlags2::VERTEX_ATTRIBUTE_READ
                    | vk::AccessFlags2::SHADER_STORAGE_READ);
            let memory_barriers = [mem_barrier2];
            let dep_info = vk::DependencyInfo::default()
                .memory_barriers(&memory_barriers);
            device.cmd_pipeline_barrier2(command_buffer, &dep_info);

            device.cmd_begin_render_pass(command_buffer, &rp_begin, vk::SubpassContents::INLINE);

            // Dynamic viewport/scissor — pipeline is extent-agnostic, set here.
            let viewports = [vk::Viewport {
                x: 0.0,
                y: 0.0,
                width:  gfx.swapchain_extent.width  as f32,
                height: gfx.swapchain_extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }];
            let scissors = [vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: gfx.swapchain_extent,
            }];
            device.cmd_set_viewport(command_buffer, 0, &viewports);
            device.cmd_set_scissor(command_buffer, 0, &scissors);

            // Bind the default pipeline once at the start; the inner loop
            // re-binds only when the kind changes.
            let mut current_kind: Option<GraphicsOreKind> = None;
            for call in &calls {
                // Pick the mold for this call's kind. Skinned draws need the
                // skinned mold; everything else uses the default ForwardLit.
                let mold: &GraphicsMold = match (call.kind, gfx.skinned_mold.as_ref()) {
                    (GraphicsOreKind::SkinnedForwardLit, Some(sm)) => sm,
                    _ => &gfx.mold,
                };
                if current_kind != Some(call.kind) {
                    device.cmd_bind_pipeline(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        mold.pipeline,
                    );
                    current_kind = Some(call.kind);
                }

                // Bind material descriptor set (set 1) when present.
                if let Some(mat_set) = call.material_set {
                    device.cmd_bind_descriptor_sets(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        mold.pipeline_layout,
                        1,
                        &[mat_set],
                        &[],
                    );
                }
                // Bind skin palette descriptor set (set 2) for skinned draws.
                if call.kind == GraphicsOreKind::SkinnedForwardLit {
                    if let Some(skin_set) = call.skin_palette_set {
                        device.cmd_bind_descriptor_sets(
                            command_buffer,
                            vk::PipelineBindPoint::GRAPHICS,
                            mold.pipeline_layout,
                            2,
                            &[skin_set],
                            &[],
                        );
                    }
                }
                // Bind per-instance mat4 SSBO (set 3). Always bind something
                // — the shader unconditionally reads `instances.m[gl_InstanceIndex]`
                // so missing binding is undefined. Real per-instance sets land
                // here when EXT_mesh_gpu_instancing is active; otherwise the
                // dummy identity set from the cache.
                if let Some(inst_set) = call.instance_set {
                    device.cmd_bind_descriptor_sets(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        mold.pipeline_layout,
                        3,
                        &[inst_set],
                        &[],
                    );
                }

                if let Some(mesh) = &call.mesh {
                    // Use the compute-shader-posed vertex buffer when the call
                    // carries an override (MorphBlend output); otherwise the
                    // rest-pose mesh buffer.
                    let vb = call.vertex_buffer_override
                        .unwrap_or(mesh.vertex_buffer.handle);
                    // Skinned pipeline needs the per-vertex joints/weights buffer at binding 1.
                    if call.kind == GraphicsOreKind::SkinnedForwardLit {
                        if let Some(skin_vb) = call.skin_vertex_buffer {
                            device.cmd_bind_vertex_buffers(
                                command_buffer, 0,
                                &[vb, skin_vb], &[0, 0],
                            );
                        } else {
                            // No skin buffer — fall back to single binding so the
                            // draw still records (visual will be wrong but no crash).
                            device.cmd_bind_vertex_buffers(
                                command_buffer, 0,
                                &[vb], &[0],
                            );
                        }
                    } else {
                        device.cmd_bind_vertex_buffers(
                            command_buffer, 0,
                            &[vb], &[0],
                        );
                    }
                    device.cmd_bind_index_buffer(
                        command_buffer,
                        mesh.index_buffer.handle,
                        0,
                        vk::IndexType::UINT32,
                    );
                    let mvp_bytes: &[u8] =
                        std::slice::from_raw_parts(call.mvp.as_ptr().cast(), 64);
                    device.cmd_push_constants(
                        command_buffer,
                        mold.pipeline_layout,
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

        // Submit + present via Sync2 (queue_submit2): one
        // SemaphoreSubmitInfo per wait with explicit per-semaphore
        // stage masks. img_avail blocks COLOR_ATTACHMENT_OUTPUT (no
        // earlier graphics stage needs the swapchain image), while
        // each compute_wait semaphore blocks only the precise stages
        // that read its compute output — VERTEX_ATTRIBUTE_INPUT (vertex
        // buffer bound from MorphBlend output) + VERTEX_SHADER (SSBO
        // bound from SkinPalette output).
        let render_done = gfx.render_finished_semaphores[image_index as usize];

        let mut wait_infos: Vec<vk::SemaphoreSubmitInfo> = Vec::with_capacity(1 + compute_wait.len());
        wait_infos.push(
            vk::SemaphoreSubmitInfo::default()
                .semaphore(img_avail)
                .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT),
        );
        for &sem in compute_wait {
            wait_infos.push(
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(sem)
                    .stage_mask(
                        vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT
                            | vk::PipelineStageFlags2::VERTEX_SHADER,
                    ),
            );
        }
        let cb_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(command_buffer)];
        let signal_infos = [vk::SemaphoreSubmitInfo::default()
            .semaphore(render_done)
            .stage_mask(vk::PipelineStageFlags2::ALL_GRAPHICS)];
        let submit_info = vk::SubmitInfo2::default()
            .wait_semaphore_infos(&wait_infos)
            .command_buffer_infos(&cb_infos)
            .signal_semaphore_infos(&signal_infos);

        let signal_semaphores = [render_done];
        let present_swapchains = [gfx.swapchain];
        let present_image_indices = [image_index];

        unsafe {
            device.queue_submit2(queue, &[submit_info], in_flight)
                .map_err(ForgeError::Vk)?;

            let present_info = vk::PresentInfoKHR::default()
                .wait_semaphores(&signal_semaphores)
                .swapchains(&present_swapchains)
                .image_indices(&present_image_indices);
            match gfx.swapchain_loader.queue_present(queue, &present_info) {
                Ok(suboptimal) => {
                    if suboptimal {
                        gfx.needs_resize = true;
                    }
                }
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) | Err(vk::Result::SUBOPTIMAL_KHR) => {
                    gfx.needs_resize = true;
                }
                Err(e) => return Err(ForgeError::Vk(e)),
            }
        }

        gfx.current_frame = (gfx.current_frame + 1) % FRAMES_IN_FLIGHT;
        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Build a swapchain + all per-image resources (image views, depth images,
/// framebuffers, render_finished semaphores). Shared by initial creation and
/// `recreate_swapchain`. On failure, partial resources are cleaned up.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn create_swapchain_resources(
    _instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
    surface_loader: &ash::khr::surface::Instance,
    swapchain_loader: &ash::khr::swapchain::Device,
    surface: vk::SurfaceKHR,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    depth_format: vk::Format,
    swapchain_format: vk::Format,
    msaa_samples: vk::SampleCountFlags,
    render_pass: vk::RenderPass,
    width: u32,
    height: u32,
    old_swapchain: vk::SwapchainKHR,
) -> ForgeResult<(
    vk::SwapchainKHR,
    ThinVec<vk::Image>,
    ThinVec<vk::ImageView>,
    vk::Extent2D,
    ThinVec<ForgeImage>,
    ThinVec<ForgeImage>,
    ThinVec<vk::Framebuffer>,
    ThinVec<vk::Semaphore>,
)> {
    let caps = unsafe {
        surface_loader
            .get_physical_device_surface_capabilities(physical_device, surface)
            .map_err(ForgeError::Vk)?
    };
    let present_modes = unsafe {
        surface_loader
            .get_physical_device_surface_present_modes(physical_device, surface)
            .map_err(ForgeError::Vk)?
    };

    let swapchain_extent = vk::Extent2D {
        width:  width.clamp(caps.min_image_extent.width,  caps.max_image_extent.width),
        height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
    };
    let present_mode = if present_modes.contains(&vk::PresentModeKHR::MAILBOX) {
        vk::PresentModeKHR::MAILBOX
    } else {
        vk::PresentModeKHR::FIFO
    };

    // Request triple-buffering when allowed; clamp to device max.
    let min_image_count = {
        let desired = caps.min_image_count + 1;
        if caps.max_image_count > 0 {
            desired.min(caps.max_image_count)
        } else {
            desired
        }
    };

    let swapchain = unsafe {
        swapchain_loader
            .create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(surface)
                    .min_image_count(min_image_count)
                    .image_format(swapchain_format)
                    .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                    .image_extent(swapchain_extent)
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                    .present_mode(present_mode)
                    .clipped(true)
                    .old_swapchain(old_swapchain),
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

    // Depth image per swapchain image, MSAA-sized to match the render pass.
    let mut depth_images: ThinVec<ForgeImage> = ThinVec::with_capacity(swapchain_images.len());
    for _ in 0..swapchain_images.len() {
        let img = ForgeImage::create_2d_msaa(
            device,
            memory_properties,
            swapchain_extent.width,
            swapchain_extent.height,
            depth_format,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            msaa_samples,
        )?;
        depth_images.push(img);
    }

    // MSAA colour image per swapchain image — only allocated when the
    // render pass actually rasterises into a multi-sample target.
    let msaa_color_images: ThinVec<ForgeImage> = if msaa_samples != vk::SampleCountFlags::TYPE_1 {
        let mut v = ThinVec::with_capacity(swapchain_images.len());
        for _ in 0..swapchain_images.len() {
            let img = ForgeImage::create_2d_msaa(
                device,
                memory_properties,
                swapchain_extent.width,
                swapchain_extent.height,
                swapchain_format,
                vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSIENT_ATTACHMENT,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
                msaa_samples,
            )?;
            v.push(img);
        }
        v
    } else {
        ThinVec::new()
    };

    // Framebuffer attachment order matches the render-pass attachment list.
    // Sync1 path: when MSAA is off → [swapchain_color, depth].
    // MSAA path:                    → [msaa_color, depth, swapchain_resolve].
    let framebuffers: ThinVec<vk::Framebuffer> = swapchain_image_views
        .iter()
        .enumerate()
        .map(|(i, &color_view)| {
            let depth = depth_images[i].view;
            let mut atts: Vec<vk::ImageView> = Vec::with_capacity(3);
            if msaa_samples != vk::SampleCountFlags::TYPE_1 {
                atts.push(msaa_color_images[i].view);
                atts.push(depth);
                atts.push(color_view);
            } else {
                atts.push(color_view);
                atts.push(depth);
            }
            unsafe {
                device.create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(render_pass)
                        .attachments(&atts)
                        .width(swapchain_extent.width)
                        .height(swapchain_extent.height)
                        .layers(1),
                    None,
                )
            }
        })
        .collect::<Result<ThinVec<_>, _>>()
        .map_err(ForgeError::Vk)?;

    // One render_finished semaphore per swapchain image.
    let mut render_finished_semaphores: ThinVec<vk::Semaphore> =
        ThinVec::with_capacity(swapchain_images.len());
    for _ in 0..swapchain_images.len() {
        let s = unsafe {
            device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?
        };
        render_finished_semaphores.push(s);
    }

    Ok((
        swapchain,
        swapchain_images,
        swapchain_image_views,
        swapchain_extent,
        depth_images,
        msaa_color_images,
        framebuffers,
        render_finished_semaphores,
    ))
}
