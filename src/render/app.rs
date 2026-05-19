//! Generic application runner — entry point for any engine-based app.
//!
//! Apps implement the `AppLogic` trait and run via `AppRunner::new(logic).run()`.
//! The runner owns Vulkan + Renderer + camera arena + per-window state; the
//! logic implementer just spawns windows, registers compute Ores, and ticks
//! its own simulation.
//!
//! ```ignore
//! struct MyApp;
//! impl AppLogic for MyApp {
//!     fn on_start(&mut self, ctx: &mut AppCtx, ev: &ActiveEventLoop) -> ForgeResult<()> {
//!         ctx.spawn_window(ev, "Main", 1024, 768)?;
//!         Ok(())
//!     }
//! }
//! fn main() { AppRunner::new(MyApp).run().unwrap(); }
//! ```
//!
//! Design notes (per the close-out plan's Step E2):
//! * `AppLogic::update` / `handle_event` take `&mut AppCtx<'_>` — a borrow-
//!   split view of `AppData` that does NOT include the logic field. This
//!   avoids the double-mut-borrow conflict the original signature had.
//! * Engine-side modules (`InstanceComputeState`, `ComputeDispatchGraph`)
//!   own descriptor allocation + semaphore threading, NOT this binary.

use ash::vk;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::{CursorGrabMode, Window as WinitWindow, WindowId};

use crate::forge_master::master::{ForgeError, ForgeResult};
use crate::forge_master::{ForgeMaster, GraphicsForgeId, GraphicsOreKind};
use crate::render::camera::{Camera, CameraArena, CameraController, CameraHandle};
use crate::render::{Renderer, VulkanContext, Window, WindowId as RenderWindowId};
use crate::resource_manager::asset_manager::gltf_loader::register_skin_morph_forges;
use crate::resource_manager::gltf_driver::{
    create_instance_pool, create_instance_set_layout,
    create_material_pool, create_skin_palette_pool, create_skin_palette_set_layout,
};
use crate::resource_manager::gltf_scene::GltfScene;
use crate::resource_manager::manager::{Arena, Handle, Id};

// Shader bytes embedded once here so AppRunner can register default forges.
const FORWARD_LIT_VERT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"), "/assets/shaders/forward_lit.vert.spv"
));
const FORWARD_LIT_FRAG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"), "/assets/shaders/forward_lit.frag.spv"
));
const SKINNED_VERT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"), "/assets/shaders/skinned_forward_lit.vert.spv"
));

// ── App handle / ID ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AppTag;
pub type AppHandle = Handle<AppTag>;

pub struct AppMarker;
pub type AppId = Id<AppMarker>;

// ── Per‑window resources ────────────────────────────────────────────────────

pub struct WindowResources {
    pub window_handle:   crate::render::WindowHandle,
    pub camera_handle:   CameraHandle,
    pub controller:      CameraController,
    pub material_pool:   vk::DescriptorPool,
    pub skin_pool:       vk::DescriptorPool,
    pub skin_set_layout: vk::DescriptorSetLayout,
    pub material_layout: vk::DescriptorSetLayout,
    pub instance_pool:   vk::DescriptorPool,
    pub instance_layout: vk::DescriptorSetLayout,
    pub winit_id:        WindowId,
    pub aspect:          f32,
    pub last_frame:      Instant,
    /// Last known absolute cursor position; used to compute per-event deltas.
    pub last_cursor:     Option<(f32, f32)>,
}

impl WindowResources {
    pub fn tick(&mut self) -> f32 {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        dt
    }

    pub fn update_aspect(&mut self, width: u32, height: u32) {
        if height > 0 {
            self.aspect = width as f32 / height as f32;
        }
    }
}

// ── AppData (field-private storage) ─────────────────────────────────────────

struct AppData {
    renderer:       Option<Renderer>,
    ctx:            Option<VulkanContext>,
    cameras:        CameraArena,
    windows:        Arena<AppTag, WindowResources>,
    next_app_id:    i64,
    next_camera_id: i64,
    /// Compute semaphores to wait on before the next draw, keyed by AppHandle.
    compute_waits:  HashMap<AppHandle, Vec<vk::Semaphore>>,
    /// Wall-clock start time — exposed via AppCtx::elapsed().
    start:          Instant,
}

