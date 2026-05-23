//! Multiview real-time editor — Unreal-style quad-split viewport.
//!
//! Layout: Perspective (top-left) | OrthoTop (top-right)
//!         OrthoFront (bottom-left) | OrthoRight (bottom-right)
//!
//! Controls:
//!   Tab         — cycle viewport layout
//!   G / R / S   — Translate / Rotate / Scale gizmo mode
//!   Del         — despawn selected actor
//!   Esc         — clear selection / close menus
//!   LMB         — click-to-select actor (ray-AABB)
//!   RMB         — clear selection

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use glam::{Affine3A, Mat4, Vec3, Vec4, Vec4Swizzles};
use thin_vec::ThinVec;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};

use dumpster_fire_engine::forge_master::master::ForgeResult;
use dumpster_fire_engine::render::app::{
    AppCtx, AppHandle, AppLogic, AppRunner, ViewportLayout,
};
use dumpster_fire_engine::render::camera::ProjectionMode;
use dumpster_fire_engine::resource_manager::{
    ActorId, ActorType,
    Environment, EnvironmentId,
    LevelHandle, LevelId, StageHandle, StageId,
    Utility, UtilityId,
    World,
};
use dumpster_fire_engine::resource_manager::component::{
    Component, ComponentType, GltfHandle,
    LightData, LightKind, MeshRef, UtilityComponent,
};
use dumpster_fire_engine::resource_manager::manager::ActorHandle;

// ── Editor state ───────────────────────────────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum GizmoMode { Translate, Rotate, Scale }

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum GizmoSpace { World, Local }

/// Active drag state captured at the click that started the gizmo grab.
/// `axis` is 0/1/2 for X/Y/Z; `mode` selects which transform op to apply.
#[derive(Copy, Clone, Debug)]
struct GizmoDrag {
    actor:        ActorHandle,
    mode:         GizmoMode,
    axis:         u8,
    start_local:  Affine3A,
    start_world:  Affine3A,
    start_cursor: [f32; 2],
    /// Screen-space arrow vector (tip - origin) captured at click time
    /// so the per-frame delta math is stable while the user drags.
    arrow_screen: [f32; 2],
    /// World-space arrow length so screen-space "progress" maps to world delta.
    arrow_world_len: f32,
}

struct EditorApp {
    asset_paths:       ThinVec<Arc<str>>,
    world:             World,
    main_level:        Option<LevelHandle>,
    main_stage:        Option<(LevelHandle, StageHandle)>,
    actors:            ThinVec<ActorHandle>,
    cam_fitted:        bool,
    start:             Instant,
    win:               Option<AppHandle>,

    // ── toolbar state
    grid_enabled:      bool,
    gizmo_mode:        GizmoMode,
    gizmo_space:       GizmoSpace,
    spawn_menu_open:   bool,
    light_submenu_open: bool,

    // ── file picker
    picker_open:       bool,
    picker_filter:     Arc<str>,
    picker_paths:      ThinVec<Arc<str>>,

    // ── outliner
    outliner_filter:   Arc<str>,

    // ── FPS
    frame_time_accum:  f32,
    frame_count_accum: u32,
    fps_display:       f32,

    // ── UI input (cursor + mouse state for immediate-mode hit tests).
    ui_cursor:         [f32; 2],
    ui_left_down:      bool,
    ui_left_just_pressed: bool,
    /// True when the most recent left-click was over a UI panel — the
    /// editor swallows the click so viewport-pick doesn't also fire.
    ui_consumed_click: bool,

    /// Active TRS-gizmo drag, set on click + cleared on release.
    gizmo_drag:        Option<GizmoDrag>,
}

impl EditorApp {
    fn spawn_light(&mut self, lk: LightKind) {
        let Some((lh, sh)) = self.main_stage else { return };
        let id = ActorId::new(self.actors.len() as i64 + 100);
        let Some(ah) = self.world.spawn_actor(lh, sh, id, Affine3A::IDENTITY) else { return };
        let utility_idx = ActorType::Utility(Utility {
            id: UtilityId::new(id.raw()), name: Arc::from(""), visible: true, toggle: true, mesh: None,
        }).index();
        let _ = self.world.spawn_sub_entity(lh, sh, ah,
            ActorType::Utility(Utility {
                id: UtilityId::new(id.raw()), name: Arc::from("Light"),
                visible: true, toggle: true, mesh: None,
            }),
            Affine3A::IDENTITY);
        self.world.add_component(lh, sh, ah, utility_idx,
            UtilityComponent {
                name: Arc::from("Light"), description: Arc::from(""),
                camera: None,
                light: Some(LightData { color: [1.0, 1.0, 1.0], intensity: 100.0, range: 0.0, kind: lk }),
                render: None,
            });
        self.actors.push(ah);
        self.world.selection = Some(ah);
    }

    fn spawn_empty(&mut self, name: &str) {
        let Some((lh, sh)) = self.main_stage else { return };
        let id = ActorId::new(self.actors.len() as i64 + 100);
        let Some(ah) = self.world.spawn_actor(lh, sh, id, Affine3A::IDENTITY) else { return };
        let _ = self.world.spawn_sub_entity(lh, sh, ah,
            ActorType::Utility(Utility {
                id: UtilityId::new(id.raw()), name: Arc::from(name),
                visible: true, toggle: true, mesh: None,
            }),
            Affine3A::IDENTITY);
        self.actors.push(ah);
        self.world.selection = Some(ah);
    }

