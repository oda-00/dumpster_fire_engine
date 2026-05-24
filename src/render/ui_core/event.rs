use thin_vec::ThinVec;

use crate::render::ui_core::id::WidgetId;
use crate::resource_manager::manager::ActorHandle;

#[derive(Debug, Clone)]
pub enum UiEvent {
    Click(WidgetId),
    ValueChanged(WidgetId, f32),
    TextChanged(WidgetId, String),
    SelectionChanged(WidgetId, Option<usize>),
    TransformChanged {
        actor: ActorHandle,
        translation: [f32; 3],
        rotation: [f32; 3],
        scale: [f32; 3],
    },
    ActorSelected(Option<ActorHandle>),
}

#[derive(Debug)]
pub enum EventHandler {
    Click(Box<dyn Fn() + Send + Sync>),
    ValueChanged(Box<dyn Fn(f32) + Send + Sync>),
    TextChanged(Box<dyn Fn(String) + Send + Sync>),
}

pub struct EventBus {
    events: ThinVec<UiEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            events: ThinVec::new(),
        }
    }

    pub fn emit(&mut self, ev: UiEvent) {
        self.events.push(ev);
    }

    pub fn drain(&mut self) -> impl Iterator<Item = UiEvent> + '_ {
        self.events.drain(..)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
