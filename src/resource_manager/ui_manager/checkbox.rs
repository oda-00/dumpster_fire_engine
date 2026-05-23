use super::layout::Rect;

#[derive(Clone, Debug)]
pub struct CheckboxData {
    pub checked:   bool,
    pub on_change: Option<u32>,
    pub last_rect: Rect,
}

impl CheckboxData {
    pub fn new(checked: bool) -> Self {
        Self { checked, on_change: None, last_rect: Rect::default() }
    }

    pub fn toggle(&mut self) { self.checked = !self.checked; }
}
