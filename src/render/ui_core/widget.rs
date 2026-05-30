use std::sync::Arc;

use thin_vec::ThinVec;

use crate::render::ui_core::event::UiEvent;
use crate::render::ui_core::id::WidgetId;
use crate::render::ui_core::layout::{Constraint, LayoutDispatch, Rect};
use crate::render::ui_core::signal::Signal;
use crate::resource_manager::world_manager::World;

/// Closure type for per-widget event sinks.
pub type EventSink = Box<dyn Fn(&UiEvent) + Send + Sync>;

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DirtyFlags {
    NONE = 0,
    LAYOUT = 1 << 0,
    CONTENT = 1 << 1,
    CHILDREN = 1 << 2,
}

pub struct Widget {
    pub id: WidgetId,
    pub kind: WidgetKind,
    pub parent: Option<WidgetId>,
    pub children: ThinVec<WidgetId>,
    pub dirty: u8,
    pub rect: Rect,
    pub constraint: Constraint,
    /// Inline enum dispatch — eliminates Box<dyn LayoutSolver> heap allocation
    /// and vtable indirection. All concrete solvers are Copy-sized.
    pub layout_solver: LayoutDispatch,
    /// Whether this widget can receive keyboard focus / tab navigation.
    /// Defaults from `WidgetKind::default_focusable`.
    pub focusable: bool,
    pub event_handlers: ThinVec<EventSink>,
    pub user_data: Option<Box<dyn std::any::Any>>,
}

pub enum WidgetKind {
    Label(LabelState),
    Button(ButtonState),
    Slider(SliderState),
    Checkbox(CheckboxState),
    Dropdown(DropdownState),
    TextEdit(TextEditState),
    Panel(PanelState),
    VirtualList(VirtualListState),
    OutlinerTree(OutlinerState),
    PropertyGrid(PropertyGridState),
}

impl WidgetKind {
    /// Whether a widget of this kind is interactive and should accept keyboard
    /// focus by default. Containers and static content are not focusable.
    pub fn default_focusable(&self) -> bool {
        matches!(
            self,
            WidgetKind::Button(_)
                | WidgetKind::Slider(_)
                | WidgetKind::Checkbox(_)
                | WidgetKind::Dropdown(_)
                | WidgetKind::TextEdit(_)
        )
    }
}

pub struct LabelState {
    pub text: Signal<String>,
    pub color: Signal<[f32; 4]>,
    pub font_size: u16,
}

pub struct ButtonState {
    pub label: Signal<String>,
    pub enabled: Signal<bool>,
    pub clicked: bool,
}

pub struct SliderState {
    pub value: Signal<f32>,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub dragging: bool,
}

pub struct CheckboxState {
    pub checked: Signal<bool>,
    pub label: Signal<String>,
}

pub struct DropdownState {
    pub selected: Signal<usize>,
    /// Options stored as ref-counted strings — avoids per-String heap
    /// allocations; multiple dropdowns can share the same option list.
    pub options: ThinVec<Arc<str>>,
    pub expanded: bool,
}

pub struct TextEditState {
    pub text: Signal<String>,
    pub cursor: usize,
    pub focus: bool,
}

pub struct PanelState {
    pub title: Signal<String>,
    pub closable: bool,
    pub close_requested: bool,
}

/// Lazily rendered list: only items in the visible window are built each frame.
/// `item_builder` is a bare function pointer (no captures) called with the
/// item index and its top-left cursor y-position.
pub struct VirtualListState {
    pub item_count: usize,
    pub item_height: f32,
    pub scroll_offset: f32,
    /// Bare fn pointer — no heap allocation, no vtable. If per-item context
    /// is needed, pass it through a thread-local or a separate arena.
    pub item_builder: fn(usize, f32),
    pub visible_widgets: ThinVec<(usize, WidgetId)>,
}

impl VirtualListState {
    /// Returns the inclusive range of item indices currently visible inside
    /// `viewport_height`.
    pub fn visible_range(&self, viewport_height: f32) -> std::ops::Range<usize> {
        if self.item_height <= 0.0 || self.item_count == 0 {
            return 0..0;
        }
        let first = (self.scroll_offset / self.item_height).floor() as usize;
        let count = (viewport_height / self.item_height).ceil() as usize + 1;
        let last = (first + count).min(self.item_count);
        first..last
    }