    fn do_spawn_mesh(&mut self, asset: GltfHandle) {
        let Some((lh, sh)) = self.main_stage else { return };
        let id = ActorId::new(self.actors.len() as i64 + 200);
        let Some(ah) = self.world.spawn_actor(lh, sh, id, Affine3A::IDENTITY) else { return };
        let _ = self.world.spawn_sub_entity(lh, sh, ah,
            ActorType::Environment(Environment {
                id: EnvironmentId::new(id.raw()),
                name: Arc::from("mesh_actor"),
                visible: true, physical: false,
                mesh: Some(MeshRef { asset }),
            }),
            Affine3A::IDENTITY);
        self.actors.push(ah);
        self.world.selection = Some(ah);
    }

    fn pick_actor(
        &self,
        ctx: &AppCtx<'_>,
        win: AppHandle,
        cursor_px: (f32, f32),
    ) -> Option<ActorHandle> {
        let (lh, sh) = self.main_stage?;
        let grid = ctx.viewport_grid(win)?;
        let focused_h = grid.focused;
        let vp = grid.get(focused_h)?;
        let (win_w, win_h) = (grid.win_w, grid.win_h);

        let px = cursor_px.0 / win_w;
        let py = cursor_px.1 / win_h;
        if !vp.rect.contains(px, py, 1.0, 1.0) { return None; }
        let local_x = (px - vp.rect.x) / vp.rect.w;
        let local_y = (py - vp.rect.y) / vp.rect.h;
        let ndc = Vec4::new(local_x * 2.0 - 1.0, local_y * 2.0 - 1.0, -1.0, 1.0);

        let cam = ctx.cameras.get(vp.camera_handle)?;
        let aspect = vp.rect.pixel_aspect(win_w, win_h);
        let inv_vp = Mat4::from_cols_array(&cam.view_projection_matrix(aspect)).inverse();

        let near = inv_vp * ndc;
        let far  = inv_vp * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
        let near_w = near.xyz() / near.w;
        let far_w  = far.xyz()  / far.w;
        let ray_dir = (far_w - near_w).normalize();
        let is_ortho = matches!(cam.projection, Some(ProjectionMode::Orthographic { .. }));
        let ray_org = if is_ortho { near_w } else { Vec3::from_array(cam.position) };

        let stage = self.world.levels.get(lh)?.stages.get(sh)?;
        let mut best_t = f32::MAX;
        let mut best_h = None;

        for ah in &self.actors {
            let idx = ah.idx as usize;
            if idx >= stage.worlds.len() { continue; }
            let world_t = stage.worlds[idx];
            let center = Vec3::from(world_t.translation);
            let half = Vec3::splat(0.5);
            if let Some(t) = ray_aabb(ray_org, ray_dir, center - half, center + half) {
                if t < best_t { best_t = t; best_h = Some(*ah); }
            }
        }
        best_h
    }
}

fn ray_aabb(org: Vec3, dir: Vec3, mn: Vec3, mx: Vec3) -> Option<f32> {
    let inv = Vec3::new(
        if dir.x.abs() > 1e-9 { 1.0 / dir.x } else { f32::MAX },
        if dir.y.abs() > 1e-9 { 1.0 / dir.y } else { f32::MAX },
        if dir.z.abs() > 1e-9 { 1.0 / dir.z } else { f32::MAX },
    );
    let t0 = (mn - org) * inv;
    let t1 = (mx - org) * inv;
    let tmin = t0.min(t1);
    let tmax = t0.max(t1);
    let enter = tmin.max_element();
    let exit  = tmax.min_element();
    if exit >= enter && exit >= 0.0 { Some(enter.max(0.0)) } else { None }
}

impl AppLogic for EditorApp {
    fn on_start(&mut self, ctx: &mut AppCtx<'_>, ev: &ActiveEventLoop) -> ForgeResult<()> {
        let win = ctx.spawn_window(ev, "Editor", 1280, 720)?;
        self.win = Some(win);
        ctx.init_viewport_grid(win, ViewportLayout::FourQuadrant)?;

        let lh = self.world.spawn_level(LevelId::new(1), "editor");
        let sh = self.world.spawn_stage(lh, StageId::new(1), "scene").unwrap();
        self.main_level = Some(lh);
        self.main_stage = Some((lh, sh));

        // Default key-light.
        let light_offset = Affine3A::from_translation(Vec3::new(4.0, 4.0, 4.0));
        let light_ah = self.world.spawn_actor(lh, sh, ActorId::new(2), light_offset).unwrap();
        let utility_idx = ActorType::Utility(Utility {
            id: UtilityId::new(1), name: Arc::from(""), visible: true, toggle: true, mesh: None,
        }).index();
        self.world.spawn_sub_entity(lh, sh, light_ah,
            ActorType::Utility(Utility {
                id: UtilityId::new(1), name: Arc::from("key_light"),
                visible: true, toggle: true, mesh: None,
            }),
            Affine3A::IDENTITY).unwrap();
        self.world.add_component(lh, sh, light_ah, utility_idx,
            UtilityComponent {
                name: Arc::from("key_light"), description: Arc::from(""),
                camera: None,
                light: Some(LightData {
                    color: [1.0, 0.95, 0.85], intensity: 5.0, range: 30.0,
                    kind: LightKind::Point,
                }),
                render: None,
            });
        self.actors.push(light_ah);

        // Load CLI-specified glb paths.
        for path_str in self.asset_paths.clone() {
            let asset = ctx.load_gltf(win, PathBuf::from(path_str.as_ref()))?;
            let offset = Affine3A::IDENTITY;
            let ah = self.world.spawn_actor(lh, sh, ActorId::new(100 + self.actors.len() as i64), offset).unwrap();
            let env_id = EnvironmentId::new(self.actors.len() as i64);
            self.world.spawn_sub_entity(lh, sh, ah,
                ActorType::Environment(Environment {
                    id: env_id, name: Arc::clone(&path_str),
                    visible: true, physical: false,
                    mesh: Some(MeshRef { asset }),
                }),
                Affine3A::IDENTITY).unwrap();
            self.actors.push(ah);
        }

        self.picker_paths = collect_glb_paths("assets/models");

        Ok(())
    }

