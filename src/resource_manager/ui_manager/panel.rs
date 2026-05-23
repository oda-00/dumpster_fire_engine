use thin_vec::ThinVec;
use crate::resource_manager::manager::Handle;
use super::layout::Rect;
use super::widget::WidgetHandle;

pub struct PanelTag;
pub type PanelHandle = Handle<PanelTag>;

pub struct Panel {
    pub rect:     Rect,
    pub children: ThinVec<WidgetHandle>,
    pub scissor:  bool,
    pub visible:  bool,
}

impl Panel {
    pub fn new(rect: Rect) -> Self {
        Self { rect, children: ThinVec::new(), scissor: false, visible: true }
    }
}
