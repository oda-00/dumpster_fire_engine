// hello_gltf — load a .glb/.gltf file and render it with the ForwardLit
// pipeline using the full `forge_gltf` API:
//
//   • walks every node-primitive draw in the primary scene (not just the
//     first primitive),
//   • samples animations over wall-clock time and rebuilds per-frame plans,
//   • uploads each unique primitive once and reuses the GpuMesh across
//     animation frames,
//   • uploads every glTF material as a Vulkan descriptor set (set 1) and
//     binds it per draw call via the new gltf_driver module.
//
//   cargo run --bin hello_gltf -- path/to/model.glb

use std::sync::Arc;
use std::time::Instant;

use ash::vk;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::{CursorGrabMode, WindowId};

<<<<<<< HEAD
=======
use dumpster_fire_engine::render::camera::{Camera, CameraController, CameraId};

>>>>>>> 5b1fd0af6298e447d49809a9dc2b0b7b85cd25b7
use dumpster_fire_engine::forge_master::ore::{GpuMesh, GpuSkinBuffer};
use dumpster_fire_engine::forge_master::{ForgeMaster, GraphicsForgeId, GraphicsOreKind};
use dumpster_fire_engine::render::{
    GraphicsTag, Proto, ProtoId, Renderer, VulkanContext, Window, WindowId as RenderWindowId,
};
use dumpster_fire_engine::resource_manager::asset_manager::{
    SkinningFrame, build_graphics_plans_maximal_with_meshes_vp, build_skin_morph_proto,
    collect_morph_output_buffers, collect_skin_palette_buffers, compute_asset_aabb,
    forge_gltf::{GltfAsset, Pose},
    pack_primitive_skin_attrs, primitive_is_skinned, register_skin_morph_forges,
<<<<<<< HEAD
    upload_all_primitive_meshes, view_projection_from_aabb,
=======
    upload_all_primitive_meshes,
>>>>>>> 5b1fd0af6298e447d49809a9dc2b0b7b85cd25b7
};
use dumpster_fire_engine::resource_manager::gltf_driver::{
    AsyncGltfLoader, GltfCache, GltfSampler, GltfUploadCtx, MaterialHandle, TEXTURE_SLOT_COUNT,
    allocate_skin_palette_set, create_material, create_material_pool, create_skin_palette_pool,
    create_skin_palette_set_layout, gltf_sampler_to_vk, upload_texture_rgba,
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

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: hello_gltf <model.glb>");
        std::process::exit(1);
    });

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let mut app = App {
        model_path: path,
        loader: None,
        start: now,
        asset_loaded: None,
        live: None,
        camera: Camera::new(
            CameraId::new(1),
            Arc::from("hello_gltf"),
            [0.0, 0.0, 5.0],
            0.0,
            0.0,
        ),
        controller: CameraController::new(5.0, 0.005),
        camera_fitted: false,
        last_cursor: None,
        last_frame: now,
    };
    event_loop.run_app(&mut app).expect("run event loop");
}

/// Field order matters here too: `asset_loaded` owns
/// `GpuMesh`/`GpuSkinBuffer`/`GltfCache`, all of which dereference
/// `live.ctx.device` in their Drop. It must be listed BEFORE `live` so
/// Rust drops it first.
struct App {
    model_path: String,
    loader: Option<AsyncGltfLoader>,
    start: Instant,
    asset_loaded: Option<AssetState>,
    live: Option<LiveState>,
<<<<<<< HEAD
}

