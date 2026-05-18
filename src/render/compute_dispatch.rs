//! Per-frame compute-Ore batching + signal-semaphore handoff.
//!
//! Every frame, the engine may dispatch SkinPalette, MorphBlend,
//! InstanceTransforms, SplatSort, and SplatBillboard compute Ores
//! before any graphics draw. `ComputeDispatchGraph` collects all of
//! them and submits as a single batched `refine_batch_async` call so
//! the GPU sees one CB submit per frame instead of N. The returned
//! semaphore feeds into `Window::draw_frame_with_compute_wait`.

use ash::vk;

use crate::forge_master::{ForgeMaster, ForgeResult};
use crate::forge_master::ingot::Ingot;
use crate::forge_master::ore::Ore;

/// Lightweight per-frame container. Push Ores in; call `dispatch` to
/// hand the whole batch to ForgeMaster.
pub struct ComputeDispatchGraph {
    ores: Vec<Ore>,
}

impl Default for ComputeDispatchGraph {
    fn default() -> Self { Self::new() }
}

impl ComputeDispatchGraph {
    pub fn new() -> Self { Self { ores: Vec::new() } }

    pub fn is_empty(&self) -> bool { self.ores.is_empty() }
    pub fn len(&self)     -> usize { self.ores.len() }

    pub fn push(&mut self, ore: Ore) { self.ores.push(ore); }

    pub fn extend<I: IntoIterator<Item = Ore>>(&mut self, ores: I) {
        self.ores.extend(ores);
    }

    /// Submit every queued Ore as a single batched compute dispatch.
    /// Returns the produced ingots (one per Ore, in push order) +
    /// the signal semaphore the downstream graphics submit must wait
    /// on at vertex stages. Returns `(vec![], None)` when nothing was
    /// queued — the graphics submit just doesn't wait on anything.
    pub fn dispatch(self, forge: &mut ForgeMaster)
        -> ForgeResult<(Vec<Ingot>, Option<vk::Semaphore>)>
    {
        if self.ores.is_empty() {
            return Ok((Vec::new(), None));
        }
        let (ingots, sem) = forge.refine_batch_async(self.ores)?;
        Ok((ingots, Some(sem)))
    }
}
