use hashbrown::HashMap;
use thin_vec::ThinVec;

use crate::render::ui_core::controller::Controller;
use crate::render::ui_core::event::EventBus;
use crate::render::ui_core::id::{WidgetArena, WidgetId, WidgetIdPath};
use crate::render::ui_core::layout::{Constraint, LayoutContext, Rect, Size};
use crate::render::ui_core::widget::{DirtyFlags, Widget};
use crate::resource_manager::world_manager::World;

pub struct UiManager {
    pub widgets: WidgetArena,
    pub root: Option<WidgetId>,
    pub event_bus: EventBus,
    pub controllers: ThinVec<Box<dyn Controller>>,
    viewport_rect: Rect,
    path_to_id: HashMap<String, WidgetId>,
}

impl UiManager {
    pub fn new(viewport_rect: Rect) -> Self {
        Self {
            widgets: WidgetArena::new(),
            root: None,
            event_bus: EventBus::new(),
            controllers: ThinVec::new(),
            viewport_rect,
            path_to_id: HashMap::new(),
        }
    }

    pub fn register_controller(&mut self, ctrl: Box<dyn Controller>) {
        self.controllers.push(ctrl);
    }

    pub fn set_root(&mut self, root: WidgetId) {
        self.root = Some(root);
    }

    pub fn set_viewport(&mut self, rect: Rect) {
        self.viewport_rect = rect;
    }

    pub fn get_widget_by_path(&mut self, path: &str) -> Option<WidgetId> {
        self.path_to_id.get(path).copied()
    }

    pub fn register_widget_path(&mut self, path: String, id: WidgetId) {
        self.path_to_id.insert(path, id);
    }

    pub fn mark_widget_dirty(&mut self, id: WidgetId, flags: DirtyFlags) {
        if let Some(w) = self.widgets.get_mut(id) {
            w.dirty |= flags as u8;
        }
        let mut cur = id;
        while let Some(parent) = self.widgets.get(cur).and_then(|w| w.parent) {
            if let Some(p) = self.widgets.get_mut(parent) {
                p.dirty |= (DirtyFlags::LAYOUT | DirtyFlags::CHILDREN) as u8;
            }
            cur = parent;
        }
    }

    pub fn layout(&mut self) {
        if let Some(root) = self.root {
            let mut ctx = LayoutContext::new();
            let constraint = Constraint {
                min_width: 0.0,
                max_width: f32::INFINITY,
                min_height: 0.0,
                max_height: f32::INFINITY,
            };
            self.measure(root, &mut ctx, constraint);
            self.arrange(root, self.viewport_rect);
        }
    }

    fn measure(&mut self, id: WidgetId, ctx: &mut LayoutContext, constraint: Constraint) -> Size {
        let w = self.widgets.get(id).expect("Widget not found");
        let child_constraints: ThinVec<Constraint> =
            w.children.iter().map(|_| constraint).collect();
        let child_refs: ThinVec<(WidgetId, Constraint)> =
            w.children.iter().copied().zip(child_constraints).collect();
        let size = w.layout_solver.measure(&child_refs, &self.widgets, ctx);
        ctx.set_size(id, size);
        size
    }

    fn arrange(&mut self, id: WidgetId, rect: Rect) {
        let w = self.widgets.get_mut(id).expect("Widget not found");
        w.rect = rect;
        let mut child_rects: ThinVec<(WidgetId, Rect)> = w
            .children
            .iter()
            .map(|&cid| (cid, Rect::default()))
            .collect();
        w.layout_solver
            .arrange(rect, &mut child_rects, &mut self.widgets);
        for (cid, child_rect) in child_rects {
            self.arrange(cid, child_rect);
        }
    }

    pub fn tick(&mut self, world: &mut World) {
        let events: ThinVec<_> = self.event_bus.drain().collect();
        for ev in events {
            let controller_count = self.controllers.len();
            for i in 0..controller_count {
                let ctrl = &self.controllers[i];
                ctrl.handle_event(&ev, world, self);
            }
        }
    }
}

pub fn mark_dirty(id: WidgetId, flags: DirtyFlags) {
    let _ = (id, flags);
}

impl Default for UiManager {
    fn default() -> Self {
        Self::new(Rect {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        })
    }
}