    fn handle_event(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::KeyboardInput { event: ke, .. } if ke.state == ElementState::Pressed => {
                match ke.physical_key {
                    PhysicalKey::Code(KeyCode::Tab) => {
                        if let Some(grid) = ctx.viewport_grid_mut(app) {
                            let next = grid.layout.next();
                            grid.set_layout(next, &[]);
                        }
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::KeyG) => { self.gizmo_mode = GizmoMode::Translate; return true; }
                    PhysicalKey::Code(KeyCode::KeyR) => { self.gizmo_mode = GizmoMode::Rotate;    return true; }
                    PhysicalKey::Code(KeyCode::KeyS) => { self.gizmo_mode = GizmoMode::Scale;     return true; }
                    PhysicalKey::Code(KeyCode::Delete) => {
                        if let (Some(ah), Some((lh, sh))) = (self.world.selection, self.main_stage) {
                            self.world.despawn_actor(lh, sh, ah);
                            self.actors.retain(|&h| h != ah);
                            self.world.selection = None;
                        }
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::Escape) => {
                        self.world.selection  = None;
                        self.spawn_menu_open  = false;
                        self.light_submenu_open = false;
                        self.picker_open      = false;
                    }
                    _ => {}
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.ui_cursor = [position.x as f32, position.y as f32];
                // While a gizmo drag is active, update the actor each move.
                if self.gizmo_drag.is_some() {
                    self.apply_gizmo_drag();
                }
            }

            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                match state {
                    ElementState::Pressed => {
                        self.ui_left_down         = true;
                        self.ui_left_just_pressed = true;
                        // Hit-test UI panels first.
                        let (win_w, _win_h) = ctx.viewport_grid(app)
                            .map(|g| (g.win_w, g.win_h))
                            .unwrap_or((1280.0, 720.0));
                        let in_toolbar  = self.ui_cursor[1] < 36.0;
                        let in_side     = self.ui_cursor[0] > win_w - 280.0;
                        let in_picker   = self.picker_open;
                        let in_spawn    = self.spawn_menu_open
                            && self.ui_cursor[0] > 200.0 && self.ui_cursor[0] < 360.0
                            && self.ui_cursor[1] > 36.0  && self.ui_cursor[1] < 236.0;
                        self.ui_consumed_click = in_toolbar || in_side || in_picker || in_spawn;
                        if !self.ui_consumed_click {
                            // Gizmo arrows take precedence over actor pick.
                            if let Some(drag) = self.start_gizmo_drag(ctx, app) {
                                self.gizmo_drag = Some(drag);
                                return true;
                            }
                            if let Some(win) = self.win {
                                if let Some(ah) = self.pick_actor(ctx, win, (self.ui_cursor[0], self.ui_cursor[1])) {
                                    self.world.selection = Some(ah);
                                    return true;
                                }
                            }
                        }
                    }
                    ElementState::Released => {
                        self.ui_left_down = false;
                        self.gizmo_drag   = None;
                    }
                }
            }

            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                self.world.selection = None;
            }

            _ => {}
        }
        false
    }

    fn update(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle, dt: f32) -> bool {
        let Some(win) = self.win else { return true };
        if win != app { return true; }

        self.frame_time_accum  += dt;
        self.frame_count_accum += 1;
        if self.frame_time_accum >= 0.5 {
            self.fps_display       = self.frame_count_accum as f32 / self.frame_time_accum;
            self.frame_time_accum  = 0.0;
            self.frame_count_accum = 0;
        }

        let _ = ctx.poll_gltf_loaders(app);
        self.world.propagate_transforms();

        if !self.cam_fitted {
            if let Some(aabb) = ctx.gltf_union_aabb_for_world(&self.world) {
                ctx.fit_all_panes_to_aabb(app, &aabb);
                self.cam_fitted = true;
            }
        }

        self.world.ui.draw_list.clear();
        self.draw_toolbar(ctx, app);
        self.draw_outliner(ctx, app);
        self.draw_inspector(ctx, app);
        self.draw_trs_gizmo(ctx, app);
        if self.picker_open { self.draw_file_picker(ctx, app); }
        // Frame is built — consume the per-frame click edge so it doesn't
        // re-fire next tick.
        self.ui_left_just_pressed = false;
        self.ui_consumed_click    = false;

        let elapsed = self.start.elapsed().as_secs_f32();
        match ctx.render_world(&self.world, app, elapsed) {
            Ok(Some(sem)) => ctx.push_compute_wait(app, sem),
            Ok(None)      => {}
            Err(e)        => eprintln!("render_world error: {e:?}"),
        }
        true
    }
}

// ── Toolbar ────────────────────────────────────────────────────────────────

impl EditorApp {
    fn ui_input(&self) -> dumpster_fire_engine::resource_manager::ui_manager::UiInputState {
        use dumpster_fire_engine::resource_manager::ui_manager::UiInputState;
        let mut s = UiInputState::default();
        s.cursor            = self.ui_cursor;
        s.left_down         = self.ui_left_down;
        s.left_just_pressed = self.ui_left_just_pressed;
        s
    }