impl AppData {
    fn new() -> Self {
        Self {
            renderer:       None,
            ctx:            None,
            cameras:        CameraArena::new(),
            windows:        Arena::new(),
            next_app_id:    1,
            next_camera_id: 1,
            compute_waits:  HashMap::new(),
            start:          Instant::now(),
        }
    }

    fn handle_of(&self, winit_id: WindowId) -> Option<AppHandle> {
        for (handle, res) in self.windows.entries() {
            if res.winit_id == winit_id {
                return Some(handle);
            }
        }
        None
    }
}

// ── AppCtx — borrow-split view passed to AppLogic methods ────────────────────
//
// Solves the double-mut-borrow issue with the prior `&mut AppRunner<Self>`
// signature: AppCtx field-borrows from AppData (renderer, vulkan, cameras,
// windows) so the logic field stays mutably borrowed from `self.data.logic`.

pub struct AppCtx<'a> {
    pub renderer: &'a mut Renderer,
    pub vulkan:   &'a VulkanContext,
    pub cameras:  &'a mut CameraArena,
    pub windows:  &'a mut Arena<AppTag, WindowResources>,
    pub next_app_id:    &'a mut i64,
    pub next_camera_id: &'a mut i64,
    compute_waits: &'a mut HashMap<AppHandle, Vec<vk::Semaphore>>,
    start:         Instant,
}

impl<'a> AppCtx<'a> {
    /// Spawn a new window with its own camera. Returns the app handle the
    /// logic can store + look up via `windows.get(handle)`.
    pub fn spawn_window(
        &mut self,
        event_loop:    &ActiveEventLoop,
        title:         &str,
        width:         u32,
        height:        u32,
    ) -> ForgeResult<AppHandle> {
        let camera = Camera::new(
            crate::render::camera::CameraId::new(*self.next_camera_id),
            Arc::from(title),
            [0.0, 0.0, 5.0], 0.0, 0.0,
        );
        self.spawn_window_with_camera(event_loop, title, width, height, camera, 5.0, 0.005)
    }

