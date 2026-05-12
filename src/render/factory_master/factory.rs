
use crate::frame::Frame;
use crate::resource_manager::manager::{Arena, Handle, Id};


pub struct FactoryTag;
pub type FactoryHandle = Handle<FactoryTag>;

pub struct FactoryMarker;
pub type FactoryId = Id<FactoryMarker>;

pub struct Factory {
    id: FactoryId,
    frames: Vec<Frame>,

}