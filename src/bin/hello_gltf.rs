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
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::WindowId;

use dumpster_fire_engine::forge_master::{ForgeMaster, GraphicsForgeId, GraphicsOreKind};
use dumpster_fire_engine::render::{
    GraphicsTag, Proto, ProtoId, Renderer, VulkanContext,
    Window, WindowId as RenderWindowId,
};
use dumpster_fire_engine::resource_manager::asset_manager::{
    build_graphics_plans_with_pose_and_materials,
    forge_gltf::{GltfAsset, Pose}, load_asset, register_skin_morph_forges,
};
use dumpster_fire_engine::resource_manager::gltf_driver::{
    AsyncGltfLoader, GltfCache, GltfUploadCtx, MaterialHandle,
    create_material, create_material_pool, gltf_sampler_to_vk, upload_texture_rgba,
    GltfSampler, TEXTURE_SLOT_COUNT,
};

const FORWARD_LIT_VERT: &[u8] = include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/forward_lit.vert.spv")
);
const FORWARD_LIT_FRAG: &[u8] = include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/forward_lit.frag.spv")
);

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: hello_gltf <model.glb>");
        std::process::exit(1);
    });

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        live: None,
        model_path: path,
        loader: None,
        asset_loaded: None,
        start: Instant::now(),
    };
    event_loop.run_app(&mut app).expect("run event loop");
}

struct App {
    live:         Option<LiveState>,
    model_path:   String,
    loader:       Option<AsyncGltfLoader>,
    asset_loaded: Option<AssetState>,
    start:        Instant,
}

struct AssetState {
    asset:           GltfAsset,
    pose:            Pose,
    /// Pre-resolved descriptor set per material index. Built once on load.
    material_sets:   Vec<Option<vk::DescriptorSet>>,
    /// Owned by the App for the lifetime of the asset.
    cache:           GltfCache,
    /// Track if we've ever animated.
    last_anim_time:  f32,
}

struct LiveState {
    ctx:             VulkanContext,
    renderer:        Renderer,
    window_handle:   dumpster_fire_engine::render::WindowHandle,
    winit_id:        WindowId,
    material_pool:   vk::DescriptorPool,
    material_layout: vk::DescriptorSetLayout,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_some() { return; }

        // ── OS window ────────────────────────────────────────────────────────
        let attrs = winit::window::Window::default_attributes()
            .with_title("hello_gltf")
            .with_inner_size(winit::dpi::LogicalSize::new(1024u32, 768u32));
        let winit_window = Arc::new(
            event_loop.create_window(attrs).expect("create window"),
        );
        let winit_id = winit_window.id();

        // ── Vulkan ───────────────────────────────────────────────────────────
        let display_handle = winit_window
            .display_handle().expect("display handle").as_raw();
        let ctx = VulkanContext::with_surface(display_handle).expect("Vulkan init");

        // ── Background asset loader (hand-rolled AsyncGltfLoader) ────────────
        self.loader = Some(AsyncGltfLoader::spawn(self.model_path.clone().into()));

        // ── ForwardLit forge ─────────────────────────────────────────────────
        let mut forge = ForgeMaster::new(
            ctx.device.clone(),
            ctx.queue,
            ctx.command_pool,
            ctx.memory_properties,
        ).expect("ForgeMaster");
        forge.add_graphics_forge_from_spirv_bytes(
            GraphicsForgeId::new(1),
            GraphicsOreKind::ForwardLit,
            FORWARD_LIT_VERT,
            FORWARD_LIT_FRAG,
        ).expect("register ForwardLit forge");
        // Pre-compile SkinPalette + MorphBlend compute pipelines once at
        // startup — per-frame plans can dispatch them without re-linking.
        register_skin_morph_forges(&mut forge).expect("register skin/morph forges");
        let graphics_forge = forge
            .graphics_forge(GraphicsOreKind::ForwardLit)
            .expect("ForwardLit forge present");

        // ── Window + swapchain + pipeline ────────────────────────────────────
        let window = Window::new_with_surface(
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
            &ctx.entry,
            graphics_forge,
        ).expect("Window::new_with_surface");

        let mut renderer = Renderer::new(forge);
        let window_handle = renderer.add_window(window);

        // Grab the material descriptor-set layout from the freshly built mold.
        let material_layout = renderer
            .window(window_handle)
            .and_then(|w| w.graphics.as_ref())
            .map(|g| g.mold.material_set_layout)
            .unwrap_or(vk::DescriptorSetLayout::null());

