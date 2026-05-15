// hello_triangle — a minimal winit + Vulkan window that draws a coloured
// triangle using the forge/frame/proto pipeline.
//
//   cargo run --bin hello_triangle

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::WindowId;

use dumpster_fire_engine::forge_master::{
    ForgeMaster, GraphicsForgeId, GraphicsOreKind,
};
use dumpster_fire_engine::render::{
    GraphicsTag, Proto, ProtoId, Renderer, VulkanContext,
    Window, WindowId as RenderWindowId,
};
use dumpster_fire_engine::forge_master::FrameId;
use dumpster_fire_engine::forge_master::GraphicsFramePlan;

// SPIR-V embedded at compile time (compiled by build.rs via glslc).
const TRIANGLE_VERT: &[u8] = include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/triangle.vert.spv")
);
const TRIANGLE_FRAG: &[u8] = include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/triangle.frag.spv")
);

fn main() {
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::default();
    event_loop.run_app(&mut app).expect("run event loop");
}

// ── Application state ───────────────────────────────────────────────────────

#[derive(Default)]
struct App {
    live: Option<LiveState>,
}

struct LiveState {
    ctx: VulkanContext,
    renderer: Renderer,
    window_handle: dumpster_fire_engine::render::WindowHandle,
    winit_id: WindowId,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_some() {
            return; // already initialised (e.g. Android resume)
        }

        // Create the OS window first so we can query its display handle.
        let attrs = winit::window::Window::default_attributes()
            .with_title("hello_triangle")
            .with_inner_size(winit::dpi::LogicalSize::new(800u32, 600u32));
        let winit_window = Arc::new(
            event_loop.create_window(attrs).expect("create window"),
        );
        let winit_id = winit_window.id();

        // Bootstrap Vulkan with surface extensions for this platform.
        let display_handle = winit_window
            .display_handle()
            .expect("display handle")
            .as_raw();
        let ctx = VulkanContext::with_surface(display_handle)
            .expect("Vulkan init with surface");

        // Register a GraphicsForge for the Ui kind (no descriptors — triangle
        // shader hardcodes its own vertices).
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
                GraphicsOreKind::Ui,
                TRIANGLE_VERT,
                TRIANGLE_FRAG,
            )
            .expect("register triangle GraphicsForge");

        let graphics_forge = forge
            .graphics_forge(GraphicsOreKind::Ui)
            .expect("Ui forge present");

        // Open a graphics window (creates the swapchain and compiles the
        // pipeline from the forge).
        let window = Window::new_with_surface(
            RenderWindowId::new(1),
            "hello_triangle",
            winit_window.clone(),
            &ctx.instance,
            ctx.physical_device,
            &ctx.device,
            ctx.queue,
            ctx.queue_family_index,
            &ctx.memory_properties,
            &ctx.entry,
            graphics_forge,
        )
        .expect("Window::new_with_surface");

        let mut renderer = Renderer::new(forge);
        let window_handle = renderer.add_window(window);

        // Build a graphics factory with a single triangle draw call (3
        // vertices, 1 instance, no vertex buffers).
        let mut proto = Proto::<GraphicsTag>::new(ProtoId::new(1), "triangle");
        proto.push_call(GraphicsFramePlan::new(
            FrameId::new(1),
            "tri",
            GraphicsOreKind::Ui,
            3, // vertex_count — matches the hardcoded positions in the shader
        ));
        renderer.build_graphics_factory(window_handle, proto);

        self.live = Some(LiveState {
            ctx,
            renderer,
            window_handle,
            winit_id,
        });
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        event: WindowEvent,
    ) {
        let Some(live) = self.live.as_mut() else { return };
        if id != live.winit_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(_) => {
                // Swapchain recreation on resize is left as a future exercise;
                // for now just skip the next frame to avoid a validation error.
            }
            WindowEvent::RedrawRequested => {
                let window = live
                    .renderer
                    .window_mut(live.window_handle)
                    .expect("window live");

                // draw_frame issues all GraphicsFrame draw calls from every
                // factory the window owns.
                let result = unsafe {
                    window.draw_frame(&live.ctx.device, live.ctx.queue)
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