=======
    camera: Camera,
    controller: CameraController,
    /// True once the camera has been fitted to the asset AABB on first load.
    camera_fitted: bool,
    /// Last absolute cursor position for delta computation.
    last_cursor: Option<(f32, f32)>,
    /// Wall-clock instant of the previous frame, for controller dt.
    last_frame: Instant,
}
>>>>>>> 5b1fd0af6298e447d49809a9dc2b0b7b85cd25b7
struct AssetState {
    asset: GltfAsset,
    pose: Pose,
    /// Pre-resolved descriptor set per material index. Built once on load.
    material_sets: Vec<Option<vk::DescriptorSet>>,
    /// One Arc<GpuMesh> per (mesh, prim) — uploaded once at load time and
    /// reused across frames. Without this, every frame would re-upload and
    /// destroy every primitive's vertex / index buffers, which both
    /// thrashes the device and creates use-after-free races against the
    /// previous frame's draw submission.
    meshes: std::collections::HashMap<(usize, usize), std::sync::Arc<GpuMesh>>,
    /// Per-skinned-primitive joints+weights vertex buffer (binding 1).
    /// (mesh_idx, prim_idx) → owned GpuSkinBuffer.
    skin_vertex_buffers: std::collections::HashMap<(usize, usize), GpuSkinBuffer>,
    /// Owns the GpuTexture / GpuMaterial GPU resources. We never read it
    /// after upload — it's stored only so Drop runs in the right order
    /// (after AssetState's other fields, before LiveState's device).
    #[allow(dead_code)]
    cache: GltfCache,
    /// Track if we've ever animated.
    last_anim_time: f32,
    /// Cached rest-pose world AABB (min, max) — computing it touches every
    /// vertex in the asset, so we do it once at load instead of per frame.
    /// Used to fit the default camera; animation can move the verts but the
    /// resulting framing only drifts slightly so a static AABB is fine.
    rest_aabb: ([f32; 3], [f32; 3]),
}

/// Field order matters for `Drop`: Rust drops fields top-to-bottom, so
/// every resource that depends on the device (descriptor pools, the
/// renderer's windows + factories) must be declared BEFORE `ctx`. The
/// renderer's `Drop` cleans up its windows + factory ingots; the pool
/// and layout fields are raw handles that we destroy in our own Drop
/// impl below — both happen before the VulkanContext (and its device)
/// drops at the bottom.
struct LiveState {
    material_pool: vk::DescriptorPool,
    material_layout: vk::DescriptorSetLayout,
    /// Skin palette descriptor set pool — reset every frame so the
    /// per-frame palette set allocations recycle.
    skin_pool: vk::DescriptorPool,
    skin_set_layout: vk::DescriptorSetLayout,
    /// Per-instance mat4 SSBO pool — re-used across frames; per-draw
    /// sets get freed each frame after the fence wait.
    instance_pool: vk::DescriptorPool,
    instance_layout: vk::DescriptorSetLayout,
    renderer: Renderer,
    window_handle: dumpster_fire_engine::render::WindowHandle,
    winit_id: WindowId,
    /// MUST drop last — every other field above this borrows from
    /// `ctx.device` (directly or transitively).
    ctx: VulkanContext,
}

