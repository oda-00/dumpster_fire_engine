#[derive(Clone, Debug)]
pub struct CheckboxData {
    pub checked:   bool,
    pub on_change: Option<u32>,
}

impl CheckboxData {
    pub fn new(checked: bool) -> Self {
        Self { checked, on_change: None }
    }

    pub fn toggle(&mut self) { self.checked = !self.checked; }
}
