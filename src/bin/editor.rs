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
use dumpster_fire_engine::render::app::{AppCtx, AppHandle, AppLogic, AppRunner, ViewportLayout};
use dumpster_fire_engine::render::camera::ProjectionMode;
use dumpster_fire_engine::resource_manager::component::{
    Component, ComponentType, GltfHandle, LightData, LightKind, MeshRef, UtilityComponent,
};
use dumpster_fire_engine::resource_manager::manager::ActorHandle;
use dumpster_fire_engine::resource_manager::ui_manager::{
    UiInputState,
    panel::{Panel, PanelHandle},
    widget::{CheckboxData, DropdownData, Widget, WidgetHandle},
};
use dumpster_fire_engine::resource_manager::{
    ActorId, ActorType, Environment, EnvironmentId, LevelHandle, LevelId, StageHandle, StageId,
    Utility, UtilityId, World,
};

// ── Layout constants ───────────────────────────────────────────────────────

const TOOLBAR_H: f32 = 28.0;
const TITLEBAR_H: f32 = 22.0;
const OUTLINER_W: f32 = 220.0;
const INSPECTOR_W: f32 = 260.0;
const SEP: [u8; 4] = [58, 58, 74, 255];
const PANEL_BG: [u8; 4] = [22, 22, 28, 240];
const TITLEBAR_BG: [u8; 4] = [35, 35, 45, 255];
const TOOLBAR_BG: [u8; 4] = [26, 26, 34, 255];

// ── Editor state ───────────────────────────────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[allow(dead_code)]
enum GizmoSpace {
    World,
    Local,
}

/// Active drag state captured at the click that started the gizmo grab.
#[derive(Copy, Clone, Debug)]
struct GizmoDrag {
    actor: ActorHandle,
    mode: GizmoMode,
    axis: u8,
    start_local: Affine3A,
    #[allow(dead_code)]
    start_world: Affine3A,
    start_cursor: [f32; 2],
    /// Screen-space arrow vector (tip - origin) for translate/scale projection.
    arrow_screen: [f32; 2],
    /// World-space gizmo arm length for translate/scale world-delta conversion.
    arrow_world_len: f32,
    /// Screen-space projected gizmo origin for rotate atan2 computation.
    ring_origin: [f32; 2],
}

struct EditorApp {
    asset_paths: ThinVec<Arc<str>>,
    world: World,
    main_level: Option<LevelHandle>,
    main_stage: Option<(LevelHandle, StageHandle)>,
    actors: ThinVec<ActorHandle>,
    cam_fitted: bool,
    start: Instant,
    win: Option<AppHandle>,

    // ── toolbar state
    grid_enabled: bool,
    gizmo_mode: GizmoMode,
    #[allow(dead_code)]
    gizmo_space: GizmoSpace,
    spawn_menu_open: bool,
    light_submenu_open: bool,

    // ── file picker
    picker_open: bool,
    picker_filter: Arc<str>,
    picker_paths: ThinVec<Arc<str>>,

    // ── outliner
    #[allow(dead_code)]
    outliner_filter: Arc<str>,
    outliner_scroll_y: f32,

    // ── inspector collapse state (retained between frames)
    insp_pos_collapsed: bool,
    insp_rot_collapsed: bool,
    insp_scl_collapsed: bool,
    insp_cmp_collapsed: bool,

    // ── FPS
    frame_time_accum: f32,
    frame_count_accum: u32,
    fps_display: f32,

    // ── UI input (cursor + mouse state for immediate-mode hit tests).
    ui_cursor: [f32; 2],
    ui_left_down: bool,
    ui_left_just_pressed: bool,
    /// True when the most recent left-click was over a UI panel.
    ui_consumed_click: bool,

    // ── retained panel/widget handles (initialized on first frame)
    ui_initialized: bool,
    toolbar_ph: Option<PanelHandle>,
    outliner_ph: Option<PanelHandle>,
    inspector_ph: Option<PanelHandle>,
    toolbar_grid_wh: Option<WidgetHandle>,
    toolbar_gizmo_wh: Option<WidgetHandle>,
    toolbar_tonemap_wh: Option<WidgetHandle>,

    /// Active TRS-gizmo drag, set on click + cleared on release.
    gizmo_drag: Option<GizmoDrag>,
}

impl EditorApp {
    fn spawn_light(&mut self, lk: LightKind) {
        let Some((lh, sh)) = self.main_stage else {
            return;
        };
        let id = ActorId::new(self.actors.len() as i64 + 100);
        let Some(ah) = self.world.spawn_actor(lh, sh, id, Affine3A::IDENTITY) else {
            return;
        };
        let utility_idx = ActorType::Utility(Utility {
            id: UtilityId::new(id.raw()),
            name: Arc::from(""),
            visible: true,
            toggle: true,
            mesh: None,
        })
        .index();
        let _ = self.world.spawn_sub_entity(
            lh,
            sh,
            ah,
            ActorType::Utility(Utility {
                id: UtilityId::new(id.raw()),
                name: Arc::from("Light"),
                visible: true,
                toggle: true,
                mesh: None,
            }),
            Affine3A::IDENTITY,
        );
        self.world.add_component(
            lh,
            sh,
            ah,
            utility_idx,
            UtilityComponent {
                name: Arc::from("Light"),
                description: Arc::from(""),
                camera: None,
                light: Some(LightData {
                    color: [1.0, 1.0, 1.0],
                    intensity: 100.0,
                    range: 0.0,
                    kind: lk,
                }),
                render: None,
            },
        );
        self.actors.push(ah);
        self.world.selection = Some(ah);
    }

    fn spawn_empty(&mut self, name: &str) {
        let Some((lh, sh)) = self.main_stage else {
            return;
        };
        let id = ActorId::new(self.actors.len() as i64 + 100);
        let Some(ah) = self.world.spawn_actor(lh, sh, id, Affine3A::IDENTITY) else {
            return;
        };
        let _ = self.world.spawn_sub_entity(
            lh,
            sh,
            ah,
            ActorType::Utility(Utility {
                id: UtilityId::new(id.raw()),
                name: Arc::from(name),
                visible: true,
                toggle: true,
                mesh: None,
            }),
            Affine3A::IDENTITY,
        );
        self.actors.push(ah);
        self.world.selection = Some(ah);
    }

