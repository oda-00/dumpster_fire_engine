use std::sync::Arc;
use thin_vec::ThinVec;
use super::layout::Rect;

#[derive(Clone, Debug)]
pub struct DropdownData {
    pub selected:  u32,
    pub options:   ThinVec<Arc<str>>,
    pub expanded:  bool,
    pub on_change: Option<u32>,
    pub last_rect: Rect,
}

impl DropdownData {
    pub fn new(options: ThinVec<Arc<str>>, selected: u32) -> Self {
        Self { selected, options, expanded: false, on_change: None, last_rect: Rect::default() }
    }

    pub fn current(&self) -> Option<&Arc<str>> {
        self.options.get(self.selected as usize)
    }
}