impl Drop for LiveState {
    fn drop(&mut self) {
        // Drain the device before tearing down descriptor pools / layouts.
        // The renderer's own Drop will then run and free its window's
        // factories without racing in-flight submissions.
        unsafe {
            let _ = self.ctx.device.device_wait_idle();
            if self.skin_pool != vk::DescriptorPool::null() {
                self.ctx
                    .device
                    .destroy_descriptor_pool(self.skin_pool, None);
            }
            if self.skin_set_layout != vk::DescriptorSetLayout::null() {
                self.ctx
                    .device
                    .destroy_descriptor_set_layout(self.skin_set_layout, None);
            }
            if self.material_pool != vk::DescriptorPool::null() {
                self.ctx
                    .device
                    .destroy_descriptor_pool(self.material_pool, None);
            }
            if self.instance_pool != vk::DescriptorPool::null() {
                self.ctx
                    .device
                    .destroy_descriptor_pool(self.instance_pool, None);
            }
            if self.instance_layout != vk::DescriptorSetLayout::null() {
                self.ctx
                    .device
                    .destroy_descriptor_set_layout(self.instance_layout, None);
            }
            // Note: material_layout is owned by the GraphicsMold (the
            // renderer's window) and gets destroyed there. We just
            // hold a copy of the handle.
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_some() {
            return;
        }

        // ── OS window ────────────────────────────────────────────────────────
        let attrs = winit::window::Window::default_attributes()
            .with_title("hello_gltf")
            .with_inner_size(winit::dpi::LogicalSize::new(1024u32, 768u32));
        let winit_window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let winit_id = winit_window.id();

        // ── Vulkan ───────────────────────────────────────────────────────────
        let display_handle = winit_window
            .display_handle()
            .expect("display handle")
            .as_raw();
        let ctx = VulkanContext::with_surface(display_handle).expect("Vulkan init");

        // ── Background asset loader (hand-rolled AsyncGltfLoader) ────────────
        self.loader = Some(AsyncGltfLoader::spawn(self.model_path.clone().into()));

        // ── ForwardLit forge ─────────────────────────────────────────────────
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
            .expect("register ForwardLit forge");
        forge
            .add_graphics_forge_from_spirv_bytes(
                GraphicsForgeId::new(2),
                GraphicsOreKind::SkinnedForwardLit,
                SKINNED_VERT,
                FORWARD_LIT_FRAG, // identical fragment stage
            )
            .expect("register SkinnedForwardLit forge");
        // Pre-compile SkinPalette + MorphBlend compute pipelines once at
        // startup — per-frame plans can dispatch them without re-linking.
        register_skin_morph_forges(&mut forge).expect("register skin/morph forges");
        let graphics_forge = forge
            .graphics_forge(GraphicsOreKind::ForwardLit)
            .expect("ForwardLit forge present");

        // ── Window + swapchain + pipeline ────────────────────────────────────
        let mut window = Window::new_with_surface(
            RenderWindowId::new(1),
            "hello_gltf",
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
        )
        .expect("Window::new_with_surface");

        // Attach the SkinnedForwardLit pipeline BEFORE handing `forge` to the
        // renderer — that way we don't need GraphicsForge: Clone or a
        // re-borrow of the renderer-owned ForgeMaster.
        {
            let skinned_forge = forge
                .graphics_forge(GraphicsOreKind::SkinnedForwardLit)
                .expect("SkinnedForwardLit registered above");
            window
                .attach_skinned_forge(&ctx.device, skinned_forge)
                .expect("attach SkinnedForwardLit pipeline");
        }

        let mut renderer = Renderer::new(forge);
        let window_handle = renderer.add_window(window);

        // Grab the material descriptor-set layout from the freshly built mold.
        let material_layout = renderer
            .window(window_handle)
            .and_then(|w| w.graphics.as_ref())
            .map(|g| g.mold.material_set_layout)
            .unwrap_or(vk::DescriptorSetLayout::null());

        // Material descriptor pool (4096 sets max — fits any reasonable glTF).
        let material_pool =
            create_material_pool(&ctx.device, 4096).expect("create material descriptor pool");

        // Skin palette descriptor layout + pool. The pool is reset every
        // frame so the per-frame palette allocations recycle their slots.
        let skin_set_layout =
            create_skin_palette_set_layout(&ctx.device).expect("create skin set layout");
        let skin_pool =
            create_skin_palette_pool(&ctx.device, 256).expect("create skin descriptor pool");

        // Per-instance mat4 descriptor layout + pool (set 3,
        // EXT_mesh_gpu_instancing). Per-draw sets get freed each frame
        // via FREE_DESCRIPTOR_SET after the fence wait.
        let instance_layout =
            dumpster_fire_engine::resource_manager::gltf_driver::create_instance_set_layout(
                &ctx.device,
            )
            .expect("create instance set layout");
        let instance_pool =
            dumpster_fire_engine::resource_manager::gltf_driver::create_instance_pool(
                &ctx.device,
                4096,
            )
            .expect("create instance descriptor pool");

        self.live = Some(LiveState {
            ctx,
            renderer,
            window_handle,
            winit_id,
            material_pool,
            material_layout,
            skin_pool,
            skin_set_layout,
            instance_pool,
            instance_layout,
        });
        if let Some(gfx) = &self
            .live
            .as_ref()
            .unwrap()
            .renderer
            .window(self.live.as_ref().unwrap().window_handle)
            .unwrap()
            .graphics
        {
            gfx.winit_window.request_redraw();
        }
        self.start = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if id != live.winit_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(new_size) => {
                if let Some(window) = live.renderer.window_mut(live.window_handle) {
                    window.resize(new_size.width, new_size.height);
                }
            }
            WindowEvent::KeyboardInput { event: key_ev, .. } => {
                if let PhysicalKey::Code(kc) = key_ev.physical_key {
                    self.controller.handle_key(kc, key_ev.state);
                    if kc == KeyCode::Escape && key_ev.state == ElementState::Pressed {
                        let grabbed = self.controller.toggle_grab();
                        set_grab_hello(live, grabbed);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let px = position.x as f32;
                let py = position.y as f32;
                let (dx, dy) = match self.last_cursor {
                    Some((lx, ly)) => (px - lx, py - ly),
                    None => (0.0, 0.0),
                };
                self.last_cursor = Some((px, py));
                if self.controller.is_grabbed() {
                    let (dyaw, dpitch) = self.controller.handle_mouse(dx, dy);
                    self.camera.rotate(dyaw, dpitch);
                }
            }
            WindowEvent::MouseInput { button, state, .. } => {
                if button == MouseButton::Left && state == ElementState::Pressed {
                    let grabbed = self.controller.toggle_grab();
                    set_grab_hello(live, grabbed);
                }
            }
            WindowEvent::RedrawRequested => {
                // First time we see the asset, install it and upload all GPU
                // textures + materials.
                if self.asset_loaded.is_none() {
                    if let Some(loader) = self.loader.as_mut() {
                        if let Some(result) = loader.try_recv() {
                            match result {
                                Ok(asset) => {
                                    let summary = format!(
                                        "loaded: {} meshes, {} nodes, {} animations, \
                                         {} materials, {} textures, {} images, {} lights",
                                        asset.meshes.len(),
                                        asset.nodes.len(),
                                        asset.animations.len(),
                                        asset.materials.len(),
                                        asset.textures.len(),
                                        asset.images.len(),
                                        asset.lights.len(),
                                    );
                                    println!("{summary}");

                                    let pose = Pose::rest(&asset);
                                    let mut cache = GltfCache::new(live.ctx.device.clone());
                                    let material_sets =
                                        upload_materials(&live.ctx, live, &mut cache, &asset);
                                    let skin_vertex_buffers =
                                        upload_skin_vertex_buffers(&live.ctx, &asset);
                                    // Upload every primitive's GpuMesh exactly once.
                                    // The HashMap is owned by AssetState; per-frame plan
                                    // builds clone the Arc<GpuMesh> into the factory.
                                    let meshes = match upload_all_primitive_meshes(
                                        &asset,
                                        &live.ctx.mesh_upload_ctx(),
                                    ) {
                                        Ok(m) => m,
                                        Err(e) => {
                                            eprintln!("mesh upload failed: {e:?}");
                                            event_loop.exit();
                                            return;
                                        }
                                    };
                                    // Cache the rest-pose AABB so the per-frame camera
                                    // doesn't walk every vertex on each redraw.
                                    let rest_aabb = compute_asset_aabb(&asset, &pose);

                                    self.asset_loaded = Some(AssetState {
                                        asset,
                                        pose,
                                        material_sets,
                                        meshes,
                                        skin_vertex_buffers,
                                        cache,
                                        last_anim_time: -1.0,
                                        rest_aabb,
                                    });
                                }
                                Err(e) => {
                                    eprintln!("asset load failed: {e:?}");
                                    event_loop.exit();
                                    return;
                                }
                            }
                        }
                    }
                }

                // Per-frame compute-completion semaphore the async compute
                // path signals; threaded into the graphics submission so
                // the GPU side-orders compute → graphics without a CPU
                // fence wait. Empty when no compute Ores were dispatched
                // (asset with no skinning + no morph targets).
                let mut compute_signal_outer: Option<vk::Semaphore> = None;
                // If the asset is in, advance any animation and (re)build
                // the per-frame draw list.
                // Advance camera velocity from held keys (capped to avoid
                // spiral on first frame where dt could be huge).
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;
                self.controller.update(&mut self.camera, dt);

                if let Some(state) = self.asset_loaded.as_mut() {
                    // Auto-fit camera to asset on first load.
                    if !self.camera_fitted {
                        fit_camera_to_aabb(&mut self.camera, &state.rest_aabb);
                        self.camera_fitted = true;
                    }

                    let t = self.start.elapsed().as_secs_f32();
                    let advanced = if let Some(anim) = state.asset.animations.first() {
                        let dur = anim.duration().max(1e-3);
                        state.pose.sample(&state.asset, anim, t.rem_euclid(dur));
                        true
                    } else {
                        false
                    };

                    let needs_rebuild = advanced || state.last_anim_time < 0.0;
                    if needs_rebuild {
                        // Wait on the most-recently-submitted frame's fence (just
                        // that one — NOT `device_wait_idle`, which blocks every
                        // queue including the transfer queue for nothing). After
                        // this returns the previous draw is off the GPU, so we
                        // can safely reset descriptor sets that draw was reading
                        // and replace compute / graphics factories whose ingots
                        // it consumed.
                        if let Err(e) = live.renderer.wait_for_last_submission(live.window_handle) {
                            eprintln!("wait for previous frame failed: {e:?}");
                        }
                        unsafe {
                            live.ctx
                                .device
                                .reset_descriptor_pool(
                                    live.skin_pool,
                                    vk::DescriptorPoolResetFlags::empty(),
                                )
                                .ok();
                        }

                        // GPU skin/morph compute pass — dispatched per frame.
                        // The factory's ingots own the compute output buffers; we
                        // harvest morph-blended vertex buffers and skin palettes
                        // from it and feed them straight into the graphics draws.
                        // Async path: refine_batch_async submits all compute
                        // dispatches in one CB with a signal semaphore. The
                        // CPU continues into graphics record without waiting
                        // on a fence; the GPU side blocks the graphics
                        // vertex stages on the returned semaphore at submit.
                        let mut compute_signal: Option<vk::Semaphore> = None;
                        let (morph_buffers, palette_buffers): (
                            std::collections::HashMap<_, _>,
                            std::collections::HashMap<_, _>,
                        ) = if let Some(compute_proto) =
                            build_skin_morph_proto(&state.asset, &state.pose, ProtoId::new(2), 0)
                        {
                            match live
                                .renderer
                                .build_compute_factory_async(live.window_handle, compute_proto)
                            {
                                Ok((handle, sem)) => {
                                    compute_signal = Some(sem);
                                    let win = live.renderer.window(live.window_handle);
                                    let factory = win.and_then(|w| w.factory_master.get(handle));
                                    match factory {
                                        Some(f) => (
                                            collect_morph_output_buffers(&state.asset, f),
                                            collect_skin_palette_buffers(&state.asset, f),
                                        ),
                                        None => Default::default(),
                                    }
                                }
                                Err(e) => {
                                    eprintln!("skin/morph compute dispatch error: {e:?}");
                                    Default::default()
                                }
                            }
                        } else {
                            Default::default()
                        };

                        // Allocate one palette descriptor set per (skinned) node
                        // pointing at that skin's palette buffer.
                        let mut palette_sets_by_node = std::collections::HashMap::new();
                        for (node_idx, node) in state.asset.nodes.iter().enumerate() {
                            let Some(skin_idx) = node.skin else { continue };
                            let Some(&buf) = palette_buffers.get(&(skin_idx as usize)) else {
                                continue;
                            };
                            let range = (state.asset.skins[skin_idx as usize].joints.len()
                                as vk::DeviceSize)
                                * 64;
                            match allocate_skin_palette_set(
                                &live.ctx.device,
                                live.skin_pool,
                                live.skin_set_layout,
                                buf,
                                range.max(64),
                            ) {
                                Ok(set) => {
                                    palette_sets_by_node.insert(node_idx, set);
                                }
                                Err(e) => eprintln!("skin palette set alloc error: {e:?}"),
                            }
                        }

                        let skin_vbs: std::collections::HashMap<(usize, usize), vk::Buffer> = state
                            .skin_vertex_buffers
                            .iter()
                            .map(|(k, gpu)| (*k, gpu.buffer.handle))
                            .collect();
                        let skinning = SkinningFrame {
                            skin_vertex_buffers: skin_vbs,
                            palette_sets_by_node,
                        };

                        // Fit a default camera around the asset's posed bounds
                        // so the whole model lands inside Vulkan's clip space
                        // (-1..1 in X/Y, 0..1 in Z). Uses the cached rest-pose
                        // AABB — animation drift is small enough that re-walking
                        // every vertex per frame isn't worth it.
                        let extent = live
                            .renderer
                            .window(live.window_handle)
                            .and_then(|w| w.graphics.as_ref())
                            .map(|g| g.swapchain_extent)
                            .unwrap_or(ash::vk::Extent2D {
                                width: 1024,
                                height: 768,
                            });
                        let aspect = extent.width as f32 / extent.height.max(1) as f32;
                        let view_proj = self.camera.view_projection_matrix(aspect);

                        // Ensure the cache has a dummy material; pass it as
                        // the per-frame fallback so draws whose primitive has
                        // `material = None` (or assets with zero materials)
                        // bind a valid set 1 instead of leaving the slot
                        // undefined.
                        let upload_ctx_for_dummy = GltfUploadCtx {
                            device: &live.ctx.device,
                            memory_properties: &live.ctx.memory_properties,
                            graphics_queue: live.ctx.queue,
                            command_pool: live.ctx.command_pool,
                            material_set_layout: live.material_layout,
                            material_pool: live.material_pool,
                            instance_set_layout: live.instance_layout,
                            instance_pool: live.instance_pool,
                        };
                        let fallback_material = state
                            .cache
                            .ensure_dummy_material(&upload_ctx_for_dummy)
                            .ok();
                        let dummy_instance_set = state
                            .cache
                            .ensure_dummy_instance_matrices(&upload_ctx_for_dummy)
                            .ok();
                        // Allocate per-draw per-instance SSBO sets for
                        // any node that declares EXT_mesh_gpu_instancing.
                        // Single upload per (mesh, prim) per frame.
                        let mut instance_sets: std::collections::HashMap<
                            (usize, usize),
                            vk::DescriptorSet,
                        > = std::collections::HashMap::new();
                        let draws_for_instances = forge_gltf::build_graphics_draws_with_matrices(
                            &state.asset,
                            &state.pose.world,
                        );
                        for d in &draws_for_instances {
                            if d.instance_matrices.is_empty() {
                                continue;
                            }
                            let key = (d.mesh as usize, d.primitive as usize);
                            if instance_sets.contains_key(&key) {
                                continue;
                            }
                            if let Ok(set) = state.cache.create_instance_matrices_set(
                                &upload_ctx_for_dummy,
                                &d.instance_matrices,
                            ) {
                                instance_sets.insert(key, set);
                            }
                        }
                        let plans = build_graphics_plans_maximal_with_meshes_vp(
                            &state.asset,
                            &state.pose,
                            &state.meshes,
                            &state.material_sets,
                            &morph_buffers,
                            &skinning,
                            &view_proj,
                            fallback_material,
                            &instance_sets,
                            dummy_instance_set,
                        );
                        let mut proto = Proto::<GraphicsTag>::new(ProtoId::new(1), "gltf_scene");
                        for plan in plans {
                            proto.push_call(plan);
                        }
                        live.renderer
                            .build_graphics_factory(live.window_handle, proto);
                        state.last_anim_time = t;
                        // Pass the compute-completion semaphore upward so
                        // the graphics submission waits on it.
                        compute_signal_outer = compute_signal;
                    }
                }

                let window = live
                    .renderer
                    .window_mut(live.window_handle)
                    .expect("window live");
                let waits: &[vk::Semaphore] = match compute_signal_outer.as_ref() {
                    Some(s) => std::slice::from_ref(s),
                    None => &[],
                };
                let result = unsafe {
                    window.draw_frame_with_compute_wait(
                        &live.ctx.instance,
                        &live.ctx.device,
                        live.ctx.queue,
                        waits,
                    )
                };
                if let Err(e) = result {
                    eprintln!("draw_frame error: {e:?}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(live) = self.live.as_ref() {
            if let Some(window) = live.renderer.window(live.window_handle) {
                if let Some(gfx) = &window.graphics {
                    gfx.winit_window.request_redraw();
                }
            }
        }
    }
}

/// Upload every glTF image as a GpuTexture and every material as a
/// descriptor-bound GpuMaterial. Returns the resolved descriptor set per
/// material index (None when creation failed).
fn upload_materials(
    ctx: &VulkanContext,
    live: &LiveState,
    cache: &mut GltfCache,
    asset: &GltfAsset,
) -> Vec<Option<vk::DescriptorSet>> {
    let upload_ctx = GltfUploadCtx {
        device: &ctx.device,
        memory_properties: &ctx.memory_properties,
        graphics_queue: ctx.queue,
        command_pool: ctx.command_pool,
        material_set_layout: live.material_layout,
        material_pool: live.material_pool,
        instance_set_layout: live.instance_layout,
        instance_pool: live.instance_pool,
    };

    // Resolve per-image sampler from the first texture pointing at it.
    let n_images = asset.images.len();
    let mut img_samplers: Vec<GltfSampler> = vec![GltfSampler::default(); n_images];
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

    // Upload each image, picking the Vulkan format from the forge_gltf
    // sRGB-vs-linear hint so sampler de-gamma fires on albedo/emissive.
    let img_handles: Vec<Option<_>> = asset
        .images
        .iter()
        .enumerate()
        .map(|(i, img)| {
            let fmt = match img.format {
                forge_gltf::ImageFormatHint::Srgb => vk::Format::R8G8B8A8_SRGB,
                forge_gltf::ImageFormatHint::Linear => vk::Format::R8G8B8A8_UNORM,
            };
            match upload_texture_rgba(
                &upload_ctx,
                img.width,
                img.height,
                &img.rgba,
                &img_samplers[i],
                fmt,
            ) {
                Ok(tex) => Some(cache.textures.insert(tex)),
                Err(e) => {
                    eprintln!("texture upload failed (image {i}): {e:?}");
                    None
                }
            }
        })
        .collect();

    // Create each material — fall back to None on failure so the draw still
    // happens with whatever was last bound.
    asset
        .materials
        .iter()
        .map(
            |mat| match create_material(mat, asset, &img_handles, &upload_ctx, cache) {
                Ok(gm) => {
                    let set = gm.descriptor_set;
                    let _h: MaterialHandle = cache.materials.insert(gm);
                    Some(set)
                }
                Err(e) => {
                    eprintln!("material upload failed: {e:?}");
                    None
                }
            },
        )
        .collect()
}

/// Upload one GpuSkinBuffer per skinned primitive in the asset. The
/// buffers stay alive for the asset's lifetime — they're the source of
/// vertex binding 1 on every SkinnedForwardLit draw.
fn upload_skin_vertex_buffers(
    ctx: &VulkanContext,
    asset: &GltfAsset,
) -> std::collections::HashMap<(usize, usize), GpuSkinBuffer> {
    let mut out = std::collections::HashMap::new();
    let mesh_ctx = ctx.mesh_upload_ctx();
    for (mi, mesh) in asset.meshes.iter().enumerate() {
        for (pi, _prim) in mesh.primitives.iter().enumerate() {
            if !primitive_is_skinned(asset, mi as u32, pi as u32) {
                continue;
            }
            let bytes = pack_primitive_skin_attrs(asset, mi as u32, pi as u32);
            let vcount = (bytes.len() / 24) as u32;
            if vcount == 0 {
                continue;
            }
            match GpuSkinBuffer::upload(&mesh_ctx, &bytes, vcount) {
                Ok(b) => {
                    out.insert((mi, pi), b);
                }
                Err(e) => eprintln!("skin vertex buffer upload failed: {e:?}"),
            }
        }
    }
    out
}

// Touch TEXTURE_SLOT_COUNT so it's not flagged unused at the binary scope.
const _SLOT_CHECK: usize = TEXTURE_SLOT_COUNT;

/// Toggle OS cursor grab/visibility on the hello_gltf window.
fn set_grab_hello(live: &LiveState, grabbed: bool) {
    if let Some(window) = live.renderer.window(live.window_handle) {
        if let Some(gfx) = &window.graphics {
            let mode = if grabbed { CursorGrabMode::Locked } else { CursorGrabMode::None };
            let _ = gfx.winit_window.set_cursor_grab(mode);
            let _ = gfx.winit_window.set_cursor_visible(!grabbed);
        }
    }
}

/// Position and orient the camera to frame the given AABB using the same
/// eye-point math as `view_projection_from_aabb`. Also scales near/far to
/// the asset's size so clipping planes don't cut off large or tiny models.
fn fit_camera_to_aabb(camera: &mut Camera, aabb: &([f32; 3], [f32; 3])) {
    let (mn, mx) = aabb;
    let center = [
        0.5 * (mn[0] + mx[0]),
        0.5 * (mn[1] + mx[1]),
        0.5 * (mn[2] + mx[2]),
    ];
    let half = [
        0.5 * (mx[0] - mn[0]).max(1e-3),
        0.5 * (mx[1] - mn[1]).max(1e-3),
        0.5 * (mx[2] - mn[2]).max(1e-3),
    ];
    let radius = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
    let dist = radius * 2.5;
    let eye = [
        center[0] + dist * 0.6,
        center[1] + dist * 0.4,
        center[2] + dist * 1.0,
    ];
    camera.position = eye;
    // Yaw/pitch to look from eye toward center.
    let dx = center[0] - eye[0];
    let dy = center[1] - eye[1];
    let dz = center[2] - eye[2];
    let horiz = (dx * dx + dz * dz).sqrt();
    camera.pitch = dy.atan2(horiz);
    camera.yaw = dz.atan2(dx);
    camera.near = (radius * 0.01).max(0.001);
    camera.far = (radius * 10.0).max(100.0);
    // Scale move speed to the asset so WASD feels right.
    camera.fov = 50.0_f32.to_radians();
}
