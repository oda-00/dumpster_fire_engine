use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU32;

use thin_vec::ThinVec;

use crate::resource_manager::manager::{Arena, Handle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetTag;
pub type WidgetId = Handle<WidgetTag>;
pub type WidgetArena = Arena<WidgetTag, super::widget::Widget>;

/// Stable path-based identity for immediate-mode widget lookup.
///
/// Each segment is a `&'static str` to avoid allocation during frame traversal.
/// The full path is only materialised into a `String` when doing a HashMap lookup.
#[derive(Clone, Debug, Default)]
pub struct WidgetIdPath(pub ThinVec<&'static str>);

impl WidgetIdPath {
    pub fn push(&mut self, seg: &'static str) {
        self.0.push(seg);
    }

    pub fn pop(&mut self) {
        self.0.pop();
    }
}

impl fmt::Display for WidgetIdPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            f.write_str(seg)?;
        }
        Ok(())
    }
}
