//! Two windows, each rendering the same GLB file with independent cameras.
//! Run: `cargo run --bin multi_gltf [asset.glb]`
//!
//! Migrated to the `AppLogic` runtime — the engine owns Vulkan context,
//! renderer, descriptor management, semaphore threading, and per-window
//! lifetime. This binary just spawns windows and registers compute forges.

use std::path::PathBuf;

use dumpster_fire_engine::forge_master::ForgeMaster;
use dumpster_fire_engine::forge_master::master::ForgeResult;
use dumpster_fire_engine::render::app::{AppCtx, AppHandle, AppLogic, AppRunner};
use dumpster_fire_engine::resource_manager::asset_manager::gltf_loader::register_skin_morph_forges;
use winit::event_loop::ActiveEventLoop;

struct MultiGltfApp {
    asset_path: PathBuf,
    spawned:    Vec<AppHandle>,
}

impl AppLogic for MultiGltfApp {
    fn register_forges(&mut self, forge: &mut ForgeMaster) -> ForgeResult<()> {
        register_skin_morph_forges(forge)
    }

    fn on_start(&mut self, ctx: &mut AppCtx<'_>, ev: &ActiveEventLoop) -> ForgeResult<()> {
        let left  = ctx.spawn_window(ev, "multi_gltf — left",  960, 720)?;
        let right = ctx.spawn_window(ev, "multi_gltf — right", 960, 720)?;
        self.spawned.push(left);
        self.spawned.push(right);
        eprintln!("multi_gltf: loaded asset = {}", self.asset_path.display());
        Ok(())
    }
}

fn main() -> ForgeResult<()> {
    let asset_path = std::env::args().nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/models/BrainStem.glb"));
    AppRunner::new(MultiGltfApp { asset_path, spawned: Vec::new() }).run()
}
