use crate::resource_manager::manager::Arena;
use super::draw::DrawList;
use super::input::UiInputState;
use super::layout::Rect;
use super::panel::{Panel, PanelHandle, PanelTag};
use super::widget::{Widget, WidgetHandle, WidgetTag};

pub struct UiManager {
    pub panels:    Arena<PanelTag,  Panel>,
    pub widgets:   Arena<WidgetTag, Widget>,
    pub draw_list: DrawList,
    pub dirty:     bool,
}

impl UiManager {
    pub fn new() -> Self {
        Self {
            panels:    Arena::new(),
            widgets:   Arena::new(),
            draw_list: DrawList::new(),
            dirty:     false,
        }
    }

    /// Cascade-tick entry point. Called by `World::tick` after world logic.
    pub fn tick(&mut self, _input: &UiInputState, _dt: f32) {
        self.draw_list.clear();
        self.dirty = false;
    }

    /// True when the UI consumed the most recent pointer event.
    pub fn consumed_input(&self) -> bool { false }

    /// True when the selection changed this tick (triggers inspector rebuild).
    pub fn selection_dirty(&self) -> bool { self.dirty }

    /// Build the inspector panel via the immediate-mode API.
    pub fn frame_inspector<F: FnOnce(&mut super::immediate::Ui<'_>)>(&mut self, f: F) {
        let rect = Rect { x: 0.0, y: 0.0, w: 320.0, h: 600.0 };
        let mut ui = super::immediate::Ui::new(&mut self.draw_list, rect);
        f(&mut ui);
    }
}

impl Default for UiManager {
    fn default() -> Self { Self::new() }
}
