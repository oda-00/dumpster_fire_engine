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
use std::io;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;
use thin_vec::ThinVec;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::{CursorGrabMode, Window as WinitWindow, WindowId};

#[cfg(feature = "wgpu-backend")]
use crate::render::backend::{DrawListSnapshot, GpuSurface, RenderSceneInput};
#[cfg(feature = "wgpu-backend")]
use crate::render::wgpu_surface::WgpuSurface;

use crate::forge_master::master::{ForgeError, ForgeResult};
use crate::forge_master::ore::MAT4_IDENTITY;
use crate::forge_master::{ForgeMaster, GraphicsForgeId, GraphicsOreKind};
use crate::render::camera::{Camera, CameraArena, CameraController, CameraHandle, ProjectionMode};
use crate::render::gltf_assets::{GltfAssetCache, fit_camera_to_aabb};
use crate::render::render_world::{CameraView, collect_and_submit, extract_frustum_planes};
use crate::render::{Renderer, VulkanContext, Window, WindowId as RenderWindowId};
use crate::resource_manager::asset_manager::gltf_loader::register_skin_morph_forges;
use crate::resource_manager::component::GltfHandle;
use crate::resource_manager::gltf_driver::{
    create_instance_pool, create_instance_set_layout, create_material_pool,
    create_skin_palette_pool, create_skin_palette_set_layout,
};
use crate::resource_manager::manager::{Arena, Handle, Id};
use crate::resource_manager::world_manager::World;

// Shader bytes embedded once here so AppRunner can register default forges.
const FORWARD_LIT_VERT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/shaders/forward_lit.vert.spv"
));
const FORWARD_LIT_FRAG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/shaders/forward_lit.frag.spv"
));
const SKINNED_VERT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/shaders/skinned_forward_lit.vert.spv"
));

// ── App handle / ID ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AppTag;
pub type AppHandle = Handle<AppTag>;

pub struct AppMarker;
pub type AppId = Id<AppMarker>;

// ── Viewport system ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ViewportTag;
pub type ViewportHandle = Handle<ViewportTag>;

/// Which kind of camera projection this pane uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportKind {
    Perspective,
    OrthoTop,
    OrthoFront,
    OrthoRight,
}

/// How the window is partitioned into panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportLayout {
    Single,
    TwoColumns,
    TwoRows,
    FourQuadrant,
}

impl ViewportLayout {
    pub fn viewport_count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::TwoColumns => 2,
            Self::TwoRows => 2,
            Self::FourQuadrant => 4,
        }
    }

    pub fn kind_at(self, i: usize) -> ViewportKind {
        match self {
            Self::Single => [ViewportKind::Perspective][i.min(0)],
            Self::TwoColumns => [ViewportKind::Perspective, ViewportKind::OrthoTop][i.min(1)],
            Self::TwoRows => [ViewportKind::Perspective, ViewportKind::OrthoFront][i.min(1)],
            Self::FourQuadrant => [
                ViewportKind::Perspective,
                ViewportKind::OrthoTop,
                ViewportKind::OrthoFront,
                ViewportKind::OrthoRight,
            ][i.min(3)],
        }
    }

    pub fn rects(self) -> &'static [NormRect] {
        match self {
            Self::Single => &[NormRect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            }],
            Self::TwoColumns => &[
                NormRect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.5,
                    h: 1.0,
                },
                NormRect {
                    x: 0.5,
                    y: 0.0,
                    w: 0.5,
                    h: 1.0,
                },
            ],
            Self::TwoRows => &[
                NormRect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 0.5,
                },
                NormRect {
                    x: 0.0,
                    y: 0.5,
                    w: 1.0,
                    h: 0.5,
                },
            ],
            Self::FourQuadrant => &[
                NormRect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.5,
                    h: 0.5,
                },
                NormRect {
                    x: 0.5,
                    y: 0.0,
                    w: 0.5,
                    h: 0.5,
                },
                NormRect {
                    x: 0.0,
                    y: 0.5,
                    w: 0.5,
                    h: 0.5,
                },
                NormRect {
                    x: 0.5,
                    y: 0.5,
                    w: 0.5,
                    h: 0.5,
                },
            ],
        }
    }

    /// Cycle to the next layout.
    pub fn next(self) -> Self {
        match self {
            Self::Single => Self::TwoColumns,
            Self::TwoColumns => Self::TwoRows,
            Self::TwoRows => Self::FourQuadrant,
            Self::FourQuadrant => Self::Single,
        }
    }
}

/// Normalised [0, 1] screen-space rect for a viewport pane.
#[derive(Debug, Clone, Copy)]
pub struct NormRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl NormRect {
    #[inline]
    pub fn to_vk_viewport(&self, e: vk::Extent2D) -> vk::Viewport {
        vk::Viewport {
            x: self.x * e.width as f32,
            y: self.y * e.height as f32,
            width: self.w * e.width as f32,
            height: self.h * e.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }
    }

    #[inline]
    pub fn to_vk_scissor(&self, e: vk::Extent2D) -> vk::Rect2D {
        vk::Rect2D {
            offset: vk::Offset2D {
                x: (self.x * e.width as f32).round() as i32,
                y: (self.y * e.height as f32).round() as i32,
            },
            extent: vk::Extent2D {
                width: (self.w * e.width as f32).round() as u32,
                height: (self.h * e.height as f32).round() as u32,
            },
        }
    }

    #[inline]
    pub fn contains(&self, cx: f32, cy: f32, win_w: f32, win_h: f32) -> bool {
        let nx = if win_w > 0.0 { cx / win_w } else { 0.0 };
        let ny = if win_h > 0.0 { cy / win_h } else { 0.0 };
        nx >= self.x && nx < self.x + self.w && ny >= self.y && ny < self.y + self.h
    }

