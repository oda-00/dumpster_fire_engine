//! Two windows, each rendering the same GLB file with independent cameras.
//! Run: cargo run --bin multi_gltf
//!
//! Prompts you to pick a .glb from assets/models/.

use std::fs;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;
use thin_vec::ThinVec;

use ash::vk;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{CursorGrabMode, Window as WinitWindow, WindowId};

use dumpster_fire_engine::forge_master::master::{ForgeError, ForgeResult};
use dumpster_fire_engine::forge_master::ore::GpuSkinBuffer;
use dumpster_fire_engine::forge_master::{ForgeMaster, GraphicsForgeId, GraphicsOreKind};
use dumpster_fire_engine::render::{Renderer, VulkanContext, Window, WindowId as RenderWindowId};
use dumpster_fire_engine::resource_manager::asset_manager::{
    SkinningFrame, build_graphics_plans_maximal, build_skin_morph_proto,
    collect_morph_output_buffers, collect_skin_palette_buffers,
    forge_gltf::{GltfAsset, Pose},
    load_asset, pack_primitive_skin_attrs, primitive_is_skinned, register_skin_morph_forges,
};
use dumpster_fire_engine::resource_manager::gltf_driver::{
    GltfCache, GltfUploadCtx, allocate_skin_palette_set, create_material, create_material_pool,
    create_skin_palette_pool, create_skin_palette_set_layout, gltf_sampler_to_vk,
    upload_texture_rgba,
};

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

// -----------------------------------------------------------------------------
// Camera (fly)
// -----------------------------------------------------------------------------
struct Camera {
    position: [f32; 3],
    yaw: f32,
    pitch: f32,
    fov: f32,
    near: f32,
    far: f32,
}

impl Camera {
    fn new(position: [f32; 3], yaw: f32, pitch: f32) -> Self {
        Self {
            position,
            yaw,
            pitch,
            fov: 45.0_f32.to_radians(),
            near: 0.1,
            far: 100.0,
        }
    }

    fn view_matrix(&self) -> [f32; 16] {
        let pos = glam::Vec3::new(self.position[0], self.position[1], self.position[2]);
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let forward =
            glam::Vec3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize();
        let right = glam::Vec3::new(-sin_yaw, 0.0, cos_yaw).normalize();
        let up = right.cross(forward).normalize();
        glam::Mat4::look_at_rh(pos, pos + forward, up).to_cols_array()
    }

    fn projection_matrix(&self, aspect: f32) -> [f32; 16] {
        glam::Mat4::perspective_rh(self.fov, aspect, self.near, self.far).to_cols_array()
    }

    fn move_forward(&mut self, delta: f32) {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let dir = glam::Vec3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize();
        self.position[0] += dir.x * delta;
        self.position[1] += dir.y * delta;
        self.position[2] += dir.z * delta;
    }

    fn move_right(&mut self, delta: f32) {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let dir = glam::Vec3::new(-sin_yaw, 0.0, cos_yaw).normalize();
        self.position[0] += dir.x * delta;
        self.position[1] += dir.y * delta;
        self.position[2] += dir.z * delta;
    }

    fn move_up(&mut self, delta: f32) {
        self.position[1] += delta;
    }

    fn rotate(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(-1.5, 1.5);
    }
}

// -----------------------------------------------------------------------------
// Per‑window state – stored in a ThinVec, linear scan by winit_id
// -----------------------------------------------------------------------------
struct WindowState {
    winit_id: WindowId,
    window_handle: dumpster_fire_engine::render::WindowHandle,
    camera: Camera,
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    mouse_grabbed: bool,
    last_mouse: (f32, f32),
    aspect: f32,
    material_pool: vk::DescriptorPool,
    skin_pool: vk::DescriptorPool,
    skin_set_layout: vk::DescriptorSetLayout,
    material_layout: vk::DescriptorSetLayout,
    cache: GltfCache,
    material_sets: ThinVec<Option<vk::DescriptorSet>>,
    skin_keys: ThinVec<(usize, usize)>,
    skin_buffers: ThinVec<GpuSkinBuffer>,
    last_anim_time: f32,
    start_time: Instant,
}