    fn do_spawn_mesh(&mut self, asset: GltfHandle) {
        let Some((lh, sh)) = self.main_stage else {
            return;
        };
        let id = ActorId::new(self.actors.len() as i64 + 200);
        let Some(ah) = self.world.spawn_actor(lh, sh, id, Affine3A::IDENTITY) else {
            return;
        };
        let _ = self.world.spawn_sub_entity(
            lh,
            sh,
            ah,
            ActorType::Environment(Environment {
                id: EnvironmentId::new(id.raw()),
                name: Arc::from("mesh_actor"),
                visible: true,
                physical: false,
                mesh: Some(MeshRef { asset }),
            }),
            Affine3A::IDENTITY,
        );
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
        if !vp.rect.contains(px, py, 1.0, 1.0) {
            return None;
        }
        let local_x = (px - vp.rect.x) / vp.rect.w;
        let local_y = (py - vp.rect.y) / vp.rect.h;
        let ndc = Vec4::new(local_x * 2.0 - 1.0, local_y * 2.0 - 1.0, -1.0, 1.0);

        let cam = ctx.cameras.get(vp.camera_handle)?;
        let aspect = vp.rect.pixel_aspect(win_w, win_h);
        let inv_vp = Mat4::from_cols_array(&cam.view_projection_matrix(aspect)).inverse();

        let near = inv_vp * ndc;
        let far = inv_vp * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
        let near_w = near.xyz() / near.w;
        let far_w = far.xyz() / far.w;
        let ray_dir = (far_w - near_w).normalize();
        let is_ortho = matches!(cam.projection, Some(ProjectionMode::Orthographic { .. }));
        let ray_org = if is_ortho {
            near_w
        } else {
            Vec3::from_array(cam.position)
        };

        let stage = self.world.levels.get(lh)?.stages.get(sh)?;
        let mut best_t = f32::MAX;
        let mut best_h = None;

        for ah in &self.actors {
            let idx = ah.idx as usize;
            if idx >= stage.worlds.len() {
                continue;
            }
            let world_t = stage.worlds[idx];
            let center = Vec3::from(world_t.translation);
            let half = Vec3::splat(0.5);
            if let Some(t) = ray_aabb(ray_org, ray_dir, center - half, center + half)
                && t < best_t
            {
                best_t = t;
                best_h = Some(*ah);
            }
        }
        best_h
    }

    fn ui_input(&self) -> UiInputState {
        UiInputState {
            cursor: self.ui_cursor,
            left_down: self.ui_left_down,
            left_just_pressed: self.ui_left_just_pressed,
            ..UiInputState::default()
        }
    }

    /// Lazily initialize retained panels + widgets on the first rendered frame.
    fn init_ui_panels(&mut self, win_w: f32, win_h: f32) {
        if self.ui_initialized {
            return;
        }
        let panel_h = win_h - TOOLBAR_H;

        let tp = self.world.ui.spawn_panel(Panel::new(
            dumpster_fire_engine::resource_manager::ui_manager::layout::Rect {
                x: 0.0,
                y: 0.0,
                w: win_w,
                h: TOOLBAR_H,
            },
        ));
        let op = self.world.ui.spawn_panel(Panel::new(
            dumpster_fire_engine::resource_manager::ui_manager::layout::Rect {
                x: 0.0,
                y: TOOLBAR_H,
                w: OUTLINER_W,
                h: panel_h,
            },
        ));
        let ip = self.world.ui.spawn_panel(Panel::new(
            dumpster_fire_engine::resource_manager::ui_manager::layout::Rect {
                x: win_w - INSPECTOR_W,
                y: TOOLBAR_H,
                w: INSPECTOR_W,
                h: panel_h,
            },
        ));

        self.toolbar_ph = Some(tp);
        self.outliner_ph = Some(op);
        self.inspector_ph = Some(ip);

        let grid_wh = self
            .world
            .ui
            .widgets
            .insert(Widget::Checkbox(CheckboxData::new(self.grid_enabled)));
        let gizmo_wh = self
            .world
            .ui
            .widgets
            .insert(Widget::Dropdown(DropdownData::new(
                thin_vec::thin_vec![
                    Arc::from("Translate"),
                    Arc::from("Rotate"),
                    Arc::from("Scale"),
                ],
                0,
            )));
        let tonemap_wh = self
            .world
            .ui
            .widgets
            .insert(Widget::Dropdown(DropdownData::new(
                thin_vec::thin_vec![Arc::from("Lin"), Arc::from("Reinhard"), Arc::from("ACES"),],
                self.world.tonemap_op,
            )));

        if let Some(p) = self.world.ui.panels.get_mut(tp) {
            p.children.push(grid_wh);
            p.children.push(gizmo_wh);
            p.children.push(tonemap_wh);
        }
        self.toolbar_grid_wh = Some(grid_wh);
        self.toolbar_gizmo_wh = Some(gizmo_wh);
        self.toolbar_tonemap_wh = Some(tonemap_wh);
        self.ui_initialized = true;
    }
}

fn ray_aabb(org: Vec3, dir: Vec3, mn: Vec3, mx: Vec3) -> Option<f32> {
    let inv = Vec3::new(
        if dir.x.abs() > 1e-9 {
            1.0 / dir.x
        } else {
            f32::MAX
        },
        if dir.y.abs() > 1e-9 {
            1.0 / dir.y
        } else {
            f32::MAX
        },
        if dir.z.abs() > 1e-9 {
            1.0 / dir.z
        } else {
            f32::MAX
        },
    );
    let t0 = (mn - org) * inv;
    let t1 = (mx - org) * inv;
    let tmin = t0.min(t1);
    let tmax = t0.max(t1);
    let enter = tmin.max_element();
    let exit = tmax.min_element();
    if exit >= enter && exit >= 0.0 {
        Some(enter.max(0.0))
    } else {
        None
    }
}