    fn draw_toolbar(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::resource_manager::ui_manager::immediate::Ui;
        use dumpster_fire_engine::resource_manager::ui_manager::layout::Rect;

        let (win_w, _) = ctx.viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let layout_label = match ctx.viewport_grid(app).map(|g| g.layout) {
            Some(ViewportLayout::Single)       => "Single",
            Some(ViewportLayout::TwoColumns)   => "2 Col",
            Some(ViewportLayout::TwoRows)      => "2 Row",
            Some(ViewportLayout::FourQuadrant) | None => "4-Quad",
        };
        let tm_label = match self.world.tonemap_op {
            1 => "TM: Rh",
            2 => "TM: ACES",
            _ => "TM: Lin",
        };
        let fps_label = format!("FPS:{:.0}", self.fps_display);
        let t_col = if self.gizmo_mode == GizmoMode::Translate { [80,160,80,255u8] } else { [60,60,70,255] };
        let r_col = if self.gizmo_mode == GizmoMode::Rotate    { [80,160,80,255u8] } else { [60,60,70,255] };
        let s_col = if self.gizmo_mode == GizmoMode::Scale     { [80,160,80,255u8] } else { [60,60,70,255] };
        let sp_col = if self.gizmo_space == GizmoSpace::Local  { [100,100,200,255u8] } else { [60,60,70,255] };
        let input = self.ui_input();

        // Build draw list — mutable borrow of self.world ends at end of block.
        let (layout_clicked, frame_all_clicked, spawn_clicked, tonemap_clicked) = {
            let rect = Rect { x: 0.0, y: 0.0, w: win_w, h: 36.0 };
            let dl   = &mut self.world.ui.draw_list;
            dl.push_rect(rect.x, rect.y, rect.w, rect.h, [0.0, 0.0, 1.0, 1.0], [30, 30, 35, 255]);

            let mut ui = Ui::with_input(dl, Rect { x: 4.0, y: 4.0, w: win_w - 8.0, h: 28.0 }, input);
            let layout_c  = ui.button(layout_label);
            ui.checkbox("Grid", &mut self.grid_enabled);
            let frame_c   = ui.button("Frame All");
            let spawn_c   = ui.button("+ Actor");
            ui.draw.push_rect(ui.cursor[0],      ui.cursor[1], 26.0, 26.0, [0.0,0.0,1.0,1.0], t_col);
            ui.draw.push_rect(ui.cursor[0]+28.0, ui.cursor[1], 26.0, 26.0, [0.0,0.0,1.0,1.0], r_col);
            ui.draw.push_rect(ui.cursor[0]+56.0, ui.cursor[1], 26.0, 26.0, [0.0,0.0,1.0,1.0], s_col);
            ui.cursor[0] += 90.0;
            ui.draw.push_rect(ui.cursor[0], ui.cursor[1], 54.0, 26.0, [0.0,0.0,1.0,1.0], sp_col);
            ui.cursor[0] += 60.0;
            let tonemap_c = ui.button(tm_label);
            ui.label(&fps_label);
            (layout_c, frame_c, spawn_c, tonemap_c)
        };

        // Act on button clicks now that the draw_list borrow is released.
        if layout_clicked {
            if let Some(grid) = ctx.viewport_grid_mut(app) {
                let next = grid.layout.next();
                grid.set_layout(next, &[]);
            }
        }
        if frame_all_clicked {
            if let Some(aabb) = ctx.gltf_union_aabb_for_world(&self.world) {
                ctx.fit_all_panes_to_aabb(app, &aabb);
            }
        }
        if spawn_clicked {
            self.spawn_menu_open = !self.spawn_menu_open;
        }
        if tonemap_clicked {
            self.world.tonemap_op = (self.world.tonemap_op + 1) % 3;
        }

        if self.spawn_menu_open {
            let ox = 200.0_f32;
            let oy = 36.0_f32;
            let click_x = self.ui_cursor[0];
            let click_y = self.ui_cursor[1];
            let just_clicked = self.ui_left_just_pressed;
            let entries: &[&str] = &[
                "Mesh Actor…", "Light: Point", "Light: Spot", "Light: Directional",
                "Camera", "Empty", "Trigger Volume", "Audio Emitter",
            ];
            let mut chosen: Option<usize> = None;
            {
                let dl = &mut self.world.ui.draw_list;
                dl.push_rect(ox, oy, 160.0, 200.0, [0.0,0.0,1.0,1.0], [25, 25, 35, 245]);
                for (i, _label) in entries.iter().enumerate() {
                    let item_y = oy + i as f32 * 24.0 + 4.0;
                    let hovered = click_x >= ox + 4.0 && click_x < ox + 156.0
                                && click_y >= item_y && click_y < item_y + 20.0;
                    let bg = if hovered { [70, 70, 100, 255] } else { [45, 45, 60, 255] };
                    dl.push_rect(ox + 4.0, item_y, 152.0, 20.0, [0.0,0.0,1.0,1.0], bg);
                    if hovered && just_clicked { chosen = Some(i); }
                }
            }
            if let Some(idx) = chosen {
                self.spawn_menu_open = false;
                match idx {
                    0 => self.picker_open = true,
                    1 => self.spawn_light(LightKind::Point),
                    2 => self.spawn_light(LightKind::Spot {
                        cone_inner: 0.5, cone_outer: 0.8, direction: [0.0, -1.0, 0.0],
                    }),
                    3 => self.spawn_light(LightKind::Directional { direction: [0.0, -1.0, 0.0] }),
                    4 => self.spawn_empty("Camera"),
                    5 => self.spawn_empty("Empty"),
                    6 => self.spawn_empty("TriggerVolume"),
                    7 => self.spawn_empty("AudioEmitter"),
                    _ => {}
                }
            }
        }
    }

}