    #[inline]
    pub fn pixel_aspect(&self, win_w: f32, win_h: f32) -> f32 {
        let ph = self.h * win_h;
        if ph > 0.0 { (self.w * win_w) / ph } else { 1.0 }
    }
}

/// One pane in the editor viewport grid.
pub struct EditorViewport {
    pub kind: ViewportKind,
    pub camera_handle: CameraHandle,
    pub rect: NormRect,
    pub controller: CameraController,
}

/// Multi-pane viewport grid. Mirrors `CameraArena`: Arena + ThinVec cache.
pub struct ViewportGrid {
    arenas: Arena<ViewportTag, EditorViewport>,
    cache: thin_vec::ThinVec<ViewportHandle>,
    pub layout: ViewportLayout,
    pub focused: ViewportHandle,
    pub win_w: f32,
    pub win_h: f32,
}

impl ViewportGrid {
    pub fn new(layout: ViewportLayout, cameras: &[CameraHandle], win_w: f32, win_h: f32) -> Self {
        let mut arenas = Arena::new();
        let mut cache = thin_vec::ThinVec::new();
        let rects = layout.rects();
        let n = layout.viewport_count();
        for (i, &rect) in rects[..n].iter().enumerate() {
            let cam = cameras
                .get(i)
                .or_else(|| cameras.last())
                .copied()
                .expect("at least one CameraHandle required");
            let h = arenas.insert(EditorViewport {
                kind: layout.kind_at(i),
                camera_handle: cam,
                rect,
                controller: CameraController::new(5.0, 0.005),
            });
            cache.push(h);
        }
        let focused = cache[0];
        Self {
            arenas,
            cache,
            layout,
            focused,
            win_w,
            win_h,
        }
    }

    pub fn get(&self, h: ViewportHandle) -> Option<&EditorViewport> {
        self.arenas.get(h)
    }

    pub fn get_mut(&mut self, h: ViewportHandle) -> Option<&mut EditorViewport> {
        self.arenas.get_mut(h)
    }

    /// Iterate panes in layout slot order.
    pub fn iter(&self) -> impl Iterator<Item = (ViewportHandle, &EditorViewport)> {
        self.cache
            .iter()
            .filter_map(|&h| self.arenas.get(h).map(|vp| (h, vp)))
    }

    /// Return the handle for the i-th layout slot.
    pub fn slot(&self, i: usize) -> Option<ViewportHandle> {
        self.cache.get(i).copied()
    }

    pub fn focused_camera(&self) -> Option<CameraHandle> {
        self.arenas.get(self.focused).map(|vp| vp.camera_handle)
    }

    /// Hit-test cursor position; updates `self.focused`. O(N), N ≤ 4.
    pub fn hit_test_focus(&mut self, cx: f32, cy: f32) {
        for &h in &self.cache {
            if let Some(vp) = self.arenas.get(h)
                && vp.rect.contains(cx, cy, self.win_w, self.win_h)
            {
                self.focused = h;
                return;
            }
        }
    }

    pub fn on_resize(&mut self, w: f32, h: f32) {
        self.win_w = w;
        self.win_h = h;
    }

    /// Rebuild layout reusing cameras from existing slots in order.
    /// `extra_cameras` provides handles for any additional slots.
    pub fn set_layout(&mut self, layout: ViewportLayout, extra_cameras: &[CameraHandle]) {
        // Collect existing camera handles in slot order before clearing.
        let old_cams: thin_vec::ThinVec<CameraHandle> = self
            .cache
            .iter()
            .filter_map(|&h| self.arenas.get(h).map(|vp| vp.camera_handle))
            .collect();

        // Clear old panes.
        for &h in &self.cache {
            self.arenas.remove(h);
        }
        self.cache.clear();

        // Re-insert with new rects and kinds.
        let rects = layout.rects();
        let n = layout.viewport_count();
        for (i, &rect) in rects[..n].iter().enumerate() {
            let cam = old_cams
                .get(i)
                .or_else(|| extra_cameras.get(i.saturating_sub(old_cams.len())))
                .or_else(|| old_cams.last())
                .copied()
                .expect("no camera handle available for new layout slot");
            let h = self.arenas.insert(EditorViewport {
                kind: layout.kind_at(i),
                camera_handle: cam,
                rect,
                controller: CameraController::new(5.0, 0.005),
            });
            self.cache.push(h);
        }
        self.layout = layout;
        self.focused = self.cache[0];
    }
}

// ── Per‑window resources ────────────────────────────────────────────────────

pub struct WindowResources {
    pub window_handle: crate::render::WindowHandle,
    pub camera_handle: CameraHandle,
    pub controller: CameraController,
    pub material_pool: vk::DescriptorPool,
    pub skin_pool: vk::DescriptorPool,
    pub skin_set_layout: vk::DescriptorSetLayout,
    pub material_layout: vk::DescriptorSetLayout,
    pub instance_pool: vk::DescriptorPool,
    pub instance_layout: vk::DescriptorSetLayout,
    pub winit_id: WindowId,
    pub aspect: f32,
    pub last_frame: Instant,
    /// Last known absolute cursor position; used to compute per-event deltas.
    pub last_cursor: Option<(f32, f32)>,
    /// Multi-pane viewport grid. `None` = single-viewport legacy mode.
    pub viewport_grid: Option<ViewportGrid>,
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
            if let Some(grid) = &mut self.viewport_grid {
                grid.on_resize(width as f32, height as f32);
            }
        }
    }
}

// ── AppData (field-private storage) ─────────────────────────────────────────

