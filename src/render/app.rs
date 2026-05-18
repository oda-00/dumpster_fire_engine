//! Generic application runner – entry point for any engine-based app.
//!
//! Usage:
//! ```
//! struct MyGame;
//! impl AppLogic for MyGame {
//!     fn on_start(&mut self, runner: &mut AppRunner<Self>, event_loop: &ActiveEventLoop) {
//!         runner.spawn_window(event_loop, "Main", 1024, 768, Camera::new([0,0,5], 0.0, 0.0), 5.0, 0.005);
//!     }
//!     fn on_update(&mut self, runner: &mut AppRunner<Self>, dt: f32) -> bool { true }
//! }
//! fn main() { AppRunner::new(MyGame).run().unwrap(); }
//! ```

use std::io;
use std::sync::Arc;
use std::time::Instant;
use thin_vec::ThinVec;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{CursorGrabMode, Window as WinitWindow, WindowId};

use crate::forge_master::master::{ForgeError, ForgeResult};
use crate::forge_master::{ForgeMaster, GraphicsOreKind};
use crate::render::camera::{Camera, CameraArena, CameraController, CameraHandle, CameraId};
use crate::render::{Renderer, VulkanContext, Window, WindowId as RenderWindowId};
use crate::resource_manager::gltf_driver::{
    create_material_pool, create_skin_palette_pool, create_skin_palette_set_layout,
};
use crate::resource_manager::manager::{Arena, Handle, Id};

// ── App handle / ID ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AppTag;
pub type AppHandle = Handle<AppTag>;

pub struct AppMarker;
pub type AppId = Id<AppMarker>;

// ── Per‑window resources ────────────────────────────────────────────────────

struct WindowResources {
    window_handle: crate::render::WindowHandle,
    camera_handle: CameraHandle,
    controller: CameraController,
    material_pool: vk::DescriptorPool,
    skin_pool: vk::DescriptorPool,
    skin_set_layout: vk::DescriptorSetLayout,
    material_layout: vk::DescriptorSetLayout,
    winit_id: WindowId,
    aspect: f32,
    last_frame: Instant,
}

impl WindowResources {
    fn tick(&mut self) -> f32 {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        dt
    }

    fn update_aspect(&mut self, width: u32, height: u32) {
        if height > 0 {
            self.aspect = width as f32 / height as f32;
        }
    }
}

// ── App data storage ────────────────────────────────────────────────────────

struct AppData<T: AppLogic> {
    logic: T,
    renderer: Option<Renderer>,
    ctx: Option<VulkanContext>,
    cameras: CameraArena,
    windows: Arena<AppTag, WindowResources>,
    cache: ThinVec<AppHandle>,
    next_app_id: i64,
    next_camera_id: i64,
}

impl<T: AppLogic> AppData<T> {
    fn new(logic: T) -> Self {
        Self {
            logic,
            renderer: None,
            ctx: None,
            cameras: CameraArena::new(),
            windows: Arena::new(),
            cache: ThinVec::new(),
            next_app_id: 1,
            next_camera_id: 1,
        }
    }

    fn insert_window(&mut self, resources: WindowResources) -> AppHandle {
        let handle = self.windows.insert(resources);
        self.cache.push(handle);
        handle
    }

    fn get_window(&self, handle: AppHandle) -> Option<&WindowResources> {
        self.windows.get(handle)
    }

    fn get_window_mut(&mut self, handle: AppHandle) -> Option<&mut WindowResources> {
        self.windows.get_mut(handle)
    }

    fn remove_window(&mut self, handle: AppHandle) -> Option<WindowResources> {
        let res = self.windows.remove(handle)?;
        if let Some(pos) = self.cache.iter().position(|&h| h == handle) {
            self.cache.swap_remove(pos);
        }
        Some(res)
    }

    fn iter_windows(&self) -> impl Iterator<Item = &WindowResources> {
        self.windows.values()
    }

