use std::sync::Arc;

use thin_vec::ThinVec;

use crate::render::ui_core::controller::Controller;
use crate::render::ui_core::event::EventBus;
use crate::render::ui_core::id::{WidgetArena, WidgetId};
use crate::render::ui_core::layout::{Constraint, LayoutContext, LayoutSolver, Rect, Size};
use crate::render::ui_core::widget::DirtyFlags;
use crate::resource_manager::world_manager::World;

pub struct UiManager {
    pub widgets: WidgetArena,
    pub root: Option<WidgetId>,
    pub event_bus: EventBus,
    pub controllers: ThinVec<Arc<dyn Controller>>,
    viewport_rect: Rect,
    /// Persistent layout measurement cache, reused across frames so clean
    /// subtrees are served from cache instead of re-measured (GUI_research.md §3.2).
    layout_cache: LayoutContext,
    /// Call-site key → WidgetId mapping. Keyed by a `u64` path hash
    /// (`id::path_key`) instead of an allocated `String`, eliminating the
    /// per-frame path allocation the immediate builder used to pay for every
    /// widget (GUI_research.md §4.2). Sorted by key so lookup uses
    /// `partition_point` (O(log N), same pattern as ScriptManager::id_to_handle).
    key_to_id: ThinVec<(u64, WidgetId)>,
}

impl UiManager {
    pub fn new(viewport_rect: Rect) -> Self {
        Self {
            widgets: WidgetArena::new(),
            root: None,
            event_bus: EventBus::new(),
            controllers: ThinVec::new(),
            viewport_rect,
            layout_cache: LayoutContext::new(),
            key_to_id: ThinVec::new(),
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

    /// Look up an immediate-mode widget by its call-site key (`id::path_key`).
    pub fn get_widget_by_key(&self, key: u64) -> Option<WidgetId> {
        let pos = self.key_to_id.partition_point(|(k, _)| *k < key);
        self.key_to_id
            .get(pos)
            .filter(|(k, _)| *k == key)
            .map(|(_, id)| *id)
    }

    /// Register (or update) the WidgetId for a call-site key, keeping
    /// `key_to_id` sorted for `partition_point` lookup.
    pub fn register_widget_key(&mut self, key: u64, id: WidgetId) {
        let pos = self.key_to_id.partition_point(|(k, _)| *k < key);
        if self.key_to_id.get(pos).map(|(k, _)| *k) == Some(key) {
            self.key_to_id[pos].1 = id; // update existing entry
        } else {
            self.key_to_id.insert(pos, (key, id));
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
            let constraint = Constraint {
                min_width: 0.0,
                max_width: self.viewport_rect.w,
                min_height: 0.0,
                max_height: self.viewport_rect.h,
            };
            // Take the persistent cache out so `measure` can borrow `self`
            // mutably; restore it afterwards (the engine's mem::take scratch idiom).
            let mut ctx = std::mem::take(&mut self.layout_cache);
            self.measure(root, &mut ctx, constraint);
            self.layout_cache = ctx;
            self.arrange(root, self.viewport_rect);
        }
    }

    fn measure(&mut self, id: WidgetId, ctx: &mut LayoutContext, constraint: Constraint) -> Size {
        // Dirty-subtree skip: `mark_widget_dirty` propagates LAYOUT dirtiness up
        // to ancestors, so a node without the LAYOUT flag has no dirty descendants
        // and its cached size (measured under the same constraint) is still valid.
        let is_dirty = match self.widgets.get(id) {
            Some(w) => w.dirty & DirtyFlags::LAYOUT as u8 != 0,
            None => return Size { w: 0.0, h: 0.0 },
        };
        if !is_dirty {
            if let Some(cached) = ctx.cached(id, &constraint) {
                return cached;
            }
        }

        let Some(w) = self.widgets.get(id) else { return Size { w: 0.0, h: 0.0 } };
        let child_constraints: ThinVec<Constraint> =
            w.children.iter().map(|_| constraint).collect();
        let child_refs: ThinVec<(WidgetId, Constraint)> =
            w.children.iter().copied().zip(child_constraints).collect();
        let size = w.layout_solver.measure(&child_refs, &self.widgets, ctx);
        ctx.record(id, constraint, size);

        // Freshly measured — clear the LAYOUT dirty bit so next frame is a cache hit
        // unless something re-dirties this subtree.
        if let Some(w) = self.widgets.get_mut(id) {
            w.dirty &= !(DirtyFlags::LAYOUT as u8);
        }
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