// ── Outliner ───────────────────────────────────────────────────────────────

impl EditorApp {
    fn draw_outliner(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::resource_manager::ui_manager::layout::Rect;

        let (win_w, win_h) = ctx.viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let panel_w = 280.0_f32;
        let panel_h = win_h * 0.45;
        let rect = Rect { x: win_w - panel_w, y: 36.0, w: panel_w, h: panel_h };
        let dl   = &mut self.world.ui.draw_list;

        dl.push_rect(rect.x, rect.y, rect.w, rect.h, [0.0,0.0,1.0,1.0], [22, 22, 28, 240]);
        dl.push_rect(rect.x, rect.y, rect.w, 22.0, [0.0,0.0,1.0,1.0], [35, 35, 50, 255]);

        let Some((lh, sh)) = self.main_stage else { return };
        let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh)) else { return };

        let mut row_y = rect.y + 26.0;
        let row_h     = 22.0;

        for (ah, actor) in stage.actors.entries() {
            if row_y + row_h > rect.y + rect.h { break; }
            let is_selected = self.world.selection == Some(ah);
            let bg = if is_selected { [60, 100, 160, 255] } else { [30, 30, 38, 255] };
            dl.push_rect(rect.x + 2.0, row_y, rect.w - 4.0, row_h - 2.0, [0.0,0.0,1.0,1.0], bg);
            let icon_col = actor_icon_color(actor);
            dl.push_rect(rect.x + 4.0, row_y + 4.0, 12.0, 12.0, [0.0,0.0,1.0,1.0], icon_col);
            row_y += row_h;
        }
    }
}

fn actor_icon_color(actor: &dumpster_fire_engine::resource_manager::manager::Actor) -> [u8; 4] {
    for se in actor.sub_entities.iter().flatten() {
        match &se.actor_type {
            ActorType::Utility(_) => {
                if let Some(Component::Utility(uc)) = &se.components[ComponentType::Utility.index()] {
                    if uc.light.is_some()  { return [230, 220, 80,  255]; }
                    if uc.camera.is_some() { return [80,  180, 230, 255]; }
                }
                return [140, 140, 150, 255];
            }
            ActorType::Environment(_) => return [80, 200, 100, 255],
            _ => {}
        }
    }
    [100, 100, 100, 255]
}

// ── Inspector ──────────────────────────────────────────────────────────────

impl EditorApp {
    fn draw_inspector(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::resource_manager::ui_manager::layout::Rect;
        use dumpster_fire_engine::resource_manager::ui_manager::immediate::Ui;

        let (win_w, win_h) = ctx.viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let Some((lh, sh)) = self.main_stage else { return };
        let Some(ah) = self.world.selection else { return };

        let panel_w = 280.0_f32;
        let panel_y = 36.0 + win_h * 0.45;
        let panel_h = win_h - panel_y;
        let rect    = Rect { x: win_w - panel_w, y: panel_y, w: panel_w, h: panel_h };

        let dl = &mut self.world.ui.draw_list;
        dl.push_rect(rect.x, rect.y, rect.w, rect.h, [0.0,0.0,1.0,1.0], [22, 22, 28, 240]);
        dl.push_rect(rect.x, rect.y, rect.w, 22.0, [0.0,0.0,1.0,1.0], [35, 35, 50, 255]);

        let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh)) else { return };
        let Some(actor) = stage.actors.get(ah) else { return };

        let mut has_transform  = false;
        let mut has_light      = false;
        let mut has_camera     = false;
        let mut is_env         = false;
        let mut light_kind_tag: Option<u32> = None;

        for se in actor.sub_entities.iter().flatten() {
            match &se.actor_type {
                ActorType::Environment(_) => { is_env = true; }
                ActorType::Utility(_) => {
                    if let Some(Component::Utility(uc)) = &se.components[ComponentType::Utility.index()] {
                        if uc.light.is_some()  {
                            has_light = true;
                            light_kind_tag = uc.light.as_ref().map(|l| l.kind.tag());
                        }
                        if uc.camera.is_some() { has_camera = true; }
                    }
                }
                _ => {}
            }
            has_transform = true;
        }

        let world_idx = ah.idx as usize;
        let world_t   = stage.worlds.get(world_idx).copied().unwrap_or(Affine3A::IDENTITY);
        let pos       = world_t.translation;

        let input = dumpster_fire_engine::resource_manager::ui_manager::UiInputState {
            cursor:             self.ui_cursor,
            cursor_prev:        self.ui_cursor,
            left_down:          self.ui_left_down,
            left_just_pressed:  self.ui_left_just_pressed,
            left_just_released: false,
            right_down:         false,
            mods:               Default::default(),
            scroll:             [0.0, 0.0],
            chars:              thin_vec::ThinVec::new(),
        };
        let mut ui = Ui::with_input(dl, Rect { x: rect.x + 4.0, y: rect.y + 26.0, w: rect.w - 8.0, h: rect.h - 30.0 }, input);

        if has_transform {
            ui.label("Transform");
            let mut px = pos.x; let mut py = pos.y; let mut pz = pos.z;
            ui.slider("X", &mut px, -100.0, 100.0);
            ui.slider("Y", &mut py, -100.0, 100.0);
            ui.slider("Z", &mut pz, -100.0, 100.0);
            ui.separator();
        }

        if is_env {
            ui.label("Mesh");
            ui.button("Replace…");
            ui.separator();
        }

        if has_light {
            ui.label("Light");
            let kind_name = light_kind_name(light_kind_tag.unwrap_or(0) as u8);
            ui.label(kind_name);
            let mut intensity = 100.0_f32;
            ui.slider("Intensity", &mut intensity, 0.0, 2000.0);
            let mut range = 0.0_f32;
            ui.slider("Range", &mut range, 0.0, 100.0);
            ui.separator();
        }

        if has_camera {
            ui.label("Camera");
            let mut focal = 50.0_f32;
            ui.slider("Focal mm", &mut focal, 14.0, 300.0);
            let mut fstop = 5.6_f32;
            ui.slider("f-stop", &mut fstop, 1.0, 22.0);
            let mut iso = 100.0_f32;
            ui.slider("ISO", &mut iso, 50.0, 12800.0);
            let mut focus = 5.0_f32;
            ui.slider("Focus dist", &mut focus, 0.1, 50.0);
            ui.separator();
        }

        ui.button("+ Add Component ▾");
    }
}