struct AppData {
    renderer: Option<Renderer>,
    ctx: Option<VulkanContext>,
    cameras: CameraArena,
    windows: Arena<AppTag, WindowResources>,
    gltf_assets: GltfAssetCache,
    next_app_id: i64,
    next_camera_id: i64,
    /// Compute semaphores to wait on before the next draw, keyed by AppHandle.
    compute_waits: ThinVec<(AppHandle, ThinVec<vk::Semaphore>)>,
    /// Wall-clock start time — exposed via AppCtx::elapsed().
    start: Instant,
    /// wgpu fallback surface — set when Vulkan is unavailable.
    #[cfg(feature = "wgpu-backend")]
    wgpu_surface: Option<WgpuSurface>,
    /// winit window for the wgpu path (needed for cursor grab / resize).
    #[cfg(feature = "wgpu-backend")]
    wgpu_winit: Option<Arc<WinitWindow>>,
}

impl AppData {
    fn new() -> Self {
        Self {
            renderer: None,
            ctx: None,
            cameras: CameraArena::new(),
            windows: Arena::new(),
            gltf_assets: GltfAssetCache::new(),
            next_app_id: 1,
            next_camera_id: 1,
            compute_waits: ThinVec::new(),
            start: Instant::now(),
            #[cfg(feature = "wgpu-backend")]
            wgpu_surface: None,
            #[cfg(feature = "wgpu-backend")]
            wgpu_winit: None,
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
    pub renderer: Option<&'a mut Renderer>,
    pub vulkan: Option<&'a VulkanContext>,
    pub cameras: &'a mut CameraArena,
    pub windows: &'a mut Arena<AppTag, WindowResources>,
    pub gltf_assets: &'a mut GltfAssetCache,
    pub next_app_id: &'a mut i64,
    pub next_camera_id: &'a mut i64,
    compute_waits: &'a mut ThinVec<(AppHandle, ThinVec<vk::Semaphore>)>,
    start: Instant,
    #[cfg(feature = "wgpu-backend")]
    wgpu_surface: &'a mut Option<WgpuSurface>,
    #[cfg(feature = "wgpu-backend")]
    wgpu_winit: &'a mut Option<Arc<WinitWindow>>,
}

impl<'a> AppCtx<'a> {
    /// Spawn a new window with its own camera. Returns the app handle the
    /// logic can store + look up via `windows.get(handle)`.
    pub fn spawn_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        title: &str,
        width: u32,
        height: u32,
    ) -> ForgeResult<AppHandle> {
        let camera = Camera::new(
            crate::render::camera::CameraId::new(*self.next_camera_id),
            Arc::from(title),
            [0.0, 0.0, 5.0],
            0.0,
            0.0,
        );
        self.spawn_window_with_camera(event_loop, title, width, height, camera, 5.0, 0.005)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_window_with_camera(
        &mut self,
        event_loop: &ActiveEventLoop,
        title: &str,
        width: u32,
        height: u32,
        camera: Camera,
        move_speed: f32,
        mouse_sens: f32,
    ) -> ForgeResult<AppHandle> {
        let attrs = WinitWindow::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height));
        let winit_window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?,
        );
        let winit_id = winit_window.id();

        // ── Backend-specific window setup ──────────────────────────────────
        let (
            window_handle,
            material_pool,
            skin_pool,
            skin_set_layout,
            material_layout,
            instance_pool,
            instance_layout,
        ) = if let Some(ctx) = self.vulkan {
            let renderer = self
                .renderer
                .as_deref_mut()
                .ok_or_else(|| ForgeError::Io(io::Error::other("renderer not ready")))?;
            let graphics_forge = renderer
                .forge
                .graphics_forge(GraphicsOreKind::ForwardLit)
                .ok_or_else(|| {
                    ForgeError::Io(io::Error::other("No ForwardLit forge registered"))
                })?;
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
            let wh = renderer.add_window(window);
            let ml = renderer
                .window(wh)
                .and_then(|w| w.graphics.as_ref())
                .map(|g| g.mold.material_set_layout)
                .unwrap_or(vk::DescriptorSetLayout::null());
            let mp = create_material_pool(&ctx.device, 4096)?;
            let sl = create_skin_palette_set_layout(&ctx.device)?;
            let sp = create_skin_palette_pool(&ctx.device, 256)?;
            let il = create_instance_set_layout(&ctx.device)?;
            let ip = create_instance_pool(&ctx.device, 4096)?;
            (wh, mp, sp, sl, ml, ip, il)
        } else {
            // wgpu fallback path — create the wgpu surface lazily on the first window.
            #[cfg(feature = "wgpu-backend")]
            if self.wgpu_surface.is_none() {
                *self.wgpu_surface = Some(WgpuSurface::new(winit_window.clone())?);
                *self.wgpu_winit = Some(winit_window.clone());
            }
            // Sentinel handle: idx out of any arena range → Renderer::window() returns None.
            let dummy_handle = crate::render::window::WindowHandle {
                idx: u32::MAX,
                generation: NonZeroU32::new(1).unwrap(),
                _tag: PhantomData,
            };
            (
                dummy_handle,
                vk::DescriptorPool::null(),
                vk::DescriptorPool::null(),
                vk::DescriptorSetLayout::null(),
                vk::DescriptorSetLayout::null(),
                vk::DescriptorPool::null(),
                vk::DescriptorSetLayout::null(),
            )
        };

