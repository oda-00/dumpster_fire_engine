use thin_vec::ThinVec;

use crate::render::ui_core::event::UiEvent;
use crate::render::ui_core::id::WidgetId;
use crate::render::ui_core::layout::{Constraint, LayoutSolver, Rect};
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
    pub layout_solver: Box<dyn LayoutSolver>,
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
    pub options: ThinVec<String>,
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
/// `item_builder` is called with the item index and a mutable cursor position;
/// it should push geometry into the provided DrawList rather than into the tree.
pub struct VirtualListState {
    pub item_count: usize,
    pub item_height: f32,
    pub scroll_offset: f32,
    /// Closure that renders one item at the given index.
    /// Signature: `fn(item_index: usize, cursor_y: f32)`.
    pub item_builder: Box<dyn Fn(usize, f32) + Send + Sync>,
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
    pub get: Box<dyn Fn(&World) -> String + Send + Sync>,
    pub set: Box<dyn Fn(&mut World, &str) + Send + Sync>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyKind {
    Float,
    Bool,
    String,
    Vec3,
}
