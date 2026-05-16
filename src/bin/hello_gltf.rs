// hello_gltf — load a .glb/.gltf file and render it with the ForwardLit pipeline.
//
//   cargo run --bin hello_gltf -- path/to/model.glb

use std::sync::Arc;

use std::sync::mpsc;
use std::thread;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::WindowId;

use dumpster_fire_engine::forge_master::{
    ForgeMaster, GraphicsForgeId, GraphicsOreKind, MeshOre,
};
use dumpster_fire_engine::forge_master::{FrameId, GpuMesh, GraphicsFramePlan};
use dumpster_fire_engine::render::{
    GraphicsTag, Proto, ProtoId, Renderer, VulkanContext,
    Window, WindowId as RenderWindowId,
};
use dumpster_fire_engine::resource_manager::asset_manager::load_first_mesh;

const FORWARD_LIT_VERT: &[u8] = include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/forward_lit.vert.spv")
);
const FORWARD_LIT_FRAG: &[u8] = include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/forward_lit.frag.spv")
);

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| {
            eprintln!("usage: hello_gltf <model.glb>");
            std::process::exit(1);
        });

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        live: None,
        model_path: path,
        mesh_rx: None,
        mesh_ready: false,
    };
       
    event_loop.run_app(&mut app).expect("run event loop");
}

// ── Application state ───────────────────────────────────────────────────────


struct App {
    live:       Option<LiveState>,
    model_path: String,
    mesh_rx:    Option<mpsc::Receiver<MeshOre>>,
    mesh_ready: bool,

}

struct LiveState {
    ctx:           VulkanContext,
    renderer:      Renderer,
    window_handle: dumpster_fire_engine::render::WindowHandle,
    winit_id:      WindowId,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_some() {
            return;
        }

        // ── OS window ────────────────────────────────────────────────────────
        let attrs = winit::window::Window::default_attributes()
            .with_title("hello_gltf")
            .with_inner_size(winit::dpi::LogicalSize::new(800u32, 600u32));
        let winit_window = Arc::new(
            event_loop.create_window(attrs).expect("create window"),
        );
        let winit_id = winit_window.id();

        // ── Vulkan ───────────────────────────────────────────────────────────
        let display_handle = winit_window
            .display_handle()
            .expect("display handle")
            .as_raw();
        let ctx = VulkanContext::with_surface(display_handle)
            .expect("Vulkan init");
        // ── Start background mesh loader ─────────────────────────────────────
        let path = self.model_path.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let ore = load_first_mesh(&path)
                .unwrap_or_else(|e| panic!("failed to load '{}': {e}", path));
            tx.send(ore).unwrap();
        });
        self.mesh_rx = Some(rx);
        self.mesh_ready = false;

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
        )
        .expect("Window::new_with_surface");

        let mut renderer = Renderer::new(forge);
        let window_handle = renderer.add_window(window);

        // ── Graphics factory (one indexed draw of the loaded mesh) ───────────
   
   

        self.live = Some(LiveState { ctx, renderer, window_handle, winit_id });
        if let Some(gfx) = &self.live.as_ref().unwrap().renderer
            .window(self.live.as_ref().unwrap().window_handle)
            .unwrap()
            .graphics
        {
            gfx.winit_window.request_redraw();
        }
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
                // If mesh not yet loaded, poll for it
                if !self.mesh_ready {
                    if let Some(rx) = &self.mesh_rx {
                        if let Ok(ore) = rx.try_recv() {
                            let gpu_mesh = GpuMesh::upload(
                                &live.ctx.mesh_upload_ctx(),
                                &ore,
                            )
                            .expect("GpuMesh upload");
                            let mesh = Arc::new(gpu_mesh);

                            let mut proto =
                                Proto::<GraphicsTag>::new(ProtoId::new(1), "mesh");
                            proto.push_call(GraphicsFramePlan::new_mesh(
                                FrameId::new(1),
                                "model",
                                mesh,
                            ));
                            live
                                .renderer
                                .build_graphics_factory(live.window_handle, proto);
                            self.mesh_ready = true;
                        }
                    }
                }

                let window = live
                    .renderer
                    .window_mut(live.window_handle)
                    .expect("window live");
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
