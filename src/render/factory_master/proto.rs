use crate::frame::Frame;
use thin_vec::ThinVec;


pub struct Proto {
    Frames: ThinVec<Frame>,
    cache: ThinVec<ProtoCacheEntry>,
}   
//...
}