use crate::resource_manager::manager::Handle;

pub struct WidgetTag;
pub type WidgetHandle = Handle<WidgetTag>;

/// Stable per-widget identity derived from a string path via FNV-1a.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct WidgetId(pub u64);

impl WidgetId {
    pub fn from_str(s: &str) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self(h)
    }
}

#[derive(Clone, Debug)]
pub enum Widget {
    Button   { label: std::sync::Arc<str>, clicked: bool },
    Label    { text:  std::sync::Arc<str> },
    Slider   { value: f32, min: f32, max: f32, dragging: bool },
    Checkbox { checked: bool },
    Spacer,
}
