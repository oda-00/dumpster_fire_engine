//! Retained + immediate-mode UI sub-manager.
//! Wired into the engine's cascade tick: World::tick → UiManager::tick → Panel::tick → Widget::tick.

pub mod atlas;
pub mod button;
pub mod checkbox;
pub mod draw;
pub mod dropdown;
pub mod font;
pub mod forge;
pub mod icon;
pub mod immediate;
pub mod input;
pub mod label;
pub mod layout;
pub mod manager;
pub mod panel;
pub mod pipeline;
pub mod scripts;
pub mod slider;
pub mod slider_vec3;
pub mod tag;
pub mod widget;

pub use atlas::Atlas;
pub use button::{ButtonData, ButtonState};
pub use checkbox::CheckboxData;
pub use draw::{DrawList, UiVertex};
pub use dropdown::DropdownData;
pub use icon::{IconData, IconId};
pub use immediate::Ui;
pub use input::{Modifiers, UiInputState};
pub use label::LabelData;
pub use layout::{Align, Axis, LayoutSpec, Padding, Rect, Sizing, measure_and_place};
pub use manager::UiManager;
pub use panel::Panel;
pub use scripts::{UiActionId, dispatch_ui_action};
pub use slider::SliderData;
pub use slider_vec3::SliderVec3Data;
pub use tag::{AtlasHandle, AtlasTag, PanelHandle, PanelTag, WidgetHandle, WidgetTag};
pub use widget::{Widget, WidgetDataKind, WidgetId, WidgetType};
