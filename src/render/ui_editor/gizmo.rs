use crate::render::ui_render::DrawList;
use crate::render::ui_core::layout::Rect;
use crate::resource_manager::world_manager::World;
use crate::resource_manager::manager::ActorHandle;

pub struct TransformGizmo {
    drag: Option<GizmoDrag>,
}

struct GizmoDrag {
    actor: ActorHandle,
    start: [f32; 3],
    axis: u8,
}

impl TransformGizmo {
    pub fn new() -> Self {
        Self { drag: None }
    }

    pub fn draw(
        &mut self,
        dl: &mut DrawList,
        _world: &World,
        _viewport_rect: Rect,
        _mouse: [f32; 2],
        _left_down: bool,
    ) -> bool {
        let colors = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
        let origin = [100.0, 100.0];
        let length = 50.0;

        dl.push_line(origin, [origin[0] + length, origin[1]], 2.0, colors[0]);
        dl.push_line(origin, [origin[0], origin[1] + length], 2.0, colors[1]);
        dl.push_line(
            origin,
            [origin[0] + length * 0.7, origin[1] + length * 0.7],
            2.0,
            colors[2],
        );

        false
    }
}

impl Default for TransformGizmo {
    fn default() -> Self {
        Self::new()
    }
}