        let camera_handle = self.cameras.insert(camera);
        let controller = CameraController::new(move_speed, mouse_sens);
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
            instance_pool,
            instance_layout,
            winit_id,
            aspect,
            last_frame: Instant::now(),
            last_cursor: None,
            viewport_grid: None,
        };
        let app_handle = self.windows.insert(resources);
        *self.next_app_id += 1;
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
        let res = match self.windows.get(app) {
            Some(r) => r,
            None => return [0.0; 16],
        };
        let cam = match self.cameras.get(res.camera_handle) {
            Some(c) => c,
            None => return [0.0; 16],
        };
        cam.view_projection_matrix(res.aspect)
    }

    /// Start loading a glTF asset from `path` for `app`'s window.
    /// Deduplicated by path; returns a handle immediately.
    /// Call `poll_gltf_loaders` each frame to finalize.
    pub fn load_gltf(
        &mut self,
        app: AppHandle,
        path: std::path::PathBuf,
    ) -> ForgeResult<GltfHandle> {
        let ctx = self.vulkan.ok_or_else(|| {
            ForgeError::Io(io::Error::other("glTF loading requires Vulkan backend"))
        })?;
        let (device, material_layout) = {
            let res = self
                .windows
                .get(app)
                .ok_or_else(|| ForgeError::Io(io::Error::other("window not found")))?;
            (ctx.device.clone(), res.material_layout)
        };
        let path_str: Arc<str> = Arc::from(path.to_str().unwrap_or(""));
        self.gltf_assets.load(path_str, device, material_layout)
    }

    /// Drive background loaders; GPU-upload any that have completed.
    pub fn poll_gltf_loaders(&mut self, _app: AppHandle) -> ForgeResult<()> {
        let Some(ctx) = self.vulkan else {
            return Ok(());
        };
        self.gltf_assets.poll(ctx)
    }

    /// Render every visible actor in `world` to `app`'s window.
    pub fn render_world(
        &mut self,
        world: &World,
        app: AppHandle,
        elapsed: f32,
    ) -> ForgeResult<Option<vk::Semaphore>> {
        // ── wgpu path ──────────────────────────────────────────────────────
        #[cfg(feature = "wgpu-backend")]
        if self.vulkan.is_none() {
            if let Some(ws) = self.wgpu_surface.as_mut() {
                let dl = &world.ui.draw_list;
                let snapshot = DrawListSnapshot {
                    vertices: &dl.vertices,
                    indices: &dl.indices,
                };
                let scene = RenderSceneInput { world, elapsed };
                ws.draw_frame(&scene, &snapshot)?;
            }
            return Ok(None);
        }

        // ── Vulkan path ────────────────────────────────────────────────────
        let ctx = self.vulkan.expect("vulkan checked above");
        let renderer = self
            .renderer
            .as_deref_mut()
            .ok_or_else(|| ForgeError::Io(io::Error::other("renderer not ready")))?;

        let (window_h, cam_views) = {
            let res = self
                .windows
                .get(app)
                .ok_or_else(|| ForgeError::Io(io::Error::other("window not found")))?;
            let wh = res.window_handle;
            let mut views: ThinVec<CameraView> = ThinVec::new();
            if let Some(grid) = &res.viewport_grid {
                for (_, vp) in grid.iter() {
                    let aspect = vp.rect.pixel_aspect(grid.win_w, grid.win_h);
                    if let Some(cam) = self.cameras.get(vp.camera_handle) {
                        let vp_mat = cam.view_projection_matrix(aspect);
                        let frustum = extract_frustum_planes(&vp_mat);
                        views.push(CameraView {
                            frustum,
                            vp_matrix: vp_mat,
                        });
                    }
                }
            } else if let Some(cam) = self.cameras.get(res.camera_handle) {
                let vp_mat = cam.view_projection_matrix(res.aspect);
                let frustum = extract_frustum_planes(&vp_mat);
                views.push(CameraView {
                    frustum,
                    vp_matrix: vp_mat,
                });
            }
            (wh, views)
        };

        // Push the UI draw list into the window's overlay pipeline.
        let dl = &world.ui.draw_list;
        if !dl.indices.is_empty() {
            let vert_bytes = unsafe {
                std::slice::from_raw_parts(
                    dl.vertices.as_ptr() as *const u8,
                    std::mem::size_of_val(dl.vertices.as_slice()),
                )
            };
            let idx_bytes = unsafe {
                std::slice::from_raw_parts(
                    dl.indices.as_ptr() as *const u8,
                    std::mem::size_of_val(dl.indices.as_slice()),
                )
            };
            if let Some(window) = renderer.window_mut(window_h) {
                let _ = window.set_ui_draw(
                    &ctx.device,
                    vert_bytes,
                    idx_bytes,
                    dl.vertices.len() as u32,
                    dl.indices.len() as u32,
                );
            }
        } else {
            if let Some(window) = renderer.window_mut(window_h) {
                let _ = window.set_ui_draw(&ctx.device, &[], &[], 0, 0);
            }
        }

        collect_and_submit(
            world,
            self.gltf_assets,
            renderer,
            window_h,
            ctx,
            &cam_views,
            elapsed,
        )
    }

    /// AABB (world space) covering all loaded, visible mesh actors in `world`.
    pub fn gltf_union_aabb_for_world(&self, world: &World) -> Option<([f32; 3], [f32; 3])> {
        use crate::render::gltf_assets::transform_aabb;
        let mut mn = [f32::MAX; 3];
        let mut mx = [f32::MIN; 3];
        let mut found = false;
        for level in world.levels.values() {
            for stage in level.stages.values() {
                for (actor_h, actor) in stage.actors.entries() {
                    let wa: glam::Affine3A = stage.worlds[actor_h.idx as usize];
                    for se in actor.sub_entities.iter().flatten() {
                        let (visible, mesh_opt) = se.actor_type.visibility_and_mesh();
                        if !visible {
                            continue;
                        }
                        if let Some(mr) = mesh_opt
                            && let Some(loaded) = self.gltf_assets.get(mr.asset)
                        {
                            let ab = transform_aabb(&loaded.rest_aabb, &wa);
                            for i in 0..3 {
                                mn[i] = mn[i].min(ab.0[i]);
                                mx[i] = mx[i].max(ab.1[i]);
                            }
                            found = true;
                        }
                    }
                }
            }
        }
        if found { Some((mn, mx)) } else { None }
    }

    /// Fit the perspective pane camera (slot 0) to the given AABB.
    pub fn fit_perspective_pane_to_aabb(&mut self, app: AppHandle, aabb: &([f32; 3], [f32; 3])) {
        let cam_h = self
            .windows
            .get(app)
            .and_then(|r| r.viewport_grid.as_ref())
            .and_then(|g| {
                let vh = g.slot(0)?;
                Some(g.get(vh)?.camera_handle)
            });
        if let Some(h) = cam_h
            && let Some(cam) = self.cameras.get_mut(h)
        {
            fit_camera_to_aabb(cam, aabb);
        }
    }

    /// Fit every viewport pane to the AABB: perspective uses `fit_camera_to_aabb`;
    /// ortho panes get their `size`/`near`/`far` set from the scene radius and
    /// their position moved along their fixed view direction to frame the scene.
    pub fn fit_all_panes_to_aabb(&mut self, app: AppHandle, aabb: &([f32; 3], [f32; 3])) {
        let mn = glam::Vec3::from(aabb.0);
        let mx = glam::Vec3::from(aabb.1);
        let center = (mn + mx) * 0.5;
        let radius = ((mx - mn).length() * 0.5).max(0.01);

        let cam_handles: thin_vec::ThinVec<CameraHandle> = {
            let Some(res) = self.windows.get(app) else {
                return;
            };
            let Some(grid) = &res.viewport_grid else {
                return;
            };
            grid.arenas.values().map(|vp| vp.camera_handle).collect()
        };

        for h in cam_handles {
            if let Some(cam) = self.cameras.get_mut(h) {
                match cam.projection {
                    None | Some(ProjectionMode::Perspective { .. }) => {
                        fit_camera_to_aabb(cam, aabb);
                    }
                    Some(ProjectionMode::Orthographic {
                        ref mut size,
                        ref mut near,
                        ref mut far,
                    }) => {
                        *size = radius * 2.2;
                        *near = -(radius * 4.0);
                        *far = radius * 4.0;
                        let (sy, cy) = cam.yaw.sin_cos();
                        let (sp, cp) = cam.pitch.sin_cos();
                        let forward = glam::Vec3::new(cp * cy, sp, cp * sy).normalize();
                        let pos = center - forward * radius * 2.0;
                        cam.position = pos.into();
                    }
                }
            }
        }
    }

    /// Fit the window's main camera to the given AABB.
    pub fn fit_window_camera_to_aabb(&mut self, app: AppHandle, aabb: &([f32; 3], [f32; 3])) {
        let h = self.windows.get(app).map(|r| r.camera_handle);
        if let Some(h) = h
            && let Some(cam) = self.cameras.get_mut(h)
        {
            fit_camera_to_aabb(cam, aabb);
        }
    }

    /// Queue a compute semaphore that will be waited on before drawing `app`'s
    /// window this frame.  Semaphores are consumed and cleared after each draw.
    pub fn push_compute_wait(&mut self, app: AppHandle, sem: vk::Semaphore) {
        if let Some(entry) = self.compute_waits.iter_mut().find(|(h, _)| *h == app) {
            entry.1.push(sem);
        } else {
            let mut v = ThinVec::new();
            v.push(sem);
            self.compute_waits.push((app, v));
        }
    }

    // ── Viewport grid ────────────────────────────────────────────────────────

    /// Create a `ViewportGrid` on `app`'s window with the given layout.
    /// The perspective slot (0) reuses the window's existing camera.
    /// Additional ortho cameras are spawned and configured here.
    /// Returns handles in layout slot order.
    pub fn init_viewport_grid(
        &mut self,
        app: AppHandle,
        layout: ViewportLayout,
    ) -> ForgeResult<thin_vec::ThinVec<ViewportHandle>> {
        use std::f32::consts::{FRAC_PI_2, PI};

        let primary_cam_h = self
            .windows
            .get(app)
            .ok_or_else(|| ForgeError::Io(io::Error::other("window not found")))?
            .camera_handle;

        let n = layout.viewport_count();
        let mut cam_handles: thin_vec::ThinVec<CameraHandle> = thin_vec::ThinVec::with_capacity(n);
        cam_handles.push(primary_cam_h);

        // Spawn one camera per non-perspective slot.
        for i in 1..n {
            let kind = layout.kind_at(i);
            let mut cam = Camera::new(
                crate::render::camera::CameraId::new(*self.next_camera_id),
                Arc::from(format!("ortho_{i}")),
                [0.0, 0.0, 0.0],
                0.0,
                0.0,
            );
            cam.projection = Some(ProjectionMode::Orthographic {
                size: 10.0,
                near: -1000.0,
                far: 1000.0,
            });
            match kind {
                ViewportKind::OrthoTop => {
                    cam.yaw = 0.0;
                    cam.pitch = -FRAC_PI_2;
                }
                ViewportKind::OrthoFront => {
                    cam.yaw = -FRAC_PI_2;
                    cam.pitch = 0.0;
                }
                ViewportKind::OrthoRight => {
                    cam.yaw = PI;
                    cam.pitch = 0.0;
                }
                ViewportKind::Perspective => {}
            }
            let h = self.cameras.insert(cam);
            cam_handles.push(h);
            *self.next_camera_id += 1;
        }

        let (win_w, win_h) = {
            let wh = self.windows.get(app).unwrap().window_handle;
            self.renderer
                .as_deref()
                .and_then(|r| r.window(wh))
                .and_then(|w| w.graphics.as_ref())
                .map(|g| {
                    (
                        g.swapchain_extent.width as f32,
                        g.swapchain_extent.height as f32,
                    )
                })
                .unwrap_or((1280.0, 720.0))
        };

        let grid = ViewportGrid::new(layout, &cam_handles, win_w, win_h);
        let handles: thin_vec::ThinVec<ViewportHandle> = grid.cache.iter().copied().collect();
        self.windows.get_mut(app).unwrap().viewport_grid = Some(grid);
        Ok(handles)
    }

    pub fn viewport_grid(&self, app: AppHandle) -> Option<&ViewportGrid> {
        self.windows.get(app)?.viewport_grid.as_ref()
    }

    pub fn viewport_grid_mut(&mut self, app: AppHandle) -> Option<&mut ViewportGrid> {
        self.windows.get_mut(app)?.viewport_grid.as_mut()
    }

    /// &mut Camera for the focused viewport pane. Falls back to the window's
    /// main camera when no grid is set.
    pub fn focused_viewport_camera_mut(&mut self, app: AppHandle) -> Option<&mut Camera> {
        let h = {
            let res = self.windows.get(app)?;
            res.viewport_grid
                .as_ref()
                .and_then(|g| g.focused_camera())
                .unwrap_or(res.camera_handle)
        };
        self.cameras.get_mut(h)
    }

    /// VP matrix for a specific viewport pane at its per-pane pixel aspect ratio.
    pub fn viewport_vp(&self, app: AppHandle, vp: ViewportHandle) -> [f32; 16] {
        let res = match self.windows.get(app) {
            Some(r) => r,
            None => return MAT4_IDENTITY,
        };
        let grid = match res.viewport_grid.as_ref() {
            Some(g) => g,
            None => return MAT4_IDENTITY,
        };
        let pane = match grid.get(vp) {
            Some(p) => p,
            None => return MAT4_IDENTITY,
        };
        let cam = match self.cameras.get(pane.camera_handle) {
            Some(c) => c,
            None => return MAT4_IDENTITY,
        };
        cam.view_projection_matrix(pane.rect.pixel_aspect(grid.win_w, grid.win_h))
    }
}

