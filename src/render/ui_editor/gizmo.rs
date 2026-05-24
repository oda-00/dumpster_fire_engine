use glam::Affine3A;

use crate::render::ui_core::layout::Rect;
use crate::render::ui_core::theme::Theme;
use crate::render::ui_render::DrawList;
use crate::resource_manager::manager::{ActorHandle, LevelHandle, StageHandle};
use crate::resource_manager::world_manager::World;

const HANDLE_RADIUS: f32 = 6.0;
const AXIS_LEN: f32 = 60.0;
const AXIS_THICKNESS: f32 = 2.5;

pub struct TransformGizmo {
    drag: Option<GizmoDrag>,
    theme: Theme,
}

struct GizmoDrag {
    actor: ActorHandle,
    level: LevelHandle,
    stage: StageHandle,
    /// Screen-space position where the drag started.
    start: [f32; 2],
    /// Axis index: 0 = X (right), 1 = Y (down), 2 = Z (diagonal).
    axis: u8,
    /// World-space translation of the actor at drag start.
    origin: [f32; 3],
}

impl TransformGizmo {
    pub fn new(theme: Theme) -> Self {
        Self { drag: None, theme }
    }

    /// Draw the gizmo and process mouse interaction.
    ///
    /// Returns `true` when the actor's transform was changed this frame.
    /// The caller must call [`World::set_actor_local`] with the new transform —
    /// this method does so internally via the stored level/stage/actor handles.
    pub fn draw(
        &mut self,
        dl: &mut DrawList,
        world: &mut World,
        viewport_rect: Rect,
        mouse: [f32; 2],
        left_down: bool,
    ) -> bool {
        // Determine screen origin for the gizmo — centre of viewport for now;
        // a real implementation would project the actor's world position.
        let origin = [
            viewport_rect.x + viewport_rect.w * 0.5,
            viewport_rect.y + viewport_rect.h * 0.5,
        ];

        // Axis endpoints and colours from theme.
        let axes: [([f32; 2], [u8; 4]); 3] = [
            ([origin[0] + AXIS_LEN, origin[1]], self.theme.error),
            ([origin[0], origin[1] + AXIS_LEN], self.theme.secondary),
            (
                [
                    origin[0] + AXIS_LEN * 0.7,
                    origin[1] + AXIS_LEN * 0.7,
                ],
                self.theme.primary,
            ),
        ];

        // ── Mouse-down: start drag if cursor is near an axis handle ───────
        if left_down && self.drag.is_none() {
            if let Some((axis, level_h, stage_h, actor_h)) =
                self.hit_test(mouse, &axes, world)
            {
                let origin_tr = self
                    .actor_translation(world, level_h, stage_h, actor_h)
                    .unwrap_or([0.0; 3]);
                self.drag = Some(GizmoDrag {
                    actor: actor_h,
                    level: level_h,
                    stage: stage_h,
                    start: mouse,
                    axis,
                    origin: origin_tr,
                });
            }
        }

        // ── Mouse-up: clear drag ──────────────────────────────────────────
        if !left_down {
            self.drag = None;
        }

        // ── Active drag: compute and apply delta ──────────────────────────
        let mut changed = false;
        if let Some(ref d) = self.drag {
            let delta_screen = [mouse[0] - d.start[0], mouse[1] - d.start[1]];
            // Map screen delta to a world-space translation along the dragged axis.
            let scale = 0.01_f32; // pixels → world units (placeholder)
            let mut tr = d.origin;
            match d.axis {
                0 => tr[0] += delta_screen[0] * scale,
                1 => tr[1] += delta_screen[1] * scale,
                2 => {
                    let diag = (delta_screen[0] + delta_screen[1]) * 0.5 * scale;
                    tr[2] += diag;
                }
                _ => {}
            }

            // Read current transform from world and apply only the translation.
            if let Some(cur) = self.actor_local(world, d.level, d.stage, d.actor) {
                let new_tf = Affine3A {
                    matrix3: cur.matrix3,
                    translation: glam::Vec3A::from(tr),
                };
                self.apply_transform(world, d.level, d.stage, d.actor, new_tf);
                changed = true;
            }

            // Highlight the active axis.
            let (tip, _) = axes[d.axis as usize];
            let active_col = [0xFF, 0xFF, 0x00, 0xFF];
            dl.push_line(origin, tip, AXIS_THICKNESS + 1.0, active_col);
        }

        // ── Draw all three axes ───────────────────────────────────────────
        for (i, (tip, col)) in axes.iter().enumerate() {
            let thickness = if self
                .drag
                .as_ref()
                .map(|d| d.axis as usize == i)
                .unwrap_or(false)
            {
                AXIS_THICKNESS + 1.0
            } else {
                AXIS_THICKNESS
            };
            dl.push_line(origin, *tip, thickness, *col);

            // Draw a small handle square at the tip.
            let r = HANDLE_RADIUS;
            let handle_rect = crate::render::ui_core::layout::Rect {
                x: tip[0] - r,
                y: tip[1] - r,
                w: r * 2.0,
                h: r * 2.0,
            };
            dl.push_rect(handle_rect, *col);
        }

        changed
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Return (axis, level, stage, actor) if `mouse` is within `HANDLE_RADIUS`
    /// of any axis tip and a selection is active in `world`.
    fn hit_test(
        &self,
        mouse: [f32; 2],
        axes: &[([f32; 2], [u8; 4]); 3],
        world: &World,
    ) -> Option<(u8, LevelHandle, StageHandle, ActorHandle)> {
        let actor_h = world.selection?;
        let (level_h, stage_h) = self.find_actor_address(world, actor_h)?;

        for (i, (tip, _)) in axes.iter().enumerate() {
            let dx = mouse[0] - tip[0];
            let dy = mouse[1] - tip[1];
            if (dx * dx + dy * dy).sqrt() <= HANDLE_RADIUS {
                return Some((i as u8, level_h, stage_h, actor_h));
            }
        }
        None
    }

    /// Search all levels and stages to find which (LevelHandle, StageHandle)
    /// owns the given actor.
    fn find_actor_address(
        &self,
        world: &World,
        target: ActorHandle,
    ) -> Option<(LevelHandle, StageHandle)> {
        for (level_h, level) in world.levels.entries() {
            for (stage_h, stage) in level.stages.entries() {
                if stage.actors.get(target).is_some() {
                    return Some((level_h, stage_h));
                }
            }
        }
        None
    }

    fn actor_local(
        &self,
        world: &World,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
    ) -> Option<Affine3A> {
        let stage = world.levels.get(level_h)?.stages.get(stage_h)?;
        let actor = stage.actors.get(actor_h)?;
        // Use the first non-None sub-entity's local transform.
        actor.sub_entities.iter().find_map(|se| se.as_ref().map(|s| s.local))
    }

    fn actor_translation(
        &self,
        world: &World,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
    ) -> Option<[f32; 3]> {
        self.actor_local(world, level_h, stage_h, actor_h)
            .map(|tf| tf.translation.into())
    }

    fn apply_transform(
        &self,
        world: &mut World,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        tf: Affine3A,
    ) {
        if let Some(level) = world.levels.get_mut(level_h) {
            level.set_actor_local(stage_h, actor_h, tf);
        }
    }
}

impl Default for TransformGizmo {
    fn default() -> Self {
        Self::new(Theme::DARK)
    }
}
