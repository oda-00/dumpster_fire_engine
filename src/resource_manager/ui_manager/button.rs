use std::sync::Arc;
use super::layout::Rect;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ButtonState { Idle, Hovered, Pressed }

#[derive(Clone, Debug)]
pub struct ButtonData {
    pub label:     Arc<str>,
    pub icon_id:   Option<u32>,
    pub state:     ButtonState,
    pub on_click:  Option<u32>,
    pub last_rect: Rect,
}

impl ButtonData {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self { label: label.into(), icon_id: None, state: ButtonState::Idle,
               on_click: None, last_rect: Rect::default() }
    }
}