    /// Total content height of all items.
    #[inline]
    pub fn content_height(&self) -> f32 {
        self.item_count as f32 * self.item_height
    }

    /// Largest valid `scroll_offset` for `viewport_height` — never negative, and
    /// 0 when the content fits (so a short list can't scroll).
    #[inline]
    pub fn max_scroll(&self, viewport_height: f32) -> f32 {
        (self.content_height() - viewport_height).max(0.0)
    }

    /// Set the scroll offset, clamped to `[0, max_scroll]`.
    #[inline]
    pub fn set_scroll(&mut self, offset: f32, viewport_height: f32) {
        self.scroll_offset = offset.clamp(0.0, self.max_scroll(viewport_height));
    }

    /// Scroll by `delta` pixels (e.g. from a wheel event), clamped to range.
    #[inline]
    pub fn scroll_by(&mut self, delta: f32, viewport_height: f32) {
        self.set_scroll(self.scroll_offset + delta, viewport_height);
    }
}

pub struct OutlinerState {
    pub roots: ThinVec<OutlinerNode>,
    pub expanded: ThinVec<bool>,
    pub selected: Option<usize>,
}

pub struct OutlinerNode {
    pub name: String,
    pub children: ThinVec<OutlinerNode>,
}

impl OutlinerNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), children: ThinVec::new() }
    }
}

pub struct PropertyGridState {
    pub properties: ThinVec<PropertyDesc>,
    pub dirty: bool,
}

pub struct PropertyDesc {
    pub name: String,
    pub kind: PropertyKind,
    /// Bare fn pointer: reads a value from World and formats it as a String.
    /// No captures needed — World carries all required context.
    pub get: fn(&World) -> String,
    /// Bare fn pointer: parses the string and writes back into World.
    pub set: fn(&mut World, &str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyKind {
    Float,
    Bool,
    String,
    Vec3,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vlist(item_count: usize, item_height: f32) -> VirtualListState {
        fn noop(_i: usize, _y: f32) {}
        VirtualListState {
            item_count,
            item_height,
            scroll_offset: 0.0,
            item_builder: noop,
            visible_widgets: ThinVec::new(),
        }
    }

    #[test]
    fn default_focusable_only_interactive_kinds() {
        assert!(WidgetKind::Button(ButtonState {
            label: crate::render::ui_core::signal::Signal::new(String::new()),
            enabled: crate::render::ui_core::signal::Signal::new(true),
            clicked: false,
        })
        .default_focusable());
        assert!(!WidgetKind::Panel(PanelState {
            title: crate::render::ui_core::signal::Signal::new(String::new()),
            closable: false,
            close_requested: false,
        })
        .default_focusable());
    }

    #[test]
    fn visible_range_windows_to_viewport() {
        let mut v = vlist(1000, 20.0);
        // Top of a 100px viewport: items 0..=5 (5 fully visible + 1 spare).
        assert_eq!(v.visible_range(100.0), 0..6);
        v.scroll_offset = 100.0; // scrolled down 5 items
        assert_eq!(v.visible_range(100.0), 5..11);
    }

    #[test]
    fn scroll_clamps_to_content() {
        let mut v = vlist(10, 20.0); // 200px content
        assert_eq!(v.max_scroll(100.0), 100.0);
        v.scroll_by(1000.0, 100.0); // overscroll down
        assert_eq!(v.scroll_offset, 100.0);
        v.scroll_by(-1000.0, 100.0); // overscroll up
        assert_eq!(v.scroll_offset, 0.0);
    }

    #[test]
    fn short_list_cannot_scroll() {
        let mut v = vlist(2, 20.0); // 40px content, 100px viewport
        assert_eq!(v.max_scroll(100.0), 0.0);
        v.scroll_by(50.0, 100.0);
        assert_eq!(v.scroll_offset, 0.0);
    }
}
