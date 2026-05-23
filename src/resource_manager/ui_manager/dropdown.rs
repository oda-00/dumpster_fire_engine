use std::sync::Arc;
use thin_vec::ThinVec;

#[derive(Clone, Debug)]
pub struct DropdownData {
    pub selected:  u32,
    pub options:   ThinVec<Arc<str>>,
    pub expanded:  bool,
    pub on_change: Option<u32>,
}

impl DropdownData {
    pub fn new(options: ThinVec<Arc<str>>, selected: u32) -> Self {
        Self { selected, options, expanded: false, on_change: None }
    }

    pub fn current(&self) -> Option<&Arc<str>> {
        self.options.get(self.selected as usize)
    }
}