impl AppLogic for EditorApp {
    fn on_start(&mut self, ctx: &mut AppCtx<'_>, ev: &ActiveEventLoop) -> ForgeResult<()> {
        let win = ctx.spawn_window(ev, "Editor", 1280, 720)?;
        self.win = Some(win);
        ctx.init_viewport_grid(win, ViewportLayout::FourQuadrant)?;

        let lh = self.world.spawn_level(LevelId::new(1), "editor");
        let sh = self
            .world
            .spawn_stage(lh, StageId::new(1), "scene")
            .unwrap();
        self.main_level = Some(lh);
        self.main_stage = Some((lh, sh));

        // Default key-light.
        let light_offset = Affine3A::from_translation(Vec3::new(4.0, 4.0, 4.0));
        let light_ah = self
            .world
            .spawn_actor(lh, sh, ActorId::new(2), light_offset)
            .unwrap();
        let utility_idx = ActorType::Utility(Utility {
            id: UtilityId::new(1),
            name: Arc::from(""),
            visible: true,
            toggle: true,
            mesh: None,
        })
        .index();
        self.world
            .spawn_sub_entity(
                lh,
                sh,
                light_ah,
                ActorType::Utility(Utility {
                    id: UtilityId::new(1),
                    name: Arc::from("key_light"),
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
            utility_idx,
            UtilityComponent {
                name: Arc::from("key_light"),
                description: Arc::from(""),
                camera: None,
                light: Some(LightData {
                    color: [1.0, 0.95, 0.85],
                    intensity: 5.0,
                    range: 30.0,
                    kind: LightKind::Point,
                }),
                render: None,
            },
        );
        self.actors.push(light_ah);

        // Load CLI-specified glb paths.
        for path_str in self.asset_paths.clone() {
            let asset = ctx.load_gltf(win, PathBuf::from(path_str.as_ref()))?;
            let offset = Affine3A::IDENTITY;
            let ah = self
                .world
                .spawn_actor(lh, sh, ActorId::new(100 + self.actors.len() as i64), offset)
                .unwrap();
            let env_id = EnvironmentId::new(self.actors.len() as i64);
            self.world
                .spawn_sub_entity(
                    lh,
                    sh,
                    ah,
                    ActorType::Environment(Environment {
                        id: env_id,
                        name: Arc::clone(&path_str),
                        visible: true,
                        physical: false,
                        mesh: Some(MeshRef { asset }),
                    }),
                    Affine3A::IDENTITY,
                )
                .unwrap();
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
                    PhysicalKey::Code(KeyCode::KeyG) => {
                        self.gizmo_mode = GizmoMode::Translate;
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::KeyR) => {
                        self.gizmo_mode = GizmoMode::Rotate;
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::KeyS) => {
                        self.gizmo_mode = GizmoMode::Scale;
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::Delete) => {
                        if let (Some(ah), Some((lh, sh))) = (self.world.selection, self.main_stage)
                        {
                            self.world.despawn_actor(lh, sh, ah);
                            self.actors.retain(|&h| h != ah);
                            self.world.selection = None;
                        }
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::Escape) => {
                        self.world.selection = None;
                        self.spawn_menu_open = false;
                        self.light_submenu_open = false;
                        self.picker_open = false;
                    }
                    _ => {}
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.ui_cursor = [position.x as f32, position.y as f32];
                if self.gizmo_drag.is_some() {
                    self.apply_gizmo_drag();
                }
            }

            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    self.ui_left_down = true;
                    self.ui_left_just_pressed = true;
                    let (win_w, _win_h) = ctx
                        .viewport_grid(app)
                        .map(|g| (g.win_w, g.win_h))
                        .unwrap_or((1280.0, 720.0));
                    let in_toolbar = self.ui_cursor[1] < TOOLBAR_H + 2.0;
                    let in_outliner =
                        self.ui_cursor[0] < OUTLINER_W && self.ui_cursor[1] >= TOOLBAR_H;
                    let in_inspector =
                        self.ui_cursor[0] >= win_w - INSPECTOR_W && self.ui_cursor[1] >= TOOLBAR_H;
                    let in_picker = self.picker_open;
                    let in_spawn = self.spawn_menu_open
                        && self.ui_cursor[0] > 200.0
                        && self.ui_cursor[0] < 370.0
                        && self.ui_cursor[1] >= TOOLBAR_H
                        && self.ui_cursor[1] < TOOLBAR_H + 220.0;
                    self.ui_consumed_click =
                        in_toolbar || in_outliner || in_inspector || in_picker || in_spawn;
                    if !self.ui_consumed_click {
                        if let Some(drag) = self.start_gizmo_drag(ctx, app) {
                            self.gizmo_drag = Some(drag);
                            return true;
                        }
                        if let Some(win) = self.win
                            && let Some(ah) =
                                self.pick_actor(ctx, win, (self.ui_cursor[0], self.ui_cursor[1]))
                        {
                            self.world.selection = Some(ah);
                            return true;
                        }
                    }
                }
                ElementState::Released => {
                    self.ui_left_down = false;
                    self.gizmo_drag = None;
                }
            },

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                self.world.selection = None;
            }

            _ => {}
        }
        false
    }

    fn update(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle, dt: f32) -> bool {
        let Some(win) = self.win else { return true };
        if win != app {
            return true;
        }

        let (win_w, win_h) = ctx
            .viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));

        if !self.ui_initialized {
            self.init_ui_panels(win_w, win_h);
        }

        self.frame_time_accum += dt;
        self.frame_count_accum += 1;
        if self.frame_time_accum >= 0.5 {
            self.fps_display = self.frame_count_accum as f32 / self.frame_time_accum;
            self.frame_time_accum = 0.0;
            self.frame_count_accum = 0;
        }

        let _ = ctx.poll_gltf_loaders(app);
        self.world.propagate_transforms();

        if !self.cam_fitted
            && let Some(aabb) = ctx.gltf_union_aabb_for_world(&self.world)
        {
            ctx.fit_all_panes_to_aabb(app, &aabb);
            self.cam_fitted = true;
        }

        self.world.ui.draw_list.clear();
        self.draw_toolbar(ctx, app);
        self.draw_outliner(ctx, app);
        self.draw_inspector(ctx, app);
        self.draw_trs_gizmo(ctx, app);
        if self.picker_open {
            self.draw_file_picker(ctx, app);
        }

        self.ui_left_just_pressed = false;
        self.ui_consumed_click = false;

        let elapsed = self.start.elapsed().as_secs_f32();
        match ctx.render_world(&self.world, app, elapsed) {
            Ok(Some(sem)) => ctx.push_compute_wait(app, sem),
            Ok(None) => {}
            Err(e) => eprintln!("render_world error: {e:?}"),
        }
        true
    }
}

// ── Toolbar ────────────────────────────────────────────────────────────────

