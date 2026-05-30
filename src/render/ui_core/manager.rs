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
    /// Widget that currently holds keyboard focus, if any.
    focused: Option<WidgetId>,
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
            focused: None,
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

    // ── Focus / keyboard navigation ─────────────────────────────────────────

    #[inline]
    pub fn focused(&self) -> Option<WidgetId> {
        self.focused
    }

    /// Set focus to `id` only if it exists and is focusable; returns success.
    pub fn set_focus(&mut self, id: WidgetId) -> bool {
        let ok = self.widgets.get(id).map(|w| w.focusable).unwrap_or(false);
        if ok {
            self.focused = Some(id);
        }
        ok
    }

    pub fn clear_focus(&mut self) {
        self.focused = None;
    }

    /// Depth-first, tree-order list of focusable widget ids under `root`.
    fn focus_order(&self) -> ThinVec<WidgetId> {
        let mut order = ThinVec::new();
        if let Some(root) = self.root {
            self.collect_focusable(root, &mut order);
        }
        order
    }

    fn collect_focusable(&self, id: WidgetId, out: &mut ThinVec<WidgetId>) {
        let Some(w) = self.widgets.get(id) else { return };
        if w.focusable {
            out.push(id);
        }
        // Clone children ids to avoid holding the borrow across recursion.
        let children: ThinVec<WidgetId> = w.children.clone();
        for child in children {
            self.collect_focusable(child, out);
        }
    }

    /// Advance focus to the next focusable widget in tree order, wrapping around.
    /// With nothing focused, focuses the first. Returns the newly focused id.
    pub fn focus_next(&mut self) -> Option<WidgetId> {
        self.step_focus(1)
    }

    /// Move focus to the previous focusable widget in tree order, wrapping.
    pub fn focus_prev(&mut self) -> Option<WidgetId> {
        self.step_focus(-1)
    }

    fn step_focus(&mut self, dir: i32) -> Option<WidgetId> {
        let order = self.focus_order();
        if order.is_empty() {
            self.focused = None;
            return None;
        }
        let n = order.len();
        let next = match self.focused.and_then(|cur| order.iter().position(|&id| id == cur)) {
            Some(i) => {
                // Wrapping step by ±1 within [0, n).
                let step = dir.rem_euclid(n as i32) as usize;
                (i + step) % n
            }
            None => {
                if dir >= 0 {
                    0
                } else {
                    n - 1
                }
            }
        };
        self.focused = Some(order[next]);
        self.focused
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::ui_core::layout::LayoutDispatch;
    use crate::render::ui_core::signal::Signal;
    use crate::render::ui_core::widget::{ButtonState, LabelState, PanelState, Widget, WidgetKind};

    fn add(m: &mut UiManager, kind: WidgetKind, parent: Option<WidgetId>) -> WidgetId {
        let focusable = kind.default_focusable();
        let id = m.widgets.insert(Widget {
            id: WidgetId {
                idx: u32::MAX,
                generation: std::num::NonZeroU32::new(1).unwrap(),
                _tag: std::marker::PhantomData,
            },
            kind,
            parent,
            children: ThinVec::new(),
            dirty: 0,
            rect: Rect::default(),
            constraint: Constraint { min_width: 0.0, max_width: 0.0, min_height: 0.0, max_height: 0.0 },
            layout_solver: LayoutDispatch::Null,
            focusable,
            event_handlers: ThinVec::new(),
            user_data: None,
        });
        if let Some(w) = m.widgets.get_mut(id) {
            w.id = id;
        }
        if let Some(p) = parent {
            if let Some(pw) = m.widgets.get_mut(p) {
                pw.children.push(id);
            }
        }
        id
    }

    fn button(m: &mut UiManager, parent: Option<WidgetId>) -> WidgetId {
        add(
            m,
            WidgetKind::Button(ButtonState {
                label: Signal::new(String::new()),
                enabled: Signal::new(true),
                clicked: false,
            }),
            parent,
        )
    }

    /// root(panel) → [label, buttonA, buttonB]; only the buttons are focusable.
    fn tree() -> (UiManager, WidgetId, WidgetId) {
        let mut m = UiManager::default();
        let root = add(
            &mut m,
            WidgetKind::Panel(PanelState {
                title: Signal::new(String::new()),
                closable: false,
                close_requested: false,
            }),
            None,
        );
        m.set_root(root);
        let _label = add(
            &mut m,
            WidgetKind::Label(LabelState {
                text: Signal::new(String::new()),
                color: Signal::new([1.0; 4]),
                font_size: 14,
            }),
            Some(root),
        );
        let a = button(&mut m, Some(root));
        let b = button(&mut m, Some(root));
        (m, a, b)
    }

    #[test]
    fn focus_next_starts_at_first_focusable_and_wraps() {
        let (mut m, a, b) = tree();
        assert_eq!(m.focused(), None);
        assert_eq!(m.focus_next(), Some(a)); // skips the non-focusable label
        assert_eq!(m.focus_next(), Some(b));
        assert_eq!(m.focus_next(), Some(a)); // wraps
    }

    #[test]
    fn focus_prev_wraps_backwards() {
        let (mut m, a, b) = tree();
        assert_eq!(m.focus_prev(), Some(b)); // nothing focused → last
        assert_eq!(m.focus_prev(), Some(a));
        assert_eq!(m.focus_prev(), Some(b)); // wraps
    }

    #[test]
    fn set_focus_rejects_non_focusable_and_accepts_focusable() {
        let (mut m, a, _b) = tree();
        // The label is the only non-focusable child; grab its id by walking root.
        let root = m.root.unwrap();
        let label = m.widgets.get(root).unwrap().children[0];
        assert!(!m.set_focus(label));
        assert_eq!(m.focused(), None);
        assert!(m.set_focus(a));
        assert_eq!(m.focused(), Some(a));
        m.clear_focus();
        assert_eq!(m.focused(), None);
    }
}
