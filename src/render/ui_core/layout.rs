use thin_vec::ThinVec;

use crate::render::ui_core::id::{WidgetId, WidgetArena};

#[derive(Copy, Clone, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn size(&self) -> Size {
        Size { w: self.w, h: self.h }
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

pub struct LayoutContext {
    sizes: ThinVec<(WidgetId, Size)>,
}

impl LayoutContext {
    pub fn new() -> Self {
        Self {
            sizes: ThinVec::new(),
        }
    }

    pub fn set_size(&mut self, id: WidgetId, size: Size) {
        self.sizes.push((id, size));
    }

    pub fn get_size(&self, id: WidgetId) -> Option<Size> {
        self.sizes.iter().find(|(i, _)| *i == id).map(|(_, s)| *s)
    }
}

pub trait LayoutSolver: Send + Sync {
    fn measure(
        &self,
        children: &[(WidgetId, Constraint)],
        arena: &WidgetArena,
        ctx: &mut LayoutContext,
    ) -> Size;
    fn arrange(
        &self,
        rect: Rect,
        children: &mut [(WidgetId, Rect)],
        arena: &mut WidgetArena,
    );
}

pub struct RowLayout {
    pub gap: f32,
    pub cross_alignment: Alignment,
}

#[derive(Copy, Clone)]
pub enum Alignment {
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
        let mut total_w = 0.0;
        let mut max_h = 0.0;
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

    fn arrange(&self, rect: Rect, children: &mut [(WidgetId, Rect)], arena: &mut WidgetArena) {
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
        let mut total_h = 0.0;
        let mut max_w = 0.0;
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

    fn arrange(&self, rect: Rect, children: &mut [(WidgetId, Rect)], arena: &mut WidgetArena) {
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

    fn arrange(
        &self,
        _rect: Rect,
        _children: &mut [(WidgetId, Rect)],
        _arena: &mut WidgetArena,
    ) {
    }
}
