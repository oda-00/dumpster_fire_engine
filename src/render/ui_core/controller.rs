use crate::render::ui_core::event::UiEvent;
use crate::render::ui_core::manager::UiManager;
use crate::resource_manager::world_manager::World;

pub trait Controller: Send + Sync + 'static {
    fn handle_event(&self, event: &UiEvent, world: &mut World, ui: &mut UiManager);
}
