//! Multiview real-time editor — Unreal-style quad-split viewport.
//!
//! Layout: Perspective (top-left) | OrthoTop (top-right)
//!         OrthoFront (bottom-left) | OrthoRight (bottom-right)
//!
//! Controls:
//!   Tab         — cycle viewport layout
//!   G / R / S   — Translate / Rotate / Scale gizmo mode
//!   F           — frame (focus cameras on) the selected actor
//!   X           — toggle gizmo snapping (grid / angle / scale steps)
//!   Ctrl+D      — duplicate the selected actor
//!   F2          — toggle the stats overlay
//!   E           — toggle mesh edit mode for the selected mesh
//!   1 / 2 / 3   — (edit mode) vertex / edge / face element select
//!   Ctrl+Z      — (edit mode) undo;  Ctrl+Shift+Z — redo
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
use dumpster_fire_engine::render::ui_editor::mesh_edit::{EditSession, ElementMode};
use forge_gltf::asset::GltfAsset;
use forge_gltf::mesh::PrimitiveTopology;
use std::path::Path as FsPath;
use dumpster_fire_engine::resource_manager::{
    ActorId, ActorType, Environment, EnvironmentId, LevelHandle, LevelId, StageHandle, StageId,
    Utility, UtilityId, World,
};

// ── Layout constants ───────────────────────────────────────────────────────

const MENUBAR_H: f32 = 22.0;
const TOOLBAR_H: f32 = 50.0; // menu row (0..MENUBAR_H) + icon row
const TITLEBAR_H: f32 = 22.0;
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

    // ── panel sizing (draggable dividers)
    outliner_w: f32,
    inspector_w: f32,
    div_drag: Option<u8>, // 0 = left divider, 1 = right divider
    div_hover: Option<u8>,

    // ── toolbar state
    grid_enabled: bool,
    gizmo_mode: GizmoMode,
    #[allow(dead_code)]
    gizmo_space: GizmoSpace,
    spawn_menu_open: bool,
    light_submenu_open: bool,

    // ── tools: gizmo snapping (X toggles)
    snap_enabled: bool,
    snap_translate: f32,
    snap_rotate_deg: f32,
    snap_scale: f32,
    // ── tools: stats overlay (F2 toggles)
    stats_open: bool,
    // ── modifier tracking (for Ctrl+D duplicate)
    ctrl_held: bool,

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

    // ── mesh edit mode (None = object mode)
    edit: Option<EditSession>,
    edit_drag: Option<EditDrag>,
    shift_held: bool,

    /// Tooltip deferred from a hovered toolbar icon, drawn last (always on top).
    pending_tooltip: Option<(f32, f32, String)>,

    /// Open menu-bar dropdown (index into MENUS) and its screen rect.
    menu_open: Option<usize>,
    menu_rect: (f32, f32, f32, f32),
    /// Output-log panel height (drag the top edge to resize).
    bottom_h: f32,
    quit_requested: bool,
    /// Output log lines: (text, color). Newest appended at the end.
    log: ThinVec<(String, [u8; 4])>,
    /// Console line buffer + keyboard focus (UE-style ` console in the log).
    console_input: String,
    console_focus: bool,
    /// In-place outliner rename: target actor + edit buffer + double-click
    /// detection (row handle + when it was last clicked).
    rename_target: Option<ActorHandle>,
    rename_buf: String,
    last_row_click: Option<(ActorHandle, std::time::Instant)>,
    /// Mesh-edit operator parameters (drag-field editable in the panel).
    extrude_offset: f32,
    inset_amount: f32,
    /// Edit-mode box (rubber-band) select: press point + whether the drag has
    /// exceeded the click threshold this gesture.
    box_sel_start: Option<[f32; 2]>,
    box_sel_active: bool,
    /// Active inspector drag-field: (key, value at press, cursor x at press).
    insp_drag: Option<(u64, f32, f32)>,
    /// Bottom panel tab: 0 = Output Log, 1 = Content Browser.
    bottom_tab: usize,
    /// Layout saved by pane-maximize (F11); restored on the next toggle.
    saved_layout: Option<ViewportLayout>,
}

/// Menu-bar definition: label + entries. Entry actions are routed in
/// `draw_menus` by (menu_idx, entry_idx).
const MENUS: [(&str, &[&str]); 4] = [
    ("File", &["Open glTF...", "Save Scene", "Load Scene", "Quit"]),
    ("Edit", &["Undo", "Redo", "Duplicate", "Delete"]),
    (
        "Window",
        &[
            "Layout: Single",
            "Layout: 2 Cols",
            "Layout: 2 Rows",
            "Layout: Quad",
            "Toggle Stats",
            "Maximize Pane",
            "Toggle Sky",
        ],
    ),
    ("Help", &["About"]),
];

fn menu_label_x(idx: usize) -> f32 {
    let mut x = 8.0;
    for (label, _) in MENUS.iter().take(idx) {
        x += label.chars().count() as f32 * 8.0 + 18.0;
    }
    x
}

/// Click results of the icon toolbar, applied after the draw-list borrow ends.
#[derive(Default)]
struct ToolbarActions {
    spawn: bool,
    object_mode: bool,
    edit_mode: bool,
    t: bool,
    r: bool,
    s: bool,
    snap: bool,
    grid: bool,
    dup: bool,
    del: bool,
    undo: bool,
    redo: bool,
    frame: bool,
    layout: bool,
    tonemap: bool,
    stats: bool,
    rt: bool,
}

/// Active edit-mode element translate-drag — a gizmo arrow grabbed at the
/// current selection centroid.
#[derive(Copy, Clone, Debug)]
struct EditDrag {
    axis: u8,
    start_cursor: [f32; 2],
    arrow_screen: [f32; 2],
    arrow_world_len: f32,
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