impl WindowState {
    fn skin_vertex_buffer(&self, mesh_idx: usize, prim_idx: usize) -> Option<vk::Buffer> {
        for i in 0..self.skin_keys.len() {
            if self.skin_keys[i] == (mesh_idx, prim_idx) {
                return Some(self.skin_buffers[i].buffer.handle);
            }
        }
        None
    }
}

// -----------------------------------------------------------------------------
// Helper: pick a GLB file from assets/models/
// -----------------------------------------------------------------------------
fn pick_model() -> Arc<str> {
    let models_dir = "assets/models";
    let entries = match fs::read_dir(models_dir) {
        Ok(e) => e,
        Err(_) => {
            eprintln!("Error: assets/models/ directory not found");
            std::process::exit(1);
        }
    };
    let mut models: ThinVec<String> = ThinVec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("glb") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                models.push(name.to_string());
            }
        }
    }
    if models.is_empty() {
        eprintln!("No .glb files found in assets/models/");
        std::process::exit(1);
    }
    println!("Available models:");
    for (i, name) in models.iter().enumerate() {
        println!("  {}: {}", i + 1, name);
    }
    print!("Select model (1-{}): ", models.len());
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let idx: usize = input.trim().parse().unwrap_or(1);
    let idx = idx.saturating_sub(1).min(models.len() - 1);
    let path = format!("{}/{}", models_dir, models[idx]);
    Arc::from(path)
}

// -----------------------------------------------------------------------------
// Main app – stores windows in a ThinVec
// -----------------------------------------------------------------------------
struct MultiGltfApp {
    model_path: Arc<str>,
    windows: ThinVec<WindowState>,
    ctx: Option<VulkanContext>,
    renderer: Option<Renderer>,
    asset: Option<Arc<GltfAsset>>,
}

impl MultiGltfApp {
    fn new(model_path: Arc<str>) -> Self {
        Self {
            model_path,
            windows: ThinVec::new(),
            ctx: None,
            renderer: None,
            asset: None,
        }
    }

    fn load_asset(&mut self) -> ForgeResult<()> {
        let asset = load_asset(&*self.model_path)
            .map_err(|e| ForgeError::Io(std::io::Error::other(format!("{e}"))))?;
        self.asset = Some(Arc::new(asset));
        Ok(())
    }

    fn window_state_mut(&mut self, id: WindowId) -> Option<&mut WindowState> {
        self.windows.iter_mut().find(|w| w.winit_id == id)
    }

    /// Upload per‑window resources (textures, materials, skin buffers)
    fn upload_window_resources(
        &self,
        ctx: &VulkanContext,
        asset: &GltfAsset,
        material_pool: vk::DescriptorPool,
        material_layout: vk::DescriptorSetLayout,
    ) -> ForgeResult<(
        GltfCache,
        ThinVec<Option<vk::DescriptorSet>>,
        ThinVec<(usize, usize)>,
        ThinVec<GpuSkinBuffer>,
    )> {
        let upload_ctx = GltfUploadCtx {
            device: &ctx.device,
            memory_properties: &ctx.memory_properties,
            graphics_queue: ctx.queue,
            command_pool: ctx.command_pool,
            material_set_layout: material_layout,
            material_pool,
        };

        // Per‑image samplers
        let n_images = asset.images.len();
        let mut img_samplers: ThinVec<
            dumpster_fire_engine::resource_manager::gltf_driver::GltfSampler,
        > = ThinVec::with_capacity(n_images);
        img_samplers.resize(n_images, Default::default());
        for tex in &asset.textures {
            let idx = tex.image as usize;
            if idx < n_images {
                if let Some(sampler_idx) = tex.sampler {
                    if let Some(s) = asset.samplers.get(sampler_idx as usize) {
                        img_samplers[idx] = gltf_sampler_to_vk(s);
                    }
                }
            }
        }

        // Upload textures
        let mut cache = GltfCache::new();
        let img_handles: ThinVec<Option<_>> = asset
            .images
            .iter()
            .enumerate()
            .map(|(i, img)| {
                upload_texture_rgba(
                    &upload_ctx,
                    img.width,
                    img.height,
                    &img.rgba,
                    &img_samplers[i],
                )
                .ok()
                .map(|t| cache.textures.insert(t))
            })
            .collect();

        // Create materials
        let material_sets: ThinVec<Option<vk::DescriptorSet>> = asset
            .materials
            .iter()
            .map(|mat| {
                create_material(mat, asset, &img_handles, &upload_ctx, &mut cache)
                    .ok()
                    .map(|gm| gm.descriptor_set)
            })
            .collect();

        // Upload skin vertex buffers
        let mut skin_keys = ThinVec::new();
        let mut skin_buffers = ThinVec::new();
        let mesh_ctx = ctx.mesh_upload_ctx();
        for (mi, mesh) in asset.meshes.iter().enumerate() {
            for (pi, _) in mesh.primitives.iter().enumerate() {
                if primitive_is_skinned(asset, mi as u32, pi as u32) {
                    let bytes = pack_primitive_skin_attrs(asset, mi as u32, pi as u32);
                    let vcount = (bytes.len() / 24) as u32;
                    if vcount > 0 {
                        if let Ok(buf) = GpuSkinBuffer::upload(&mesh_ctx, &bytes, vcount) {
                            skin_keys.push((mi, pi));
                            skin_buffers.push(buf);
                        }
                    }
                }
            }
        }

        Ok((cache, material_sets, skin_keys, skin_buffers))
    }
}

