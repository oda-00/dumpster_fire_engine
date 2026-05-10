use std::collections::BinaryHeap;

use thin_vec::ThinVec;

use crate::resource_manager::asset_manager::*;

pub enum Message {
    None,
    Pending(AssetId),
    // Use an Arc or a Channel receiver here so the Pipe can "pull" the data
    Ready {
        id: AssetId,
        data: AssetKind, // The loaded texture/mesh
    },
    Failed(AssetId, Arc<str>),
}

impl Default for Message {
    fn default() -> Self {
        Self::None
    }
}
pub struct PipeMarker;
pub type PipeId = Id<PipeMarker>;

pub struct Pipe {
    pub id:   PipeId,
    pub fetch:  Message,
    pub send:   Message,
    pub d_path: AssetArena,
    pub d_loaded:BinaryHeap<AssetData>,
    pub status: PipeStatus,
}
impl Pipe {
    pub fn new(id: PipeId) -> Self {
        Self {
            id,
            fetch: Message::default(),
            send: Message::default(),
            d_path: AssetArena::new(),
            d_loaded: BinaryHeap::new(),
            status: PipeStatus::Idle,
        }
    }
}

pub enum PipeStatus {
    Idle,
    Fetching,
    Sending,
}

impl PipeStatus {
    pub fn is_idle(&self) -> bool {
        matches!(self, PipeStatus::Idle)
    }

    pub fn is_fetching(&self) -> bool {
        matches!(self, PipeStatus::Fetching)
    }

    pub fn is_sending(&self) -> bool {
        matches!(self, PipeStatus::Sending)
    }

    pub fn set_idle(&mut self) {
        *self = PipeStatus::Idle;
    }

    pub fn set_fetching(&mut self) {
        *self = PipeStatus::Fetching;
    }
    
    pub fn set_sending(&mut self) {
        *self = PipeStatus::Sending;
    }

}
