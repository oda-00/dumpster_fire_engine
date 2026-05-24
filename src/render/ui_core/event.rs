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

pub enum EventHandler {
    Click(Box<dyn Fn() + Send + Sync>),
    ValueChanged(Box<dyn Fn(f32) + Send + Sync>),
    TextChanged(Box<dyn Fn(String) + Send + Sync>),
}

#[derive(Debug, Default)]
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::ui_core::id::WidgetId;
    use std::marker::PhantomData;
    use std::num::NonZeroU32;

    fn dummy_id(idx: u32) -> WidgetId {
        WidgetId { idx, generation: NonZeroU32::new(1).unwrap(), _tag: PhantomData }
    }

    #[test]
    fn event_bus_emit_and_drain() {
        let mut bus = EventBus::new();
        bus.emit(UiEvent::Click(dummy_id(0)));
        bus.emit(UiEvent::Click(dummy_id(1)));
        let drained: Vec<_> = bus.drain().collect();
        assert_eq!(drained.len(), 2);
    }

    #[test]
    fn event_bus_drain_clears() {
        let mut bus = EventBus::new();
        bus.emit(UiEvent::Click(dummy_id(0)));
        let _ = bus.drain().count();
        let second: Vec<_> = bus.drain().collect();
        assert!(second.is_empty());
    }

    #[test]
    fn ui_event_value_changed_preserves_value() {
        let ev = UiEvent::ValueChanged(dummy_id(5), 3.14);
        match ev {
            UiEvent::ValueChanged(_, v) => assert!((v - 3.14).abs() < 1e-6),
            _ => panic!("wrong variant"),
        }
    }
}
