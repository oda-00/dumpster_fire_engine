use std::sync::Arc;

use thin_vec::ThinVec;

use crate::render::ui_core::controller::Controller;
use crate::render::ui_core::event::EventBus;
use crate::render::ui_core::id::{WidgetArena, WidgetId, WidgetIdPath};
use crate::render::ui_core::layout::{Constraint, LayoutContext, Rect, Size};
use crate::render::ui_core::widget::DirtyFlags;
use crate::resource_manager::world_manager::World;

pub struct UiManager {
    pub widgets: WidgetArena,
    pub root: Option<WidgetId>,
    pub event_bus: EventBus,
    pub controllers: ThinVec<Arc<dyn Controller>>,
    viewport_rect: Rect,
    /// Path → WidgetId mapping. Sorted by path so lookup uses
    /// `partition_point` (O(log N), same pattern as ScriptManager::id_to_handle).
    path_to_id: ThinVec<(String, WidgetId)>,
}

impl UiManager {
    pub fn new(viewport_rect: Rect) -> Self {
        Self {
            widgets: WidgetArena::new(),
            root: None,
            event_bus: EventBus::new(),
            controllers: ThinVec::new(),
            viewport_rect,
            path_to_id: ThinVec::new(),
        }
    }

    pub fn register_controller(&mut self, ctrl: Arc<dyn Controller>) {
        self.controllers.push(ctrl);
    }

    pub fn set_root(&mut self, root: WidgetId) {
        self.root = Some(root);
    }

    pub fn set_viewport(&mut self, rect: Rect) {
        self.viewport_rect = rect;
    }

    #[inline]
    pub fn viewport_width(&self) -> f32 {
        self.viewport_rect.w
    }

    #[inline]
    pub fn viewport_height(&self) -> f32 {
        self.viewport_rect.h
    }

    pub fn get_widget_by_path(&self, path: &str) -> Option<WidgetId> {
        let pos = self.path_to_id.partition_point(|(p, _)| p.as_str() < path);
        self.path_to_id
            .get(pos)
            .filter(|(p, _)| p.as_str() == path)
            .map(|(_, id)| *id)
    }

    pub fn register_widget_path(&mut self, path: String, id: WidgetId) {
        let pos = self.path_to_id.partition_point(|(p, _)| p.as_str() < path.as_str());
        if self.path_to_id.get(pos).map(|(p, _)| p.as_str()) == Some(path.as_str()) {
            self.path_to_id[pos].1 = id; // update existing entry
        } else {
            self.path_to_id.insert(pos, (path, id));
        }
    }

    #[inline]
    pub fn mark_dirty(&mut self, id: WidgetId, flags: DirtyFlags) {
        self.mark_widget_dirty(id, flags);
    }

    pub fn mark_widget_dirty(&mut self, id: WidgetId, flags: DirtyFlags) {
        if let Some(w) = self.widgets.get_mut(id) {
            w.dirty |= flags as u8;
        }
        let mut cur = id;
        while let Some(parent) = self.widgets.get(cur).and_then(|w| w.parent) {
            if let Some(p) = self.widgets.get_mut(parent) {
                p.dirty |= DirtyFlags::LAYOUT as u8 | DirtyFlags::CHILDREN as u8;
            }
            cur = parent;
        }
    }

    pub fn layout(&mut self) {
        if let Some(root) = self.root {
            let mut ctx = LayoutContext::new();
            let constraint = Constraint {
                min_width: 0.0,
                max_width: self.viewport_rect.w,
                min_height: 0.0,
                max_height: self.viewport_rect.h,
            };
            self.measure(root, &mut ctx, constraint);
            self.arrange(root, self.viewport_rect);
        }
    }

    fn measure(&mut self, id: WidgetId, ctx: &mut LayoutContext, constraint: Constraint) -> Size {
        let Some(w) = self.widgets.get(id) else { return Size { w: 0.0, h: 0.0 } };
        let child_constraints: ThinVec<Constraint> =
            w.children.iter().map(|_| constraint).collect();
        let child_refs: ThinVec<(WidgetId, Constraint)> =
            w.children.iter().copied().zip(child_constraints).collect();
        let size = w.layout_solver.measure(&child_refs, &self.widgets, ctx);
        ctx.set_size(id, size);
        size
    }

    fn arrange(&mut self, id: WidgetId, rect: Rect) {
        // Set rect and collect children — then release the mutable borrow before
        // calling layout_solver.arrange (which needs &WidgetArena for child sizes).
        let children: ThinVec<WidgetId> = {
            let Some(w) = self.widgets.get_mut(id) else { return };
            w.rect = rect;
            w.children.clone()
        };

        let mut child_rects: ThinVec<(WidgetId, Rect)> =
            children.iter().map(|&cid| (cid, Rect::default())).collect();

        if let Some(w) = self.widgets.get(id) {
            w.layout_solver.arrange(rect, &mut child_rects, &self.widgets);
        }

        for (cid, child_rect) in child_rects {
            self.arrange(cid, child_rect);
        }
    }

    pub fn tick(&mut self, world: &mut World) {
        let events: ThinVec<_> = self.event_bus.drain().collect();
        // Clone Arcs before the loop so we don't hold a borrow on self.controllers
        // while passing &mut self to handle_event.
        let controllers: ThinVec<Arc<dyn Controller>> =
            self.controllers.iter().cloned().collect();
        for ev in &events {
            for ctrl in &controllers {
                ctrl.handle_event(ev, world, self);
            }
        }
    }
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