// ── AppLogic trait ───────────────────────────────────────────────────────────

pub trait AppLogic: 'static {
    /// Called once after Vulkan + Renderer are ready, before the event
    /// loop spins. Typical use: register compute forges, spawn windows,
    /// load initial assets.
    fn on_start(
        &mut self,
        _ctx: &mut AppCtx<'_>,
        _event_loop: &ActiveEventLoop,
    ) -> ForgeResult<()> {
        Ok(())
    }

    /// Called per frame after winit fires RedrawRequested. Return `false`
    /// to request exit.
    fn update(&mut self, _ctx: &mut AppCtx<'_>, _app: AppHandle, _dt: f32) -> bool {
        true
    }

    /// Called per winit event. Return `true` if handled (skips default
    /// camera-controller handling for that event).
    fn handle_event(
        &mut self,
        _ctx: &mut AppCtx<'_>,
        _app: AppHandle,
        _event: &WindowEvent,
    ) -> bool {
        false
    }

    /// Called during initialisation to register any custom compute /
    /// graphics forges this app needs (beyond the engine's defaults).
    fn register_forges(&mut self, _forge: &mut ForgeMaster) -> ForgeResult<()> {
        Ok(())
    }
}

// ── ViewportSpecBuf — zero-alloc per-frame viewport specs ────────────────────

