use super::layout::Align;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct LabelData {
    pub text: Arc<str>,
    pub color: [f32; 4],
    pub align: Align,
}

impl LabelData {
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            color: [1.0; 4],
            align: Align::Start,
        }
    }
}