impl EditorApp {
    fn draw_toolbar(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::resource_manager::ui_manager::{
            draw as uidraw, immediate::Ui, layout::Rect,
        };

        let (win_w, _) = ctx
            .viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let input = self.ui_input();

        let layout_label = match ctx.viewport_grid(app).map(|g| g.layout) {
            Some(ViewportLayout::Single) => "Single",
            Some(ViewportLayout::TwoColumns) => "2Col",
            Some(ViewportLayout::TwoRows) => "2Row",
            _ => "4Q",
        };
        let tm_label = match self.world.tonemap_op {
            1 => "Reinh",
            2 => "ACES",
            _ => "Lin",
        };
        let fps_str = format!("{:.0}fps", self.fps_display);
        let gizmo_mode = self.gizmo_mode;
        let mut grid_enabled = self.grid_enabled;

        let t_col = if gizmo_mode == GizmoMode::Translate {
            [70, 160, 80, 255u8]
        } else {
            [55, 55, 70, 255]
        };
        let r_col = if gizmo_mode == GizmoMode::Rotate {
            [70, 160, 80, 255u8]
        } else {
            [55, 55, 70, 255]
        };
        let s_col = if gizmo_mode == GizmoMode::Scale {
            [70, 160, 80, 255u8]
        } else {
            [55, 55, 70, 255]
        };

        let (lc, fc, sc, tc, tile_x, tile_y) = {
            let dl = &mut self.world.ui.draw_list;
            dl.push_panel_bg(0.0, 0.0, win_w, TOOLBAR_H, TOOLBAR_BG);
            dl.push_hsep(0.0, TOOLBAR_H, win_w, SEP);

            // Horizontal toolbar — cursor advances on X axis
            let mut ui = Ui::with_input(
                dl,
                Rect {
                    x: 4.0,
                    y: 4.0,
                    w: win_w - 8.0,
                    h: TOOLBAR_H - 4.0,
                },
                input.clone(),
            );

            let lc = ui.hbutton(layout_label, 52.0);
            ui.hcheckbox("Grid", &mut grid_enabled);
            let fc = ui.hbutton("Frame", 48.0);
            let sc = ui.hbutton("+Actor", 56.0);
            ui.hgap(6.0);

            // Gizmo T / R / S tiles
            let tile_x = ui.cursor[0];
            let tile_y = ui.cursor[1];
            ui.htile(22.0, 20.0, t_col);
            ui.htile(22.0, 20.0, r_col);
            ui.htile(22.0, 20.0, s_col);
            // Labels on tiles
            ui.draw.push_rect(
                tile_x + 2.0,
                tile_y + 2.0,
                8.0,
                16.0,
                dumpster_fire_engine::resource_manager::ui_manager::font::glyph_rect('T'),
                [210, 210, 220, 255],
            );
            ui.draw.push_rect(
                tile_x + 24.0,
                tile_y + 2.0,
                8.0,
                16.0,
                dumpster_fire_engine::resource_manager::ui_manager::font::glyph_rect('R'),
                [210, 210, 220, 255],
            );
            ui.draw.push_rect(
                tile_x + 46.0,
                tile_y + 2.0,
                8.0,
                16.0,
                dumpster_fire_engine::resource_manager::ui_manager::font::glyph_rect('S'),
                [210, 210, 220, 255],
            );
            ui.hgap(6.0);

            let tc = ui.hbutton(tm_label, 48.0);
            ui.hgap(8.0);
            // FPS as inline text (no button chrome)
            let fps_x = ui.cursor[0];
            let fps_y = ui.cursor[1];
            ui.text_at(fps_x, fps_y + 2.0, &fps_str, [130, 180, 130, 255]);

            (lc, fc, sc, tc, tile_x, tile_y)
        };

        self.grid_enabled = grid_enabled;

        // Gizmo tile click detection
        if self.ui_left_just_pressed {
            let cx = self.ui_cursor[0];
            let cy = self.ui_cursor[1];
            if cy >= tile_y && cy < tile_y + 20.0 {
                if cx >= tile_x && cx < tile_x + 22.0 {
                    self.gizmo_mode = GizmoMode::Translate;
                } else if cx >= tile_x + 22.0 && cx < tile_x + 44.0 {
                    self.gizmo_mode = GizmoMode::Rotate;
                } else if cx >= tile_x + 44.0 && cx < tile_x + 66.0 {
                    self.gizmo_mode = GizmoMode::Scale;
                }
            }
        }

        if lc && let Some(grid) = ctx.viewport_grid_mut(app) {
            let next = grid.layout.next();
            grid.set_layout(next, &[]);
        }
        if fc && let Some(aabb) = ctx.gltf_union_aabb_for_world(&self.world) {
            ctx.fit_all_panes_to_aabb(app, &aabb);
        }
        if sc {
            self.spawn_menu_open = !self.spawn_menu_open;
        }
        if tc {
            self.world.tonemap_op = (self.world.tonemap_op + 1) % 3;
        }

        // Spawn dropdown menu
        if self.spawn_menu_open {
            let ox = 200.0_f32;
            let oy = TOOLBAR_H;
            let entries: &[&str] = &[
                "Mesh Actor...",
                "Light: Point",
                "Light: Spot",
                "Light: Dir",
                "Camera",
                "Empty",
                "Trigger",
                "AudioEmitter",
            ];
            let mut chosen: Option<usize> = None;
            let cx = self.ui_cursor[0];
            let cy = self.ui_cursor[1];
            let jp = self.ui_left_just_pressed;
            {
                let dl = &mut self.world.ui.draw_list;
                let menu_h = entries.len() as f32 * 24.0 + 8.0;
                dl.push_panel_bg(ox, oy, 168.0, menu_h, [26, 26, 36, 248]);
                dl.push_line(ox, oy, ox + 168.0, oy, 1.0, SEP);
                dl.push_line(ox + 168.0, oy, ox + 168.0, oy + menu_h, 1.0, SEP);
                dl.push_line(ox, oy + menu_h, ox + 168.0, oy + menu_h, 1.0, SEP);
                for (i, entry) in entries.iter().enumerate() {
                    let iy = oy + i as f32 * 24.0 + 4.0;
                    let hov = cx >= ox + 4.0 && cx < ox + 164.0 && cy >= iy && cy < iy + 20.0;
                    let bg = if hov {
                        [70, 70, 105, 255]
                    } else {
                        [42, 42, 58, 255]
                    };
                    dl.push_rect(ox + 4.0, iy, 160.0, 20.0, uidraw::SOLID, bg);
                    // Entry label
                    let mut ex = ox + 8.0;
                    for c in entry.chars() {
                        let uv =
                            dumpster_fire_engine::resource_manager::ui_manager::font::glyph_rect(c);
                        if c != ' ' && uv != [0.0_f32; 4] {
                            dl.push_rect(ex, iy + 2.0, 8.0, 16.0, uv, [200, 200, 210, 255]);
                        }
                        ex += 8.0;
                    }
                    if hov && jp {
                        chosen = Some(i);
                    }
                }
            }
            if let Some(idx) = chosen {
                self.spawn_menu_open = false;
                match idx {
                    0 => self.picker_open = true,
                    1 => self.spawn_light(LightKind::Point),
                    2 => self.spawn_light(LightKind::Spot {
                        cone_inner: 0.5,
                        cone_outer: 0.8,
                        direction: [0.0, -1.0, 0.0],
                    }),
                    3 => self.spawn_light(LightKind::Directional {
                        direction: [0.0, -1.0, 0.0],
                    }),
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
        let (_, win_h) = ctx
            .viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let panel_h = win_h - TOOLBAR_H;
        let x = 0.0_f32;
        let y = TOOLBAR_H;
        let content_y = y + TITLEBAR_H + 2.0;
        let content_h = panel_h - TITLEBAR_H - 2.0;
        let row_h = 22.0_f32;
        let input = self.ui_input();

        // Collect actor rows before taking draw_list borrow.
        // Each row: (handle, icon_color, is_selected, hovered, clicked)
        let mut rows: ThinVec<(ActorHandle, [u8; 4], bool, bool, bool)> = ThinVec::new();
        let mut clicked_ah: Option<ActorHandle> = None;
        if let Some((lh, sh)) = self.main_stage
            && let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh))
        {
            let max_scroll = (stage.actors.len() as f32 * row_h - content_h).max(0.0);
            self.outliner_scroll_y = self.outliner_scroll_y.clamp(0.0, max_scroll);
            let mut row_y = content_y - self.outliner_scroll_y;
            for (ah, actor) in stage.actors.entries() {
                if row_y + row_h < content_y {
                    row_y += row_h;
                    continue;
                }
                if row_y > y + panel_h {
                    break;
                }
                let is_sel = self.world.selection == Some(ah);
                let hovered = input.cursor[0] >= x
                    && input.cursor[0] < x + OUTLINER_W
                    && input.cursor[1] >= row_y
                    && input.cursor[1] < row_y + row_h;
                let clicked = hovered && input.left_just_pressed;
                if clicked {
                    clicked_ah = Some(ah);
                }
                rows.push((ah, actor_icon_color(actor), is_sel, hovered, clicked));
                row_y += row_h;
            }
        }
        if let Some(ah) = clicked_ah {
            self.world.selection = Some(ah);
        }

        // Now borrow draw_list and render.
        let dl = &mut self.world.ui.draw_list;
        dl.push_panel_bg(x, y, OUTLINER_W, panel_h, PANEL_BG);
        dl.push_title_bar(x, y, OUTLINER_W, TITLEBAR_H, TITLEBAR_BG, SEP);
        dl.push_vsep(x + OUTLINER_W, y, panel_h, SEP);

        let mut row_y = content_y;
        if let Some((lh, sh)) = self.main_stage
            && let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh))
        {
            row_y -= self.outliner_scroll_y;
            let _ = stage; // borrow ends before we iterate rows below
        }

