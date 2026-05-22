//! Multiview real-time editor — Unreal-style quad-split viewport.
//!
//! Perspective (top-left), OrthoTop (top-right), OrthoFront (bottom-left),
//! OrthoRight (bottom-right). Tab cycles layouts. Click to grab mouse, Escape
//! to release. WASD fly, Q/E up/down, Z/C roll, scroll zoom, middle-drag pan.
//!
//! Actors: one `Environment` mesh actor per CLI arg. A default key-light
//! `Utility` actor at (4, 4, 4). Four camera `Utility` actors back the
//! viewport panes (managed by the viewport grid).
//!
//! Run: cargo run --bin editor [path/to/model.glb [...]]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use glam::{Affine3A, Vec3};
use thin_vec::ThinVec;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};

use dumpster_fire_engine::forge_master::master::ForgeResult;
use dumpster_fire_engine::render::app::{
    AppCtx, AppHandle, AppLogic, AppRunner, ViewportLayout,
};
use dumpster_fire_engine::resource_manager::{
    ActorId, ActorType,
    Environment, EnvironmentId,
    LevelHandle, LevelId, StageHandle, StageId,
    Utility, UtilityId,
    World,
};
use dumpster_fire_engine::resource_manager::component::{
    LightData, LightKind, MeshRef, UtilityComponent,
};
use dumpster_fire_engine::resource_manager::manager::ActorHandle;

struct EditorApp {
    asset_paths: Vec<PathBuf>,
    world:       World,
    main_stage:  Option<(LevelHandle, StageHandle)>,
    actors:      ThinVec<ActorHandle>,
    cam_fitted:  bool,
    start:       Instant,
    win:         Option<AppHandle>,
}

impl AppLogic for EditorApp {
    fn on_start(&mut self, ctx: &mut AppCtx<'_>, ev: &ActiveEventLoop) -> ForgeResult<()> {
        let win = ctx.spawn_window(ev, "Editor", 1280, 720)?;
        self.win = Some(win);
        ctx.init_viewport_grid(win, ViewportLayout::FourQuadrant)?;

        let lh = self.world.spawn_level(LevelId::new(1), "editor");
        let sh = self.world.spawn_stage(lh, StageId::new(1), "scene").unwrap();
        self.main_stage = Some((lh, sh));

        // Default point-light actor.
        let light_ah = self.world.spawn_actor(lh, sh, ActorId::new(2),
            Affine3A::from_translation(Vec3::new(4.0, 4.0, 4.0))).unwrap();
        self.world.spawn_sub_entity(lh, sh, light_ah,
            ActorType::Utility(Utility {
                id:      UtilityId::new(1),
                name:    Arc::from("key_light"),
                visible: true,
                toggle:  true,
                mesh:    None,
            }),
            Affine3A::IDENTITY).unwrap();
        self.world.add_component(lh, sh, light_ah, 3,
            UtilityComponent {
                name:        Arc::from("key_light"),
                description: Arc::from(""),
                camera:      None,
                light:       Some(LightData {
                    color:     [1.0, 0.95, 0.85],
                    intensity: 5.0,
                    range:     30.0,
                    kind:      LightKind::Point,
                }),
                render: None,
            });

        // One Environment actor per asset path.
        for (i, path) in self.asset_paths.iter().enumerate() {
            let asset = ctx.load_gltf(win, path.clone())?;
            let offset = Affine3A::from_translation(Vec3::new(i as f32 * 3.0, 0.0, 0.0));
            let ah = self.world.spawn_actor(lh, sh,
                ActorId::new(100 + i as i64), offset).unwrap();
            self.world.spawn_sub_entity(lh, sh, ah,
                ActorType::Environment(Environment {
                    id:       EnvironmentId::new(i as i64),
                    name:     Arc::from(format!("model_{i}")),
                    visible:  true,
                    physical: false,
                    mesh:     Some(MeshRef { asset }),
                }),
                Affine3A::IDENTITY).unwrap();
            self.actors.push(ah);
        }
        Ok(())
    }

    fn handle_event(
        &mut self,
        ctx:   &mut AppCtx<'_>,
        app:   AppHandle,
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

        ctx.poll_gltf_loaders(app).ok();
        self.world.propagate_transforms();

        if !self.cam_fitted {
            if let Some(aabb) = ctx.gltf_union_aabb_for_world(&self.world) {
                ctx.fit_all_panes_to_aabb(app, &aabb);
                self.cam_fitted = true;
            }
        }

        let elapsed = self.start.elapsed().as_secs_f32();
        match ctx.render_world(&self.world, app, elapsed) {
            Ok(Some(sem)) => ctx.push_compute_wait(app, sem),
            Ok(None)      => {}
            Err(e)        => eprintln!("render_world error: {e:?}"),
        }
        true
    }
}

fn main() -> ForgeResult<()> {
    let paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let paths = if paths.is_empty() {
        vec![PathBuf::from("assets/models/BrainStem.glb")]
    } else {
        paths
    };
    AppRunner::new(EditorApp {
        asset_paths: paths,
        world:       World::new(dumpster_fire_engine::resource_manager::WorldId::new(1)),
        main_stage:  None,
        actors:      ThinVec::new(),
        cam_fitted:  false,
        start:       Instant::now(),
        win:         None,
    }).run()
}
