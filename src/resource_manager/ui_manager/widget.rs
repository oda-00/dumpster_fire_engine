//! Widget enum + declare_widgets! macro mirroring `declare_components!`.
//! `WidgetId` derived via FNV-1a (matches lang_frontend's hash).

pub use super::tag::{WidgetTag, WidgetHandle};
pub use super::button::{ButtonData, ButtonState};
pub use super::label::LabelData;
pub use super::slider::SliderData;
pub use super::slider_vec3::SliderVec3Data;
pub use super::dropdown::DropdownData;
pub use super::checkbox::CheckboxData;
pub use super::icon::{IconData, IconId};

mod sealed { pub trait Sealed {} }

pub trait WidgetDataKind: sealed::Sealed { const TYPE: WidgetType; }

/// Stable per-widget identity derived from a string path via FNV-1a 64-bit.
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

macro_rules! declare_widgets {
    ($($variant:ident : $data:ty),+ $(,)?) => {
        #[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
        #[repr(u8)]
        pub enum WidgetType { $($variant),+ }

        impl WidgetType {
            pub const COUNT: usize = [$( stringify!($variant) ),+].len();
            #[inline] pub const fn index(self) -> usize { self as usize }
            pub const ALL: [WidgetType; Self::COUNT] = [$(WidgetType::$variant),+];
        }

        #[derive(Clone, Debug)]
        pub enum Widget {
            Spacer,
            $($variant($data)),+
        }

        impl Widget {
            pub fn widget_type(&self) -> Option<WidgetType> {
                match self {
                    Widget::Spacer => None,
                    $(Widget::$variant(_) => Some(WidgetType::$variant),)+
                }
            }
        }

        $(
            impl From<$data> for Widget {
                fn from(d: $data) -> Self { Widget::$variant(d) }
            }
            impl sealed::Sealed for $data {}
            impl WidgetDataKind for $data {
                const TYPE: WidgetType = WidgetType::$variant;
            }
        )+
    };
}

declare_widgets! {
    Button:      ButtonData,
    Label:       LabelData,
    Slider:      SliderData,
    SliderVec3:  SliderVec3Data,
    Dropdown:    DropdownData,
    Checkbox:    CheckboxData,
    Icon:        IconData,
}

impl Widget {
    /// Per-widget tick: updates retained hover/pressed/dragging state from input.
    pub fn tick(&mut self, input: &super::input::UiInputState, _dt: f32) {
        match self {
            Widget::Button(b) => {
                let hov = b.last_rect.contains(input.cursor[0], input.cursor[1]);
                b.state = if hov && input.left_just_pressed { ButtonState::Pressed }
                          else if hov { ButtonState::Hovered }
                          else { ButtonState::Idle };
            }
            Widget::Checkbox(c) => {
                if input.left_just_pressed
                    && c.last_rect.contains(input.cursor[0], input.cursor[1]) {
                    c.checked = !c.checked;
                }
            }
            Widget::Dropdown(d) => {
                if d.expanded && input.left_just_pressed
                    && !d.last_rect.contains(input.cursor[0], input.cursor[1]) {
                    d.expanded = false;
                }
            }
            Widget::Slider(s) => {
                if s.dragging && !input.left_down { s.dragging = false; }
                if input.left_just_pressed
                    && s.last_rect.contains(input.cursor[0], input.cursor[1]) {
                    s.dragging = true;
                }
                if s.dragging {
                    let t = ((input.cursor[0] - s.last_rect.x)
                             / s.last_rect.w.max(1e-5)).clamp(0.0, 1.0);
                    s.apply(s.min + t * (s.max - s.min));
                }
            }
            _ => {}
        }
    }
}