        use dumpster_fire_engine::resource_manager::ui_manager::{draw as uidraw, font};
        for (ah, icon_col, is_sel, hovered, _) in &rows {
            let bg = if *is_sel {
                [55, 95, 155, 255]
            } else if *hovered {
                [38, 38, 52, 255]
            } else if (ah.idx & 1) == 0 {
                [26, 26, 33, 255]
            } else {
                [24, 24, 30, 255]
            };
            dl.push_rect(x, row_y, OUTLINER_W, row_h - 1.0, uidraw::SOLID, bg);
            dl.push_rect(x + 6.0, row_y + 4.0, 14.0, 14.0, uidraw::SOLID, *icon_col);
            // Actor index as readable text
            let label = format!("Actor {}", ah.idx);
            let tc: [u8; 4] = if *is_sel {
                [200, 220, 255, 220]
            } else {
                [155, 155, 175, 200]
            };
            let mut lx = x + 26.0;
            for c in label.chars() {
                if lx + 8.0 > x + OUTLINER_W - 4.0 {
                    break;
                }
                if c != ' ' {
                    let uv = font::glyph_rect(c);
                    if uv != [0.0_f32; 4] {
                        dl.push_rect(lx, row_y + 3.0, 8.0, 16.0, uv, tc);
                    }
                }
                lx += 8.0;
            }
            row_y += row_h;
        }
    }
}

