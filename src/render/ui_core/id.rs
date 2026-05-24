use crate::resource_manager::manager::{Arena, Handle};
use thin_vec::ThinVec;

pub struct WidgetTag;
pub type WidgetId = Handle<WidgetTag>;

pub struct Widget;
pub type WidgetArena = Arena<WidgetTag, Widget>;

#[derive(Clone)]
pub struct WidgetIdPath(pub ThinVec<&'static str>);
impl WidgetIdPath {
    pub fn to_string(&self) -> String {
        self.0.join("/")
    }
    pub fn push(&mut self, seg: &'static str) {
        self.0.push(seg);
    }
    pub fn pop(&mut self) {
        self.0.pop();
    }
}