struct ViewportSpecBuf {
    data: [(vk::Viewport, vk::Rect2D, [f32; 16]); 4],
    count: usize,
}

// ── AppRunner ────────────────────────────────────────────────────────────────

pub struct AppRunner<T: AppLogic> {
    logic: T,
    data: AppData,
}

impl<T: AppLogic> AppRunner<T> {
    pub fn new(logic: T) -> Self {
        Self {
            logic,
            data: AppData::new(),
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

    /// Initialise VulkanContext + Renderer lazily on first `resumed()`.
    /// Falls back to the wgpu surface when `wgpu-backend` feature is enabled
    /// and Vulkan is unavailable.
    fn ensure_initialised(&mut self, event_loop: &ActiveEventLoop) -> ForgeResult<()> {
        if self.data.ctx.is_some() {
            return Ok(());
        }
        // Already using wgpu (surface created in spawn_window_with_camera).
        #[cfg(feature = "wgpu-backend")]
        if self.data.wgpu_surface.is_some() {
            return Ok(());
        }

        let display_handle = event_loop
            .display_handle()
            .map_err(|e| ForgeError::Io(io::Error::other(format!("{e}"))))?
            .as_raw();

        match VulkanContext::with_surface(display_handle) {
            Ok(ctx) => {
                let mut forge = ForgeMaster::new(
                    ctx.device.clone(),
                    ctx.queue,
                    ctx.command_pool,
                    ctx.memory_properties,
                )?;
                forge.add_graphics_forge_from_spirv_bytes(
                    GraphicsForgeId::new(1),
                    GraphicsOreKind::ForwardLit,
                    FORWARD_LIT_VERT,
                    FORWARD_LIT_FRAG,
                )?;
                forge.add_graphics_forge_from_spirv_bytes(
                    GraphicsForgeId::new(2),
                    GraphicsOreKind::SkinnedForwardLit,
                    SKINNED_VERT,
                    FORWARD_LIT_FRAG,
                )?;
                register_skin_morph_forges(&mut forge)?;
                self.logic.register_forges(&mut forge)?;
                let renderer = Renderer::new(forge);
                self.data.ctx = Some(ctx);
                self.data.renderer = Some(renderer);
            }
            Err(e) => {
                #[cfg(feature = "wgpu-backend")]
                {
                    eprintln!("Vulkan unavailable ({e}), falling back to wgpu");
                    // wgpu surface is created lazily in spawn_window_with_camera
                }
                #[cfg(not(feature = "wgpu-backend"))]
                return Err(e);
            }
        }
        Ok(())
    }

    fn ctx_for_logic<'a>(data: &'a mut AppData) -> AppCtx<'a> {
        AppCtx {
            renderer: data.renderer.as_mut(),
            vulkan: data.ctx.as_ref(),
            cameras: &mut data.cameras,
            windows: &mut data.windows,
            gltf_assets: &mut data.gltf_assets,
            next_app_id: &mut data.next_app_id,
            next_camera_id: &mut data.next_camera_id,
            compute_waits: &mut data.compute_waits,
            start: data.start,
            #[cfg(feature = "wgpu-backend")]
            wgpu_surface: &mut data.wgpu_surface,
            #[cfg(feature = "wgpu-backend")]
            wgpu_winit: &mut data.wgpu_winit,
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
        winit_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(app_handle) = self.data.handle_of(winit_id) else {
            return;
        };

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
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                if let Some(res) = self.data.windows.get_mut(app_handle) {
                    res.update_aspect(size.width, size.height);
                }
                let (w, h) = (size.width, size.height);
                #[cfg(feature = "wgpu-backend")]
                {
                    if let Some(ws) = self.data.wgpu_surface.as_mut() {
                        ws.resize(w, h);
                        return;
                    }
                }
                if let Some(renderer) = self.data.renderer.as_mut()
                    && let Some(res) = self.data.windows.get(app_handle)
                    && let Some(window) = renderer.window_mut(res.window_handle)
                {
                    window.resize(w, h);
                }
            }

            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                if let PhysicalKey::Code(keycode) = key_event.physical_key
                    && let Some(res) = self.data.windows.get_mut(app_handle)
                {
                    if let Some(grid) = &mut res.viewport_grid {
                        // Route to focused pane's controller.
                        if let Some(vp) = grid.arenas.get_mut(grid.focused) {
                            vp.controller.handle_key(keycode, key_event.state);
                            if keycode == KeyCode::Escape
                                && key_event.state == ElementState::Pressed
                            {
                                let grabbed = vp.controller.toggle_grab();
                                self.set_cursor_grab(app_handle, grabbed);
                            }
                        }
                    } else {
                        // Legacy single-camera path.
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

                // Compute deltas + update focus + decide what to do.
                let action: Option<(CameraHandle, f32, f32, bool /*pan*/)> = {
                    let Some(res) = self.data.windows.get_mut(app_handle) else {
                        return;
                    };
                    let (dx, dy) = match res.last_cursor {
                        Some((lx, ly)) => (pos_x - lx, pos_y - ly),
                        None => (0.0, 0.0),
                    };
                    res.last_cursor = Some((pos_x, pos_y));

                    if let Some(grid) = &mut res.viewport_grid {
                        let any_grabbed = grid.arenas.values().any(|vp| {
                            vp.controller.is_grabbed() || vp.controller.is_middle_grabbed()
                        });
                        if !any_grabbed {
                            grid.hit_test_focus(pos_x, pos_y);
                        }
                        let vp = match grid.arenas.get_mut(grid.focused) {
                            Some(v) => v,
                            None => return,
                        };
                        let cam_h = vp.camera_handle;
                        let grabbed = vp.controller.is_grabbed();
                        let mid_grabbed = vp.controller.is_middle_grabbed();
                        if grabbed {
                            let (dyaw, dpitch) = vp.controller.handle_mouse(dx, dy);
                            Some((cam_h, dyaw, dpitch, false))
                        } else if mid_grabbed {
                            Some((cam_h, dx, dy, true))
                        } else {
                            None
                        }
                    } else {
                        if !res.controller.is_grabbed() {
                            return;
                        }
                        let (dyaw, dpitch) = res.controller.handle_mouse(dx, dy);
                        Some((res.camera_handle, dyaw, dpitch, false))
                    }
                };

                if let Some((cam_h, da, db, is_pan)) = action
                    && let Some(cam) = self.data.cameras.get_mut(cam_h)
                {
                    if is_pan {
                        // Re-read pan_sensitivity from focused controller.
                        let sens = self
                            .data
                            .windows
                            .get(app_handle)
                            .and_then(|r| r.viewport_grid.as_ref())
                            .and_then(|g| g.arenas.get(g.focused))
                            .map(|vp| vp.controller.pan_sensitivity())
                            .unwrap_or(0.01);
                        cam.pan(da * sens, -db * sens);
                    } else {
                        cam.rotate(da, db);
                    }
                }
            }

            WindowEvent::MouseInput { button, state, .. } => {
                if let Some(res) = self.data.windows.get_mut(app_handle) {
                    if let Some(grid) = &mut res.viewport_grid {
                        let vp = match grid.arenas.get_mut(grid.focused) {
                            Some(v) => v,
                            None => return,
                        };
                        match button {
                            MouseButton::Left if *state == ElementState::Pressed => {
                                let grabbed = vp.controller.toggle_grab();
                                self.set_cursor_grab(app_handle, grabbed);
                            }
                            MouseButton::Middle => {
                                vp.controller.handle_middle_button(*state);
                            }
                            _ => {}
                        }
                    } else if *button == MouseButton::Left && *state == ElementState::Pressed {
                        let grabbed = res.controller.toggle_grab();
                        self.set_cursor_grab(app_handle, grabbed);
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => *y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 * 0.01,
                };
                let cam_h = self.data.windows.get(app_handle).map(|res| {
                    res.viewport_grid
                        .as_ref()
                        .and_then(|g| g.focused_camera())
                        .unwrap_or(res.camera_handle)
                });
                if let Some(cam_h) = cam_h
                    && let Some(cam) = self.data.cameras.get_mut(cam_h)
                {
                    cam.zoom(dy);
                }
            }

            WindowEvent::RedrawRequested => {
                self.draw_one(app_handle, event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(feature = "wgpu-backend")]
        if let Some(w) = self.data.wgpu_winit.as_ref() {
            w.request_redraw();
            return;
        }
        if let Some(renderer) = self.data.renderer.as_ref() {
            for res in self.data.windows.values() {
                if let Some(window) = renderer.window(res.window_handle)
                    && let Some(gfx) = &window.graphics
                {
                    gfx.winit_window.request_redraw();
                }
            }
        }
    }
}

impl<T: AppLogic> AppRunner<T> {
    fn collect_viewport_specs(data: &AppData, app: AppHandle) -> ViewportSpecBuf {
        const ZERO_VP: vk::Viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            min_depth: 0.0,
            max_depth: 0.0,
        };
        const ZERO_RECT: vk::Rect2D = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: 0,
                height: 0,
            },
        };
        let mut buf = ViewportSpecBuf {
            data: [(ZERO_VP, ZERO_RECT, MAT4_IDENTITY); 4],
            count: 0,
        };
        let Some(res) = data.windows.get(app) else {
            return buf;
        };
        let Some(grid) = res.viewport_grid.as_ref() else {
            return buf;
        };
        let extent = data
            .renderer
            .as_ref()
            .and_then(|r| r.window(res.window_handle))
            .and_then(|w| w.graphics.as_ref())
            .map(|g| g.swapchain_extent)
            .unwrap_or(vk::Extent2D {
                width: 1,
                height: 1,
            });
        for (_, vp) in grid.iter() {
            if buf.count >= 4 {
                break;
            }
            let viewport = vp.rect.to_vk_viewport(extent);
            let scissor = vp.rect.to_vk_scissor(extent);
            let aspect = vp.rect.pixel_aspect(grid.win_w, grid.win_h);
            let vp_mat = data
                .cameras
                .get(vp.camera_handle)
                .map(|c| c.view_projection_matrix(aspect))
                .unwrap_or(MAT4_IDENTITY);
            buf.data[buf.count] = (viewport, scissor, vp_mat);
            buf.count += 1;
        }
        buf
    }

