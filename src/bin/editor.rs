//! Multiview real-time editor — Unreal-style quad-split viewport.
//!
//! Perspective (top-left), OrthoTop (top-right), OrthoFront (bottom-left),
//! OrthoRight (bottom-right). Tab cycles layouts. Click to grab mouse, Escape
//! to release. WASD fly, Q/E up/down, Z/C roll, scroll zoom, middle-drag pan.
//!
//! Run: cargo run --bin editor [path/to/model.glb]

use std::path::PathBuf;
use std::time::Instant;

use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};

use dumpster_fire_engine::forge_master::master::ForgeResult;
use dumpster_fire_engine::render::app::{
    AppCtx, AppHandle, AppLogic, AppRunner, ViewportLayout,
};
use dumpster_fire_engine::resource_manager::gltf_scene::GltfScene;

struct EditorApp {
    asset_path: PathBuf,
    win:        Option<AppHandle>,
    scene:      Option<GltfScene>,
    cam_fitted: bool,
    start:      Instant,
}

impl AppLogic for EditorApp {
    fn on_start(&mut self, ctx: &mut AppCtx<'_>, ev: &ActiveEventLoop) -> ForgeResult<()> {
        let win = ctx.spawn_window(ev, "Editor", 1280, 720)?;
        ctx.init_viewport_grid(win, ViewportLayout::FourQuadrant)?;
        let mut scene = ctx.new_gltf_scene(win)?;
        scene.load(self.asset_path.clone());
        eprintln!("editor: loading {}", self.asset_path.display());
        self.win   = Some(win);
        self.scene = Some(scene);
        Ok(())
    }

    fn handle_event(
        &mut self,
        ctx: &mut AppCtx<'_>,
        app: AppHandle,
        event: &WindowEvent,
    ) -> bool {
        if let WindowEvent::KeyboardInput { event: ke, .. } = event {
            if let PhysicalKey::Code(KeyCode::Tab) = ke.physical_key {
                if ke.state == ElementState::Pressed {
                    if let Some(grid) = ctx.viewport_grid_mut(app) {
                        let next = grid.layout.next();
                        grid.set_layout(next, &[]);
                    }
                    return true;
                }
            }
        }
        false
    }

    fn update(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle, _dt: f32) -> bool {
        let Some(win) = self.win else { return true };
        if win != app { return true; }
        let scene = match &mut self.scene { Some(s) => s, None => return true };

        if scene.is_loaded() && !self.cam_fitted {
            // Fit the perspective camera (slot 0) to scene bounds.
            let persp_cam_h = ctx.viewport_grid(app).and_then(|g| {
                let vh = g.slot(0)?;
                Some(g.get(vh)?.camera_handle)
            });
            if let Some(cam_h) = persp_cam_h {
                if let Some(cam) = ctx.cameras.get_mut(cam_h) {
                    scene.fit_camera(cam);
                }
            }
            self.cam_fitted = true;
        }

        let elapsed = self.start.elapsed().as_secs_f32();
        match ctx.gltf_update_model_only(scene, app, elapsed) {
            Ok(Some(sem)) => ctx.push_compute_wait(app, sem),
            Ok(None)      => {}
            Err(e)        => eprintln!("gltf_update error: {e:?}"),
        }
        true
    }
}

fn main() -> ForgeResult<()> {
    let path = std::env::args().nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/models/BrainStem.glb"));
    AppRunner::new(EditorApp {
        asset_path: path,
        win:        None,
        scene:      None,
        cam_fitted: false,
        start:      Instant::now(),
    }).run()
}