fn light_kind_name(tag: u8) -> &'static str {
    match tag {
        0  => "Point",        1  => "Spot",
        2  => "Directional",  3  => "Sun",
        4  => "Sphere",       5  => "Disk",
        6  => "Rectangle",    7  => "Polygon",
        8  => "Linear",       9  => "Tube",
        10 => "Volumetric",   11 => "VolumeBox",
        12 => "VolumeCone",   13 => "VolumeCylinder",
        14 => "VolumeMesh",   15 => "IES",
        16 => "Mesh",         17 => "Environment",
        18 => "AnalyticSky",  19 => "Ambient",
        _  => "Unknown",
    }
}

// ── File picker ────────────────────────────────────────────────────────────

impl EditorApp {
    fn draw_file_picker(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::resource_manager::ui_manager::layout::Rect;
        use dumpster_fire_engine::resource_manager::ui_manager::immediate::Ui;

        let (win_w, win_h) = ctx.viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let pw = 480.0_f32;
        let ph = 360.0_f32;
        let rect = Rect {
            x: (win_w - pw) * 0.5,
            y: (win_h - ph) * 0.5,
            w: pw, h: ph,
        };

        let dl = &mut self.world.ui.draw_list;
        dl.push_rect(0.0, 0.0, win_w, win_h, [0.0,0.0,1.0,1.0], [0, 0, 0, 140]);
        dl.push_rect(rect.x, rect.y, rect.w, rect.h, [0.0,0.0,1.0,1.0], [28, 28, 36, 255]);
        dl.push_rect(rect.x, rect.y, rect.w, 28.0, [0.0,0.0,1.0,1.0], [40, 40, 60, 255]);

        // Collect matching paths before borrowing draw list.
        let filter_lower: Arc<str> = Arc::from(self.picker_filter.to_lowercase().as_str());
        let filtered: ThinVec<Arc<str>> = self.picker_paths.iter()
            .filter(|p| filter_lower.is_empty() || p.to_lowercase().contains(filter_lower.as_ref()))
            .take(10)
            .cloned()
            .collect();

        let mut load_path: Option<Arc<str>> = None;
        let mut cancel = false;
        let input = self.ui_input();

        {
            let dl2 = &mut self.world.ui.draw_list;
            let mut ui = Ui::with_input(dl2, Rect { x: rect.x + 4.0, y: rect.y + 32.0, w: rect.w - 8.0, h: ph - 60.0 }, input.clone());
            ui.label("Select .glb file");

            for path in &filtered {
                let name = path.rsplit('/').next().unwrap_or(path.as_ref());
                let name = name.rsplit('\\').next().unwrap_or(name);
                if ui.button(name) {
                    load_path = Some(Arc::clone(path));
                }
            }

            let footer = Rect { x: rect.x + 4.0, y: rect.y + ph - 32.0, w: rect.w - 8.0, h: 28.0 };
            let mut u2 = Ui::with_input(dl2, footer, input);
            if u2.button("Cancel") { cancel = true; }
        }

        if let Some(path) = load_path {
            if let Some(win) = self.win {
                if let Ok(asset) = ctx.load_gltf(win, PathBuf::from(path.as_ref())) {
                    self.do_spawn_mesh(asset);
                }
            }
            self.picker_open = false;
        }
        if cancel { self.picker_open = false; }
    }
}

// ── TRS gizmo drag math ───────────────────────────────────────────────────