    fn draw_one(&mut self, app_handle: AppHandle, event_loop: &ActiveEventLoop) {
        let (window_h, dt) = {
            let Some(res) = self.data.windows.get_mut(app_handle) else {
                return;
            };
            let dt = res.tick();
            if let Some(grid) = &mut res.viewport_grid {
                if let Some(vp) = grid.arenas.get_mut(grid.focused) {
                    let cam_h = vp.camera_handle;
                    if let Some(cam) = self.data.cameras.get_mut(cam_h) {
                        vp.controller.update(cam, dt);
                    }
                }
            } else {
                let camera_h = res.camera_handle;
                if let Some(cam) = self.data.cameras.get_mut(camera_h) {
                    res.controller.update(cam, dt);
                }
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
        // On the wgpu path, render_world (called inside logic.update above) already
        // presented the frame — nothing more to do here.
        #[cfg(feature = "wgpu-backend")]
        if self.data.ctx.is_none() {
            return;
        }

        let compute_sems: ThinVec<vk::Semaphore> = {
            let pos = self
                .data
                .compute_waits
                .iter()
                .position(|(h, _)| *h == app_handle);
            if let Some(i) = pos {
                self.data.compute_waits.swap_remove(i).1
            } else {
                ThinVec::new()
            }
        };

        let specs = Self::collect_viewport_specs(&self.data, app_handle);
        if let (Some(renderer), Some(ctx)) = (self.data.renderer.as_mut(), self.data.ctx.as_ref())
            && let Some(window) = renderer.window_mut(window_h)
        {
            unsafe {
                let result = if specs.count > 0 {
                    window.draw_frame_with_viewports(
                        &ctx.instance,
                        &ctx.device,
                        ctx.queue,
                        &compute_sems,
                        &specs.data[..specs.count],
                    )
                } else {
                    window.draw_frame_with_compute_wait(
                        &ctx.instance,
                        &ctx.device,
                        ctx.queue,
                        &compute_sems,
                    )
                };
                if let Err(e) = result {
                    eprintln!("draw_frame error: {e:?}");
                    event_loop.exit();
                }
            }
        }
    }

    fn set_cursor_grab(&self, app_handle: AppHandle, grabbed: bool) {
        let mode = if grabbed {
            CursorGrabMode::Locked
        } else {
            CursorGrabMode::None
        };
        #[cfg(feature = "wgpu-backend")]
        if let Some(w) = self.data.wgpu_winit.as_ref() {
            let _ = w.set_cursor_grab(mode);
            let _ = w.set_cursor_visible(!grabbed);
            return;
        }
        let Some(res) = self.data.windows.get(app_handle) else {
            return;
        };
        if let Some(renderer) = self.data.renderer.as_ref()
            && let Some(window) = renderer.window(res.window_handle)
            && let Some(gfx) = &window.graphics
        {
            let _ = gfx.winit_window.set_cursor_grab(mode);
            gfx.winit_window.set_cursor_visible(!grabbed);
        }
    }
}
