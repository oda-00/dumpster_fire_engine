//! Two windows, each rendering the same GLB file with independent cameras.
//! Run: `cargo run --bin multi_gltf [asset.glb]`
//!
//! Uses the `AppLogic` runtime — the engine owns Vulkan, the renderer,
//! descriptor management, and per-window camera/input.
//!
//! ForwardLit, SkinnedForwardLit, and the skin/morph compute forges are
//! registered automatically by `AppRunner`; no `register_forges` override needed.

use std::path::PathBuf;
use std::time::Instant;

use dumpster_fire_engine::render::app::{AppCtx, AppHandle, AppLogic, AppRunner};
use dumpster_fire_engine::resource_manager::gltf_scene::GltfScene;
use winit::event_loop::ActiveEventLoop;
use dumpster_fire_engine::forge_master::master::ForgeResult;

struct WindowState {
    scene:       GltfScene,
    cam_fitted:  bool,
}

struct MultiGltfApp {
    asset_path: PathBuf,
    windows:    Vec<(AppHandle, WindowState)>,
    start:      Instant,
}

impl AppLogic for MultiGltfApp {
    fn on_start(&mut self, ctx: &mut AppCtx<'_>, ev: &ActiveEventLoop) -> ForgeResult<()> {
        for title in ["multi_gltf — left", "multi_gltf — right"] {
            let handle = ctx.spawn_window(ev, title, 960, 720)?;
            let mut scene = ctx.new_gltf_scene(handle)?;
            scene.load(self.asset_path.clone());
            self.windows.push((handle, WindowState { scene, cam_fitted: false }));
        }
        eprintln!("multi_gltf: loading {}", self.asset_path.display());
        Ok(())
    }

    fn update(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle, _dt: f32) -> bool {
        let elapsed = self.start.elapsed().as_secs_f32();
        // Find the window state for this app handle.
        let Some((_handle, ws)) = self.windows.iter_mut().find(|(h, _)| *h == app) else {
            return true;
        };
        // Auto-fit camera once the asset is loaded.
        if ws.scene.is_loaded() && !ws.cam_fitted {
            if let Some(cam) = ctx.camera_mut(app) {
                ws.scene.fit_camera(cam);
            }
            ws.cam_fitted = true;
        }
        let vp = ctx.camera_vp(app);
        match ctx.gltf_update(&mut ws.scene, app, elapsed, &vp) {
            Ok(Some(sem)) => ctx.push_compute_wait(app, sem),
            Ok(None) => {}
            Err(e) => eprintln!("gltf_update error: {e:?}"),
        }
        true
    }
}

fn main() -> ForgeResult<()> {
    let asset_path = std::env::args().nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/models/BrainStem.glb"));
    AppRunner::new(MultiGltfApp {
        asset_path,
        windows: Vec::new(),
        start: Instant::now(),
    }).run()
}