        // Material descriptor pool (4096 sets max — fits any reasonable glTF).
        let material_pool = create_material_pool(&ctx.device, 4096)
            .expect("create material descriptor pool");

        self.live = Some(LiveState {
            ctx, renderer, window_handle, winit_id,
            material_pool, material_layout,
        });
        if let Some(gfx) = &self.live.as_ref().unwrap().renderer
            .window(self.live.as_ref().unwrap().window_handle)
            .unwrap()
            .graphics
        {
            gfx.winit_window.request_redraw();
        }
        self.start = Instant::now();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        event: WindowEvent,
    ) {
        let Some(live) = self.live.as_mut() else { return };
        if id != live.winit_id { return; }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(new_size) => {
                if let Some(window) = live.renderer.window_mut(live.window_handle) {
                    window.resize(new_size.width, new_size.height);
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
                                        asset.meshes.len(), asset.nodes.len(),
                                        asset.animations.len(), asset.materials.len(),
                                        asset.textures.len(), asset.images.len(),
                                        asset.lights.len(),
                                    );
                                    println!("{summary}");

                                    let pose = Pose::rest(&asset);
                                    let mut cache = GltfCache::new();
                                    let material_sets =
                                        upload_materials(&live.ctx, live, &mut cache, &asset);

                                    self.asset_loaded = Some(AssetState {
                                        asset, pose, material_sets, cache,
                                        last_anim_time: -1.0,
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

                // If the asset is in, advance any animation and (re)build
                // the per-frame draw list.
                if let Some(state) = self.asset_loaded.as_mut() {
                    let t = self.start.elapsed().as_secs_f32();
                    let advanced = if let Some(anim) = state.asset.animations.first() {
                        let dur = anim.duration().max(1e-3);
                        state.pose.sample(&state.asset, anim, t.rem_euclid(dur));
                        true
                    } else { false };

                    let needs_rebuild = advanced || state.last_anim_time < 0.0;
                    if needs_rebuild {
                        let upload_ctx = live.ctx.mesh_upload_ctx();
                        match build_graphics_plans_with_pose_and_materials(
                            &state.asset,
                            &state.pose,
                            &upload_ctx,
                            &state.material_sets,
                        ) {
                            Ok(plans) => {
                                let mut proto = Proto::<GraphicsTag>::new(ProtoId::new(1), "gltf_scene");
                                for plan in plans { proto.push_call(plan); }
                                live.renderer.build_graphics_factory(live.window_handle, proto);
                            }
                            Err(e) => eprintln!("plan build error: {e:?}"),
                        }
                        state.last_anim_time = t;
                    }
                }

                let window = live.renderer.window_mut(live.window_handle).expect("window live");
                let result = unsafe {
                    window.draw_frame(&live.ctx.instance, &live.ctx.device, live.ctx.queue)
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
    ctx:   &VulkanContext,
    live:  &LiveState,
    cache: &mut GltfCache,
    asset: &GltfAsset,
) -> Vec<Option<vk::DescriptorSet>> {
    let upload_ctx = GltfUploadCtx {
        device:              &ctx.device,
        memory_properties:   &ctx.memory_properties,
        graphics_queue:      ctx.queue,
        command_pool:        ctx.command_pool,
        material_set_layout: live.material_layout,
        material_pool:       live.material_pool,
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

    // Upload each image.
    let img_handles: Vec<Option<_>> = asset.images.iter().enumerate()
        .map(|(i, img)| {
            match upload_texture_rgba(&upload_ctx, img.width, img.height, &img.rgba, &img_samplers[i]) {
                Ok(tex) => Some(cache.textures.insert(tex)),
                Err(e)  => {
                    eprintln!("texture upload failed (image {i}): {e:?}");
                    None
                }
            }
        }).collect();

    // Create each material — fall back to None on failure so the draw still
    // happens with whatever was last bound.
    asset.materials.iter().map(|mat| {
        match create_material(mat, asset, &img_handles, &upload_ctx, cache) {
            Ok(gm) => {
                let set = gm.descriptor_set;
                let _h: MaterialHandle = cache.materials.insert(gm);
                Some(set)
            }
            Err(e) => {
                eprintln!("material upload failed: {e:?}");
                None
            }
        }
    }).collect()
}

// Touch TEXTURE_SLOT_COUNT so it's not flagged unused at the binary scope.
const _SLOT_CHECK: usize = TEXTURE_SLOT_COUNT;
