use thin_vec::ThinVec;

use crate::render::ui_core::id::WidgetId;
use crate::render::ui_core::layout::{Constraint, LayoutSolver, Rect};
use crate::render::ui_core::signal::Signal;
use crate::resource_manager::world_manager::World;

#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq)]
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
    pub event_handlers: ThinVec<Box<dyn Fn(&crate::render::ui_core::event::UiEvent) + Send + Sync>>,
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

pub struct VirtualListState {
    pub item_count: usize,
    pub item_height: f32,
    pub scroll_offset: f32,
    pub visible_widgets: ThinVec<(usize, WidgetId)>,
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

pub struct PropertyGridState {
    pub properties: ThinVec<PropertyDesc>,
    pub dirty: bool,
}

pub struct PropertyDesc {
    pub name: String,
    pub kind: PropertyKind,
    pub get: Box<dyn Fn(&World) -> String>,
    pub set: Box<dyn Fn(&mut World, String)>,
}

pub enum PropertyKind {
    Float,
    Bool,
    String,
    Vec3,
}
