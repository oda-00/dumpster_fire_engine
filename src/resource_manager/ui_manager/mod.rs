//! Retained + immediate-mode UI sub-manager.
//! Wired into the engine's cascade tick: World::tick → UiManager::tick.

pub mod manager;
pub mod widget;
pub mod panel;
pub mod layout;
pub mod draw;
pub mod input;
pub mod immediate;

pub use manager::UiManager;
pub use widget::{Widget, WidgetHandle, WidgetId};
pub use panel::{Panel, PanelHandle};
pub use layout::{Rect, Sizing, Axis, Align};
pub use draw::DrawList;
pub use input::UiInputState;
pub use immediate::Ui;
