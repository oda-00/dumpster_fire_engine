use std::collections::BinaryHeap;
use std::cmp::Ordering;
use thin_vec::ThinVec;
use super::asset::{AssetArena, AssetHandle, AssetId};

pub struct QueueEntry {
    pub priority: u32,
    pub id:       AssetId,
    pub handle:   AssetHandle,
}

impl PartialEq  for QueueEntry { fn eq(&self, o: &Self) -> bool { self.priority == o.priority } }
impl Eq         for QueueEntry {}
impl PartialOrd for QueueEntry { fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) } }
impl Ord        for QueueEntry { fn cmp(&self, o: &Self) -> Ordering { self.priority.cmp(&o.priority) } }

pub struct Pipeline {
    pub pipes:    ThinVec<AssetArena>,
    pub fetchers: ThinVec<AssetArena>,
    pub senders:  ThinVec<AssetArena>,
    pub queue:    BinaryHeap<QueueEntry>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(queue_cap: usize) -> Self {
        Self {
            pipes:    ThinVec::new(),
            fetchers: ThinVec::new(),
            senders:  ThinVec::new(),
            queue:    BinaryHeap::with_capacity(queue_cap),
        }
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            pipes:    ThinVec::new(),
            fetchers: ThinVec::new(),
            senders:  ThinVec::new(),
            queue:    BinaryHeap::new(),
        }
    }
}