    pub fn spawn_window_with_camera(
        &mut self,
        event_loop:    &ActiveEventLoop,
        title:         &str,
        width:         u32,
        height:        u32,
        camera:        Camera,
        move_speed:    f32,
        mouse_sens:    f32,
    ) -> ForgeResult<AppHandle> {
        let ctx = self.vulkan;
        let attrs = WinitWindow::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height));
        let winit_window = Arc::new(
            event_loop.create_window(attrs)
                .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?,
        );
        let winit_id = winit_window.id();

        let graphics_forge = self.renderer.forge
            .graphics_forge(GraphicsOreKind::ForwardLit)
            .ok_or_else(|| ForgeError::Io(io::Error::other("No ForwardLit forge registered")))?;

        let window = Window::new_with_surface(
            RenderWindowId::new(*self.next_app_id),
            title,
            winit_window.clone(),
            &ctx.instance,
            ctx.physical_device,
            &ctx.device,
            ctx.queue,
            ctx.queue_family_index,
            &ctx.memory_properties,
            ctx.depth_format,
            ctx.msaa_samples,
            &ctx.entry,
            graphics_forge,
        )?;

        let window_handle = self.renderer.add_window(window);
        let material_layout = self.renderer
            .window(window_handle)
            .and_then(|w| w.graphics.as_ref())
            .map(|g| g.mold.material_set_layout)
            .unwrap_or(vk::DescriptorSetLayout::null());
        let material_pool   = create_material_pool(&ctx.device, 4096)?;
        let skin_set_layout = create_skin_palette_set_layout(&ctx.device)?;
        let skin_pool       = create_skin_palette_pool(&ctx.device, 256)?;
        let instance_layout = create_instance_set_layout(&ctx.device)?;
        let instance_pool   = create_instance_pool(&ctx.device, 4096)?;

        let camera_handle = self.cameras.insert(camera);
        let controller    = CameraController::new(move_speed, mouse_sens);
        let aspect = if height > 0 { width as f32 / height as f32 } else { 1.0 };

        let resources = WindowResources {
            window_handle, camera_handle, controller,
            material_pool, skin_pool, skin_set_layout, material_layout,
            instance_pool, instance_layout,
            winit_id, aspect, last_frame: Instant::now(), last_cursor: None,
        };
        let app_handle = self.windows.insert(resources);
        *self.next_app_id    += 1;
        *self.next_camera_id += 1;
        Ok(app_handle)
    }

    pub fn camera(&self, app: AppHandle) -> Option<&Camera> {
        let r = self.windows.get(app)?;
        self.cameras.get(r.camera_handle)
    }
    pub fn camera_mut(&mut self, app: AppHandle) -> Option<&mut Camera> {
        let h = self.windows.get(app)?.camera_handle;
        self.cameras.get_mut(h)
    }

    /// Seconds since `AppRunner::run()` was called.
    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }

    /// View-projection matrix for the window's camera at its current aspect.
    /// Returns identity if the window or camera does not exist.
    pub fn camera_vp(&self, app: AppHandle) -> [f32; 16] {
        let res = match self.windows.get(app) { Some(r) => r, None => return [0.0; 16] };
        let cam = match self.cameras.get(res.camera_handle) { Some(c) => c, None => return [0.0; 16] };
        cam.view_projection_matrix(res.aspect)
    }

    /// Create a [`GltfScene`] pre-configured with the descriptor layouts from
    /// `app`'s window.  Call [`GltfScene::load`] on the result to start
    /// background asset loading.
    pub fn new_gltf_scene(&self, app: AppHandle) -> ForgeResult<GltfScene> {
        let res = self.windows.get(app)
            .ok_or_else(|| ForgeError::Io(io::Error::other("window not found")))?;
        GltfScene::new(self.vulkan.device.clone(), res.material_layout)
    }

    /// Drive a scene for one frame: animate, compute dispatch, graphics plan.
    ///
    /// If a compute semaphore is returned you should call
    /// `push_compute_wait(app, sem)` so the draw submit waits for it.
    pub fn gltf_update(
        &mut self,
        scene:   &mut GltfScene,
        app:     AppHandle,
        elapsed: f32,
        vp:      &[f32; 16],
    ) -> ForgeResult<Option<vk::Semaphore>> {
        let wh = self.windows.get(app)
            .ok_or_else(|| ForgeError::Io(io::Error::other("window not found")))?
            .window_handle;
        scene.update(self.vulkan, self.renderer, wh, vp, elapsed)
    }

    /// Queue a compute semaphore that will be waited on before drawing `app`'s
    /// window this frame.  Semaphores are consumed and cleared after each draw.
    pub fn push_compute_wait(&mut self, app: AppHandle, sem: vk::Semaphore) {
        self.compute_waits.entry(app).or_default().push(sem);
    }
}

// ── AppLogic trait ───────────────────────────────────────────────────────────

pub trait AppLogic: 'static {
    /// Called once after Vulkan + Renderer are ready, before the event
    /// loop spins. Typical use: register compute forges, spawn windows,
    /// load initial assets.
    fn on_start(&mut self, _ctx: &mut AppCtx<'_>, _event_loop: &ActiveEventLoop) -> ForgeResult<()> {
        Ok(())
    }

    /// Called per frame after winit fires RedrawRequested. Return `false`
    /// to request exit.
    fn update(&mut self, _ctx: &mut AppCtx<'_>, _app: AppHandle, _dt: f32) -> bool {
        true
    }

    /// Called per winit event. Return `true` if handled (skips default
    /// camera-controller handling for that event).
    fn handle_event(&mut self, _ctx: &mut AppCtx<'_>, _app: AppHandle, _event: &WindowEvent) -> bool {
        false
    }

    /// Called during initialisation to register any custom compute /
    /// graphics forges this app needs (beyond the engine's defaults).
    fn register_forges(&mut self, _forge: &mut ForgeMaster) -> ForgeResult<()> {
        Ok(())
    }
}

