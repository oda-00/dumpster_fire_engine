use std::cmp::Ordering;
use std::collections::BinaryHeap;

use thin_vec::ThinVec;

use super::asset::{Asset, AssetArena, AssetHandle, AssetId};
use super::pipe::{Pipe, PipeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetSource {
    Fetcher(usize),
    Sender(usize),
    Pipe(PipeId),
}

#[derive(Debug, Clone, Copy)]
pub struct QueueEntry {
    pub priority: u32,
    pub id: AssetId,
    pub source: AssetSource,
    pub handle: AssetHandle,
}

impl QueueEntry {
    pub fn new(priority: u32, id: AssetId, source: AssetSource, handle: AssetHandle) -> Self {
        Self {
            priority,
            id,
            source,
            handle,
        }
    }
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for QueueEntry {}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

pub struct Pipeline {
    pipes: ThinVec<Pipe>,
    fetchers: ThinVec<AssetArena>,
    senders: ThinVec<AssetArena>,
    queue: BinaryHeap<QueueEntry>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(queue_cap: usize) -> Self {
        Self {
            pipes: ThinVec::new(),
            fetchers: ThinVec::new(),
            senders: ThinVec::new(),
            queue: BinaryHeap::with_capacity(queue_cap),
        }
    }

    pub fn add_pipe(&mut self, pipe: Pipe) -> usize {
        self.pipes.push(pipe);
        self.pipes.len() - 1
    }

    pub fn add_fetcher(&mut self, arena: AssetArena) -> usize {
        self.fetchers.push(arena);
        self.fetchers.len() - 1
    }

    pub fn add_sender(&mut self, arena: AssetArena) -> usize {
        self.senders.push(arena);
        self.senders.len() - 1
    }

    pub fn fetcher(&self, index: usize) -> Option<&AssetArena> {
        self.fetchers.get(index)
    }

    pub fn fetcher_mut(&mut self, index: usize) -> Option<&mut AssetArena> {
        self.fetchers.get_mut(index)
    }

    pub fn sender(&self, index: usize) -> Option<&AssetArena> {
        self.senders.get(index)
    }

    pub fn sender_mut(&mut self, index: usize) -> Option<&mut AssetArena> {
        self.senders.get_mut(index)
    }

    pub fn pipe(&self, id: PipeId) -> Option<&Pipe> {
        self.pipes.iter().find(|pipe| pipe.id() == id)
    }

    pub fn pipe_mut(&mut self, id: PipeId) -> Option<&mut Pipe> {
        self.pipes.iter_mut().find(|pipe| pipe.id() == id)
    }

    pub fn push_queue(&mut self, entry: QueueEntry) {
        self.queue.push(entry);
    }

    pub fn pop_queue(&mut self) -> Option<QueueEntry> {
        self.queue.pop()
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn queue(&self) -> &BinaryHeap<QueueEntry> {
        &self.queue
    }

    pub fn resolve(&self, entry: &QueueEntry) -> Option<&Asset> {
        match entry.source {
            AssetSource::Fetcher(index) => self.fetchers.get(index)?.get(entry.handle),
            AssetSource::Sender(index) => self.senders.get(index)?.get(entry.handle),
            AssetSource::Pipe(id) => self.pipe(id)?.paths().get(entry.handle),
        }
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            pipes: ThinVec::new(),
            fetchers: ThinVec::new(),
            senders: ThinVec::new(),
            queue: BinaryHeap::new(),
        }
    }
}