impl EditorApp {
    /// Hit-test the three axis arrows of the currently rendered gizmo.
    /// Returns a GizmoDrag if the cursor is within 10 px of an arrow.
    fn start_gizmo_drag(&self, ctx: &AppCtx<'_>, app: AppHandle) -> Option<GizmoDrag> {
        let ah = self.world.selection?;
        let (lh, sh) = self.main_stage?;
        let grid = ctx.viewport_grid(app)?;
        let vp = grid.get(grid.focused)?;
        let cam = ctx.cameras.get(vp.camera_handle)?;
        let stage = self.world.levels.get(lh)?.stages.get(sh)?;
        let world_t = *stage.worlds.get(ah.idx as usize)?;
        let local_t = *stage.locals.get(ah.idx as usize).unwrap_or(&world_t);
        let center = Vec3::from(world_t.translation);

        let (win_w, win_h) = (grid.win_w, grid.win_h);
        let aspect = vp.rect.pixel_aspect(win_w, win_h);
        let vp_mat = Mat4::from_cols_array(&cam.view_projection_matrix(aspect));
        let pane_x = vp.rect.x * win_w;
        let pane_y = vp.rect.y * win_h;
        let pane_w = vp.rect.w * win_w;
        let pane_h = vp.rect.h * win_h;
        let project = |p: Vec3| -> Option<[f32; 2]> {
            let clip = vp_mat * Vec4::new(p.x, p.y, p.z, 1.0);
            if clip.w <= 1e-5 { return None; }
            Some([
                pane_x + (clip.x / clip.w * 0.5 + 0.5) * pane_w,
                pane_y + (clip.y / clip.w * 0.5 + 0.5) * pane_h,
            ])
        };

        let cam_pos = Vec3::from_array(cam.position);
        let len = 0.15 * (cam_pos - center).length().max(0.5);
        let origin_s = project(center)?;
        let cursor = self.ui_cursor;

        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        let mut best: Option<(u8, [f32; 2])> = None;
        let mut best_dist = 12.0_f32;
        for (i, axis) in axes.iter().enumerate() {
            let tip = project(center + *axis * len)?;
            let dist = point_to_segment(cursor, origin_s, tip);
            if dist < best_dist {
                best_dist = dist;
                best = Some((i as u8, [tip[0] - origin_s[0], tip[1] - origin_s[1]]));
            }
        }
        let (axis_i, arrow_screen) = best?;
        Some(GizmoDrag {
            actor:           ah,
            mode:            self.gizmo_mode,
            axis:            axis_i,
            start_local:     local_t,
            start_world:     world_t,
            start_cursor:    cursor,
            arrow_screen,
            arrow_world_len: len,
        })
    }

    /// Apply per-frame delta from cursor motion to the dragged actor.
    fn apply_gizmo_drag(&mut self) {
        let Some(drag) = self.gizmo_drag else { return };
        let Some((lh, sh)) = self.main_stage else { return };
        let dx = self.ui_cursor[0] - drag.start_cursor[0];
        let dy = self.ui_cursor[1] - drag.start_cursor[1];
        let arrow_len_sq = drag.arrow_screen[0].powi(2) + drag.arrow_screen[1].powi(2);
        if arrow_len_sq < 1.0 { return; }
        // Scalar projection of mouse delta onto the screen-space arrow:
        // t = (dot(delta, arrow)) / |arrow|² → unitless [-N..+N]
        let t = (dx * drag.arrow_screen[0] + dy * drag.arrow_screen[1]) / arrow_len_sq;
        let axis = match drag.axis { 0 => Vec3::X, 1 => Vec3::Y, _ => Vec3::Z };
        let new_local = match drag.mode {
            GizmoMode::Translate => {
                let world_delta = axis * (t * drag.arrow_world_len);
                let mut nl = drag.start_local;
                nl.translation += glam::Vec3A::from(world_delta);
                nl
            }
            GizmoMode::Scale => {
                // 1.0 + t scales the chosen axis basis vector. Clamp at 0.01
                // so the matrix can't collapse.
                let factor = (1.0 + t).max(0.01);
                let mut nl = drag.start_local;
                let mut col = nl.matrix3.col(drag.axis as usize);
                col *= factor / col.length().max(1e-5) * (col.length() * factor);
                // Simpler: rebuild via diagonal scale of the chosen axis.
                let mut scale_vec = Vec3::ONE;
                scale_vec[drag.axis as usize] = factor;
                nl.matrix3 = drag.start_local.matrix3 * glam::Mat3A::from_diagonal(scale_vec);
                let _ = col;
                nl
            }
            GizmoMode::Rotate => {
                // Rotate around the chosen world axis by an angle proportional
                // to the cursor's tangential travel.
                let angle = t * std::f32::consts::PI;
                let rot = glam::Quat::from_axis_angle(axis, angle);
                let mut nl = drag.start_local;
                nl.matrix3 = glam::Mat3A::from_quat(rot) * drag.start_local.matrix3;
                nl
            }
        };
        self.world.set_actor_local(lh, sh, drag.actor, new_local);
    }
}

/// Perpendicular distance from a 2D point to a line segment.
fn point_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-5 {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len_sq).clamp(0.0, 1.0);
    let cx = a[0] + t * dx;
    let cy = a[1] + t * dy;
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}

// ── TRS gizmo (rendered in 2D screen space projected from focused pane) ───

