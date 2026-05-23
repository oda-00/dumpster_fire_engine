//! Retained + immediate-mode UI sub-manager.
//! Wired into the engine's cascade tick: World::tick → UiManager::tick → Panel::tick → Widget::tick.

pub mod tag;
pub mod widget;
pub mod panel;
pub mod button;
pub mod label;
pub mod slider;
pub mod slider_vec3;
pub mod dropdown;
pub mod checkbox;
pub mod icon;
pub mod layout;
pub mod draw;
pub mod atlas;
pub mod font;
pub mod input;
pub mod immediate;
pub mod forge;
pub mod pipeline;
pub mod manager;
pub mod scripts;

pub use tag::{PanelTag, PanelHandle, WidgetTag, WidgetHandle, AtlasTag, AtlasHandle};
pub use widget::{Widget, WidgetId, WidgetType, WidgetDataKind};
pub use button::{ButtonData, ButtonState};
pub use label::LabelData;
pub use slider::SliderData;
pub use slider_vec3::SliderVec3Data;
pub use dropdown::DropdownData;
pub use checkbox::CheckboxData;
pub use icon::{IconData, IconId};
pub use panel::Panel;
pub use layout::{Rect, Sizing, Axis, Align, Padding, LayoutSpec, measure_and_place};
pub use draw::{DrawList, UiVertex};
pub use atlas::Atlas;
pub use input::{UiInputState, Modifiers};
pub use immediate::Ui;
pub use manager::UiManager;
pub use scripts::{UiActionId, dispatch_ui_action};