// ── AppRunner ────────────────────────────────────────────────────────────────

pub struct AppRunner<T: AppLogic> {
    logic: T,
    data:  AppData,
}

impl<T: AppLogic> AppRunner<T> {
    pub fn new(logic: T) -> Self {
        Self { logic, data: AppData::new() }
    }

    pub fn run(mut self) -> ForgeResult<()> {
        let event_loop = EventLoop::new()
            .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?;
        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop.run_app(&mut self)
            .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))
    }

    /// Initialise VulkanContext + Renderer lazily on first `resumed()`.
    fn ensure_initialised(&mut self, event_loop: &ActiveEventLoop) -> ForgeResult<()> {
        if self.data.ctx.is_some() { return Ok(()); }
        let display_handle = event_loop.display_handle()
            .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?
            .as_raw();
        let ctx = VulkanContext::with_surface(display_handle)?;
        let mut forge = ForgeMaster::new(
            ctx.device.clone(), ctx.queue, ctx.command_pool, ctx.memory_properties,
        )?;
        // Register engine defaults so apps don't have to.
        forge.add_graphics_forge_from_spirv_bytes(
            GraphicsForgeId::new(1), GraphicsOreKind::ForwardLit,
            FORWARD_LIT_VERT, FORWARD_LIT_FRAG,
        )?;
        forge.add_graphics_forge_from_spirv_bytes(
            GraphicsForgeId::new(2), GraphicsOreKind::SkinnedForwardLit,
            SKINNED_VERT, FORWARD_LIT_FRAG,
        )?;
        register_skin_morph_forges(&mut forge)?;
        // App-specific forges on top of the defaults.
        self.logic.register_forges(&mut forge)?;
        let renderer = Renderer::new(forge);
        self.data.ctx      = Some(ctx);
        self.data.renderer = Some(renderer);
        Ok(())
    }

    fn ctx_for_logic<'a>(data: &'a mut AppData) -> AppCtx<'a> {
        AppCtx {
            renderer: data.renderer.as_mut().expect("renderer ready"),
            vulkan:   data.ctx.as_ref().expect("vulkan ready"),
            cameras:  &mut data.cameras,
            windows:  &mut data.windows,
            next_app_id:    &mut data.next_app_id,
            next_camera_id: &mut data.next_camera_id,
            compute_waits:  &mut data.compute_waits,
            start:          data.start,
        }
    }
}

