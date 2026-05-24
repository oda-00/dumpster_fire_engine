//! hello_gltf — load a .glb/.gltf file and render it via the World/render_world path.
//!
//! cargo run --bin hello_gltf -- path/to/model.glb

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use glam::Affine3A;
use winit::event_loop::ActiveEventLoop;

use dumpster_fire_engine::forge_master::master::ForgeResult;
use dumpster_fire_engine::render::app::{AppCtx, AppHandle, AppLogic, AppRunner};
use dumpster_fire_engine::resource_manager::component::{
    LightData, LightKind, MeshRef, UtilityComponent,
};
use dumpster_fire_engine::resource_manager::{
    ActorId, ActorType, Environment, EnvironmentId, LevelHandle, LevelId, StageHandle, StageId,
    Utility, UtilityId, World, WorldId,
};

struct HelloGltfApp {
    asset_path: PathBuf,
    world: World,
    lh_sh: Option<(LevelHandle, StageHandle)>,
    fitted: bool,
    start: Instant,
    win: Option<AppHandle>,
}

impl AppLogic for HelloGltfApp {
    fn on_start(&mut self, ctx: &mut AppCtx<'_>, ev: &ActiveEventLoop) -> ForgeResult<()> {
        let win = ctx.spawn_window(ev, "hello_gltf", 1024, 768)?;
        self.win = Some(win);

        let lh = self.world.spawn_level(LevelId::new(1), "scene");
        let sh = self.world.spawn_stage(lh, StageId::new(1), "main").unwrap();
        self.lh_sh = Some((lh, sh));

        // Key light.
        let light_ah = self
            .world
            .spawn_actor(
                lh,
                sh,
                ActorId::new(1),
                glam::Affine3A::from_translation(glam::Vec3::new(4.0, 4.0, 4.0)),
            )
            .unwrap();
        self.world
            .spawn_sub_entity(
                lh,
                sh,
                light_ah,
                ActorType::Utility(Utility {
                    id: UtilityId::new(1),
                    name: Arc::from("light"),
                    visible: true,
                    toggle: true,
                    mesh: None,
                }),
                Affine3A::IDENTITY,
            )
            .unwrap();
        self.world.add_component(
            lh,
            sh,
            light_ah,
            3,
            UtilityComponent {
                name: Arc::from("light"),
                description: Arc::from(""),
                camera: None,
                light: Some(LightData {
                    color: [1.0, 1.0, 1.0],
                    intensity: 5.0,
                    range: 50.0,
                    kind: LightKind::Point,
                }),
                render: None,
            },
        );

        // Mesh actor.
        let asset = ctx.load_gltf(win, self.asset_path.clone())?;
        let mesh_ah = self
            .world
            .spawn_actor(lh, sh, ActorId::new(2), Affine3A::IDENTITY)
            .unwrap();
        self.world
            .spawn_sub_entity(
                lh,
                sh,
                mesh_ah,
                ActorType::Environment(Environment {
                    id: EnvironmentId::new(1),
                    name: Arc::from("model"),
                    visible: true,
                    physical: false,
                    mesh: Some(MeshRef { asset }),
                }),
                Affine3A::IDENTITY,
            )
            .unwrap();

        eprintln!("hello_gltf: loading {}", self.asset_path.display());
        Ok(())
    }

    fn update(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle, _dt: f32) -> bool {
        let Some(win) = self.win else { return true };
        if win != app {
            return true;
        }

        ctx.poll_gltf_loaders(app).ok();
        self.world.propagate_transforms();

        if !self.fitted
            && let Some(aabb) = ctx.gltf_union_aabb_for_world(&self.world)
        {
            ctx.fit_window_camera_to_aabb(app, &aabb);
            self.fitted = true;
        }

        let elapsed = self.start.elapsed().as_secs_f32();
        match ctx.render_world(&self.world, app, elapsed) {
            Ok(Some(sem)) => ctx.push_compute_wait(app, sem),
            Ok(None) => {}
            Err(e) => eprintln!("render_world error: {e:?}"),
        }
        true
    }
}

fn main() -> ForgeResult<()> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/models/BrainStem.glb"));
    AppRunner::new(HelloGltfApp {
        asset_path: path,
        world: World::new(WorldId::new(1)),
        lh_sh: None,
        fitted: false,
        start: Instant::now(),
        win: None,
    })
    .run()
}
