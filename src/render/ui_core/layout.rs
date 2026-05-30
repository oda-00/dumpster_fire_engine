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

    /// True if point `(px, py)` lies inside the rect (half-open on the far edges).
    /// Used for pointer hit-testing.
    #[inline]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

#[derive(Copy, Clone, Debug, PartialEq)]
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
    /// The constraint each `id` was last measured under, parallel to `sizes`.
    /// Persisted across frames (the manager keeps one `LayoutContext`) so a
    /// clean subtree can be served from cache instead of re-measured — the
    /// Yoga "measure once" optimization (GUI_research.md §3.2).
    constraints: ThinVec<Option<Constraint>>,
}

impl LayoutContext {
    pub fn new() -> Self {
        Self {
            sizes: ThinVec::new(),
            constraints: ThinVec::new(),
        }
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

    /// Cross-frame cache probe: the cached size for `id`, but only if it was
    /// last measured under an equal `constraint`. `None` forces a re-measure.
    #[inline]
    pub fn cached(&self, id: WidgetId, constraint: &Constraint) -> Option<Size> {
        let idx = id.idx as usize;
        if self.constraints.get(idx).copied().flatten().as_ref() == Some(constraint) {
            self.sizes.get(idx).copied().flatten()
        } else {
            None
        }
    }

    /// Record a measured size together with the constraint it was measured under,
    /// so a later `cached` probe can reuse it.
    #[inline]
    pub fn record(&mut self, id: WidgetId, constraint: Constraint, size: Size) {
        let idx = id.idx as usize;
        if idx >= self.sizes.len() {
            self.sizes.resize(idx + 1, None);
        }
        if idx >= self.constraints.len() {
            self.constraints.resize(idx + 1, None);
        }
        self.sizes[idx] = Some(size);
        self.constraints[idx] = Some(constraint);
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

/// Flattened (SoA) sizing input for the flex grow pass.
///
/// A child's contribution is pre-lowered to a fixed pixel amount plus a fill
/// mask (`1.0` if it grows, else `0.0`). Summing this flat stream is branchless
/// and auto-vectorizes (`vaddps`, 8× unrolled) — versus a per-child
/// `match Sizing { Fill/Fixed/Hug }` which stays scalar. See GUI_research.md
/// §4.4 / `docs/gui_research/asm/exp4_layout.rs`. This is the lowering the
/// unified Fill/Fixed/Hug layout uses for its distribution pass.
#[derive(Copy, Clone, Debug, Default)]
pub struct FillItem {
    /// Fixed + hug pixels already resolved for this child.
    pub fixed_px: f32,
    /// `1.0` if this child grows to fill remaining space, else `0.0`.
    pub is_fill: f32,
}

/// Returns the per-fill pixel size: leftover space (`avail` minus all fixed/hug
/// pixels) divided across the growing children. Branchless inner loop.
#[inline]
pub fn distribute_fill(items: &[FillItem], avail: f32) -> f32 {
    let mut used = 0.0f32;
    let mut fills = 0.0f32;
    for it in items {
        used += it.fixed_px;
        fills += it.is_fill;
    }
    let remaining = (avail - used).max(0.0);
    if fills == 0.0 {
        remaining
    } else {
        remaining / fills
    }
}

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
    fn layout_cache_reuses_only_on_matching_constraint() {
        let mut ctx = LayoutContext::new();
        let id = crate::render::ui_core::id::WidgetId {
            idx: 3,
            generation: std::num::NonZeroU32::new(1).unwrap(),
            _tag: std::marker::PhantomData,
        };
        let c1 = Constraint { min_width: 0.0, max_width: 100.0, min_height: 0.0, max_height: 50.0 };
        ctx.record(id, c1, Size { w: 80.0, h: 20.0 });
        // Same constraint → cache hit.
        assert!(ctx.cached(id, &c1).is_some());
        // Different constraint → cache miss (forces re-measure).
        let c2 = Constraint { max_width: 200.0, ..c1 };
        assert!(ctx.cached(id, &c2).is_none());
    }

    #[test]
    fn distribute_fill_splits_remaining_space() {
        // 100 px available, one 30 px fixed child, two fill children → 35 each.
        let items = [
            FillItem { fixed_px: 30.0, is_fill: 0.0 },
            FillItem { fixed_px: 0.0, is_fill: 1.0 },
            FillItem { fixed_px: 0.0, is_fill: 1.0 },
        ];
        assert!((distribute_fill(&items, 100.0) - 35.0).abs() < 1e-6);
        // No fills → all remaining returned as the single leftover.
        let fixed = [FillItem { fixed_px: 40.0, is_fill: 0.0 }];
        assert!((distribute_fill(&fixed, 100.0) - 60.0).abs() < 1e-6);
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