impl<T: AppLogic> ApplicationHandler for AppRunner<T> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(e) = self.ensure_initialised(event_loop) {
            eprintln!("Vulkan init failed: {e}");
            event_loop.exit();
            return;
        }
        let mut ctx = Self::ctx_for_logic(&mut self.data);
        if let Err(e) = self.logic.on_start(&mut ctx, event_loop) {
            eprintln!("on_start error: {e}");
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        winit_id:   WindowId,
        event:      WindowEvent,
    ) {
        let Some(app_handle) = self.data.handle_of(winit_id) else { return };

        // 1) Logic gets first crack at the event (read-only path).
        {
            let mut ctx = Self::ctx_for_logic(&mut self.data);
            if self.logic.handle_event(&mut ctx, app_handle, &event) {
                if matches!(event, WindowEvent::RedrawRequested) {
                    self.draw_one(app_handle, event_loop);
                }
                return;
            }
        }

        // 2) Default handling.
        match &event {
            WindowEvent::CloseRequested => { event_loop.exit(); }
            WindowEvent::Resized(size) => {
                if let Some(res) = self.data.windows.get_mut(app_handle) {
                    res.update_aspect(size.width, size.height);
                    if let Some(renderer) = self.data.renderer.as_mut() {
                        if let Some(window) = renderer.window_mut(res.window_handle) {
                            window.resize(size.width, size.height);
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput { event: key_event, .. } => {
                if let PhysicalKey::Code(keycode) = key_event.physical_key {
                    if let Some(res) = self.data.windows.get_mut(app_handle) {
                        res.controller.handle_key(keycode, key_event.state);
                        if keycode == KeyCode::Escape && key_event.state == ElementState::Pressed {
                            let grabbed = res.controller.toggle_grab();
                            self.set_cursor_grab(app_handle, grabbed);
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos_x = position.x as f32;
                let pos_y = position.y as f32;
                let (grabbed, dyaw, dpitch) = {
                    let Some(res) = self.data.windows.get_mut(app_handle) else { return };
                    let (dx, dy) = match res.last_cursor {
                        Some((lx, ly)) => (pos_x - lx, pos_y - ly),
                        None => (0.0, 0.0),
                    };
                    res.last_cursor = Some((pos_x, pos_y));
                    if !res.controller.is_grabbed() { return; }
                    let (dyaw, dpitch) = res.controller.handle_mouse(dx, dy);
                    (true, dyaw, dpitch)
                };
                if grabbed {
                    if let Some(res) = self.data.windows.get(app_handle) {
                        let ch = res.camera_handle;
                        if let Some(cam) = self.data.cameras.get_mut(ch) {
                            cam.rotate(dyaw, dpitch);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { button, state, .. } => {
                if *button == MouseButton::Left && *state == ElementState::Pressed {
                    if let Some(res) = self.data.windows.get_mut(app_handle) {
                        let grabbed = res.controller.toggle_grab();
                        self.set_cursor_grab(app_handle, grabbed);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.draw_one(app_handle, event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(renderer) = self.data.renderer.as_ref() {
            for res in self.data.windows.values() {
                if let Some(window) = renderer.window(res.window_handle) {
                    if let Some(gfx) = &window.graphics {
                        gfx.winit_window.request_redraw();
                    }
                }
            }
        }
    }
}

impl<T: AppLogic> AppRunner<T> {
    fn draw_one(&mut self, app_handle: AppHandle, event_loop: &ActiveEventLoop) {
        // Tick controller + camera + custom update.
        let (window_h, dt) = {
            let Some(res) = self.data.windows.get_mut(app_handle) else { return };
            let dt = res.tick();
            let camera_h = res.camera_handle;
            if let Some(cam) = self.data.cameras.get_mut(camera_h) {
                res.controller.update(cam, dt);
            }
            (res.window_handle, dt)
        };
        {
            let mut ctx = Self::ctx_for_logic(&mut self.data);
            let keep_running = self.logic.update(&mut ctx, app_handle, dt);
            if !keep_running {
                event_loop.exit();
                return;
            }
        }
        // Collect and drain any compute semaphores the logic registered.
        let compute_sems: Vec<vk::Semaphore> = self.data
            .compute_waits
            .remove(&app_handle)
            .unwrap_or_default();

        if let (Some(renderer), Some(ctx)) =
            (self.data.renderer.as_mut(), self.data.ctx.as_ref())
        {
            if let Some(window) = renderer.window_mut(window_h) {
                unsafe {
                    let result = window.draw_frame_with_compute_wait(
                        &ctx.instance, &ctx.device, ctx.queue, &compute_sems,
                    );
                    if let Err(e) = result {
                        eprintln!("draw_frame error: {e:?}");
                        event_loop.exit();
                    }
                }
            }
        }
    }

    fn set_cursor_grab(&self, app_handle: AppHandle, grabbed: bool) {
        let Some(res) = self.data.windows.get(app_handle) else { return };
        if let Some(renderer) = self.data.renderer.as_ref() {
            if let Some(window) = renderer.window(res.window_handle) {
                if let Some(gfx) = &window.graphics {
                    let mode = if grabbed { CursorGrabMode::Locked } else { CursorGrabMode::None };
                    let _ = gfx.winit_window.set_cursor_grab(mode);
                    let _ = gfx.winit_window.set_cursor_visible(!grabbed);
                }
            }
        }
    }
}