fn actor_icon_color(actor: &dumpster_fire_engine::resource_manager::manager::Actor) -> [u8; 4] {
    for se in actor.sub_entities.iter().flatten() {
        match &se.actor_type {
            ActorType::Utility(_) => {
                if let Some(Component::Utility(uc)) = &se.components[ComponentType::Utility.index()]
                {
                    if uc.light.is_some() {
                        return [230, 220, 80, 255];
                    }
                    if uc.camera.is_some() {
                        return [80, 180, 230, 255];
                    }
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
        use dumpster_fire_engine::resource_manager::ui_manager::{immediate::Ui, layout::Rect};

        let (win_w, win_h) = ctx
            .viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let panel_h = win_h - TOOLBAR_H;
        let ix = win_w - INSPECTOR_W;
        let iy = TOOLBAR_H;

        // Helper: draw empty panel chrome and return
        macro_rules! draw_empty {
            () => {{
                let dl = &mut self.world.ui.draw_list;
                dl.push_panel_bg(ix, iy, INSPECTOR_W, panel_h, PANEL_BG);
                dl.push_title_bar(ix, iy, INSPECTOR_W, TITLEBAR_H, TITLEBAR_BG, SEP);
                dl.push_vsep(ix, iy, panel_h, SEP);
                return;
            }};
        }

        let input = self.ui_input();
        let Some((lh, sh)) = self.main_stage else {
            draw_empty!()
        };
        let Some(ah) = self.world.selection else {
            draw_empty!()
        };

        // Collect actor data in a scoped read block — releases borrows before dl borrow.
        let actor_data = {
            let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh)) else {
                draw_empty!()
            };
            let Some(actor) = stage.actors.get(ah) else {
                draw_empty!()
            };
            let world_t = stage
                .worlds
                .get(ah.idx as usize)
                .copied()
                .unwrap_or(Affine3A::IDENTITY);
            let pos = world_t.translation;
            let mut has_light = false;
            let mut has_camera = false;
            let mut is_env = false;
            let mut light_kind_tag: Option<u32> = None;
            for se in actor.sub_entities.iter().flatten() {
                match &se.actor_type {
                    ActorType::Environment(_) => {
                        is_env = true;
                    }
                    ActorType::Utility(_) => {
                        if let Some(Component::Utility(uc)) =
                            &se.components[ComponentType::Utility.index()]
                        {
                            if uc.light.is_some() {
                                has_light = true;
                                light_kind_tag = uc.light.as_ref().map(|l| l.kind.tag());
                            }
                            if uc.camera.is_some() {
                                has_camera = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            (pos, has_light, has_camera, is_env, light_kind_tag)
        };
        let (pos, has_light, has_camera, is_env, light_kind_tag) = actor_data;

        // Mutable slider values — drawn this frame, written back after dl borrow.
        let mut px = pos.x;
        let mut py = pos.y;
        let mut pz = pos.z;
        let mut rx = 0.0_f32;
        let mut ry = 0.0_f32;
        let mut rz = 0.0_f32;
        let mut sx = 1.0_f32;
        let mut sy = 1.0_f32;
        let mut sz = 1.0_f32;

        {
            let content_y = iy + TITLEBAR_H + 4.0;
            let content_w = INSPECTOR_W - 8.0;
            let dl = &mut self.world.ui.draw_list;
            dl.push_panel_bg(ix, iy, INSPECTOR_W, panel_h, PANEL_BG);
            dl.push_title_bar(ix, iy, INSPECTOR_W, TITLEBAR_H, TITLEBAR_BG, SEP);
            dl.push_vsep(ix, iy, panel_h, SEP);
            let mut ui = Ui::with_input(
                dl,
                Rect {
                    x: ix + 4.0,
                    y: content_y,
                    w: content_w,
                    h: panel_h - TITLEBAR_H - 8.0,
                },
                input,
            );

            ui.section_header("TRANSFORM");
            if ui.collapsible_header("Position", &mut self.insp_pos_collapsed) {
                ui.slider("X", &mut px, -500.0, 500.0);
                ui.slider("Y", &mut py, -500.0, 500.0);
                ui.slider("Z", &mut pz, -500.0, 500.0);
            }
            if ui.collapsible_header("Rotation", &mut self.insp_rot_collapsed) {
                ui.slider("RX", &mut rx, -180.0, 180.0);
                ui.slider("RY", &mut ry, -180.0, 180.0);
                ui.slider("RZ", &mut rz, -180.0, 180.0);
            }
            if ui.collapsible_header("Scale", &mut self.insp_scl_collapsed) {
                ui.slider("SX", &mut sx, 0.01, 10.0);
                ui.slider("SY", &mut sy, 0.01, 10.0);
                ui.slider("SZ", &mut sz, 0.01, 10.0);
            }
            ui.separator();

            if is_env {
                ui.section_header("MESH");
                ui.button("Replace Asset...");
                ui.separator();
            }
            if has_light {
                ui.section_header("LIGHT");
                ui.label(light_kind_name(light_kind_tag.unwrap_or(0) as u8));
                let mut intensity = 100.0_f32;
                let mut range = 0.0_f32;
                ui.slider("Intensity", &mut intensity, 0.0, 2000.0);
                ui.slider("Range", &mut range, 0.0, 100.0);
                ui.separator();
            }
            if has_camera {
                ui.section_header("CAMERA");
                let mut focal = 50.0_f32;
                let mut fstop = 5.6_f32;
                let mut iso = 100.0_f32;
                let mut focus = 5.0_f32;
                ui.slider("Focal mm", &mut focal, 14.0, 300.0);
                ui.slider("f-stop", &mut fstop, 1.0, 22.0);
                ui.slider("ISO", &mut iso, 50.0, 12800.0);
                ui.slider("Focus m", &mut focus, 0.1, 50.0);
                ui.separator();
            }
            if ui.collapsible_header("Components", &mut self.insp_cmp_collapsed) {
                ui.button("+ Add Component");
            }
        } // dl borrow released here

        // Write slider values back to the actor's world transform.
        if let Some(stage) = self
            .world
            .levels
            .get_mut(lh)
            .and_then(|l| l.stages.get_mut(sh))
            && let Some(t) = stage.worlds.get_mut(ah.idx as usize)
        {
            t.translation = glam::Vec3A::new(px, py, pz);
            let quat = glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                rx.to_radians(),
                ry.to_radians(),
                rz.to_radians(),
            );
            t.matrix3 = glam::Mat3A::from_quat(quat)
                * glam::Mat3A::from_diagonal(glam::Vec3::new(sx, sy, sz));
        }
    }
}

fn light_kind_name(tag: u8) -> &'static str {
    match tag {
        0 => "Point",
        1 => "Spot",
        2 => "Directional",
        3 => "Sun",
        4 => "Sphere",
        5 => "Disk",
        6 => "Rectangle",
        7 => "Polygon",
        8 => "Linear",
        9 => "Tube",
        10 => "Volumetric",
        11 => "VolumeBox",
        12 => "VolumeCone",
        13 => "VolumeCylinder",
        14 => "VolumeMesh",
        15 => "IES",
        16 => "Mesh",
        17 => "Environment",
        18 => "AnalyticSky",
        19 => "Ambient",
        _ => "Unknown",
    }
}

// ── File picker ────────────────────────────────────────────────────────────

impl EditorApp {
    fn draw_file_picker(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::resource_manager::ui_manager::{immediate::Ui, layout::Rect};

        let (win_w, win_h) = ctx
            .viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let pw = 480.0_f32;
        let ph = 380.0_f32;
        let rx = (win_w - pw) * 0.5;
        let ry = (win_h - ph) * 0.5;
        let input = self.ui_input();

        let filter_lower: Arc<str> = Arc::from(self.picker_filter.to_lowercase().as_str());
        let filtered: ThinVec<Arc<str>> = self
            .picker_paths
            .iter()
            .filter(|p| filter_lower.is_empty() || p.to_lowercase().contains(filter_lower.as_ref()))
            .take(12)
            .cloned()
            .collect();

        let mut load_path: Option<Arc<str>> = None;
        let mut cancel = false;

        {
            let dl = &mut self.world.ui.draw_list;
            dl.push_rect(
                0.0,
                0.0,
                win_w,
                win_h,
                dumpster_fire_engine::resource_manager::ui_manager::draw::SOLID,
                [0, 0, 0, 145],
            );
            dl.push_panel_bg(rx, ry, pw, ph, [28, 28, 38, 255]);
            dl.push_title_bar(rx, ry, pw, TITLEBAR_H, [40, 40, 58, 255], SEP);
            dl.push_line(rx, ry, rx + pw, ry, 1.5, [78, 78, 100, 255]);
            dl.push_line(rx, ry + ph, rx + pw, ry + ph, 1.5, [78, 78, 100, 255]);
            dl.push_line(rx, ry, rx, ry + ph, 1.5, [78, 78, 100, 255]);
            dl.push_line(rx + pw, ry, rx + pw, ry + ph, 1.5, [78, 78, 100, 255]);

            let mut ui = Ui::with_input(
                dl,
                Rect {
                    x: rx + 4.0,
                    y: ry + TITLEBAR_H + 4.0,
                    w: pw - 8.0,
                    h: ph - TITLEBAR_H - 36.0,
                },
                input.clone(),
            );
            ui.label("Select .glb file");
            for path in &filtered {
                let name = path.rsplit('/').next().unwrap_or(path.as_ref());
                let name = name.rsplit('\\').next().unwrap_or(name);
                if ui.button(name) {
                    load_path = Some(Arc::clone(path));
                }
            }

            let mut u2 = Ui::with_input(
                dl,
                Rect {
                    x: rx + 4.0,
                    y: ry + ph - 30.0,
                    w: pw - 8.0,
                    h: 26.0,
                },
                input,
            );
            if u2.button("Cancel") {
                cancel = true;
            }
        }

        if let Some(path) = load_path {
            if let Some(win) = self.win
                && let Ok(asset) = ctx.load_gltf(win, PathBuf::from(path.as_ref()))
            {
                self.do_spawn_mesh(asset);
            }
            self.picker_open = false;
        }
        if cancel {
            self.picker_open = false;
        }
    }
}

// ── TRS gizmo drag math ───────────────────────────────────────────────────

impl EditorApp {
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
            if clip.w <= 1e-5 {
                return None;
            }
            Some([
                pane_x + (clip.x / clip.w * 0.5 + 0.5) * pane_w,
                pane_y + (clip.y / clip.w * 0.5 + 0.5) * pane_h,
            ])
        };

        let cam_pos = Vec3::from_array(cam.position);
        let len = 0.15 * (cam_pos - center).length().max(0.5);
        let origin_s = project(center)?;
        let cursor = self.ui_cursor;

        let mut best: Option<(u8, [f32; 2])> = None;
        let mut best_dist = 22.0_f32;

        if matches!(self.gizmo_mode, GizmoMode::Rotate) {
            // Hit-test ring circles: sample 32 points on each projected ring.
            let ring_bases: [(Vec3, Vec3, u8); 3] = [
                (Vec3::Y, Vec3::Z, 0),
                (Vec3::X, Vec3::Z, 1),
                (Vec3::X, Vec3::Y, 2),
            ];
            for (u, v, axis_i) in ring_bases {
                for j in 0..32usize {
                    let t = (j as f32) / 32.0 * std::f32::consts::TAU;
                    let p = center + (u * t.cos() + v * t.sin()) * len;
                    let Some(s) = project(p) else { continue };
                    let dist = ((s[0] - cursor[0]).powi(2) + (s[1] - cursor[1]).powi(2)).sqrt();
                    if dist < best_dist {
                        best_dist = dist;
                        best = Some((axis_i, [1.0, 0.0]));
                    }
                }
            }
        } else {
            // Hit-test axis arrow lines for Translate/Scale.
            for (i, axis) in [Vec3::X, Vec3::Y, Vec3::Z].iter().enumerate() {
                let Some(tip) = project(center + *axis * len) else {
                    continue;
                };
                let dist = point_to_segment(cursor, origin_s, tip);
                if dist < best_dist {
                    best_dist = dist;
                    best = Some((i as u8, [tip[0] - origin_s[0], tip[1] - origin_s[1]]));
                }
            }
        }

        let (axis_i, arrow_screen) = best?;
        Some(GizmoDrag {
            actor: ah,
            mode: self.gizmo_mode,
            axis: axis_i,
            start_local: local_t,
            start_world: world_t,
            start_cursor: cursor,
            arrow_screen,
            arrow_world_len: len,
            ring_origin: origin_s,
        })
    }

    fn apply_gizmo_drag(&mut self) {
        let Some(drag) = self.gizmo_drag else { return };
        let Some((lh, sh)) = self.main_stage else {
            return;
        };
        let cursor = self.ui_cursor;
        let dx = cursor[0] - drag.start_cursor[0];
        let dy = cursor[1] - drag.start_cursor[1];
        let axis = match drag.axis {
            0 => Vec3::X,
            1 => Vec3::Y,
            _ => Vec3::Z,
        };

        let new_local = match drag.mode {
            GizmoMode::Translate => {
                let arrow_len_sq = drag.arrow_screen[0].powi(2) + drag.arrow_screen[1].powi(2);
                if arrow_len_sq < 1.0 {
                    return;
                }
                let t = (dx * drag.arrow_screen[0] + dy * drag.arrow_screen[1]) / arrow_len_sq;
                let world_delta = axis * (t * drag.arrow_world_len);
                let mut nl = drag.start_local;
                nl.translation += glam::Vec3A::from(world_delta);
                nl
            }
            GizmoMode::Scale => {
                let arrow_len_sq = drag.arrow_screen[0].powi(2) + drag.arrow_screen[1].powi(2);
                if arrow_len_sq < 1.0 {
                    return;
                }
                let t = (dx * drag.arrow_screen[0] + dy * drag.arrow_screen[1]) / arrow_len_sq;
                let factor = (1.0 + t).max(0.01);
                let mut nl = drag.start_local;
                let mut scale_vec = Vec3::ONE;
                scale_vec[drag.axis as usize] = factor;
                nl.matrix3 = drag.start_local.matrix3 * glam::Mat3A::from_diagonal(scale_vec);
                nl
            }
            GizmoMode::Rotate => {
                // Atan2-based: angle = difference of cursor-angle around ring center.
                let cx = drag.ring_origin[0];
                let cy = drag.ring_origin[1];
                let start_a = (drag.start_cursor[1] - cy).atan2(drag.start_cursor[0] - cx);
                let cur_a = (cursor[1] - cy).atan2(cursor[0] - cx);
                let angle = cur_a - start_a;
                let rot = glam::Quat::from_axis_angle(axis, angle);
                let mut nl = drag.start_local;
                nl.matrix3 = glam::Mat3A::from_quat(rot) * drag.start_local.matrix3;
                nl
            }
        };
        self.world.set_actor_local(lh, sh, drag.actor, new_local);
    }
}

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

// ── TRS gizmo rendering ────────────────────────────────────────────────────

impl EditorApp {
    fn draw_trs_gizmo(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        let Some(ah) = self.world.selection else {
            return;
        };
        let Some((lh, sh)) = self.main_stage else {
            return;
        };
        let Some(grid) = ctx.viewport_grid(app) else {
            return;
        };
        let focused_h = grid.focused;
        let Some(vp) = grid.get(focused_h) else {
            return;
        };
        let (win_w, win_h) = (grid.win_w, grid.win_h);
        let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh)) else {
            return;
        };
        let world_t = match stage.worlds.get(ah.idx as usize) {
            Some(t) => *t,
            None => return,
        };
        let center = Vec3::from(world_t.translation);

        let Some(cam) = ctx.cameras.get(vp.camera_handle) else {
            return;
        };
        let aspect = vp.rect.pixel_aspect(win_w, win_h);
        let vp_mat = Mat4::from_cols_array(&cam.view_projection_matrix(aspect));

        let pane_x = vp.rect.x * win_w;
        let pane_y = vp.rect.y * win_h;
        let pane_w = vp.rect.w * win_w;
        let pane_h = vp.rect.h * win_h;

        let project = |p: Vec3| -> Option<[f32; 2]> {
            let clip = vp_mat * Vec4::new(p.x, p.y, p.z, 1.0);
            if clip.w <= 1e-5 {
                return None;
            }
            let ndc_x = clip.x / clip.w;
            let ndc_y = clip.y / clip.w;
            let sx = pane_x + (ndc_x * 0.5 + 0.5) * pane_w;
            let sy = pane_y + (ndc_y * 0.5 + 0.5) * pane_h;
            Some([sx, sy])
        };

        let cam_pos = Vec3::from_array(cam.position);
        let dist = (cam_pos - center).length().max(0.5);
        let len = 0.15 * dist;

        let Some(origin_s) = project(center) else {
            return;
        };
        let axes: [(Vec3, [u8; 4]); 3] = [
            (Vec3::X, [220, 60, 60, 255]),
            (Vec3::Y, [70, 200, 70, 255]),
            (Vec3::Z, [70, 100, 220, 255]),
        ];

        let dl = &mut self.world.ui.draw_list;

        match self.gizmo_mode {
            GizmoMode::Translate | GizmoMode::Scale => {
                for (axis, color) in axes {
                    let tip_world = center + axis * len;
                    if let Some(tip_s) = project(tip_world) {
                        dl.push_line(origin_s[0], origin_s[1], tip_s[0], tip_s[1], 4.0, color);
                        let sz = 14.0_f32;
                        dl.push_rect(
                            tip_s[0] - sz * 0.5,
                            tip_s[1] - sz * 0.5,
                            sz,
                            sz,
                            [0.0, 0.0, 1.0, 1.0],
                            color,
                        );
                    }
                }
                let sz = 14.0_f32;
                dl.push_rect(
                    origin_s[0] - sz * 0.5,
                    origin_s[1] - sz * 0.5,
                    sz,
                    sz,
                    [0.0, 0.0, 1.0, 1.0],
                    [230, 220, 70, 255],
                );
            }
            GizmoMode::Rotate => {
                const N: usize = 32;
                let basis = [
                    (Vec3::Y, Vec3::Z, [220, 60, 60, 255u8]),
                    (Vec3::X, Vec3::Z, [70, 200, 70, 255]),
                    (Vec3::X, Vec3::Y, [70, 100, 220, 255]),
                ];
                for (u, v, color) in basis {
                    let mut prev: Option<[f32; 2]> = None;
                    let mut first: Option<[f32; 2]> = None;
                    for i in 0..=N {
                        let t = (i as f32) / (N as f32) * std::f32::consts::TAU;
                        let p = center + (u * t.cos() + v * t.sin()) * len;
                        if let Some(s) = project(p) {
                            if let Some(prev_s) = prev {
                                dl.push_line(prev_s[0], prev_s[1], s[0], s[1], 3.5, color);
                            }
                            if first.is_none() {
                                first = Some(s);
                            }
                            prev = Some(s);
                        }
                    }
                    let _ = first;
                }
                let sz = 12.0_f32;
                dl.push_rect(
                    origin_s[0] - sz * 0.5,
                    origin_s[1] - sz * 0.5,
                    sz,
                    sz,
                    [0.0, 0.0, 1.0, 1.0],
                    [230, 220, 70, 255],
                );
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
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(s) = p.to_str() {
                collect_glb_paths_into(s, out);
            }
        } else if p.extension().and_then(|e| e.to_str()) == Some("glb")
            && let Some(s) = p.to_str()
        {
            out.push(Arc::from(s));
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
        asset_paths: paths,
        world: World::new(dumpster_fire_engine::resource_manager::WorldId::new(1)),
        main_level: None,
        main_stage: None,
        actors: ThinVec::new(),
        cam_fitted: false,
        start: Instant::now(),
        win: None,
        grid_enabled: true,
        gizmo_mode: GizmoMode::Translate,
        gizmo_space: GizmoSpace::World,
        spawn_menu_open: false,
        light_submenu_open: false,
        picker_open: false,
        picker_filter: Arc::from(""),
        picker_paths: ThinVec::new(),
        outliner_filter: Arc::from(""),
        outliner_scroll_y: 0.0,
        insp_pos_collapsed: false,
        insp_rot_collapsed: false,
        insp_scl_collapsed: false,
        insp_cmp_collapsed: true,
        frame_time_accum: 0.0,
        frame_count_accum: 0,
        fps_display: 0.0,
        ui_cursor: [0.0, 0.0],
        ui_left_down: false,
        ui_left_just_pressed: false,
        ui_consumed_click: false,
        ui_initialized: false,
        toolbar_ph: None,
        outliner_ph: None,
        inspector_ph: None,
        toolbar_grid_wh: None,
        toolbar_gizmo_wh: None,
        toolbar_tonemap_wh: None,
        gizmo_drag: None,
    })
    .run()
}
