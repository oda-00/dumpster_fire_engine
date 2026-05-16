// hello_gltf — load a .glb/.gltf file and render it with the ForwardLit
// pipeline using the full `forge_gltf` API:
//
//   • walks every node-primitive draw in the primary scene (not just the
//     first primitive),
//   • samples animations over wall-clock time and rebuilds per-frame plans,
//   • uploads each unique primitive once and reuses the GpuMesh across
//     animation frames.
//
//   cargo run --bin hello_gltf -- path/to/model.glb

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

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
    build_graphics_plans_with_pose, forge_gltf::{GltfAsset, Pose}, load_asset,
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
        asset_rx: None,
        asset_loaded: None,
        start: Instant::now(),
    };
    event_loop.run_app(&mut app).expect("run event loop");
}

struct App {
    live:         Option<LiveState>,
    model_path:   String,
    asset_rx:     Option<mpsc::Receiver<GltfAsset>>,
    asset_loaded: Option<AssetState>,
    start:        Instant,
}

struct AssetState {
    asset: GltfAsset,
    pose:  Pose,
    /// Cached so we don't rebuild the proto on every redraw — only the
    /// MVPs inside it change between frames; the engine reads the latest
    /// plans each redraw.
    last_anim_time: f32,
}

struct LiveState {
    ctx:           VulkanContext,
    renderer:      Renderer,
    window_handle: dumpster_fire_engine::render::WindowHandle,
    winit_id:      WindowId,
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

        // ── Background asset loader: pull the full document, not just one
        //    primitive, so per-frame replanning sees the whole scene.
        let path = self.model_path.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let asset = load_asset(&path)
                .unwrap_or_else(|e| panic!("failed to load '{path}': {e}"));
            tx.send(asset).unwrap();
        });
        self.asset_rx = Some(rx);

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

        self.live = Some(LiveState { ctx, renderer, window_handle, winit_id });
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
                // First time we see the asset, install it.
                if self.asset_loaded.is_none() {
                    if let Some(rx) = &self.asset_rx {
                        if let Ok(asset) = rx.try_recv() {
                            let pose = Pose::rest(&asset);
                            let summary = format!(
                                "loaded: {} meshes, {} nodes, {} animations, {} materials, {} lights",
                                asset.meshes.len(), asset.nodes.len(),
                                asset.animations.len(), asset.materials.len(), asset.lights.len(),
                            );
                            println!("{summary}");
                            self.asset_loaded = Some(AssetState {
                                asset, pose, last_anim_time: -1.0,
                            });
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

                    // Rebuild plans every frame for animated assets; static
                    // assets only build once.
                    let needs_rebuild = advanced || state.last_anim_time < 0.0;
                    if needs_rebuild {
                        let upload_ctx = live.ctx.mesh_upload_ctx();
                        match build_graphics_plans_with_pose(&state.asset, &state.pose, &upload_ctx) {
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
