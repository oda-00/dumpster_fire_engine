use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

use super::asset::{AssetArena, AssetHandle, AssetId, AssetKind};
use crate::resource_manager::manager::Id;

pub struct PipeMarker;
pub type PipeId = Id<PipeMarker>;

#[derive(Debug, Clone)]
pub enum PipeState {
    Idle,
    Fetching(AssetId),
    Ready { id: AssetId, data: AssetKind },
    Sending { id: AssetId, handle: AssetHandle },
    Failed { id: AssetId, reason: Arc<str> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipeStatus {
    Idle,
    Fetching,
    Ready,
    Sending,
    Failed,
}

impl PipeState {
    pub fn status(&self) -> PipeStatus {
        match self {
            PipeState::Idle => PipeStatus::Idle,
            PipeState::Fetching(_) => PipeStatus::Fetching,
            PipeState::Ready { .. } => PipeStatus::Ready,
            PipeState::Sending { .. } => PipeStatus::Sending,
            PipeState::Failed { .. } => PipeStatus::Failed,
        }
    }
}

pub struct Pipe {
    id: PipeId,
    state: PipeState,
    paths: AssetArena,
    loaded: BinaryHeap<AssetData>,
}

impl Pipe {
    pub fn new(id: PipeId) -> Self {
        Self {
            id,
            state: PipeState::Idle,
            paths: AssetArena::new(),
            loaded: BinaryHeap::new(),
        }
    }

    pub fn id(&self) -> PipeId {
        self.id
    }

    pub fn state(&self) -> &PipeState {
        &self.state
    }

    pub fn status(&self) -> PipeStatus {
        self.state.status()
    }

    pub fn is_idle(&self) -> bool {
        self.status() == PipeStatus::Idle
    }

    pub fn paths(&self) -> &AssetArena {
        &self.paths
    }

    pub fn paths_mut(&mut self) -> &mut AssetArena {
        &mut self.paths
    }

    pub fn loaded_len(&self) -> usize {
        self.loaded.len()
    }

    pub fn push_loaded(&mut self, data: AssetData) {
        self.loaded.push(data);
    }

    pub fn pop_loaded(&mut self) -> Option<AssetData> {
        self.loaded.pop()
    }

    pub fn set_idle(&mut self) {
        self.state = PipeState::Idle;
    }

    pub fn set_fetching(&mut self, id: AssetId) {
        self.state = PipeState::Fetching(id);
    }

    pub fn set_ready(&mut self, id: AssetId, data: AssetKind) {
        self.state = PipeState::Ready { id, data };
    }

    pub fn set_sending(&mut self, id: AssetId, handle: AssetHandle) {
        self.state = PipeState::Sending { id, handle };
    }

    pub fn set_failed(&mut self, id: AssetId, reason: impl Into<Arc<str>>) {
        self.state = PipeState::Failed {
            id,
            reason: reason.into(),
        };
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AssetData {
    pub priority: u32,
    pub id: AssetId,
    pub handle: AssetHandle,
}

impl AssetData {
    pub fn new(priority: u32, id: AssetId, handle: AssetHandle) -> Self {
        Self {
            priority,
            id,
            handle,
        }
    }
}

impl PartialEq for AssetData {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for AssetData {}

impl PartialOrd for AssetData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AssetData {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}