impl EditorApp {
    fn draw_trs_gizmo(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        let Some(ah) = self.world.selection else { return };
        let Some((lh, sh)) = self.main_stage else { return };
        let Some(grid) = ctx.viewport_grid(app) else { return };
        let focused_h = grid.focused;
        let Some(vp) = grid.get(focused_h) else { return };
        let (win_w, win_h) = (grid.win_w, grid.win_h);
        let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh)) else { return };
        let world_t = match stage.worlds.get(ah.idx as usize) {
            Some(t) => *t,
            None    => return,
        };
        let center = Vec3::from(world_t.translation);

        let Some(cam) = ctx.cameras.get(vp.camera_handle) else { return };
        let aspect = vp.rect.pixel_aspect(win_w, win_h);
        let vp_mat = Mat4::from_cols_array(&cam.view_projection_matrix(aspect));

        // Compute pane pixel rect for NDC → screen conversion.
        let pane_x = vp.rect.x * win_w;
        let pane_y = vp.rect.y * win_h;
        let pane_w = vp.rect.w * win_w;
        let pane_h = vp.rect.h * win_h;

        // Project a world point into pane-screen pixels. Returns None when
        // the point is behind the near plane.
        let project = |p: Vec3| -> Option<[f32; 2]> {
            let clip = vp_mat * Vec4::new(p.x, p.y, p.z, 1.0);
            if clip.w <= 1e-5 { return None; }
            let ndc_x = clip.x / clip.w;
            let ndc_y = clip.y / clip.w;
            let sx = pane_x + (ndc_x * 0.5 + 0.5) * pane_w;
            let sy = pane_y + (ndc_y * 0.5 + 0.5) * pane_h;
            Some([sx, sy])
        };

        // Constant-screen-size by tying world length to camera distance.
        let cam_pos = Vec3::from_array(cam.position);
        let dist = (cam_pos - center).length().max(0.5);
        let len  = 0.15 * dist;

        let Some(origin_s) = project(center) else { return };
        let axes: [(Vec3, [u8; 4]); 3] = [
            (Vec3::X, [220,  60,  60, 255]),
            (Vec3::Y, [ 70, 200,  70, 255]),
            (Vec3::Z, [ 70, 100, 220, 255]),
        ];

        let dl = &mut self.world.ui.draw_list;

        match self.gizmo_mode {
            GizmoMode::Translate | GizmoMode::Scale => {
                for (axis, color) in axes {
                    let tip_world = center + axis * len;
                    if let Some(tip_s) = project(tip_world) {
                        dl.push_line(origin_s[0], origin_s[1], tip_s[0], tip_s[1], 2.5, color);
                        // Tip glyph: small box for both translate (arrowhead
                        // stand-in) and scale.
                        let sz = 8.0_f32;
                        dl.push_rect(tip_s[0] - sz * 0.5, tip_s[1] - sz * 0.5,
                                     sz, sz, [0.0, 0.0, 1.0, 1.0], color);
                    }
                }
                // Central yellow free-move / uniform-scale handle.
                let sz = 10.0_f32;
                dl.push_rect(origin_s[0] - sz * 0.5, origin_s[1] - sz * 0.5,
                             sz, sz, [0.0, 0.0, 1.0, 1.0], [230, 220, 70, 255]);
            }
            GizmoMode::Rotate => {
                // Three rings: one perpendicular to each axis. Sample 32
                // points; project each; connect with line segments.
                const N: usize = 32;
                let basis = [
                    (Vec3::Y, Vec3::Z, [220,  60,  60, 255]), // ring ⟂ X
                    (Vec3::X, Vec3::Z, [ 70, 200,  70, 255]), // ring ⟂ Y
                    (Vec3::X, Vec3::Y, [ 70, 100, 220, 255]), // ring ⟂ Z
                ];
                for (u, v, color) in basis {
                    let mut prev: Option<[f32; 2]> = None;
                    let mut first: Option<[f32; 2]> = None;
                    for i in 0..=N {
                        let t = (i as f32) / (N as f32) * std::f32::consts::TAU;
                        let p = center + (u * t.cos() + v * t.sin()) * len;
                        if let Some(s) = project(p) {
                            if let Some(prev_s) = prev {
                                dl.push_line(prev_s[0], prev_s[1], s[0], s[1], 1.8, color);
                            }
                            if first.is_none() { first = Some(s); }
                            prev = Some(s);
                        }
                    }
                    let _ = first;
                }
                // Center screen-space rotation circle (yellow dot).
                let sz = 8.0_f32;
                dl.push_rect(origin_s[0] - sz * 0.5, origin_s[1] - sz * 0.5,
                             sz, sz, [0.0, 0.0, 1.0, 1.0], [230, 220, 70, 255]);
            }
        }
    }
}

// ── Utilities ──────────────────────────────────────────────────────────────

fn collect_glb_paths(root: &str) -> ThinVec<Arc<str>> {
    let mut out: ThinVec<Arc<str>> = ThinVec::new();
    collect_glb_paths_into(root, &mut out);
    out.sort_unstable();
    out
}

fn collect_glb_paths_into(root: &str, out: &mut ThinVec<Arc<str>>) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(s) = p.to_str() {
                collect_glb_paths_into(s, out);
            }
        } else if p.extension().and_then(|e| e.to_str()) == Some("glb") {
            if let Some(s) = p.to_str() {
                out.push(Arc::from(s));
            }
        }
    }
}

// ── main ───────────────────────────────────────────────────────────────────

fn main() -> ForgeResult<()> {
    let paths: ThinVec<Arc<str>> = std::env::args()
        .skip(1)
        .map(|s| Arc::from(s.as_str()))
        .collect();
    let paths = if paths.is_empty() {
        ThinVec::from(["assets/models/BrainStem.glb"].map(Arc::from))
    } else {
        paths
    };
    AppRunner::new(EditorApp {
        asset_paths:        paths,
        world:              World::new(dumpster_fire_engine::resource_manager::WorldId::new(1)),
        main_level:         None,
        main_stage:         None,
        actors:             ThinVec::new(),
        cam_fitted:         false,
        start:              Instant::now(),
        win:                None,
        grid_enabled:       true,
        gizmo_mode:         GizmoMode::Translate,
        gizmo_space:        GizmoSpace::World,
        spawn_menu_open:    false,
        light_submenu_open: false,
        picker_open:        false,
        picker_filter:      Arc::from(""),
        picker_paths:       ThinVec::new(),
        outliner_filter:    Arc::from(""),
        frame_time_accum:   0.0,
        frame_count_accum:  0,
        fps_display:        0.0,
        ui_cursor:          [0.0, 0.0],
        ui_left_down:       false,
        ui_left_just_pressed: false,
        ui_consumed_click:  false,
        gizmo_drag:         None,
    }).run()
}
