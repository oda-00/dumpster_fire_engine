//! Experiment 1 — Widget/layout dispatch strategy.
//!
//! Mirrors `dumpster_fire_engine`'s `LayoutDispatch` (src/render/ui_core/layout.rs)
//! vs the `Box<dyn LayoutSolver>` it replaced, plus a bare-fn-pointer variant.
//!
//! Build: rustc -O --emit asm --crate-type=lib exp1_dispatch.rs
//! We expose three `#[no_mangle]` entry points so the codegen is easy to find.

#[derive(Clone, Copy)]
pub struct Size { pub w: f32, pub h: f32 }

// ---- enum dispatch (what the engine uses now) ----------------------------
#[derive(Clone, Copy)]
pub struct Row { pub gap: f32 }
#[derive(Clone, Copy)]
pub struct Col { pub gap: f32 }

#[derive(Clone, Copy)]
pub enum LayoutDispatch { Row(Row), Col(Col), Null }

impl LayoutDispatch {
    #[inline]
    fn measure(&self, n: u32, child: Size) -> Size {
        match self {
            LayoutDispatch::Row(r) => Size { w: child.w * n as f32 + r.gap * (n.saturating_sub(1)) as f32, h: child.h },
            LayoutDispatch::Col(c) => Size { w: child.w, h: child.h * n as f32 + c.gap * (n.saturating_sub(1)) as f32 },
            LayoutDispatch::Null => Size { w: 0.0, h: 0.0 },
        }
    }
}

#[no_mangle]
pub fn dispatch_enum(d: &LayoutDispatch, n: u32, child: Size) -> Size {
    d.measure(n, child)
}

// ---- dyn trait object dispatch (the Box<dyn> the engine removed) ---------
pub trait LayoutSolver { fn measure(&self, n: u32, child: Size) -> Size; }
impl LayoutSolver for Row {
    #[inline]
    fn measure(&self, n: u32, child: Size) -> Size {
        Size { w: child.w * n as f32 + self.gap * (n.saturating_sub(1)) as f32, h: child.h }
    }
}
impl LayoutSolver for Col {
    #[inline]
    fn measure(&self, n: u32, child: Size) -> Size {
        Size { w: child.w, h: child.h * n as f32 + self.gap * (n.saturating_sub(1)) as f32 }
    }
}

#[no_mangle]
pub fn dispatch_dyn(d: &dyn LayoutSolver, n: u32, child: Size) -> Size {
    d.measure(n, child)
}

// ---- bare fn pointer dispatch (used for VirtualList item_builder) --------
#[no_mangle]
pub fn dispatch_fnptr(f: fn(u32, Size) -> Size, n: u32, child: Size) -> Size {
    f(n, child)
}

// A realistic hot loop: solve a Vec of sibling layouts, the actual per-frame
// pattern. Shows whether the enum match stays branch-predictable / inlinable
// across a homogeneous run vs the indirect call the vtable forces every time.
#[no_mangle]
pub fn solve_many_enum(items: &[LayoutDispatch], child: Size) -> f32 {
    let mut acc = 0.0f32;
    for it in items {
        acc += it.measure(8, child).w;
    }
    acc
}

#[no_mangle]
pub fn solve_many_dyn(items: &[&dyn LayoutSolver], child: Size) -> f32 {
    let mut acc = 0.0f32;
    for it in items {
        acc += it.measure(8, child).w;
    }
    acc
}