    fn iter_windows_mut(&mut self) -> impl Iterator<Item = &mut WindowResources> {
        self.windows.values_mut()
    }

    fn handle_of(&self, winit_id: WindowId) -> Option<AppHandle> {
        for &h in &self.cache {
            if let Some(app) = self.windows.get(h) {
                if app.winit_id == winit_id {
                    return Some(h);
                }
            }
        }
        None
    }
}

// ── Public AppRunner ────────────────────────────────────────────────────────

pub struct AppRunner<T: AppLogic> {
    data: AppData<T>,
}

impl<T: AppLogic> AppRunner<T> {
    pub fn new(logic: T) -> Self {
        Self {
            data: AppData::new(logic),
        }
    }

    pub fn run(mut self) -> ForgeResult<()> {
        let event_loop =
            EventLoop::new().map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?;
        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop
            .run_app(&mut self)
            .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))
    }

    /// Spawn a new window with its own camera.
    pub fn spawn_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        title: &str,
        width: u32,
        height: u32,
        camera: Camera,
        move_speed: f32,
        mouse_sensitivity: f32,
    ) -> ForgeResult<AppHandle> {
        let ctx = self
            .data
            .ctx
            .as_ref()
            .ok_or_else(|| ForgeError::Io(io::Error::other("Vulkan context not ready")))?;
        let renderer = self
            .data
            .renderer
            .as_mut()
            .ok_or_else(|| ForgeError::Io(io::Error::other("Renderer not ready")))?;

        let attrs = WinitWindow::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height));
        let winit_window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?,
        );
        let winit_id = winit_window.id();

        let display_handle = winit_window
            .display_handle()
            .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?
            .as_raw();
        let window_handle_raw = winit_window
            .window_handle()
            .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?
            .as_raw();
        let _surface = unsafe {
            ash_window::create_surface(
                &ctx.entry,
                &ctx.instance,
                display_handle,
                window_handle_raw,
                None,
            )
            .map_err(ForgeError::Vk)?
        };

        let graphics_forge = renderer
            .forge
            .graphics_forge(GraphicsOreKind::ForwardLit)
            .ok_or_else(|| ForgeError::Io(io::Error::other("No ForwardLit forge registered")))?;

        let window = Window::new_with_surface(
            RenderWindowId::new(self.data.next_app_id),
            title,
            winit_window.clone(),
            &ctx.instance,
            ctx.physical_device,
            &ctx.device,
            ctx.queue,
            ctx.queue_family_index,
            &ctx.memory_properties,
            ctx.depth_format,
            &ctx.entry,
            graphics_forge,
        )?;

        let window_handle = renderer.add_window(window);
        let material_layout = renderer
            .window(window_handle)
            .and_then(|w| w.graphics.as_ref())
            .map(|g| g.mold.material_set_layout)
            .unwrap_or(vk::DescriptorSetLayout::null());
        let material_pool = create_material_pool(&ctx.device, 4096)?;
        let skin_set_layout = create_skin_palette_set_layout(&ctx.device)?;
        let skin_pool = create_skin_palette_pool(&ctx.device, 256)?;

        let camera_handle = self.data.cameras.insert(camera);
        let controller = CameraController::new(move_speed, mouse_sensitivity);
        let aspect = if height > 0 {
            width as f32 / height as f32
        } else {
            1.0
        };

        let resources = WindowResources {
            window_handle,
            camera_handle,
            controller,
            material_pool,
            skin_pool,
            skin_set_layout,
            material_layout,
            winit_id,
            aspect,
            last_frame: Instant::now(),
        };
        let app_handle = self.data.insert_window(resources);
        self.data.next_app_id += 1;
        self.data.next_camera_id += 1;
        Ok(app_handle)
    }

    pub fn renderer(&self) -> &Renderer {
        self.data.renderer.as_ref().unwrap()
    }
    pub fn renderer_mut(&mut self) -> &mut Renderer {
        self.data.renderer.as_mut().unwrap()
    }
    pub fn vulkan_context(&self) -> &VulkanContext {
        self.data.ctx.as_ref().unwrap()
    }
    pub fn cameras(&self) -> &CameraArena {
        &self.data.cameras
    }
    pub fn cameras_mut(&mut self) -> &mut CameraArena {
        &mut self.data.cameras
    }
    pub fn camera(&self, app_handle: AppHandle) -> Option<&Camera> {
        let res = self.data.get_window(app_handle)?;
        self.data.cameras.get(res.camera_handle)
    }
    pub fn camera_mut(&mut self, app_handle: AppHandle) -> Option<&mut Camera> {
        let res = self.data.get_window_mut(app_handle)?;
        self.data.cameras.get_mut(res.camera_handle)
    }
    pub fn controller(&self, app_handle: AppHandle) -> Option<&CameraController> {
        self.data.get_window(app_handle).map(|r| &r.controller)
    }
    pub fn controller_mut(&mut self, app_handle: AppHandle) -> Option<&mut CameraController> {
        self.data
            .get_window_mut(app_handle)
            .map(|r| &mut r.controller)
    }
    pub fn window_handle(&self, app_handle: AppHandle) -> Option<crate::render::WindowHandle> {
        self.data.get_window(app_handle).map(|r| r.window_handle)
    }
    pub fn window_resources(&self, app_handle: AppHandle) -> Option<&WindowResources> {
        self.data.get_window(app_handle)
    }

    fn process_window(
        &mut self,
        app_handle: AppHandle,
        winit_id: WindowId,
        event: &WindowEvent,
        event_loop: &ActiveEventLoop,
    ) -> ForgeResult<bool> {
        let resources = match self.data.get_window_mut(app_handle) {
            Some(r) => r,
            None => return Ok(false),
        };
        let camera = match self.data.cameras.get_mut(resources.camera_handle) {
            Some(c) => c,
            None => return Ok(false),
        };
        let controller = &mut resources.controller;

        if self
            .data
            .logic
            .handle_event(app_handle, camera, controller, event)
        {
            return Ok(true);
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return Ok(true);
            }
            WindowEvent::Resized(size) => {
                resources.update_aspect(size.width, size.height);
                if let Some(renderer) = self.data.renderer.as_mut() {
                    if let Some(window) = renderer.window_mut(resources.window_handle) {
                        window.resize(size.width, size.height);
                    }
                }
            }
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                if let PhysicalKey::Code(keycode) = key_event.physical_key {
                    controller.handle_key(keycode, key_event.state);
                    if keycode == KeyCode::Escape && key_event.state == ElementState::Pressed {
                        let grabbed = controller.toggle_grab();
                        if let Some(renderer) = self.data.renderer.as_ref() {
                            if let Some(window) = renderer.window(resources.window_handle) {
                                if let Some(gfx) = &window.graphics {
                                    let mode = if grabbed {
                                        CursorGrabMode::Locked
                                    } else {
                                        CursorGrabMode::None
                                    };
                                    let _ = gfx.winit_window.set_cursor_grab(mode);
                                    let _ = gfx.winit_window.set_cursor_visible(!grabbed);
                                }
                            }
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } if controller.is_grabbed() => {
                let (dyaw, dpitch) = controller.handle_mouse(position.x as f32, position.y as f32);
                camera.rotate(dyaw, dpitch);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                if *button == MouseButton::Left && *state == ElementState::Pressed {
                    let grabbed = controller.toggle_grab();
                    if let Some(renderer) = self.data.renderer.as_ref() {
                        if let Some(window) = renderer.window(resources.window_handle) {
                            if let Some(gfx) = &window.graphics {
                                let mode = if grabbed {
                                    CursorGrabMode::Locked
                                } else {
                                    CursorGrabMode::None
                                };
                                let _ = gfx.winit_window.set_cursor_grab(mode);
                                let _ = gfx.winit_window.set_cursor_visible(!grabbed);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn update_window(
        &mut self,
        app_handle: AppHandle,
        _event_loop: &ActiveEventLoop,
    ) -> ForgeResult<bool> {
        let resources = match self.data.get_window_mut(app_handle) {
            Some(r) => r,
            None => return Ok(true),
        };
        let camera = match self.data.cameras.get_mut(resources.camera_handle) {
            Some(c) => c,
            None => return Ok(true),
        };
        let dt = resources.tick();
        resources.controller.update(camera, dt);

        let keep_running =
            self.data
                .logic
                .update(app_handle, camera, &resources.controller, self, dt);
        if !keep_running {
            return Ok(false);
        }

        if let Some(renderer) = self.data.renderer.as_mut() {
            let window = renderer.window_mut(resources.window_handle).unwrap();
            let ctx = self.data.ctx.as_ref().unwrap();
            unsafe {
                window.draw_frame(&ctx.instance, &ctx.device, ctx.queue)?;
            }
        }
        Ok(true)
    }

    fn ensure_initialised(&mut self, event_loop: &ActiveEventLoop) -> ForgeResult<()> {
        if self.data.ctx.is_some() {
            return Ok(());
        }
        // Dummy window to get display handle
        let dummy_attrs = WinitWindow::default_attributes()
            .with_title("dummy")
            .with_visible(false);
        let dummy_window = event_loop
            .create_window(dummy_attrs)
            .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?;
        let display_handle = dummy_window
            .display_handle()
            .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?
            .as_raw();
        let ctx = VulkanContext::with_surface(display_handle)?;
        let mut forge = ForgeMaster::new(
            ctx.device.clone(),
            ctx.queue,
            ctx.command_pool,
            ctx.memory_properties,
        )?;
        self.data.logic.register_forges(&mut forge)?;
        let renderer = Renderer::new(forge);
        self.data.ctx = Some(ctx);
        self.data.renderer = Some(renderer);
        Ok(())
    }
}

impl<T: AppLogic> ApplicationHandler for AppRunner<T> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(e) = self.ensure_initialised(event_loop) {
            eprintln!("Failed to initialise Vulkan: {e}");
            event_loop.exit();
        }
        if let Err(e) = self.data.logic.on_start(self, event_loop) {
            eprintln!("on_start error: {e}");
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        winit_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(app_handle) = self.data.handle_of(winit_id) else {
            return;
        };
        if let Err(e) = self.process_window(app_handle, winit_id, &event, event_loop) {
            eprintln!("Window event error: {e}");
            event_loop.exit();
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            if let Err(e) = self.update_window(app_handle, event_loop) {
                eprintln!("Update error: {e}");
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(renderer) = self.data.renderer.as_ref() {
            for res in self.data.iter_windows() {
                if let Some(window) = renderer.window(res.window_handle) {
                    if let Some(gfx) = &window.graphics {
                        gfx.winit_window.request_redraw();
                    }
                }
            }
        }
    }
}

// ── AppLogic trait (public) ─────────────────────────────────────────────────

pub trait AppLogic {
    fn on_start(
        &mut self,
        _runner: &mut AppRunner<Self>,
        _event_loop: &ActiveEventLoop,
    ) -> ForgeResult<()> {
        Ok(())
    }
    fn update(
        &mut self,
        _app_handle: AppHandle,
        _camera: &mut Camera,
        _controller: &CameraController,
        _runner: &mut AppRunner<Self>,
        _dt: f32,
    ) -> bool {
        true
    }
    fn handle_event(
        &mut self,
        _app_handle: AppHandle,
        _camera: &mut Camera,
        _controller: &mut CameraController,
        _event: &WindowEvent,
    ) -> bool {
        false
    }
    fn register_forges(&mut self, _forge: &mut ForgeMaster) -> ForgeResult<()> {
        Ok(())
    }
}
