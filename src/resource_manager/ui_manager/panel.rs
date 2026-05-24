use super::input::UiInputState;
use super::layout::{LayoutSpec, Rect};
pub use super::tag::{PanelHandle, PanelTag};
use super::widget::WidgetHandle;
use thin_vec::ThinVec;

pub struct Panel {
    pub rect: Rect,
    pub layout: LayoutSpec,
    pub children: ThinVec<WidgetHandle>,
    pub scissor: bool,
    pub visible: bool,
}

impl Panel {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            layout: LayoutSpec::default(),
            children: ThinVec::new(),
            scissor: false,
            visible: true,
        }
    }

    /// Cascade-tick gear: Panel forwards into each child Widget directly.
    /// Cascade order: World → UiManager → Panel → Widget.
    pub fn tick(
        &mut self,
        widgets: &mut crate::resource_manager::manager::Arena<
            crate::resource_manager::ui_manager::widget::WidgetTag,
            crate::resource_manager::ui_manager::widget::Widget,
        >,
        input: &UiInputState,
        dt: f32,
    ) {
        if !self.visible {
            return;
        }
        for &child_h in &self.children {
            if let Some(w) = widgets.get_mut(child_h) {
                w.tick(input, dt);
            }
        }
    }
}
