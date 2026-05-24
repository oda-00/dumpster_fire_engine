use crate::render::ui_core::{UiEvent, Controller, UiManager};
use crate::resource_manager::world_manager::World;

pub struct TransformController;

impl Controller for TransformController {
    fn handle_event(&self, event: &UiEvent, _world: &mut World, _ui: &mut UiManager) {
        match event {
            UiEvent::ValueChanged(_wid, _value) => {
                // Controller receives value change events and can update world state
            }
            UiEvent::Click(_wid) => {
                // Handle click events
            }
            _ => {}
        }
    }
}