impl ApplicationHandler for MultiGltfApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.ctx.is_some() {
            return;
        }

        // Load the GLB asset
        if let Err(e) = self.load_asset() {
            eprintln!("Failed to load GLB: {e}");
            event_loop.exit();
            return;
        }
        let asset = self.asset.as_ref().unwrap().clone();

        // Dummy window to get Vulkan surface extensions
        let dummy_attrs = WinitWindow::default_attributes()
            .with_title("dummy")
            .with_visible(false);
        let dummy_window = event_loop
            .create_window(dummy_attrs)
            .expect("create dummy window");
        let display_handle = dummy_window
            .display_handle()
            .expect("display handle")
            .as_raw();
        let ctx = VulkanContext::with_surface(display_handle).expect("Vulkan init");

        // Register forges (shared across windows)
        let mut forge = ForgeMaster::new(
            ctx.device.clone(),
            ctx.queue,
            ctx.command_pool,
            ctx.memory_properties,
        )
        .expect("ForgeMaster");
        forge
            .add_graphics_forge_from_spirv_bytes(
                GraphicsForgeId::new(1),
                GraphicsOreKind::ForwardLit,
                FORWARD_LIT_VERT,
                FORWARD_LIT_FRAG,
            )
            .expect("ForwardLit forge");
        forge
            .add_graphics_forge_from_spirv_bytes(
                GraphicsForgeId::new(2),
                GraphicsOreKind::SkinnedForwardLit,
                SKINNED_VERT,
                FORWARD_LIT_FRAG,
            )
            .expect("SkinnedForwardLit forge");
        register_skin_morph_forges(&mut forge).expect("skin/morph forges");

        let mut renderer = Renderer::new(forge);

        // Create two windows
        for i in 1..=2 {
            let title = format!("Multi GLTF – Window {}", i);
            let attrs = WinitWindow::default_attributes()
                .with_title(&title)
                .with_inner_size(winit::dpi::LogicalSize::new(1024, 768));
            let winit_window = Arc::new(event_loop.create_window(attrs).expect("create window"));
            let winit_id = winit_window.id();

            let display_handle = winit_window
                .display_handle()
                .expect("display handle")
                .as_raw();
            let window_handle_raw = winit_window
                .window_handle()
                .expect("window handle")
                .as_raw();
            let _surface = unsafe {
                ash_window::create_surface(
                    &ctx.entry,
                    &ctx.instance,
                    display_handle,
                    window_handle_raw,
                    None,
                )
                .map_err(ForgeError::Vk)
                .expect("create surface")
            };

            let graphics_forge = renderer
                .forge
                .graphics_forge(GraphicsOreKind::ForwardLit)
                .expect("ForwardLit forge present");

            let window = Window::new_with_surface(
                RenderWindowId::new(i),
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
            )
            .expect("create window");

            let window_handle = renderer.add_window(window);

            let material_layout = renderer
                .window(window_handle)
                .and_then(|w| w.graphics.as_ref())
                .map(|g| g.mold.material_set_layout)
                .unwrap_or(vk::DescriptorSetLayout::null());

            let material_pool = create_material_pool(&ctx.device, 4096).expect("material pool");
            let skin_set_layout = create_skin_palette_set_layout(&ctx.device).expect("skin layout");
            let skin_pool = create_skin_palette_pool(&ctx.device, 256).expect("skin pool");

            let (cache, material_sets, skin_keys, skin_buffers) = self
                .upload_window_resources(&ctx, &asset, material_pool, material_layout)
                .expect("upload resources");

            let camera = Camera::new([0.0, 2.0, 5.0], -1.57, 0.2);
            let aspect = 1024.0 / 768.0;

            let state = WindowState {
                winit_id,
                window_handle,
                camera,
                forward: false,
                backward: false,
                left: false,
                right: false,
                up: false,
                down: false,
                mouse_grabbed: false,
                last_mouse: (0.0, 0.0),
                aspect,
                material_pool,
                skin_pool,
                skin_set_layout,
                material_layout,
                cache,
                material_sets,
                skin_keys,
                skin_buffers,
                last_anim_time: -1.0,
                start_time: Instant::now(),
            };
            self.windows.push(state);
        }

        self.ctx = Some(ctx);
        self.renderer = Some(renderer);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        winit_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.window_state_mut(winit_id) else {
            return;
        };
        let renderer = self.renderer.as_mut().unwrap();
        let ctx = self.ctx.as_ref().unwrap();
        let asset = self.asset.as_ref().unwrap();

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.aspect = size.width as f32 / size.height as f32;
                if let Some(window) = renderer.window_mut(state.window_handle) {
                    window.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                if let PhysicalKey::Code(keycode) = key_event.physical_key {
                    let pressed = key_event.state == ElementState::Pressed;
                    match keycode {
                        KeyCode::KeyW => state.forward = pressed,
                        KeyCode::KeyS => state.backward = pressed,
                        KeyCode::KeyA => state.left = pressed,
                        KeyCode::KeyD => state.right = pressed,
                        KeyCode::KeyQ => state.up = pressed,
                        KeyCode::KeyE => state.down = pressed,
                        KeyCode::Escape if pressed => {
                            let grabbed = !state.mouse_grabbed;
                            state.mouse_grabbed = grabbed;
                            if let Some(window) = renderer.window(state.window_handle) {
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
                        _ => {}
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } if state.mouse_grabbed => {
                let dx = position.x as f32 - state.last_mouse.0;
                let dy = position.y as f32 - state.last_mouse.1;
                state.camera.rotate(-dx * 0.005, -dy * 0.005);
                state.last_mouse = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput {
                button,
                state: btn_state,
                ..
            } if button == MouseButton::Left => {
                if btn_state == ElementState::Pressed {
                    state.mouse_grabbed = true;
                    if let Some(window) = renderer.window(state.window_handle) {
                        if let Some(gfx) = &window.graphics {
                            let _ = gfx.winit_window.set_cursor_grab(CursorGrabMode::Locked);
                            let _ = gfx.winit_window.set_cursor_visible(false);
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                // Update camera movement
                let dt = state.start_time.elapsed().as_secs_f32();
                let speed = 5.0 * dt;
                if state.forward {
                    state.camera.move_forward(speed);
                }
                if state.backward {
                    state.camera.move_forward(-speed);
                }
                if state.right {
                    state.camera.move_right(speed);
                }
                if state.left {
                    state.camera.move_right(-speed);
                }
                if state.up {
                    state.camera.move_up(speed);
                }
                if state.down {
                    state.camera.move_up(-speed);
                }

                // Animate
                let t = state.start_time.elapsed().as_secs_f32();
                let mut advanced = false;
                let mut pose = Pose::rest(asset);
                if let Some(anim) = asset.animations.first() {
                    let dur = anim.duration().max(1e-3);
                    pose.sample(asset, anim, t.rem_euclid(dur));
                    advanced = true;
                }
                if advanced || state.last_anim_time < 0.0 {
                    // Reset skin descriptor pool
                    unsafe {
                        ctx.device
                            .reset_descriptor_pool(
                                state.skin_pool,
                                vk::DescriptorPoolResetFlags::empty(),
                            )
                            .ok();
                    }

                    // Compute pass
                    let (morph_buffers, palette_buffers) = if let Some(compute_proto) =
                        build_skin_morph_proto(
                            asset,
                            &pose,
                            dumpster_fire_engine::render::factory_master::proto::ProtoId::new(2),
                            0,
                        ) {
                        match renderer.build_compute_factory(state.window_handle, compute_proto) {
                            Ok(handle) => {
                                let win = renderer.window(state.window_handle);
                                if let Some(factory) =
                                    win.and_then(|w| w.factory_master.get(handle))
                                {
                                    (
                                        collect_morph_output_buffers(asset, factory),
                                        collect_skin_palette_buffers(asset, factory),
                                    )
                                } else {
                                    (Default::default(), Default::default())
                                }
                            }
                            Err(_) => (Default::default(), Default::default()),
                        }
                    } else {
                        (Default::default(), Default::default())
                    };

                    // Build palette sets by node (linear search over nodes)
                    let mut palette_sets_by_node = ThinVec::new();
                    for (node_idx, node) in asset.nodes.iter().enumerate() {
                        if let Some(skin_idx) = node.skin {
                            if let Some(&buf) = palette_buffers.get(&(skin_idx as usize)) {
                                let range = (asset.skins[skin_idx as usize].joints.len()
                                    as vk::DeviceSize)
                                    * 64;
                                if let Ok(set) = allocate_skin_palette_set(
                                    &ctx.device,
                                    state.skin_pool,
                                    state.skin_set_layout,
                                    buf,
                                    range.max(64),
                                ) {
                                    palette_sets_by_node.push((node_idx, set));
                                }
                            }
                        }
                    }

                    // Build skinning frame – using linear collections
                    let mut skin_vertex_buffers = ThinVec::new();
                    for (mesh_idx, prim_idx) in state.skin_keys.iter() {
                        if let Some(buf) = state.skin_vertex_buffer(*mesh_idx, *prim_idx) {
                            skin_vertex_buffers.push(((*mesh_idx, *prim_idx), buf));
                        }
                    }
                    let skinning = SkinningFrame {
                        skin_vertex_buffers: skin_vertex_buffers
                            .iter()
                            .map(|&(k, v)| (k, v))
                            .collect(),
                        palette_sets_by_node: palette_sets_by_node
                            .iter()
                            .map(|&(k, v)| (k, v))
                            .collect(),
                    };
                    // morph_buffers is a HashMap from the asset_manager API – unavoidable.
                    let morph_buffers_map: std::collections::HashMap<_, _> = morph_buffers;

                    let upload_ctx = ctx.mesh_upload_ctx();
                    if let Ok(plans) = build_graphics_plans_maximal(
                        asset,
                        &pose,
                        &upload_ctx,
                        &state.material_sets,
                        &morph_buffers_map,
                        &skinning,
                    ) {
                        let mut proto = dumpster_fire_engine::render::factory_master::proto::Proto::<
                            dumpster_fire_engine::render::factory_master::proto::GraphicsTag,
                        >::new(
                            dumpster_fire_engine::render::factory_master::proto::ProtoId::new(1),
                            "gltf_scene",
                        );
                        for plan in plans {
                            proto.push_call(plan);
                        }
                        renderer.build_graphics_factory(state.window_handle, proto);
                    }
                    state.last_anim_time = t;
                }

                // Draw frame
                let window = renderer.window_mut(state.window_handle).unwrap();
                let result = unsafe { window.draw_frame(&ctx.instance, &ctx.device, ctx.queue) };
                if let Err(e) = result {
                    eprintln!("draw_frame error: {e:?}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(renderer) = self.renderer.as_ref() {
            for state in self.windows.iter() {
                if let Some(window) = renderer.window(state.window_handle) {
                    if let Some(gfx) = &window.graphics {
                        gfx.winit_window.request_redraw();
                    }
                }
            }
        }
    }
}

fn main() -> ForgeResult<()> {
    let model_path = pick_model();
    let event_loop =
        EventLoop::new().map_err(|e| ForgeError::Io(std::io::Error::other(format!("{e}"))))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = MultiGltfApp::new(model_path);
    event_loop
        .run_app(&mut app)
        .map_err(|e| ForgeError::Io(std::io::Error::other(format!("{e}"))))
}