    fn do_spawn_mesh(&mut self, asset: GltfHandle, path: Arc<str>) {
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
                // Store the source path as the name so edit mode can reload geometry.
                name: path,
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

    /// World-space AABB of an actor: the transformed mesh rest AABB when a
    /// glTF asset is attached, else a unit box around the actor's position
    /// (matches the pick_actor fallback).
    fn actor_world_aabb(
        &self,
        ctx: &AppCtx<'_>,
        ah: ActorHandle,
    ) -> Option<([f32; 3], [f32; 3])> {
        use dumpster_fire_engine::render::gltf_assets::transform_aabb;
        let (lh, sh) = self.main_stage?;
        let stage = self.world.levels.get(lh)?.stages.get(sh)?;
        let actor = stage.actors.get(ah)?;
        let wa = *stage.worlds.get(ah.idx as usize)?;
        for se in actor.sub_entities.iter().flatten() {
            let (_, mesh_opt) = se.actor_type.visibility_and_mesh();
            if let Some(mr) = mesh_opt
                && let Some(loaded) = ctx.gltf_assets.get(mr.asset)
            {
                return Some(transform_aabb(&loaded.rest_aabb, &wa));
            }
        }
        let c = wa.translation;
        Some((
            [c.x - 0.5, c.y - 0.5, c.z - 0.5],
            [c.x + 0.5, c.y + 0.5, c.z + 0.5],
        ))
    }

    /// Unreal-style selection brackets: project the selected actor's AABB
    /// into the focused pane and draw orange corner brackets around its
    /// screen-space bounds.
    fn draw_selection_brackets(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        let Some(ah) = self.world.selection else {
            return;
        };
        let Some((mn, mx)) = self.actor_world_aabb(ctx, ah) else {
            return;
        };
        let Some(grid) = ctx.viewport_grid(app) else {
            return;
        };
        let Some(vp) = grid.get(grid.focused) else {
            return;
        };
        let (win_w, win_h) = (grid.win_w, grid.win_h);
        let aspect = vp.rect.pixel_aspect(win_w, win_h);
        let Some(cam) = ctx.cameras.get(vp.camera_handle) else {
            return;
        };
        let vpm = Mat4::from_cols_array(&cam.view_projection_matrix(aspect));
        let (px, py) = (vp.rect.x * win_w, vp.rect.y * win_h);
        let (pw, ph) = (vp.rect.w * win_w, vp.rect.h * win_h);

        let mut smin = [f32::MAX; 2];
        let mut smax = [f32::MIN; 2];
        for i in 0..8u32 {
            let c = Vec4::new(
                if i & 1 == 0 { mn[0] } else { mx[0] },
                if i & 2 == 0 { mn[1] } else { mx[1] },
                if i & 4 == 0 { mn[2] } else { mx[2] },
                1.0,
            );
            let h = vpm * c;
            if h.w <= 1e-6 {
                return; // corner behind the eye — skip brackets this frame
            }
            let ndc = [h.x / h.w, h.y / h.w];
            let sx = px + (ndc[0] * 0.5 + 0.5) * pw;
            let sy = py + (ndc[1] * 0.5 + 0.5) * ph;
            smin[0] = smin[0].min(sx);
            smin[1] = smin[1].min(sy);
            smax[0] = smax[0].max(sx);
            smax[1] = smax[1].max(sy);
        }
        // Clamp to the pane and skip degenerate rects.
        smin[0] = smin[0].max(px);
        smin[1] = smin[1].max(py.max(TOOLBAR_H));
        smax[0] = smax[0].min(px + pw);
        smax[1] = smax[1].min(py + ph);
        if smax[0] - smin[0] < 4.0 || smax[1] - smin[1] < 4.0 {
            return;
        }
        let arm = ((smax[0] - smin[0]).min(smax[1] - smin[1]) * 0.25).clamp(6.0, 22.0);
        let col = [255, 160, 60, 230u8];
        let dl = &mut self.world.ui.draw_list;
        for (cx, cy, dx, dy) in [
            (smin[0], smin[1], 1.0_f32, 1.0_f32),
            (smax[0], smin[1], -1.0, 1.0),
            (smin[0], smax[1], 1.0, -1.0),
            (smax[0], smax[1], -1.0, -1.0),
        ] {
            dl.push_line(cx, cy, cx + dx * arm, cy, 1.5, col);
            dl.push_line(cx, cy, cx, cy + dy * arm, 1.5, col);
        }
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
        let panel_h = win_h - TOOLBAR_H - self.bottom_h;

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
                w: self.outliner_w,
                h: panel_h,
            },
        ));
        let ip = self.world.ui.spawn_panel(Panel::new(
            dumpster_fire_engine::resource_manager::ui_manager::layout::Rect {
                x: win_w - self.inspector_w,
                y: TOOLBAR_H,
                w: self.inspector_w,
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
                // Console / rename own the keyboard while focused — editor
                // hotkeys and the camera must not fire while typing.
                if self.console_focus || self.rename_target.is_some() {
                    let renaming = self.rename_target.is_some();
                    match ke.physical_key {
                        PhysicalKey::Code(KeyCode::Enter) | PhysicalKey::Code(KeyCode::NumpadEnter) => {
                            if renaming {
                                self.commit_rename();
                            } else {
                                let cmd = std::mem::take(&mut self.console_input);
                                if !cmd.trim().is_empty() {
                                    self.execute_console(ctx, app, cmd.trim());
                                }
                            }
                        }
                        PhysicalKey::Code(KeyCode::Escape) => {
                            if renaming {
                                self.rename_target = None;
                                self.rename_buf.clear();
                            } else {
                                self.console_focus = false;
                                self.console_input.clear();
                            }
                        }
                        PhysicalKey::Code(KeyCode::Backspace) => {
                            if renaming {
                                self.rename_buf.pop();
                            } else {
                                self.console_input.pop();
                            }
                        }
                        _ => {
                            if let Some(t) = ke.text.as_ref() {
                                let buf = if renaming {
                                    &mut self.rename_buf
                                } else {
                                    &mut self.console_input
                                };
                                for c in t.chars() {
                                    if !c.is_control() && buf.len() < 120 {
                                        buf.push(c);
                                    }
                                }
                            }
                        }
                    }
                    return true;
                }
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
                    PhysicalKey::Code(KeyCode::KeyF) => {
                        self.frame_selected(ctx, app);
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::KeyX) if self.edit.is_none() => {
                        self.snap_enabled = !self.snap_enabled;
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::KeyL) => {
                        if let Some(mode) = ctx.toggle_lighting_mode(app) {
                            println!("editor: lighting mode → {mode:?}");
                        }
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::F2) => {
                        self.stats_open = !self.stats_open;
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::F11) => {
                        self.toggle_maximize_pane(ctx, app);
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::KeyD) if self.ctrl_held => {
                        self.duplicate_selected();
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::KeyE) => {
                        // Blender semantics: in edit mode E extrudes the
                        // selected region; in object mode E enters edit mode.
                        if let Some(e) = self.edit.as_mut() {
                            let off = self.extrude_offset;
                            if e.extrude_selected(off) {
                                self.push_log("LogMesh: extruded region.", [140, 200, 140, 255]);
                            }
                        } else {
                            self.toggle_edit_mode();
                        }
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::KeyI) => {
                        if let Some(e) = self.edit.as_mut() {
                            let am = self.inset_amount;
                            if e.inset_selected(am) {
                                self.push_log("LogMesh: inset faces.", [140, 200, 140, 255]);
                            }
                            return true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyN) => {
                        if let Some(e) = self.edit.as_mut() {
                            e.recalc_normals();
                            self.push_log("LogMesh: recalculated normals.", [140, 200, 140, 255]);
                            return true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyA) => {
                        if let Some(e) = self.edit.as_mut() {
                            if e.selected_vertex_count() > 0 {
                                e.select_none();
                            } else {
                                e.select_all();
                            }
                            return true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::Delete) | PhysicalKey::Code(KeyCode::KeyX)
                        if self.edit.is_some() =>
                    {
                        if let Some(e) = self.edit.as_mut() {
                            e.delete_selected();
                        }
                        return true;
                    }
                    PhysicalKey::Code(KeyCode::Digit1) => {
                        if let Some(e) = self.edit.as_mut() {
                            e.mode = ElementMode::Vertex;
                            return true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::Digit2) => {
                        if let Some(e) = self.edit.as_mut() {
                            e.mode = ElementMode::Edge;
                            return true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::Digit3) => {
                        if let Some(e) = self.edit.as_mut() {
                            e.mode = ElementMode::Face;
                            return true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyZ) if self.ctrl_held => {
                        if let Some(e) = self.edit.as_mut() {
                            if self.shift_held {
                                e.redo();
                            } else {
                                e.undo();
                            }
                            return true;
                        }
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
                        if self.edit.is_some() {
                            self.edit = None;
                            self.edit_drag = None;
                            return true;
                        }
                        self.world.selection = None;
                        self.spawn_menu_open = false;
                        self.light_submenu_open = false;
                        self.picker_open = false;
                        self.edit_drag = None;
                        if let Some(e) = self.edit.as_mut() {
                            e.clear_selection();
                        }
                    }
                    _ => {}
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.ctrl_held = mods.state().control_key();
                self.shift_held = mods.state().shift_key();
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.ui_cursor = [position.x as f32, position.y as f32];
                let cx = self.ui_cursor[0];
                let cy = self.ui_cursor[1];
                let (win_w, win_h) = ctx
                    .viewport_grid(app)
                    .map(|g| (g.win_w, g.win_h))
                    .unwrap_or((1280.0, 720.0));
                let left_div_x = self.outliner_w;
                let right_div_x = win_w - self.inspector_w;
                let bottom_div_y = win_h - self.bottom_h;
                if cy >= TOOLBAR_H {
                    if (cx - left_div_x).abs() < 5.0 {
                        self.div_hover = Some(0);
                    } else if (cx - right_div_x).abs() < 5.0 {
                        self.div_hover = Some(1);
                    } else if (cy - bottom_div_y).abs() < 5.0
                        && cx >= left_div_x
                        && cx < right_div_x
                    {
                        self.div_hover = Some(2);
                    } else {
                        self.div_hover = None;
                    }
                } else {
                    self.div_hover = None;
                }
                if let Some(side) = self.div_drag {
                    match side {
                        0 => self.outliner_w = cx.clamp(80.0, 600.0),
                        1 => self.inspector_w = (win_w - cx).clamp(80.0, 600.0),
                        _ => self.bottom_h = (win_h - cy).clamp(60.0, 420.0),
                    }
                }
                if self.gizmo_drag.is_some() {
                    self.apply_gizmo_drag();
                }
                if self.edit_drag.is_some() {
                    self.apply_edit_drag();
                }
                if let Some(start) = self.box_sel_start {
                    let dx = self.ui_cursor[0] - start[0];
                    let dy = self.ui_cursor[1] - start[1];
                    if dx * dx + dy * dy > 25.0 {
                        self.box_sel_active = true;
                    }
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
                    if self.div_hover.is_some() {
                        self.div_drag = self.div_hover;
                        return true;
                    }
                    let in_toolbar = self.ui_cursor[1] < TOOLBAR_H + 2.0;
                    let in_outliner =
                        self.ui_cursor[0] < self.outliner_w && self.ui_cursor[1] >= TOOLBAR_H;
                    let in_inspector = self.ui_cursor[0] >= win_w - self.inspector_w
                        && self.ui_cursor[1] >= TOOLBAR_H;
                    let in_picker = self.picker_open;
                    let in_bottom = {
                        let win_h = ctx
                            .viewport_grid(app)
                            .map(|g| g.win_h)
                            .unwrap_or(720.0);
                        self.ui_cursor[1] >= win_h - self.bottom_h
                    };
                    let in_menu = self.menu_open.is_some() && {
                        let (mx, my, mw, mh) = self.menu_rect;
                        self.ui_cursor[0] >= mx
                            && self.ui_cursor[0] < mx + mw
                            && self.ui_cursor[1] >= my
                            && self.ui_cursor[1] < my + mh
                    };
                    let in_spawn = self.spawn_menu_open
                        && self.ui_cursor[0] > 200.0
                        && self.ui_cursor[0] < 370.0
                        && self.ui_cursor[1] >= TOOLBAR_H
                        && self.ui_cursor[1] < TOOLBAR_H + 220.0;
                    self.ui_consumed_click = in_toolbar
                        || in_outliner
                        || in_inspector
                        || in_picker
                        || in_spawn
                        || in_menu
                        || in_bottom;
                    if self.ui_consumed_click {
                        // UI chrome owns this click — it must never fall
                        // through to the app's camera grab toggle, which
                        // would silently arm FPS-look on a menu click.
                        return true;
                    }
                    {
                        // Edit mode owns clicks in the viewport: grab a gizmo arrow
                        // to translate the element selection, else pick an element.
                        if self.edit.is_some() {
                            if let Some(d) = self.start_edit_drag(ctx, app) {
                                self.edit_drag = Some(d);
                                return true;
                            }
                            // Begin a click-or-box gesture; resolved on release
                            // (a small movement is a pick, a drag is box-select).
                            self.box_sel_start = Some(self.ui_cursor);
                            self.box_sel_active = false;
                            return true;
                        }
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
                    if self.edit_drag.take().is_some() {
                        if let Some(e) = self.edit.as_mut() {
                            e.commit_transform();
                        }
                    } else if let Some(start) = self.box_sel_start.take() {
                        if self.box_sel_active {
                            self.box_select(ctx, app, start, self.ui_cursor);
                        } else {
                            self.edit_pick(ctx, app);
                        }
                        self.box_sel_active = false;
                    }
                    self.gizmo_drag = None;
                    self.div_drag = None;
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
        self.draw_viewport_chrome(ctx, app);
        self.draw_selection_brackets(ctx, app);
        // 3D-projected overlays (gizmo, edit wireframe) draw before the side
        // panels so panel chrome covers any out-of-pane spill.
        self.draw_trs_gizmo(ctx, app);
        self.draw_edit_overlay(ctx, app);
        self.draw_outliner(ctx, app);
        self.draw_inspector(ctx, app);
        self.draw_bottom_panel(ctx, app);
        self.draw_stats(win_w);
        if self.picker_open {
            self.draw_file_picker(ctx, app);
        }
        self.draw_menus(ctx, app);
        // Deferred toolbar tooltip — drawn after every panel so it sits on top.
        if let Some((tx, ty, text)) = self.pending_tooltip.take() {
            dumpster_fire_engine::resource_manager::ui_manager::immediate::Ui::draw_tooltip(
                &mut self.world.ui.draw_list,
                tx,
                ty,
                &text,
            );
        }

        self.ui_left_just_pressed = false;
        self.ui_consumed_click = false;

        let elapsed = self.start.elapsed().as_secs_f32();
        match ctx.render_world(&self.world, app, elapsed) {
            Ok(Some(sem)) => ctx.push_compute_wait(app, sem),
            Ok(None) => {}
            Err(e) => eprintln!("render_world error: {e:?}"),
        }
        !self.quit_requested
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
        let in_edit = self.edit.is_some();
        let snap = self.snap_enabled;
        let stats = self.stats_open;
        let grid_on = self.grid_enabled;
        let rt_on = matches!(
            ctx.lighting_mode(app),
            Some(dumpster_fire_engine::render::window::LightingMode::RayTraced)
        );

        let menu_open = self.menu_open;
        let (a, tip, menu_toggle) = {
            let dl = &mut self.world.ui.draw_list;
            dl.push_panel_bg(0.0, 0.0, win_w, TOOLBAR_H, TOOLBAR_BG);
            dl.push_hsep(0.0, MENUBAR_H, win_w, [44, 44, 58, 255]);
            dl.push_hsep(0.0, TOOLBAR_H, win_w, SEP);

            // Menu bar row (File / Edit / Window / Help)
            let mut menu_toggle: Option<usize> = None;
            {
                use dumpster_fire_engine::resource_manager::ui_manager::font;
                let cx = input.cursor[0];
                let cy = input.cursor[1];
                for (i, (label, _)) in MENUS.iter().enumerate() {
                    let lx = menu_label_x(i);
                    let lw = label.chars().count() as f32 * 8.0 + 14.0;
                    let hovered = cy < MENUBAR_H && cx >= lx - 7.0 && cx < lx - 7.0 + lw;
                    let open = menu_open == Some(i);
                    if open || hovered {
                        let bg = if open {
                            [52, 70, 110, 255]
                        } else {
                            [44, 44, 60, 255]
                        };
                        let solid = dumpster_fire_engine::resource_manager::ui_manager::draw::SOLID;
                        dl.push_rect(lx - 7.0, 0.0, lw, MENUBAR_H, solid, bg);
                    }
                    if hovered && input.left_just_pressed {
                        menu_toggle = Some(i);
                    }
                    // Hover-switch while a menu is open, DCC-style.
                    if hovered && menu_open.is_some() && menu_open != Some(i) {
                        menu_toggle = Some(i);
                    }
                    let mut tx = lx;
                    for c in label.chars() {
                        let uv = font::glyph_rect(c);
                        if uv != [0.0_f32; 4] {
                            dl.push_rect(tx, 3.0, 8.0, 16.0, uv, [205, 210, 222, 255]);
                        }
                        tx += 8.0;
                    }
                }
            }

            // Horizontal icon toolbar — grouped tools with active-state accents
            let mut ui = Ui::with_input(
                dl,
                Rect {
                    x: 4.0,
                    y: MENUBAR_H + 3.0,
                    w: win_w - 8.0,
                    h: TOOLBAR_H - MENUBAR_H - 4.0,
                },
                input.clone(),
            );

            use dumpster_fire_engine::resource_manager::ui_manager::font::IconId;
            let mut a = ToolbarActions::default();
            a.spawn = ui.hicon(IconId::Plus, false, "Add actor");
            ui.hsep_v(22.0);
            a.object_mode = ui.hicon(IconId::Pointer, !in_edit, "Object mode");
            a.edit_mode = ui.hicon(IconId::Box, in_edit, "Mesh edit mode (E)");
            ui.hsep_v(22.0);
            a.t = ui.hicon(
                IconId::Move,
                gizmo_mode == GizmoMode::Translate,
                "Translate (G)",
            );
            a.r = ui.hicon(IconId::Rotate, gizmo_mode == GizmoMode::Rotate, "Rotate (R)");
            a.s = ui.hicon(IconId::Scale, gizmo_mode == GizmoMode::Scale, "Scale (S)");
            a.snap = ui.hicon(IconId::Ruler, snap, "Snap (X)");
            a.grid = ui.hicon(IconId::Grid, grid_on, "Grid");
            ui.hsep_v(22.0);
            a.dup = ui.hicon(IconId::Copy, false, "Duplicate (Ctrl+D)");
            a.del = ui.hicon(IconId::Trash, false, "Delete (Del)");
            a.undo = ui.hicon(IconId::Undo, false, "Undo (Ctrl+Z)");
            a.redo = ui.hicon(IconId::Redo, false, "Redo (Ctrl+Shift+Z)");
            ui.hsep_v(22.0);
            a.frame = ui.hicon(IconId::Axis, false, "Frame (F)");
            a.layout = ui.hicon(IconId::Layers, false, "Viewport layout");
            a.tonemap = ui.hicon(IconId::Settings, false, "Tonemap");
            a.rt = ui.hicon(IconId::Pen, rt_on, "Ray-traced lighting (L)");
            a.stats = ui.hicon(
                if stats { IconId::Eye } else { IconId::EyeOff },
                stats,
                "Stats (F2)",
            );
            ui.hgap(10.0);

            // Mode badge + right-aligned status line
            let by = ui.cursor[1];
            let badge = if in_edit { "EDIT" } else { "OBJ" };
            let bc = if in_edit {
                [255, 190, 90, 255]
            } else {
                [150, 160, 180, 255]
            };
            let bx = ui.cursor[0];
            ui.text_at(bx, by + 3.0, badge, bc);
            let lt = if rt_on { "RT" } else { "RASTER" };
            let status = format!("{layout_label} | {tm_label} | {lt} | {fps_str}");
            let st_w = status.chars().count() as f32 * 8.0;
            ui.text_at(win_w - st_w - 10.0, by + 3.0, &status, [130, 180, 130, 255]);

            let tip = ui.pending_tooltip.take();
            (a, tip, menu_toggle)
        };
        self.pending_tooltip = tip;
        if let Some(i) = menu_toggle {
            self.menu_open = if self.menu_open == Some(i) && self.ui_left_just_pressed {
                None
            } else {
                Some(i)
            };
        }

        if a.spawn {
            self.spawn_menu_open = !self.spawn_menu_open;
        }
        if (a.object_mode && self.edit.is_some()) || (a.edit_mode && self.edit.is_none()) {
            self.toggle_edit_mode();
        }
        if a.t {
            self.gizmo_mode = GizmoMode::Translate;
        }
        if a.r {
            self.gizmo_mode = GizmoMode::Rotate;
        }
        if a.s {
            self.gizmo_mode = GizmoMode::Scale;
        }
        if a.snap {
            self.snap_enabled = !self.snap_enabled;
        }
        if a.grid {
            self.grid_enabled = !self.grid_enabled;
            self.world.grid_enabled = self.grid_enabled;
        }
        if a.dup {
            self.duplicate_selected();
        }
        if a.del && let (Some(ah), Some((lh, sh))) = (self.world.selection, self.main_stage) {
            self.world.despawn_actor(lh, sh, ah);
            self.actors.retain(|&h| h != ah);
            self.world.selection = None;
        }
        if a.undo && let Some(e) = self.edit.as_mut() {
            e.undo();
        }
        if a.redo && let Some(e) = self.edit.as_mut() {
            e.redo();
        }
        if a.frame {
            if self.world.selection.is_some() {
                self.frame_selected(ctx, app);
            } else if let Some(aabb) = ctx.gltf_union_aabb_for_world(&self.world) {
                ctx.fit_all_panes_to_aabb(app, &aabb);
            }
        }
        if a.layout && let Some(grid) = ctx.viewport_grid_mut(app) {
            let next = grid.layout.next();
            grid.set_layout(next, &[]);
        }
        if a.tonemap {
            self.world.tonemap_op = (self.world.tonemap_op + 1) % 3;
        }
        if a.stats {
            self.stats_open = !self.stats_open;
        }
        if a.rt && let Some(mode) = ctx.toggle_lighting_mode(app) {
            println!("editor: lighting mode → {mode:?}");
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
    /// Per-pane viewport chrome: a camera-kind tag at the pane's top-left and
    /// an accent border around the focused pane. Drawn right after the toolbar
    /// so the side panels naturally cover any under-panel overlap.
    fn draw_viewport_chrome(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::render::app::ViewportKind;
        use dumpster_fire_engine::resource_manager::ui_manager::{draw as uidraw, font};

        let Some(grid) = ctx.viewport_grid(app) else {
            return;
        };
        let (win_w, win_h) = (grid.win_w, grid.win_h);
        let mut panes: ThinVec<(f32, f32, f32, f32, &'static str, bool)> = ThinVec::new();
        for (h, vp) in grid.iter() {
            let label = match vp.kind {
                ViewportKind::Perspective => "Perspective",
                ViewportKind::OrthoTop => "Top",
                ViewportKind::OrthoFront => "Front",
                ViewportKind::OrthoRight => "Right",
            };
            panes.push((
                vp.rect.x * win_w,
                vp.rect.y * win_h,
                vp.rect.w * win_w,
                vp.rect.h * win_h,
                label,
                h == grid.focused,
            ));
        }
        let multi = panes.len() > 1;
        let ow = self.outliner_w;
        let dl = &mut self.world.ui.draw_list;
        for (px, py, pw, ph, label, focused) in panes {
            // Focused-pane accent border (subtle line for unfocused panes,
            // only worth drawing when there is more than one pane).
            if multi {
                let bc: [u8; 4] = if focused {
                    [95, 150, 225, 220]
                } else {
                    [58, 58, 74, 110]
                };
                let (x1, y1) = (px + pw - 1.0, py + ph - 1.0);
                dl.push_line(px, py + 1.0, x1, py + 1.0, 1.0, bc);
                dl.push_line(px, y1, x1, y1, 1.0, bc);
                dl.push_line(px + 1.0, py, px + 1.0, y1, 1.0, bc);
                dl.push_line(x1, py, x1, y1, 1.0, bc);
            }
            // Camera-kind tag, nudged inside the visible viewport area.
            let lx = px.max(ow) + 8.0;
            let ly = py.max(TOOLBAR_H) + 6.0;
            let tw = label.chars().count() as f32 * 8.0 + 10.0;
            dl.push_rect(lx, ly, tw, 20.0, uidraw::SOLID, [16, 16, 24, 200]);
            let tc: [u8; 4] = if focused {
                [170, 200, 245, 230]
            } else {
                [150, 155, 175, 200]
            };
            let mut tx = lx + 5.0;
            for c in label.chars() {
                let uv = font::glyph_rect(c);
                if uv != [0.0_f32; 4] {
                    dl.push_rect(tx, ly + 2.0, 8.0, 16.0, uv, tc);
                }
                tx += 8.0;
            }
        }
    }

    /// F11 — collapse to a single pane keeping the focused camera; toggle
    /// back to restore the saved multi-pane layout.
    fn toggle_maximize_pane(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle) {
        if let Some(saved) = self.saved_layout.take() {
            if let Some(grid) = ctx.viewport_grid_mut(app) {
                grid.set_layout(saved, &[]);
            }
        } else if let Some(grid) = ctx.viewport_grid_mut(app)
            && grid.layout != ViewportLayout::Single
        {
            let focused_cam = grid.focused_camera();
            self.saved_layout = Some(grid.layout);
            grid.set_layout(ViewportLayout::Single, &[]);
            if let (Some(cam), Some(h)) = (focused_cam, grid.slot(0))
                && let Some(pane) = grid.get_mut(h)
            {
                pane.camera_handle = cam;
            }
        }
    }

    /// Serialize the stage to a line-based native scene file. One line per
    /// actor: kind, kind-specific fields, then the world Affine3A as
    /// 12 floats (matrix3 columns + translation).
    fn save_scene(&mut self, path: &str) {
        let Some((lh, sh)) = self.main_stage else {
            return;
        };
        let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh)) else {
            return;
        };
        let mut out = String::from("dfe-scene v1\n");
        for (ah, actor) in stage.actors.entries() {
            let Some(wa) = stage.worlds.get(ah.idx as usize) else {
                continue;
            };
            let m = wa.matrix3.to_cols_array();
            let t = wa.translation;
            let affine = format!(
                "{} {} {} {} {} {} {} {} {} {} {} {}",
                m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], t.x, t.y, t.z
            );
            let mut written = false;
            for se in actor.sub_entities.iter().flatten() {
                match &se.actor_type {
                    ActorType::Environment(e) if e.mesh.is_some() => {
                        out.push_str(&format!("mesh {} {affine}\n", e.name));
                        written = true;
                    }
                    ActorType::Utility(u) => {
                        if let Some(Component::Utility(uc)) =
                            &se.components[ComponentType::Utility.index()]
                            && let Some(l) = &uc.light
                        {
                            let head = match &l.kind {
                                LightKind::Point => "light point".to_string(),
                                LightKind::Spot {
                                    cone_inner,
                                    cone_outer,
                                    direction,
                                } => format!(
                                    "light spot {cone_inner} {cone_outer} {} {} {}",
                                    direction[0], direction[1], direction[2]
                                ),
                                LightKind::Directional { direction } => format!(
                                    "light dir {} {} {}",
                                    direction[0], direction[1], direction[2]
                                ),
                                _ => continue, // editor doesn't spawn other kinds
                            };
                            out.push_str(&format!(
                                "{head} {} {} {} {} {} {affine}\n",
                                l.color[0], l.color[1], l.color[2], l.intensity, l.range
                            ));
                            written = true;
                        } else if !written {
                            out.push_str(&format!("empty {} {affine}\n", u.name));
                            written = true;
                        }
                    }
                    _ => {}
                }
                if written {
                    break;
                }
            }
        }
        match std::fs::write(path, &out) {
            Ok(()) => {
                let n = out.lines().count() - 1;
                self.push_log(
                    format!("LogEditor: saved {n} actors to {path}"),
                    [140, 200, 140, 255],
                );
            }
            Err(e) => self.push_log(
                format!("LogEditor: save failed: {e}"),
                [230, 120, 120, 255],
            ),
        }
    }

    /// Load a scene file saved by `save_scene`: despawns every current actor,
    /// then respawns from the file (meshes re-loaded through the asset cache).
    fn load_scene(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle, path: &str) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.push_log(
                    format!("LogEditor: load failed: {e}"),
                    [230, 120, 120, 255],
                );
                return;
            }
        };
        let Some((lh, sh)) = self.main_stage else {
            return;
        };
        // Clear the current stage.
        for ah in std::mem::take(&mut self.actors) {
            self.world.despawn_actor(lh, sh, ah);
        }
        self.world.selection = None;

        let parse_affine = |tok: &[&str]| -> Option<Affine3A> {
            if tok.len() < 12 {
                return None;
            }
            let mut f = [0.0_f32; 12];
            for (i, v) in tok[..12].iter().enumerate() {
                f[i] = v.parse().ok()?;
            }
            let mut m = [0.0_f32; 9];
            m.copy_from_slice(&f[..9]);
            Some(Affine3A {
                matrix3: glam::Mat3A::from_cols_array(&m),
                translation: glam::Vec3A::new(f[9], f[10], f[11]),
            })
        };
        let mut loaded = 0usize;
        for line in text.lines().skip(1) {
            let tok: ThinVec<&str> = line.split_whitespace().collect();
            if tok.is_empty() {
                continue;
            }
            let spawned: Option<(Option<Affine3A>, ActorHandle)> = match tok[0] {
                "mesh" if tok.len() >= 14 => {
                    let mesh_path: Arc<str> = Arc::from(tok[1]);
                    match ctx.load_gltf(app, PathBuf::from(tok[1])) {
                        Ok(asset) => {
                            self.do_spawn_mesh(asset, mesh_path);
                            self.world
                                .selection
                                .map(|ah| (parse_affine(&tok[2..]), ah))
                        }
                        Err(e) => {
                            self.push_log(
                                format!("LogEditor: asset {} failed: {e:?}", tok[1]),
                                [230, 120, 120, 255],
                            );
                            None
                        }
                    }
                }
                "light" if tok.len() >= 2 => {
                    let (kind, rest) = match tok[1] {
                        "spot" if tok.len() >= 7 => (
                            tok[2].parse().ok().and_then(|ci| {
                                Some(LightKind::Spot {
                                    cone_inner: ci,
                                    cone_outer: tok[3].parse().ok()?,
                                    direction: [
                                        tok[4].parse().ok()?,
                                        tok[5].parse().ok()?,
                                        tok[6].parse().ok()?,
                                    ],
                                })
                            }),
                            &tok[7..],
                        ),
                        "dir" if tok.len() >= 5 => (
                            (|| {
                                Some(LightKind::Directional {
                                    direction: [
                                        tok[2].parse().ok()?,
                                        tok[3].parse().ok()?,
                                        tok[4].parse().ok()?,
                                    ],
                                })
                            })(),
                            &tok[5..],
                        ),
                        _ => (Some(LightKind::Point), &tok[2..]),
                    };
                    match (kind, rest.len() >= 17) {
                        (Some(k), true) => {
                            self.spawn_light(k);
                            // Patch color / intensity / range over the defaults.
                            if let (Some(ah), Ok(r), Ok(g), Ok(b), Ok(iv), Ok(rv)) = (
                                self.world.selection,
                                rest[0].parse::<f32>(),
                                rest[1].parse::<f32>(),
                                rest[2].parse::<f32>(),
                                rest[3].parse::<f32>(),
                                rest[4].parse::<f32>(),
                            ) {
                                if let Some(stage) = self
                                    .world
                                    .levels
                                    .get_mut(lh)
                                    .and_then(|l| l.stages.get_mut(sh))
                                    && let Some(actor) = stage.actors.get_mut(ah)
                                {
                                    for se in actor.sub_entities.iter_mut().flatten() {
                                        if let Some(Component::Utility(uc)) =
                                            &mut se.components[ComponentType::Utility.index()]
                                            && let Some(l) = uc.light.as_mut()
                                        {
                                            l.color = [r, g, b];
                                            l.intensity = iv;
                                            l.range = rv;
                                        }
                                    }
                                }
                                Some((parse_affine(&rest[5..]), ah))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                }
                "empty" if tok.len() >= 14 => {
                    let name = tok[1].to_string();
                    self.spawn_empty(&name);
                    self.world
                        .selection
                        .map(|ah| (parse_affine(&tok[2..]), ah))
                }
                _ => None,
            };
            if let Some((Some(wa), ah)) = spawned
                && let Some(stage) = self
                    .world
                    .levels
                    .get_mut(lh)
                    .and_then(|l| l.stages.get_mut(sh))
                && let Some(t) = stage.worlds.get_mut(ah.idx as usize)
            {
                *t = wa;
                loaded += 1;
            }
        }
        self.world.selection = None;
        self.push_log(
            format!("LogEditor: loaded {loaded} actors from {path}"),
            [140, 200, 140, 255],
        );
    }

    /// Execute one console line. Commands mirror the menu/toolbar actions so
    /// everything is scriptable from the keyboard, UE-console style.
    fn execute_console(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle, cmd: &str) {
        self.push_log(format!("> {cmd}"), [160, 170, 190, 255]);
        let tok: ThinVec<&str> = cmd.split_whitespace().collect();
        match tok[0] {
            "help" => {
                self.push_log(
                    "  save/load [path] | spawn point|spot|dir|empty | grid on|off | sky on|off | rt | stats | tonemap 0..2 | clear | quit",
                    [150, 160, 180, 255],
                );
            }
            "clear" => self.log.clear(),
            "save" => self.save_scene(tok.get(1).copied().unwrap_or("scene.dfescene")),
            "load" => {
                let path = tok.get(1).copied().unwrap_or("scene.dfescene").to_string();
                self.load_scene(ctx, app, &path);
            }
            "spawn" => match tok.get(1).copied() {
                Some("point") => self.spawn_light(LightKind::Point),
                Some("spot") => self.spawn_light(LightKind::Spot {
                    cone_inner: 0.5,
                    cone_outer: 0.8,
                    direction: [0.0, -1.0, 0.0],
                }),
                Some("dir") => self.spawn_light(LightKind::Directional {
                    direction: [0.0, -1.0, 0.0],
                }),
                Some(name) => self.spawn_empty(name),
                None => self.push_log("  usage: spawn point|spot|dir|<name>", [200, 160, 110, 255]),
            },
            "grid" => {
                let on = tok.get(1) != Some(&"off");
                self.grid_enabled = on;
                self.world.grid_enabled = on;
            }
            "sky" => self.world.sky_enabled = tok.get(1) != Some(&"off"),
            "rt" => {
                if let Some(mode) = ctx.toggle_lighting_mode(app) {
                    self.push_log(format!("  lighting → {mode:?}"), [150, 160, 180, 255]);
                }
            }
            "stats" => self.stats_open = !self.stats_open,
            "tonemap" => {
                if let Some(op) = tok.get(1).and_then(|t| t.parse::<u32>().ok()) {
                    self.world.tonemap_op = op.min(2);
                }
            }
            "quit" => self.quit_requested = true,
            other => self.push_log(
                format!("  unknown command '{other}' — try help"),
                [230, 120, 120, 255],
            ),
        }
    }

    /// Apply the in-place outliner rename. Mesh actors are named by their
    /// asset path (load-bearing for edit mode + scene save), so only
    /// Utility-typed sub-entities (lights, empties, cameras) are renamed.
    fn commit_rename(&mut self) {
        let (Some(ah), Some((lh, sh))) = (self.rename_target.take(), self.main_stage) else {
            self.rename_buf.clear();
            return;
        };
        let name = std::mem::take(&mut self.rename_buf);
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let mut renamed = false;
        let mut is_mesh = false;
        if let Some(stage) = self
            .world
            .levels
            .get_mut(lh)
            .and_then(|l| l.stages.get_mut(sh))
            && let Some(actor) = stage.actors.get_mut(ah)
        {
            for se in actor.sub_entities.iter_mut().flatten() {
                match &mut se.actor_type {
                    ActorType::Utility(u) => {
                        u.name = Arc::from(name);
                        renamed = true;
                    }
                    ActorType::Environment(e) if e.mesh.is_some() => is_mesh = true,
                    _ => {}
                }
            }
        }
        if renamed {
            self.push_log(format!("LogEditor: renamed to '{name}'"), [140, 200, 140, 255]);
        } else if is_mesh {
            self.push_log(
                "LogEditor: mesh actors are named by asset path — rename skipped.",
                [200, 160, 110, 255],
            );
        }
    }

    fn push_log(&mut self, text: impl Into<String>, color: [u8; 4]) {
        self.log.push((text.into(), color));
        if self.log.len() > 200 {
            self.log.remove(0);
        }
    }

    /// Draw the open menu dropdown and route entry clicks. Called after all
    /// panels so the dropdown sits on top of the outliner/viewport.
    fn draw_menus(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::resource_manager::ui_manager::{draw as uidraw, font};
        let Some(mi) = self.menu_open else {
            self.menu_rect = (0.0, 0.0, 0.0, 0.0);
            return;
        };
        let entries = MENUS[mi].1;
        let mx = menu_label_x(mi) - 7.0;
        let my = MENUBAR_H;
        let mw = 176.0_f32;
        let mh = entries.len() as f32 * 24.0 + 8.0;
        self.menu_rect = (mx, my, mw, mh);

        let cx = self.ui_cursor[0];
        let cy = self.ui_cursor[1];
        let jp = self.ui_left_just_pressed;
        let mut chosen: Option<usize> = None;
        {
            let dl = &mut self.world.ui.draw_list;
            dl.push_panel_bg(mx, my, mw, mh, [26, 26, 36, 250]);
            dl.push_line(mx, my, mx + mw, my, 1.0, SEP);
            dl.push_line(mx + mw, my, mx + mw, my + mh, 1.0, SEP);
            dl.push_line(mx, my + mh, mx + mw, my + mh, 1.0, SEP);
            dl.push_line(mx, my, mx, my + mh, 1.0, SEP);
            for (i, entry) in entries.iter().enumerate() {
                let iy = my + i as f32 * 24.0 + 4.0;
                let hov = cx >= mx + 2.0 && cx < mx + mw - 2.0 && cy >= iy && cy < iy + 20.0;
                let bg = if hov {
                    [62, 84, 130, 255]
                } else {
                    [30, 30, 42, 255]
                };
                dl.push_rect(mx + 2.0, iy, mw - 4.0, 20.0, uidraw::SOLID, bg);
                let mut ex = mx + 10.0;
                for c in entry.chars() {
                    let uv = font::glyph_rect(c);
                    if c != ' ' && uv != [0.0_f32; 4] {
                        dl.push_rect(ex, iy + 2.0, 8.0, 16.0, uv, [205, 208, 220, 255]);
                    }
                    ex += 8.0;
                }
                if hov && jp {
                    chosen = Some(i);
                }
            }
        }

        // Click outside the dropdown and below the menu bar closes it.
        if jp && chosen.is_none() && cy >= MENUBAR_H {
            let inside = cx >= mx && cx < mx + mw && cy >= my && cy < my + mh;
            if !inside {
                self.menu_open = None;
            }
        }

        let Some(ei) = chosen else { return };
        self.menu_open = None;
        match (mi, ei) {
            (0, 0) => self.picker_open = true,
            (0, 1) => self.save_scene("scene.dfescene"),
            (0, 2) => self.load_scene(ctx, app, "scene.dfescene"),
            (0, 3) => self.quit_requested = true,
            (1, 0) => {
                if let Some(e) = self.edit.as_mut() {
                    e.undo();
                } else {
                    self.push_log("LogEditor: undo — enter mesh edit mode (E).", [180, 160, 110, 255]);
                }
            }
            (1, 1) => {
                if let Some(e) = self.edit.as_mut() {
                    e.redo();
                }
            }
            (1, 2) => self.duplicate_selected(),
            (1, 3) => {
                if let (Some(ah), Some((lh, sh))) = (self.world.selection, self.main_stage) {
                    self.world.despawn_actor(lh, sh, ah);
                    self.actors.retain(|&h| h != ah);
                    self.world.selection = None;
                }
            }
            (2, 4) => self.stats_open = !self.stats_open,
            (2, 5) => self.toggle_maximize_pane(ctx, app),
            (2, 6) => self.world.sky_enabled = !self.world.sky_enabled,
            (2, li) => {
                let layout = match li {
                    0 => ViewportLayout::Single,
                    1 => ViewportLayout::TwoColumns,
                    2 => ViewportLayout::TwoRows,
                    _ => ViewportLayout::FourQuadrant,
                };
                if let Some(grid) = ctx.viewport_grid_mut(app) {
                    grid.set_layout(layout, &[]);
                }
            }
            (3, _) => {
                self.push_log(
                    "LogEditor: DumpsterFire — native GUI + RT lighting, zero UI deps.",
                    [130, 180, 220, 255],
                );
            }
            _ => {}
        }
    }

    fn draw_outliner(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        let (_, win_h) = ctx
            .viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let panel_h = win_h - TOOLBAR_H - self.bottom_h;
        let ow = self.outliner_w;
        let x = 0.0_f32;
        let y = TOOLBAR_H;
        let content_y = y + TITLEBAR_H + 2.0;
        let content_h = panel_h - TITLEBAR_H - 2.0;
        let row_h = 22.0_f32;
        let input = self.ui_input();

        // Collect actor rows before taking draw_list borrow.
        // Each row: (handle, icon_color, is_selected, hovered, clicked)
        let mut rows: ThinVec<(
            ActorHandle,
            (dumpster_fire_engine::resource_manager::ui_manager::font::IconId, [u8; 4]),
            bool,
            bool,
            bool,
        )> = ThinVec::new();
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
                    && input.cursor[0] < x + ow
                    && input.cursor[1] >= row_y
                    && input.cursor[1] < row_y + row_h;
                let clicked = hovered && input.left_just_pressed;
                if clicked {
                    clicked_ah = Some(ah);
                }
                rows.push((ah, actor_icon(actor), is_sel, hovered, clicked));
                row_y += row_h;
            }
        }
        if let Some(ah) = clicked_ah {
            self.world.selection = Some(ah);
            // Double-click on an already-selected row starts an in-place rename.
            let now = std::time::Instant::now();
            if let Some((prev, at)) = self.last_row_click
                && prev == ah
                && now.duration_since(at).as_millis() < 600
            {
                self.rename_target = Some(ah);
                self.rename_buf.clear();
                self.console_focus = false;
            }
            self.last_row_click = Some((ah, now));
        }

        // Now borrow draw_list and render.
        let dl = &mut self.world.ui.draw_list;
        dl.push_panel_bg(x, y, ow, panel_h, PANEL_BG);
        dl.push_title_bar(x, y, ow, TITLEBAR_H, TITLEBAR_BG, SEP);
        // Title bar label "OUTLINER"
        {
            use dumpster_fire_engine::resource_manager::ui_manager::font;
            let mut tx = x + 6.0;
            for c in "OUTLINER".chars() {
                let uv = font::glyph_rect(c);
                if uv != [0f32; 4] {
                    dl.push_rect(tx, y + 3.0, 8.0, 16.0, uv, [200, 210, 230, 255]);
                }
                tx += 8.0;
            }
        }
        // Left divider — highlight when hovering or dragging
        let div_col = if self.div_hover == Some(0) || self.div_drag == Some(0) {
            [120, 160, 220, 255u8]
        } else {
            SEP
        };
        dl.push_vsep(x + ow, y, panel_h + self.bottom_h, div_col);

        let mut row_y = content_y;
        if let Some((lh, sh)) = self.main_stage
            && let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh))
        {
            row_y -= self.outliner_scroll_y;
            let _ = stage; // borrow ends before we iterate rows below
        }

        use dumpster_fire_engine::resource_manager::ui_manager::{draw as uidraw, font};
        if let Some((lh, sh)) = self.main_stage
            && let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh))
        {
            for (ah, (icon_id, icon_col), is_sel, hovered, _) in &rows {
                let bg = if *is_sel {
                    [55, 95, 155, 255]
                } else if *hovered {
                    [38, 38, 52, 255]
                } else if (ah.idx & 1) == 0 {
                    [26, 26, 33, 255]
                } else {
                    [24, 24, 30, 255]
                };
                dl.push_rect(x, row_y, ow, row_h - 1.0, uidraw::SOLID, bg);
                dl.push_rect(x + 5.0, row_y + 3.0, 16.0, 16.0, font::icon_rect(*icon_id), *icon_col);
                let label: &str = stage
                    .actors
                    .get(*ah)
                    .and_then(|actor| {
                        actor
                            .sub_entities
                            .iter()
                            .flatten()
                            .map(|se| match &se.actor_type {
                                ActorType::Environment(e) => e.name.as_ref(),
                                ActorType::Character(c) => c.name.as_ref(),
                                ActorType::Item(i) => i.name.as_ref(),
                                ActorType::Utility(u) => u.name.as_ref(),
                            })
                            .find(|s: &&str| !s.is_empty())
                    })
                    .unwrap_or("Entity");
                let renaming = self.rename_target == Some(*ah);
                let label: &str = if renaming { &self.rename_buf } else { label };
                let tc: [u8; 4] = if renaming {
                    [255, 235, 180, 255]
                } else if *is_sel {
                    [200, 220, 255, 220]
                } else {
                    [155, 155, 175, 200]
                };
                if renaming {
                    dl.push_rect(
                        x + 24.0,
                        row_y + 1.0,
                        ow - 28.0,
                        row_h - 3.0,
                        uidraw::SOLID,
                        [20, 24, 36, 255],
                    );
                }
                let mut lx = x + 26.0;
                for c in label.chars() {
                    if lx + 8.0 > x + ow - 4.0 {
                        break;
                    }
                    let uv = font::glyph_rect(c);
                    if uv != [0.0_f32; 4] {
                        dl.push_rect(lx, row_y + 3.0, 8.0, 16.0, uv, tc);
                    }
                    lx += 8.0;
                }
                if renaming {
                    dl.push_rect(lx + 1.0, row_y + 3.0, 2.0, 16.0, uidraw::SOLID, [255, 235, 180, 255]);
                }
                row_y += row_h;
            }
        }
    }
}

/// (icon, tint) for an outliner row: lights get the grip cluster, cameras the
/// 3D axis, meshes the box; anything else the layers glyph.
fn actor_icon(
    actor: &dumpster_fire_engine::resource_manager::manager::Actor,
) -> (dumpster_fire_engine::resource_manager::ui_manager::font::IconId, [u8; 4]) {
    use dumpster_fire_engine::resource_manager::ui_manager::font::IconId;
    let color = actor_icon_color(actor);
    for se in actor.sub_entities.iter().flatten() {
        if let ActorType::Utility(_) = &se.actor_type
            && let Some(Component::Utility(uc)) = &se.components[ComponentType::Utility.index()]
        {
            if uc.light.is_some() {
                return (IconId::Grip, color);
            }
            if uc.camera.is_some() {
                return (IconId::Axis, color);
            }
        }
    }
    let is_mesh = actor.sub_entities.iter().flatten().any(|se| {
        matches!(
            &se.actor_type,
            ActorType::Environment(_) | ActorType::Character(_) | ActorType::Item(_)
        )
    });
    if is_mesh {
        (IconId::Box, color)
    } else {
        (IconId::Layers, color)
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
    /// Edit-mode replacement for the details panel: a Blender-style mesh
    /// tools window — element modes, selection commands, topology operators
    /// with parameter drag-fields, undo/redo, and live mesh statistics.
    fn draw_mesh_edit_panel(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::render::ui_core::id::path_key;
        use dumpster_fire_engine::resource_manager::ui_manager::{immediate::Ui, layout::Rect};

        let (win_w, win_h) = ctx
            .viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let panel_h = win_h - TOOLBAR_H - self.bottom_h;
        let iw = self.inspector_w;
        let ix = win_w - iw;
        let iy = TOOLBAR_H;
        let input = self.ui_input();

        let mut extrude_offset = self.extrude_offset;
        let mut inset_amount = self.inset_amount;
        #[derive(Default)]
        struct Acts {
            mode_v: bool,
            mode_e: bool,
            mode_f: bool,
            sel_all: bool,
            sel_none: bool,
            sel_inv: bool,
            sel_grow: bool,
            sel_shrink: bool,
            extrude: bool,
            inset: bool,
            subdivide: bool,
            delete: bool,
            merge: bool,
            weld: bool,
            flip: bool,
            smooth: bool,
            poke: bool,
            recalc: bool,
            undo: bool,
            redo: bool,
        }
        let mut a = Acts::default();
        let (mode, nv, ne, nf, nsel) = {
            let e = self.edit.as_ref().expect("edit panel needs a session");
            (
                e.mode,
                e.vertex_count(),
                e.edge_count(),
                e.face_count(),
                e.selected_vertex_count(),
            )
        };
        let mut new_drag = None;
        {
            let dl = &mut self.world.ui.draw_list;
            dl.push_panel_bg(ix, iy, iw, panel_h, PANEL_BG);
            dl.push_title_bar(ix, iy, iw, TITLEBAR_H, TITLEBAR_BG, SEP);
            {
                use dumpster_fire_engine::resource_manager::ui_manager::font;
                let mut tx = ix + 6.0;
                for c in "MESH EDIT".chars() {
                    if c == ' ' {
                        tx += 8.0;
                        continue;
                    }
                    let uv = font::glyph_rect(c);
                    if uv != [0f32; 4] {
                        dl.push_rect(tx, iy + 3.0, 8.0, 16.0, uv, [255, 200, 120, 255]);
                    }
                    tx += 8.0;
                }
            }
            let div_col = if self.div_hover == Some(1) || self.div_drag == Some(1) {
                [120, 160, 220, 255u8]
            } else {
                SEP
            };
            dl.push_vsep(ix, iy, panel_h + self.bottom_h, div_col);

            let mut ui = Ui::with_input(
                dl,
                Rect {
                    x: ix + 8.0,
                    y: iy + TITLEBAR_H + 6.0,
                    w: iw - 16.0,
                    h: panel_h - TITLEBAR_H - 10.0,
                },
                input.clone(),
            );
            ui.drag_state = self.insp_drag;

            ui.section_header("MODE");
            {
                let y = ui.cursor[1];
                ui.cursor = [ix + 8.0, y];
                a.mode_v = ui.hbutton(
                    if mode == ElementMode::Vertex { "[Vert]" } else { "Vert" },
                    56.0,
                );
                a.mode_e = ui.hbutton(
                    if mode == ElementMode::Edge { "[Edge]" } else { "Edge" },
                    56.0,
                );
                a.mode_f = ui.hbutton(
                    if mode == ElementMode::Face { "[Face]" } else { "Face" },
                    56.0,
                );
                ui.cursor = [ix + 8.0, y + 26.0];
            }

            ui.section_header("SELECT");
            {
                let y = ui.cursor[1];
                ui.cursor = [ix + 8.0, y];
                a.sel_all = ui.hbutton("All", 44.0);
                a.sel_none = ui.hbutton("None", 48.0);
                a.sel_inv = ui.hbutton("Inv", 40.0);
                ui.cursor = [ix + 8.0, y + 24.0];
                let y = ui.cursor[1];
                a.sel_grow = ui.hbutton("Grow", 52.0);
                a.sel_shrink = ui.hbutton("Shrink", 60.0);
                ui.cursor = [ix + 8.0, y + 28.0];
            }

            ui.section_header("TOOLS");
            a.extrude = ui.button("Extrude (E)");
            ui.drag_field(path_key(&["medit"], "exoff"), "Offset", &mut extrude_offset, 0.01);
            a.inset = ui.button("Inset (I)");
            ui.drag_field(path_key(&["medit"], "insam"), "Amount", &mut inset_amount, 0.005);
            a.subdivide = ui.button("Subdivide");
            a.delete = ui.button("Delete Faces (X)");
            a.merge = ui.button("Merge at Center");
            a.weld = ui.button("Weld 1mm");
            a.flip = ui.button("Flip Normals");
            a.recalc = ui.button("Recalc Normals (N)");
            a.smooth = ui.button("Smooth Verts");
            a.poke = ui.button("Poke Faces");

            ui.section_header("HISTORY");
            {
                let y = ui.cursor[1];
                ui.cursor = [ix + 8.0, y];
                a.undo = ui.hbutton("Undo", 52.0);
                a.redo = ui.hbutton("Redo", 52.0);
                ui.cursor = [ix + 8.0, y + 28.0];
            }

            ui.section_header("MESH");
            ui.label(&format!("v{nv} e{ne} f{nf} sel{nsel}"));
            new_drag = ui.begin_drag.take();
        }
        if let Some(bd) = new_drag {
            self.insp_drag = Some(bd);
        }
        if !self.ui_left_down {
            self.insp_drag = None;
        }
        self.extrude_offset = extrude_offset;
        self.inset_amount = inset_amount;

        let Some(e) = self.edit.as_mut() else { return };
        if a.mode_v {
            e.mode = ElementMode::Vertex;
        }
        if a.mode_e {
            e.mode = ElementMode::Edge;
        }
        if a.mode_f {
            e.mode = ElementMode::Face;
        }
        if a.sel_all {
            e.select_all();
        }
        if a.sel_none {
            e.select_none();
        }
        if a.sel_inv {
            e.invert_selection();
        }
        if a.sel_grow {
            e.grow_selection();
        }
        if a.sel_shrink {
            e.shrink_selection();
        }
        let mut log: Option<(String, [u8; 4])> = None;
        if a.extrude {
            let off = self.extrude_offset;
            if e.extrude_selected(off) {
                log = Some(("LogMesh: extruded region.".into(), [140, 200, 140, 255]));
            } else {
                log = Some(("LogMesh: extrude needs selected faces.".into(), [200, 160, 110, 255]));
            }
        }
        if a.inset {
            let am = self.inset_amount;
            if e.inset_selected(am) {
                log = Some(("LogMesh: inset faces.".into(), [140, 200, 140, 255]));
            } else {
                log = Some(("LogMesh: inset needs selected faces.".into(), [200, 160, 110, 255]));
            }
        }
        if a.subdivide && e.subdivide_selected() {
            log = Some(("LogMesh: subdivided.".into(), [140, 200, 140, 255]));
        }
        if a.delete && !e.delete_selected() {
            log = Some(("LogMesh: delete needs selected faces.".into(), [200, 160, 110, 255]));
        }
        if a.merge && !e.merge_selected() {
            log = Some(("LogMesh: merge needs 2+ selected verts.".into(), [200, 160, 110, 255]));
        }
        if a.weld {
            e.weld(0.001);
        }
        if a.flip {
            e.flip_selected();
        }
        if a.recalc {
            e.recalc_normals();
            log = Some(("LogMesh: recalculated normals.".into(), [140, 200, 140, 255]));
        }
        if a.smooth && e.smooth_selected(2, 0.5) {
            log = Some(("LogMesh: smoothed vertices.".into(), [140, 200, 140, 255]));
        }
        if a.poke {
            e.poke_selected();
            log = Some(("LogMesh: poked faces.".into(), [140, 200, 140, 255]));
        }
        if a.undo {
            e.undo();
        }
        if a.redo {
            e.redo();
        }
        if let Some((t, c)) = log {
            self.push_log(t, c);
        }
    }

    fn draw_inspector(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::resource_manager::ui_manager::{immediate::Ui, layout::Rect};

        if self.edit.is_some() {
            self.draw_mesh_edit_panel(ctx, app);
            return;
        }

        let (win_w, win_h) = ctx
            .viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let panel_h = win_h - TOOLBAR_H - self.bottom_h;
        let iw = self.inspector_w;
        let ix = win_w - iw;
        let iy = TOOLBAR_H;

        let draw_chrome = |dl: &mut dumpster_fire_engine::resource_manager::ui_manager::draw::DrawList,
                           div_hover: Option<u8>,
                           div_drag: Option<u8>| {
            dl.push_panel_bg(ix, iy, iw, panel_h, PANEL_BG);
            dl.push_title_bar(ix, iy, iw, TITLEBAR_H, TITLEBAR_BG, SEP);
            {
                use dumpster_fire_engine::resource_manager::ui_manager::font;
                let mut tx = ix + 6.0;
                for c in "DETAILS".chars() {
                    let uv = font::glyph_rect(c);
                    if uv != [0f32; 4] {
                        dl.push_rect(tx, iy + 3.0, 8.0, 16.0, uv, [200, 210, 230, 255]);
                    }
                    tx += 8.0;
                }
            }
            let div_col = if div_hover == Some(1) || div_drag == Some(1) {
                [120, 160, 220, 255u8]
            } else {
                SEP
            };
            dl.push_vsep(ix, iy, panel_h + self.bottom_h, div_col);
        };

        // Helper: draw empty panel chrome and return
        macro_rules! draw_empty {
            () => {{
                let dh = self.div_hover;
                let dd = self.div_drag;
                let dl = &mut self.world.ui.draw_list;
                draw_chrome(dl, dh, dd);
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

        // Decompose the current world transform so the fields show live values
        // (and so an untouched inspector never overwrites gizmo edits).
        let (rot0, scl0) = {
            let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh)) else {
                draw_empty!()
            };
            let m = stage
                .worlds
                .get(ah.idx as usize)
                .copied()
                .unwrap_or(Affine3A::IDENTITY)
                .matrix3;
            let sx = m.x_axis.length().max(1e-6);
            let sy = m.y_axis.length().max(1e-6);
            let sz = m.z_axis.length().max(1e-6);
            let rot = glam::Mat3A::from_cols(m.x_axis / sx, m.y_axis / sy, m.z_axis / sz);
            let q = glam::Quat::from_mat3a(&rot);
            let (ex, ey, ez) = q.to_euler(glam::EulerRot::XYZ);
            (
                [ex.to_degrees(), ey.to_degrees(), ez.to_degrees()],
                [sx, sy, sz],
            )
        };
        let mut px = pos.x;
        let mut py = pos.y;
        let mut pz = pos.z;
        let mut rx = rot0[0];
        let mut ry = rot0[1];
        let mut rz = rot0[2];
        let mut sx = scl0[0];
        let mut sy = scl0[1];
        let mut sz = scl0[2];
        let (mut light_intensity, mut light_range) = {
            let mut iv = None;
            let mut rv = None;
            if let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh))
                && let Some(actor) = stage.actors.get(ah)
            {
                for se in actor.sub_entities.iter().flatten() {
                    if let Some(Component::Utility(uc)) =
                        &se.components[ComponentType::Utility.index()]
                        && let Some(light) = &uc.light
                    {
                        iv = Some(light.intensity);
                        rv = Some(light.range);
                    }
                }
            }
            (iv, rv)
        };
        let mut transform_changed = false;
        let mut light_changed = false;

        {
            let content_y = iy + TITLEBAR_H + 4.0;
            let content_w = iw - 8.0;
            let dh = self.div_hover;
            let dd = self.div_drag;
            let dl = &mut self.world.ui.draw_list;
            draw_chrome(dl, dh, dd);
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

            use dumpster_fire_engine::render::ui_core::id::path_key;
            ui.drag_state = self.insp_drag;

            ui.section_header("TRANSFORM");
            if ui.collapsible_header("Position", &mut self.insp_pos_collapsed) {
                transform_changed |= ui.drag_field(path_key(&["insp"], "px"), "X", &mut px, 0.02);
                transform_changed |= ui.drag_field(path_key(&["insp"], "py"), "Y", &mut py, 0.02);
                transform_changed |= ui.drag_field(path_key(&["insp"], "pz"), "Z", &mut pz, 0.02);
            }
            if ui.collapsible_header("Rotation", &mut self.insp_rot_collapsed) {
                transform_changed |= ui.drag_field(path_key(&["insp"], "rx"), "Roll", &mut rx, 0.5);
                transform_changed |=
                    ui.drag_field(path_key(&["insp"], "ry"), "Pitch", &mut ry, 0.5);
                transform_changed |= ui.drag_field(path_key(&["insp"], "rz"), "Yaw", &mut rz, 0.5);
            }
            if ui.collapsible_header("Scale", &mut self.insp_scl_collapsed) {
                transform_changed |=
                    ui.drag_field(path_key(&["insp"], "sx"), "X", &mut sx, 0.01);
                transform_changed |=
                    ui.drag_field(path_key(&["insp"], "sy"), "Y", &mut sy, 0.01);
                transform_changed |=
                    ui.drag_field(path_key(&["insp"], "sz"), "Z", &mut sz, 0.01);
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
                if let (Some(iv), Some(rv)) = (light_intensity.as_mut(), light_range.as_mut()) {
                    light_changed |=
                        ui.drag_field(path_key(&["insp"], "li"), "Intensity", iv, 2.0);
                    light_changed |= ui.drag_field(path_key(&["insp"], "lr"), "Range", rv, 0.1);
                    *iv = iv.max(0.0);
                    *rv = rv.max(0.0);
                }
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
            if let Some(bd) = ui.begin_drag.take() {
                self.insp_drag = Some(bd);
            }
        } // dl borrow released here
        if !self.ui_left_down {
            self.insp_drag = None;
        }

        // Write values back only when a field actually changed this frame, so
        // an idle inspector never stomps gizmo edits.
        if transform_changed
            && let Some(stage) = self
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
                * glam::Mat3A::from_diagonal(glam::Vec3::new(
                    sx.max(0.01),
                    sy.max(0.01),
                    sz.max(0.01),
                ));
        }
        if light_changed
            && let (Some(iv), Some(rv)) = (light_intensity, light_range)
            && let Some(stage) = self
                .world
                .levels
                .get_mut(lh)
                .and_then(|l| l.stages.get_mut(sh))
            && let Some(actor) = stage.actors.get_mut(ah)
        {
            for se in actor.sub_entities.iter_mut().flatten() {
                if let Some(Component::Utility(uc)) =
                    &mut se.components[ComponentType::Utility.index()]
                    && let Some(light) = uc.light.as_mut()
                {
                    light.intensity = iv;
                    light.range = rv;
                }
            }
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

// ── Bottom output log panel ────────────────────────────────────────────────

impl EditorApp {
    fn draw_bottom_panel(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle) {
        use dumpster_fire_engine::resource_manager::ui_manager::{draw as uidraw, font};

        let (win_w, win_h) = ctx
            .viewport_grid(app)
            .map(|g| (g.win_w, g.win_h))
            .unwrap_or((1280.0, 720.0));
        let by = win_h - self.bottom_h;
        let dl = &mut self.world.ui.draw_list;

        dl.push_panel_bg(0.0, by, win_w, self.bottom_h, PANEL_BG);
        let div_col = if self.div_hover == Some(2) || self.div_drag == Some(2) {
            [120, 160, 220, 255u8]
        } else {
            SEP
        };
        dl.push_hsep(0.0, by, win_w, div_col);
        dl.push_title_bar(0.0, by, win_w, TITLEBAR_H, TITLEBAR_BG, SEP);

        // Tab strip: OUTPUT LOG | CONTENT
        let mut clicked_tab: Option<usize> = None;
        {
            use dumpster_fire_engine::resource_manager::ui_manager::draw as uidraw;
            let tabs = ["OUTPUT LOG", "CONTENT"];
            let mut tx0 = 0.0_f32;
            for (ti, name) in tabs.iter().enumerate() {
                let tw = name.chars().count() as f32 * 8.0 + 18.0;
                let active = self.bottom_tab == ti;
                let hov = self.ui_cursor[0] >= tx0
                    && self.ui_cursor[0] < tx0 + tw
                    && self.ui_cursor[1] >= by
                    && self.ui_cursor[1] < by + TITLEBAR_H;
                if active {
                    dl.push_rect(tx0, by, tw, TITLEBAR_H, uidraw::SOLID, [46, 46, 64, 255]);
                    dl.push_rect(tx0, by + TITLEBAR_H - 2.0, tw, 2.0, uidraw::SOLID, [120, 190, 255, 255]);
                } else if hov {
                    dl.push_rect(tx0, by, tw, TITLEBAR_H, uidraw::SOLID, [38, 38, 52, 255]);
                }
                if hov && self.ui_left_just_pressed {
                    clicked_tab = Some(ti);
                }
                let mut tx = tx0 + 9.0;
                let tc: [u8; 4] = if active {
                    [210, 220, 240, 255]
                } else {
                    [150, 155, 175, 220]
                };
                for c in name.chars() {
                    if c != ' ' {
                        let uv = font::glyph_rect(c);
                        if uv != [0f32; 4] {
                            dl.push_rect(tx, by + 3.0, 8.0, 16.0, uv, tc);
                        }
                    }
                    tx += 8.0;
                }
                tx0 += tw + 2.0;
            }
        }

        let mut spawn_asset: Option<Arc<str>> = None;
        if self.bottom_tab == 0 {
            // FPS stats line
            let fps_str = format!(
                "{:.1} ms   {:.0} fps",
                if self.fps_display > 0.0 {
                    1000.0 / self.fps_display
                } else {
                    0.0
                },
                self.fps_display
            );
            let stats_y = by + TITLEBAR_H + 4.0;
            {
                let mut tx = 8.0_f32;
                for c in fps_str.chars() {
                    if c == ' ' {
                        tx += 8.0;
                        continue;
                    }
                    let uv = font::glyph_rect(c);
                    if uv != [0f32; 4] {
                        dl.push_rect(tx, stats_y, 8.0, 16.0, uv, [130, 190, 130, 255]);
                    }
                    tx += 8.0;
                }
            }

            // Divider between stats and content
            dl.push_hsep(0.0, stats_y + 18.0, win_w, [42, 42, 56, 255]);

            // Log content — newest lines that fit, tail-anchored.
            let line_y_start = stats_y + 22.0;
            let avail = ((win_h - 30.0 - line_y_start) / 18.0).max(0.0) as usize;
            let skip = self.log.len().saturating_sub(avail);
            for (i, (text, color)) in self.log.iter().skip(skip).enumerate() {
                let ly = line_y_start + i as f32 * 18.0;
                if ly + 16.0 > win_h - 4.0 {
                    break;
                }
                let mut tx = 8.0_f32;
                for c in text.chars() {
                    if tx + 8.0 > win_w - 8.0 {
                        break;
                    }
                    if c == ' ' {
                        tx += 8.0;
                        continue;
                    }
                    let uv = font::glyph_rect(c);
                    if uv != [0f32; 4] {
                        dl.push_rect(tx, ly, 8.0, 14.0, uv, *color);
                    }
                    tx += 8.0;
                }
            }

            // ── Console input row (click to focus, Enter executes) ────────
            let row_y = win_h - 26.0;
            let row_h = 22.0;
            let in_row = self.ui_cursor[0] >= 4.0
                && self.ui_cursor[0] < win_w - 4.0
                && self.ui_cursor[1] >= row_y
                && self.ui_cursor[1] < row_y + row_h;
            if self.ui_left_just_pressed {
                self.console_focus = in_row;
            }
            let bg = if self.console_focus {
                [22, 26, 38, 255]
            } else {
                [18, 18, 24, 255]
            };
            dl.push_rect(4.0, row_y, win_w - 8.0, row_h, uidraw::SOLID, bg);
            let bc = if self.console_focus {
                [110, 150, 220, 255u8]
            } else {
                [58, 58, 74, 255]
            };
            dl.push_line(4.0, row_y, win_w - 4.0, row_y, 1.0, bc);
            dl.push_line(4.0, row_y + row_h, win_w - 4.0, row_y + row_h, 1.0, bc);
            let mut tx = 10.0;
            let prompt = format!("> {}", self.console_input);
            for c in prompt.chars() {
                if tx + 8.0 > win_w - 12.0 {
                    break;
                }
                if c != ' ' {
                    let uv = font::glyph_rect(c);
                    if uv != [0f32; 4] {
                        dl.push_rect(tx, row_y + 3.0, 8.0, 16.0, uv, [210, 215, 228, 255]);
                    }
                }
                tx += 8.0;
            }
            if self.console_focus {
                dl.push_rect(
                    tx + 1.0,
                    row_y + 3.0,
                    2.0,
                    16.0,
                    uidraw::SOLID,
                    [180, 200, 240, 255],
                );
            }
        } else {
            // Content browser — asset tiles from the scanned model paths.
            // Click a tile to spawn it into the scene.
            use dumpster_fire_engine::resource_manager::ui_manager::font::IconId;
            let cb_y = by + TITLEBAR_H + 8.0;
            let tile_w = 116.0_f32;
            let tile_h = 64.0_f32;
            let per_row = (((win_w - 16.0) / (tile_w + 8.0)) as usize).max(1);
            if self.picker_paths.is_empty() {
                let mut tx = 8.0_f32;
                for c in "No assets found under assets/models".chars() {
                    if c != ' ' {
                        let uv = font::glyph_rect(c);
                        if uv != [0f32; 4] {
                            dl.push_rect(tx, cb_y, 8.0, 16.0, uv, [150, 150, 170, 255]);
                        }
                    }
                    tx += 8.0;
                }
            }
            for (i, path) in self.picker_paths.iter().enumerate() {
                let col = i % per_row;
                let row = i / per_row;
                let tx0 = 8.0 + col as f32 * (tile_w + 8.0);
                let ty0 = cb_y + row as f32 * (tile_h + 8.0);
                if ty0 + tile_h > win_h - 4.0 {
                    break;
                }
                let hov = self.ui_cursor[0] >= tx0
                    && self.ui_cursor[0] < tx0 + tile_w
                    && self.ui_cursor[1] >= ty0
                    && self.ui_cursor[1] < ty0 + tile_h;
                let bg: [u8; 4] = if hov {
                    [48, 60, 92, 255]
                } else {
                    [32, 32, 44, 255]
                };
                dl.push_rect(tx0, ty0, tile_w, tile_h, uidraw::SOLID, bg);
                dl.push_rect(
                    tx0,
                    ty0 + tile_h - 1.0,
                    tile_w,
                    1.0,
                    uidraw::SOLID,
                    [58, 58, 76, 255],
                );
                // 32px mesh icon, centered in the upper tile half
                dl.push_rect(
                    tx0 + (tile_w - 32.0) * 0.5,
                    ty0 + 4.0,
                    32.0,
                    32.0,
                    font::icon_rect(IconId::Box),
                    if hov {
                        [170, 210, 255, 255]
                    } else {
                        [120, 170, 120, 255]
                    },
                );
                // file name (no dir, no extension), truncated to the tile
                let name = path.rsplit('/').next().unwrap_or(path.as_ref());
                let name = name.strip_suffix(".glb").unwrap_or(name);
                let max_chars = ((tile_w - 8.0) / 8.0) as usize;
                let mut nx = tx0 + 4.0;
                for c in name.chars().take(max_chars) {
                    if c != ' ' {
                        let uv = font::glyph_rect(c);
                        if uv != [0f32; 4] {
                            dl.push_rect(nx, ty0 + 42.0, 8.0, 16.0, uv, [195, 200, 214, 255]);
                        }
                    }
                    nx += 8.0;
                }
                if hov && self.ui_left_just_pressed {
                    spawn_asset = Some(Arc::clone(path));
                }
            }
        }

        if let Some(t) = clicked_tab {
            self.bottom_tab = t;
        }
        if let Some(path) = spawn_asset {
            if let Some(win) = self.win
                && let Ok(asset) = ctx.load_gltf(win, PathBuf::from(path.as_ref()))
            {
                self.do_spawn_mesh(asset, Arc::clone(&path));
                self.push_log(
                    format!("LogContent: spawned {}", path),
                    [140, 200, 140, 255],
                );
            } else {
                self.push_log(
                    format!("LogContent: failed to load {}", path),
                    [220, 140, 120, 255],
                );
            }
        }
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
                self.do_spawn_mesh(asset, Arc::clone(&path));
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
                if self.snap_enabled {
                    let a = drag.axis as usize;
                    let s = self.snap_translate.max(1e-4);
                    let mut tr = glam::Vec3::from(nl.translation);
                    tr[a] = (tr[a] / s).round() * s; // snap dragged axis to grid
                    nl.translation = glam::Vec3A::from(tr);
                }
                nl
            }
            GizmoMode::Scale => {
                let arrow_len_sq = drag.arrow_screen[0].powi(2) + drag.arrow_screen[1].powi(2);
                if arrow_len_sq < 1.0 {
                    return;
                }
                let t = (dx * drag.arrow_screen[0] + dy * drag.arrow_screen[1]) / arrow_len_sq;
                let mut factor = (1.0 + t).max(0.01);
                if self.snap_enabled {
                    let s = self.snap_scale.max(1e-4);
                    factor = ((factor / s).round() * s).max(s); // snap scale to steps
                }
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
                let mut angle = cur_a - start_a;
                if self.snap_enabled {
                    let step = self.snap_rotate_deg.to_radians().max(1e-4);
                    angle = (angle / step).round() * step; // snap rotation to fixed angles
                }
                let rot = glam::Quat::from_axis_angle(axis, angle);
                let mut nl = drag.start_local;
                nl.matrix3 = glam::Mat3A::from_quat(rot) * drag.start_local.matrix3;
                nl
            }
        };
        self.world.set_actor_local(lh, sh, drag.actor, new_local);
    }
}

// ── Tools: frame-selected, duplicate, stats overlay ─────────────────────────

impl EditorApp {
    /// F — move/orbit all viewport cameras to focus the selected actor.
    fn frame_selected(&mut self, ctx: &mut AppCtx<'_>, app: AppHandle) {
        let Some(ah) = self.world.selection else { return };
        let Some((lh, sh)) = self.main_stage else { return };
        let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh)) else {
            return;
        };
        let Some(world_t) = stage.worlds.get(ah.idx as usize) else { return };
        let c = Vec3::from(world_t.translation);
        let r = 2.0_f32; // focus box half-extent around the actor origin
        let aabb = ([c.x - r, c.y - r, c.z - r], [c.x + r, c.y + r, c.z + r]);
        ctx.fit_all_panes_to_aabb(app, &aabb);
    }

    /// Ctrl+D — clone the selected actor (transform + sub-entities) and select
    /// the copy. Covers the kinds the editor creates: mesh `Environment` and
    /// `Utility` (lights/empties); a light's `UtilityComponent` is recreated.
    fn duplicate_selected(&mut self) {
        let Some(src) = self.world.selection else { return };
        let Some((lh, sh)) = self.main_stage else { return };

        // Read the source actor first (immutable), then spawn (mutable).
        let (local, subs) = {
            let Some(stage) = self.world.levels.get(lh).and_then(|l| l.stages.get(sh)) else {
                return;
            };
            let Some(actor) = stage.actors.get(src) else { return };
            let local = stage
                .locals
                .get(src.idx as usize)
                .copied()
                .unwrap_or(Affine3A::IDENTITY);
            let mut subs: ThinVec<(ActorType, Option<LightData>)> = ThinVec::new();
            for se in actor.sub_entities.iter().flatten() {
                let at = match &se.actor_type {
                    ActorType::Environment(e) => ActorType::Environment(Environment {
                        id: e.id,
                        name: Arc::clone(&e.name),
                        visible: e.visible,
                        physical: e.physical,
                        mesh: e.mesh.as_ref().map(|m| MeshRef { asset: m.asset }),
                    }),
                    ActorType::Utility(u) => ActorType::Utility(Utility {
                        id: u.id,
                        name: Arc::clone(&u.name),
                        visible: u.visible,
                        toggle: u.toggle,
                        mesh: u.mesh.as_ref().map(|m| MeshRef { asset: m.asset }),
                    }),
                    _ => continue,
                };
                let light = match &se.components[ComponentType::Utility.index()] {
                    Some(Component::Utility(uc)) => uc.light.clone(),
                    _ => None,
                };
                subs.push((at, light));
            }
            (local, subs)
        };
        if subs.is_empty() {
            return;
        }

        // Offset the copy so it's visibly distinct from the original.
        let offset = Affine3A::from_translation(Vec3::new(0.5, 0.0, 0.5)) * local;
        let id = ActorId::new(self.actors.len() as i64 + 500);
        let Some(ah) = self.world.spawn_actor(lh, sh, id, offset) else { return };

        for (at, light) in subs {
            let is_util_light = matches!(&at, ActorType::Utility(_)) && light.is_some();
            let _ = self.world.spawn_sub_entity(lh, sh, ah, at, Affine3A::IDENTITY);
            if is_util_light {
                let utility_idx = ActorType::Utility(Utility {
                    id: UtilityId::new(0),
                    name: Arc::from(""),
                    visible: true,
                    toggle: true,
                    mesh: None,
                })
                .index();
                self.world.add_component(
                    lh,
                    sh,
                    ah,
                    utility_idx,
                    UtilityComponent {
                        name: Arc::from("Light"),
                        description: Arc::from(""),
                        camera: None,
                        light,
                        render: None,
                    },
                );
            }
        }
        self.actors.push(ah);
        self.world.selection = Some(ah);
    }

    /// F2 — a small heads-up overlay with frame/scene stats.
    fn draw_stats(&mut self, win_w: f32) {
        if !self.stats_open {
            return;
        }
        use dumpster_fire_engine::resource_manager::ui_manager::{immediate::Ui, layout::Rect};
        let w = 210.0_f32;
        let h = 176.0_f32;
        let x = (win_w - self.inspector_w - w - 14.0).max(8.0);
        let y = TOOLBAR_H + 12.0;
        let frame_ms = if self.fps_display > 0.0 { 1000.0 / self.fps_display } else { 0.0 };
        let sel = self.world.selection.map(|hh| hh.idx as i64).unwrap_or(-1);
        let lines = [
            format!("FPS {:.0}  ({frame_ms:.2} ms)", self.fps_display),
            format!("actors {}", self.actors.len()),
            format!("selection {sel}"),
            format!("gizmo {:?}", self.gizmo_mode),
            format!("snap {}", if self.snap_enabled { "ON" } else { "off" }),
            "[F2] hide  [X] snap".to_string(),
        ];
        let input = self.ui_input();
        let dl = &mut self.world.ui.draw_list;
        dl.push_panel_bg(x, y, w, h, [18, 18, 26, 235]);
        dl.push_title_bar(x, y, w, TITLEBAR_H, [40, 40, 58, 255], SEP);
        let mut ui = Ui::with_input(
            dl,
            Rect { x: x + 8.0, y: y + TITLEBAR_H + 4.0, w: w - 16.0, h: h - TITLEBAR_H - 8.0 },
            input,
        );
        ui.label("Stats");
        for ln in &lines {
            ui.label(ln.as_str());
        }
    }
}

// ── Mesh edit mode ──────────────────────────────────────────────────────────

/// Cached focused-viewport projection data for edit-mode picking/drawing.
struct EditView {
    vp: Mat4,
    px: f32,
    py: f32,
    pw: f32,
    ph: f32,
    cam_pos: Vec3,
    is_ortho: bool,
}

impl EditorApp {
    /// Toggle edit mode for the selected mesh actor (reloading its glTF geometry),
    /// falling back to a unit cube when no editable source is available.
    fn toggle_edit_mode(&mut self) {
        if self.edit.is_some() {
            self.edit = None;
            self.edit_drag = None;
            return;
        }
        let sess = self
            .selected_mesh_path()
            .and_then(|p| Self::load_edit_session(&p))
            .unwrap_or_else(|| EditSession::cube(1.0));
        self.edit = Some(sess);
    }

    /// Source path of the selected actor's mesh, if it names a real file.
    fn selected_mesh_path(&self) -> Option<Arc<str>> {
        let ah = self.world.selection?;
        let (lh, sh) = self.main_stage?;
        let stage = self.world.levels.get(lh)?.stages.get(sh)?;
        let actor = stage.actors.get(ah)?;
        for se in actor.sub_entities.iter().flatten() {
            if let ActorType::Environment(e) = &se.actor_type {
                if FsPath::new(e.name.as_ref()).is_file() {
                    return Some(Arc::clone(&e.name));
                }
            }
        }
        None
    }

    /// Load the first triangle primitive of a glTF file into an edit session.
    fn load_edit_session(path: &str) -> Option<EditSession> {
        let asset = GltfAsset::load(path).ok()?;
        for m in &asset.meshes {
            for p in &m.primitives {
                if p.topology == PrimitiveTopology::Triangles
                    && !p.indices.is_empty()
                    && !p.streams.positions.is_empty()
                {
                    if let Some(s) = EditSession::from_indexed(&p.streams.positions, &p.indices) {
                        return Some(s);
                    }
                }
            }
        }
        None
    }

    fn edit_view(&self, ctx: &AppCtx<'_>, app: AppHandle) -> Option<EditView> {
        let grid = ctx.viewport_grid(app)?;
        let vp = grid.get(grid.focused)?;
        let cam = ctx.cameras.get(vp.camera_handle)?;
        let (win_w, win_h) = (grid.win_w, grid.win_h);
        let aspect = vp.rect.pixel_aspect(win_w, win_h);
        Some(EditView {
            vp: Mat4::from_cols_array(&cam.view_projection_matrix(aspect)),
            px: vp.rect.x * win_w,
            py: vp.rect.y * win_h,
            pw: vp.rect.w * win_w,
            ph: vp.rect.h * win_h,
            cam_pos: Vec3::from_array(cam.position),
            is_ortho: matches!(cam.projection, Some(ProjectionMode::Orthographic { .. })),
        })
    }

    /// Box-select vertices inside the screen rectangle `a..b` (Shift adds to
    /// the current selection). Uses the focused pane's projection.
    fn box_select(&mut self, ctx: &AppCtx<'_>, app: AppHandle, a: [f32; 2], b: [f32; 2]) {
        let Some(ev) = self.edit_view(ctx, app) else { return };
        let lo = [a[0].min(b[0]), a[1].min(b[1])];
        let hi = [a[0].max(b[0]), a[1].max(b[1])];
        let additive = self.shift_held;
        let n = if let Some(e) = self.edit.as_mut() {
            e.box_select_screen(lo, hi, additive, |p| {
                let clip = ev.vp * Vec4::new(p[0], p[1], p[2], 1.0);
                if clip.w <= 1e-5 {
                    return None;
                }
                Some([
                    ev.px + (clip.x / clip.w * 0.5 + 0.5) * ev.pw,
                    ev.py + (clip.y / clip.w * 0.5 + 0.5) * ev.ph,
                ])
            })
        } else {
            0
        };
        self.push_log(format!("LogMesh: box-selected {n} verts."), [150, 180, 210, 255]);
    }

    fn edit_pick(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        let Some(ev) = self.edit_view(ctx, app) else { return };
        let cursor = self.ui_cursor;
        let project = |p: Vec3| -> Option<[f32; 2]> {
            let clip = ev.vp * Vec4::new(p.x, p.y, p.z, 1.0);
            if clip.w <= 1e-5 {
                return None;
            }
            Some([
                ev.px + (clip.x / clip.w * 0.5 + 0.5) * ev.pw,
                ev.py + (clip.y / clip.w * 0.5 + 0.5) * ev.ph,
            ])
        };
        let Some(mode) = self.edit.as_ref().map(|e| e.mode) else { return };
        match mode {
            ElementMode::Vertex => {
                let v = self
                    .edit
                    .as_ref()
                    .unwrap()
                    .nearest_vertex(cursor, |p| project(Vec3::from_array(p)), 14.0);
                if let Some(v) = v {
                    self.edit.as_mut().unwrap().toggle_vertex(v);
                }
            }
            ElementMode::Edge => {
                let e = self
                    .edit
                    .as_ref()
                    .unwrap()
                    .nearest_edge(cursor, |p| project(Vec3::from_array(p)), 10.0);
                if let Some(e) = e {
                    self.edit.as_mut().unwrap().toggle_edge(e);
                }
            }
            ElementMode::Face => {
                let lx = (cursor[0] - ev.px) / ev.pw;
                let ly = (cursor[1] - ev.py) / ev.ph;
                if !(0.0..=1.0).contains(&lx) || !(0.0..=1.0).contains(&ly) {
                    return;
                }
                let ndc = Vec4::new(lx * 2.0 - 1.0, ly * 2.0 - 1.0, -1.0, 1.0);
                let inv = ev.vp.inverse();
                let near = inv * ndc;
                let far = inv * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
                let nw = near.xyz() / near.w;
                let fw = far.xyz() / far.w;
                let dir = (fw - nw).normalize();
                let org = if ev.is_ortho { nw } else { ev.cam_pos };
                let f = self
                    .edit
                    .as_ref()
                    .unwrap()
                    .pick_face([org.x, org.y, org.z], [dir.x, dir.y, dir.z]);
                if let Some(f) = f {
                    self.edit.as_mut().unwrap().toggle_face(f);
                }
            }
        }
    }

    fn start_edit_drag(&mut self, ctx: &AppCtx<'_>, app: AppHandle) -> Option<EditDrag> {
        let ev = self.edit_view(ctx, app)?;
        let center = Vec3::from_array(self.edit.as_ref()?.selection_centroid()?);
        let project = |p: Vec3| -> Option<[f32; 2]> {
            let clip = ev.vp * Vec4::new(p.x, p.y, p.z, 1.0);
            if clip.w <= 1e-5 {
                return None;
            }
            Some([
                ev.px + (clip.x / clip.w * 0.5 + 0.5) * ev.pw,
                ev.py + (clip.y / clip.w * 0.5 + 0.5) * ev.ph,
            ])
        };
        let origin_s = project(center)?;
        let len = 0.15 * (ev.cam_pos - center).length().max(0.5);
        let cursor = self.ui_cursor;
        let mut best: Option<(u8, [f32; 2])> = None;
        let mut bd = 22.0_f32;
        for (i, axis) in [Vec3::X, Vec3::Y, Vec3::Z].iter().enumerate() {
            let Some(tip) = project(center + *axis * len) else { continue };
            let d = point_to_segment(cursor, origin_s, tip);
            if d < bd {
                bd = d;
                best = Some((i as u8, [tip[0] - origin_s[0], tip[1] - origin_s[1]]));
            }
        }
        let (axis_i, arrow_screen) = best?;
        self.edit.as_mut().unwrap().begin_transform();
        Some(EditDrag { axis: axis_i, start_cursor: cursor, arrow_screen, arrow_world_len: len })
    }

    fn apply_edit_drag(&mut self) {
        let Some(drag) = self.edit_drag else { return };
        let cursor = self.ui_cursor;
        let dx = cursor[0] - drag.start_cursor[0];
        let dy = cursor[1] - drag.start_cursor[1];
        let axis = match drag.axis {
            0 => Vec3::X,
            1 => Vec3::Y,
            _ => Vec3::Z,
        };
        let len2 = drag.arrow_screen[0].powi(2) + drag.arrow_screen[1].powi(2);
        if len2 < 1.0 {
            return;
        }
        let t = (dx * drag.arrow_screen[0] + dy * drag.arrow_screen[1]) / len2;
        let mut wd = axis * (t * drag.arrow_world_len);
        if self.snap_enabled {
            let s = self.snap_translate.max(1e-4);
            let a = drag.axis as usize;
            let mut v = [wd.x, wd.y, wd.z];
            v[a] = (v[a] / s).round() * s;
            wd = Vec3::new(v[0], v[1], v[2]);
        }
        if let Some(e) = self.edit.as_mut() {
            e.update_transform([wd.x, wd.y, wd.z]);
        }
    }

    fn draw_edit_overlay(&mut self, ctx: &AppCtx<'_>, app: AppHandle) {
        if self.edit.is_none() {
            return;
        }
        let Some(ev) = self.edit_view(ctx, app) else { return };
        let project = |p: Vec3| -> Option<[f32; 2]> {
            let clip = ev.vp * Vec4::new(p.x, p.y, p.z, 1.0);
            if clip.w <= 1e-5 {
                return None;
            }
            Some([
                ev.px + (clip.x / clip.w * 0.5 + 0.5) * ev.pw,
                ev.py + (clip.y / clip.w * 0.5 + 0.5) * ev.ph,
            ])
        };
        let sess = self.edit.as_ref().unwrap();
        let dl = &mut self.world.ui.draw_list;
        // Wireframe edges (selected edges highlighted).
        for &(a, b) in sess.edges() {
            let pa = sess.positions()[a as usize];
            let pb = sess.positions()[b as usize];
            if let (Some(sa), Some(sb)) =
                (project(Vec3::from_array(pa)), project(Vec3::from_array(pb)))
            {
                let sel = sess.is_vertex_selected(a) && sess.is_vertex_selected(b);
                let col = if sel { [255, 180, 60, 255] } else { [120, 120, 150, 200] };
                dl.push_line(sa[0], sa[1], sb[0], sb[1], if sel { 2.0 } else { 1.0 }, col);
            }
        }
        // Vertex handles.
        for v in 0..sess.vertex_count() as u32 {
            if let Some(s) = project(Vec3::from_array(sess.positions()[v as usize])) {
                let selv = sess.is_vertex_selected(v);
                let sz = if selv { 7.0 } else { 4.0 };
                let col = if selv { [255, 210, 80, 255] } else { [180, 180, 200, 220] };
                dl.push_rect(s[0] - sz * 0.5, s[1] - sz * 0.5, sz, sz, [0.0, 0.0, 1.0, 1.0], col);
            }
        }
        // Translate gizmo at the selection centroid.
        if let Some(c) = sess.selection_centroid() {
            let center = Vec3::from_array(c);
            if let Some(origin_s) = project(center) {
                let len = 0.15 * (ev.cam_pos - center).length().max(0.5);
                let axes = [
                    (Vec3::X, [220, 60, 60, 255u8]),
                    (Vec3::Y, [70, 200, 70, 255]),
                    (Vec3::Z, [70, 100, 220, 255]),
                ];
                for (axis, col) in axes {
                    if let Some(tip) = project(center + axis * len) {
                        dl.push_line(origin_s[0], origin_s[1], tip[0], tip[1], 3.0, col);
                    }
                }
            }
        }

        // Rubber-band rectangle for an in-progress box select.
        if self.box_sel_active
            && let Some(start) = self.box_sel_start
        {
            let c = self.ui_cursor;
            let (x0, y0) = (start[0].min(c[0]), start[1].min(c[1]));
            let (x1, y1) = (start[0].max(c[0]), start[1].max(c[1]));
            let col = [120, 180, 240, 200u8];
            let dl = &mut self.world.ui.draw_list;
            dl.push_line(x0, y0, x1, y0, 1.0, col);
            dl.push_line(x0, y1, x1, y1, 1.0, col);
            dl.push_line(x0, y0, x0, y1, 1.0, col);
            dl.push_line(x1, y0, x1, y1, 1.0, col);
        }
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
        outliner_w: 220.0,
        inspector_w: 260.0,
        div_drag: None,
        div_hover: None,
        grid_enabled: true,
        gizmo_mode: GizmoMode::Translate,
        gizmo_space: GizmoSpace::World,
        spawn_menu_open: false,
        light_submenu_open: false,
        snap_enabled: false,
        snap_translate: 0.25,
        snap_rotate_deg: 15.0,
        snap_scale: 0.1,
        stats_open: false,
        ctrl_held: false,
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
        edit: None,
        edit_drag: None,
        shift_held: false,
        pending_tooltip: None,
        menu_open: None,
        menu_rect: (0.0, 0.0, 0.0, 0.0),
        bottom_h: 140.0,
        quit_requested: false,
        insp_drag: None,
        bottom_tab: 0,
        saved_layout: None,
        console_input: String::new(),
        console_focus: false,
        rename_target: None,
        rename_buf: String::new(),
        last_row_click: None,
        extrude_offset: 0.5,
        inset_amount: 0.2,
        box_sel_start: None,
        box_sel_active: false,
        log: {
            let mut l = ThinVec::new();
            l.push(("LogInit: Engine initialized.".to_string(), [140, 200, 140, 255u8]));
            l.push(("LogRenderer: Overlay pipeline online.".to_string(), [130, 180, 220, 255]));
            l.push(("LogEditor: Scene loaded.".to_string(), [180, 180, 180, 255]));
            l.push(("LogEditor: Ready.".to_string(), [100, 140, 100, 255]));
            l
        },
    })
    .run()
}
