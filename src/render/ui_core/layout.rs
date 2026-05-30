use thin_vec::ThinVec;

use crate::render::ui_core::id::{WidgetArena, WidgetId};

#[derive(Copy, Clone, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn size(&self) -> Size {
        Size {
            w: self.w,
            h: self.h,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

#[derive(Copy, Clone, Debug)]
pub struct Constraint {
    pub min_width: f32,
    pub max_width: f32,
    pub min_height: f32,
    pub max_height: f32,
}

impl Constraint {
    pub fn clamp(&self, size: Size) -> Size {
        Size {
            w: size.w.max(self.min_width).min(self.max_width),
            h: size.h.max(self.min_height).min(self.max_height),
        }
    }
}

/// Per-frame layout scratch: maps WidgetId → measured Size.
///
/// Uses a direct-index ThinVec keyed by `WidgetId::idx` — the same
/// range-compressed pattern as `Play::id_lookup`.  Arena indices are dense
/// (0, 1, 2, …) so gaps are rare and the array stays small.  Cleared and
/// reused each layout pass to avoid allocations.
pub struct LayoutContext {
    sizes: ThinVec<Option<Size>>,
}

impl LayoutContext {
    pub fn new() -> Self {
        Self { sizes: ThinVec::new() }
    }

    #[inline]
    pub fn set_size(&mut self, id: WidgetId, size: Size) {
        let idx = id.idx as usize;
        if idx >= self.sizes.len() {
            self.sizes.resize(idx + 1, None);
        }
        self.sizes[idx] = Some(size);
    }

    #[inline]
    pub fn get_size(&self, id: WidgetId) -> Option<Size> {
        self.sizes.get(id.idx as usize).copied().flatten()
    }
}

impl Default for LayoutContext {
    fn default() -> Self {
        Self::new()
    }
}

pub trait LayoutSolver: Send + Sync {
    fn measure(
        &self,
        children: &[(WidgetId, Constraint)],
        arena: &WidgetArena,
        ctx: &mut LayoutContext,
    ) -> Size;
    fn arrange(&self, rect: Rect, children: &mut [(WidgetId, Rect)], arena: &WidgetArena);
}

#[derive(Debug, Clone, Copy)]
pub struct RowLayout {
    pub gap: f32,
    pub cross_alignment: Alignment,
}

#[derive(Debug, Copy, Clone, Default)]
pub enum Alignment {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

impl LayoutSolver for RowLayout {
    fn measure(
        &self,
        children: &[(WidgetId, Constraint)],
        arena: &WidgetArena,
        ctx: &mut LayoutContext,
    ) -> Size {
        let mut total_w: f32 = 0.0;
        let mut max_h: f32 = 0.0;
        for (id, constraint) in children {
            if let Some(child) = arena.get(*id) {
                let size = child.layout_solver.measure(&[], arena, ctx);
                let clamped = constraint.clamp(size);
                ctx.set_size(*id, clamped);
                total_w += clamped.w + self.gap;
                max_h = max_h.max(clamped.h);
            }
        }
        if !children.is_empty() {
            total_w -= self.gap;
        }
        Size {
            w: total_w,
            h: max_h,
        }
    }

    fn arrange(&self, rect: Rect, children: &mut [(WidgetId, Rect)], arena: &WidgetArena) {
        let mut x = rect.x;
        for (id, child_rect) in children {
            if let Some(child) = arena.get(*id) {
                let size = child.rect.size();
                *child_rect = Rect {
                    x,
                    y: rect.y,
                    w: size.w,
                    h: rect.h,
                };
                x += size.w + self.gap;
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ColumnLayout {
    pub gap: f32,
    pub cross_alignment: Alignment,
}

impl LayoutSolver for ColumnLayout {
    fn measure(
        &self,
        children: &[(WidgetId, Constraint)],
        arena: &WidgetArena,
        ctx: &mut LayoutContext,
    ) -> Size {
        let mut total_h: f32 = 0.0;
        let mut max_w: f32 = 0.0;
        for (id, constraint) in children {
            if let Some(child) = arena.get(*id) {
                let size = child.layout_solver.measure(&[], arena, ctx);
                let clamped = constraint.clamp(size);
                ctx.set_size(*id, clamped);
                total_h += clamped.h + self.gap;
                max_w = max_w.max(clamped.w);
            }
        }
        if !children.is_empty() {
            total_h -= self.gap;
        }
        Size {
            w: max_w,
            h: total_h,
        }
    }

    fn arrange(&self, rect: Rect, children: &mut [(WidgetId, Rect)], arena: &WidgetArena) {
        let mut y = rect.y;
        for (id, child_rect) in children {
            if let Some(child) = arena.get(*id) {
                let size = child.rect.size();
                *child_rect = Rect {
                    x: rect.x,
                    y,
                    w: rect.w,
                    h: size.h,
                };
                y += size.h + self.gap;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NullLayout;

impl LayoutSolver for NullLayout {
    fn measure(
        &self,
        _children: &[(WidgetId, Constraint)],
        _arena: &WidgetArena,
        _ctx: &mut LayoutContext,
    ) -> Size {
        Size { w: 0.0, h: 0.0 }
    }

    fn arrange(&self, _rect: Rect, _children: &mut [(WidgetId, Rect)], _arena: &WidgetArena) {}
}

/// Inline enum dispatch for layout solvers.
///
/// Replaces `Box<dyn LayoutSolver>` in every `Widget`, eliminating one heap
/// allocation and one vtable indirection per widget. The three concrete types
/// are all `Copy`-sized (<= 16 bytes each), so the enum fits in a register pair.
#[derive(Debug, Clone, Copy, Default)]
pub enum LayoutDispatch {
    Row(RowLayout),
    Column(ColumnLayout),
    #[default]
    Null,
}

impl LayoutSolver for LayoutDispatch {
    #[inline]
    fn measure(
        &self,
        children: &[(WidgetId, Constraint)],
        arena: &WidgetArena,
        ctx: &mut LayoutContext,
    ) -> Size {
        match self {
            Self::Row(r) => r.measure(children, arena, ctx),
            Self::Column(c) => c.measure(children, arena, ctx),
            Self::Null => Size { w: 0.0, h: 0.0 },
        }
    }

    #[inline]
    fn arrange(&self, rect: Rect, children: &mut [(WidgetId, Rect)], arena: &WidgetArena) {
        match self {
            Self::Row(r) => r.arrange(rect, children, arena),
            Self::Column(c) => c.arrange(rect, children, arena),
            Self::Null => {}
        }
    }
}

/// Guard the §4.1 dispatch win: `LayoutDispatch` must stay small enough to live
/// in a register pair (no spill) so the inline `match` keeps beating a
/// `Box<dyn LayoutSolver>` vtable call. See `docs/gui_research/asm/exp1_dispatch.rs`.
const _: () = assert!(core::mem::size_of::<LayoutDispatch>() <= 16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_context_get_set_roundtrip() {
        let mut ctx = LayoutContext::new();
        // Fabricate a WidgetId using the public fields on Handle.
        let id = crate::render::ui_core::id::WidgetId {
            idx: 0,
            generation: std::num::NonZeroU32::new(1).unwrap(),
            _tag: std::marker::PhantomData,
        };
        ctx.set_size(id, Size { w: 100.0, h: 50.0 });
        let got = ctx.get_size(id).unwrap();
        assert!((got.w - 100.0).abs() < 1e-6);
        assert!((got.h - 50.0).abs() < 1e-6);
    }

    #[test]
    fn layout_context_missing_id_returns_none() {
        let ctx = LayoutContext::new();
        let id = crate::render::ui_core::id::WidgetId {
            idx: 99,
            generation: std::num::NonZeroU32::new(1).unwrap(),
            _tag: std::marker::PhantomData,
        };
        assert!(ctx.get_size(id).is_none());
    }

    #[test]
    fn constraint_clamp_enforces_bounds() {
        let c = Constraint { min_width: 10.0, max_width: 200.0, min_height: 5.0, max_height: 100.0 };
        let too_small = c.clamp(Size { w: 1.0, h: 1.0 });
        assert!((too_small.w - 10.0).abs() < 1e-6);
        assert!((too_small.h - 5.0).abs() < 1e-6);
        let too_large = c.clamp(Size { w: 999.0, h: 999.0 });
        assert!((too_large.w - 200.0).abs() < 1e-6);
        assert!((too_large.h - 100.0).abs() < 1e-6);
    }
}
