use crate::render::ui_core::{Controller, UiEvent, UiManager};
use crate::resource_manager::world_manager::World;

#[derive(Debug, Default)]
pub struct TransformController;

impl Controller for TransformController {
    fn handle_event(&self, event: &UiEvent, _world: &mut World, _ui: &mut UiManager) {
        match event {
            UiEvent::ValueChanged(_, _) | UiEvent::Click(_) => {}
            _ => {}
        }
    }
}
