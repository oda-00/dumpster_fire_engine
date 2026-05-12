use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

use thin_vec::ThinVec;

use crate::forge_master::master::{ForgeMaster, ForgeResult};
use crate::resource_manager::asset_manager::send::Sender;
use crate::resource_manager::asset_manager::fetch::Fetcher;
use super::asset::{Asset, AssetHandle, AssetId, AssetKind, IngotBuffer, IngotImage};
use super::pipe::{ForgeJob, Pipe, PipeId};

// ── QueueEntry ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetSource {
    Fetcher(usize),
    Sender(usize),
    Pipe(PipeId),
}

#[derive(Debug, Clone, Copy)]
pub struct QueueEntry {
    pub priority: u32,
    pub id:       AssetId,
    pub source:   AssetSource,
    pub handle:   AssetHandle,
}

impl QueueEntry {
    pub fn new(priority: u32, id: AssetId, source: AssetSource, handle: AssetHandle) -> Self {
        Self { priority, id, source, handle }
    }
}

impl PartialEq  for QueueEntry { fn eq(&self, o: &Self) -> bool { self.priority == o.priority } }
impl Eq         for QueueEntry {}
impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) }
}
impl Ord for QueueEntry {
    fn cmp(&self, o: &Self) -> Ordering { self.priority.cmp(&o.priority) }
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

pub struct Pipeline {
    pipes:    ThinVec<Pipe>,
    fetchers: ThinVec<Fetcher>,
    senders:  ThinVec<Sender>,
    queue:    BinaryHeap<QueueEntry>,
    /// Pending GPU compute jobs. Drained highest-priority-first by `forge_tick`.
    /// Lives here rather than per-pipe so the caller drives dispatch timing
    /// (e.g. only during level-load, or N jobs per frame).
    forge_queue: BinaryHeap<ForgeJob>,
}

impl Pipeline {
    pub fn new() -> Self { Self::default() }

    pub fn with_capacity(queue_cap: usize) -> Self {
        Self {
            pipes:       ThinVec::new(),
            fetchers:    ThinVec::new(),
            senders:     ThinVec::new(),
            queue:       BinaryHeap::with_capacity(queue_cap),
            forge_queue: BinaryHeap::new(),
        }
    }

    // ── Registration ──────────────────────────────────────────────────────────

    pub fn add_pipe(&mut self, pipe: Pipe) -> usize {
        self.pipes.push(pipe);
        self.pipes.len() - 1
    }

    pub fn add_fetcher(&mut self, fetcher: Fetcher) -> usize {
        self.fetchers.push(fetcher);
        self.fetchers.len() - 1
    }

    pub fn add_sender(&mut self, sender: Sender) -> usize {
        self.senders.push(sender);
        self.senders.len() - 1
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn fetcher(&self, index: usize)         -> Option<&Fetcher>      { self.fetchers.get(index) }
    pub fn fetcher_mut(&mut self, index: usize) -> Option<&mut Fetcher>  { self.fetchers.get_mut(index) }
    pub fn sender(&self, index: usize)          -> Option<&Sender>      { self.senders.get(index) }
    pub fn sender_mut(&mut self, index: usize)  -> Option<&mut Sender>  { self.senders.get_mut(index) }

    pub fn pipe(&self, id: PipeId)         -> Option<&Pipe>      { self.pipes.iter().find(|p| p.id() == id) }
    pub fn pipe_mut(&mut self, id: PipeId) -> Option<&mut Pipe>  { self.pipes.iter_mut().find(|p| p.id() == id) }

    // ── Standard priority queue ───────────────────────────────────────────────

    pub fn push_queue(&mut self, entry: QueueEntry) { self.queue.push(entry); }
    pub fn pop_queue(&mut self)  -> Option<QueueEntry> { self.queue.pop() }
    pub fn queue_len(&self)      -> usize { self.queue.len() }
    pub fn queue(&self)          -> &BinaryHeap<QueueEntry> { &self.queue }

    pub fn resolve(&self, entry: &QueueEntry) -> Option<&Asset> {
        match entry.source {
            AssetSource::Fetcher(i) => self.fetchers.get(i)?.get(entry.handle),
            AssetSource::Sender(i)  => self.senders.get(i)?.get(entry.handle),
            AssetSource::Pipe(id)   => self.pipe(id)?.paths().get(entry.handle),
        }
    }

    // ── GPU compute dispatch ──────────────────────────────────────────────────

    /// Queue a GPU compute job for later dispatch by `forge_tick`.
    /// Higher `priority` values are dispatched first.
    pub fn enqueue_forge(&mut self, job: ForgeJob) {
        self.forge_queue.push(job);
    }

    /// Pending forge job count.
    pub fn forge_queue_len(&self) -> usize { self.forge_queue.len() }

    /// Drive pending GPU compute jobs through `master`.
    ///
    /// Called once per frame (or during a loading screen) by the owner of both
    /// the `Pipeline` and the `ForgeMaster`. Each call drains up to `max_per_call`
    /// jobs — pass `usize::MAX` to drain everything.
    ///
    /// On success, the target `Pipe` transitions to `Ready { id, data }` where
    /// `data` is `AssetKind::IngotBuffer` or `AssetKind::IngotImage`. On GPU
    /// error, the pipe transitions to `Failed` with the error message. In either
    /// case the job is consumed and the GPU staging/result buffers are freed.
    ///
    /// Returns the number of jobs dispatched.
    pub fn forge_tick(
        &mut self,
        master:       &mut ForgeMaster,
        max_per_call: usize,
    ) -> ForgeResult<usize> {
        let mut dispatched = 0usize;

        while dispatched < max_per_call {
            let Some(job) = self.forge_queue.pop() else { break };
            let pipe_id  = job.pipe_id;
            let asset_id = job.asset_id;

            match master.refine(job.ore) {
                Ok(mut ingot) => {
                    // Copy readback bytes into an Arc before freeing GPU resources.
                    let data: Arc<[u8]> = ingot.as_bytes().into();
                    let ore_kind = ingot.kind; // Copy

                    let kind = if let Some(img) = ingot.result_image() {
                        // Extract Copy fields while the borrow is live.
                        let (w, h) = (img.extent.width, img.extent.height);
                        AssetKind::IngotImage(IngotImage { ore_kind, width: w, height: h, data })
                    } else {
                        AssetKind::IngotBuffer(IngotBuffer { ore_kind, data })
                    };

                    // SAFETY: all readback bytes are in `data` (Arc<[u8]>); the
                    // Ingot's Vulkan resources (result buffer/image, readback
                    // buffer) are no longer needed. `master.device` is the same
                    // device on which they were allocated.
                    unsafe { ingot.destroy(&master.device); }

                    if let Some(pipe) = self.pipe_mut(pipe_id) {
                        pipe.set_ready(asset_id, kind);
                    }
                }
                Err(e) => {
                    if let Some(pipe) = self.pipe_mut(pipe_id) {
                        pipe.set_failed(asset_id, format!("{e}"));
                    }
                }
            }

            dispatched += 1;
        }

        Ok(dispatched)
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            pipes:       ThinVec::new(),
            fetchers:    ThinVec::new(),
            senders:     ThinVec::new(),
            queue:       BinaryHeap::new(),
            forge_queue: BinaryHeap::new(),
        }
    }
}
