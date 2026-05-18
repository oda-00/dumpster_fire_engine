use ash::vk;
use std::error::Error;
use std::fmt;

use crate::resource_manager::manager::Arena;

use super::forge::{
    Forge, ForgeHandle, ForgeId, ForgeTag, GraphicsForge, GraphicsForgeHandle, GraphicsForgeId,
    GraphicsForgeTag, write_forge_descriptors,
};
use super::ingot::Ingot;
use super::ore::{GraphicsOreKind, IngotSpec, Ore, OreKind};

pub type ForgeResult<T> = Result<T, ForgeError>;

#[derive(Debug)]
pub enum ForgeError {
    Vk(vk::Result),
    Io(std::io::Error),
    NoMemoryType {
        type_bits: u32,
        properties: vk::MemoryPropertyFlags,
    },
    MissingForge(OreKind),
    EmptyShader {
        kind: OreKind,
    },
    NoCompatibleQueue,
    NoPhysicalDevice,
    LoaderUnavailable(ash::LoadingError),
}

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForgeError::Vk(err) => write!(f, "vulkan error: {err:?}"),
            ForgeError::Io(err) => write!(f, "io error: {err}"),
            ForgeError::NoMemoryType {
                type_bits,
                properties,
            } => {
                write!(
                    f,
                    "no memory type for bits {type_bits:#x} with properties {properties:?}"
                )
            }
            ForgeError::MissingForge(kind) => write!(f, "no forge registered for {kind:?}"),
            ForgeError::EmptyShader { kind } => write!(f, "forge shader for {kind:?} is empty"),
            ForgeError::NoCompatibleQueue => {
                write!(f, "no queue family supports COMPUTE on the selected device")
            }
            ForgeError::NoPhysicalDevice => {
                write!(f, "no Vulkan physical device available")
            }
            ForgeError::LoaderUnavailable(err) => {
                write!(f, "vulkan loader unavailable: {err}")
            }
        }
    }
}

impl From<ash::LoadingError> for ForgeError {
    fn from(value: ash::LoadingError) -> Self {
        ForgeError::LoaderUnavailable(value)
    }
}

impl Error for ForgeError {}

impl From<vk::Result> for ForgeError {
    fn from(value: vk::Result) -> Self {
        ForgeError::Vk(value)
    }
}

impl From<std::io::Error> for ForgeError {
    fn from(value: std::io::Error) -> Self {
        ForgeError::Io(value)
    }
}

// OreKind is the dispatch key during refine; ForgeId is the stable identity.
// Slotted directly by OreKind::index() so the cache is one indexed load —
// matches AssetArena::cache and Stage::cache. ForgeId lookup is a linear
// scan across <= OreKind::COMPUTE_COUNT entries. Graphics sub-kinds
// (`OreKind::Graphics(_)`) live in their own arena/cache and are rejected
// here.
#[derive(Clone, Copy)]
pub struct ForgeCacheEntry {
    pub id: ForgeId,
    pub handle: ForgeHandle,
}

/// Graphics analog of `ForgeCacheEntry`. Slotted by `GraphicsOreKind::index()`
/// — separate from the compute cache because graphics forges have their own
/// arena and don't share dispatch machinery.
#[derive(Clone, Copy)]
pub struct GraphicsForgeCacheEntry {
    pub id: GraphicsForgeId,
    pub handle: GraphicsForgeHandle,
}

/// Per-async-batch state tracked until the batch's fence has signalled.
/// At that point the resources can be returned to their pools.
struct PendingBatch {
    fence:           vk::Fence,
    semaphore:       vk::Semaphore,
    command_buffer:  vk::CommandBuffer,
    descriptor_sets: Vec<vk::DescriptorSet>,
    staged_inputs:   Vec<super::ore::StagedOre>,
}

pub struct ForgeMaster {
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub command_pool: vk::CommandPool,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    descriptor_pool: vk::DescriptorPool,
    /// Legacy fence used by the synchronous `refine()` path. The async
    /// path allocates a dedicated fence per submitted batch.
    fence: vk::Fence,
    forges: Arena<ForgeTag, Forge>,
    cache: [Option<ForgeCacheEntry>; OreKind::COMPUTE_COUNT],
    graphics_forges: Arena<GraphicsForgeTag, GraphicsForge>,
    graphics_cache: [Option<GraphicsForgeCacheEntry>; GraphicsOreKind::COUNT],
    /// Batches submitted via `refine_batch_async`. Front = oldest. At
    /// every async-call entry we drain completed batches from the front.
    pending: std::collections::VecDeque<PendingBatch>,
}

impl ForgeMaster {
    pub fn new(
        device: ash::Device,
        queue: vk::Queue,
        command_pool: vk::CommandPool,
        memory_properties: vk::PhysicalDeviceMemoryProperties,
    ) -> ForgeResult<Self> {
        Self::with_capacity(device, queue, command_pool, memory_properties, 256)
    }

    pub fn with_capacity(
        device: ash::Device,
        queue: vk::Queue,
        command_pool: vk::CommandPool,
        memory_properties: vk::PhysicalDeviceMemoryProperties,
        max_refines_in_pool: u32,
    ) -> ForgeResult<Self> {
        let descriptor_pool = create_descriptor_pool(&device, max_refines_in_pool)?;
        let fence_info = vk::FenceCreateInfo::default();
        let fence = unsafe { device.create_fence(&fence_info, None)? };

        Ok(Self {
            device,
            queue,
            command_pool,
            memory_properties,
            descriptor_pool,
            fence,
            forges: Arena::with_capacity(OreKind::COMPUTE_COUNT),
            cache: [None; OreKind::COMPUTE_COUNT],
            graphics_forges: Arena::with_capacity(GraphicsOreKind::COUNT),
            graphics_cache: [None; GraphicsOreKind::COUNT],
            pending: std::collections::VecDeque::new(),
        })
    }

    // At most one forge per OreKind. Inserting a second for the same kind
    // evicts and destroys the previous one so we don't leak its pipeline.
    pub fn add_forge(&mut self, forge: Forge) -> ForgeHandle {
        let kind_idx = forge.kind.index();
        if let Some(stale) = self.cache[kind_idx].take()
            && let Some(mut old) = self.forges.remove(stale.handle)
        {
            unsafe { old.destroy(&self.device) };
        }
        let id = forge.id;
        let handle = self.forges.insert(forge);
        self.cache[kind_idx] = Some(ForgeCacheEntry { id, handle });
        handle
    }

    pub fn add_forge_from_spirv_bytes(
        &mut self,
        id: ForgeId,
        kind: OreKind,
        spirv: &[u8],
    ) -> ForgeResult<ForgeHandle> {
        let forge = Forge::from_spirv_bytes(&self.device, id, kind, spirv)?;
        Ok(self.add_forge(forge))
    }

    pub fn add_forge_from_spirv_words(
        &mut self,
        id: ForgeId,
        kind: OreKind,
        spirv: &[u32],
    ) -> ForgeResult<ForgeHandle> {
        let forge = Forge::from_spirv_words(&self.device, id, kind, spirv)?;
        Ok(self.add_forge(forge))
    }

    pub fn forge(&self, kind: OreKind) -> Option<&Forge> {
        self.handle_for_kind(kind).and_then(|h| self.forges.get(h))
    }

    pub fn forge_by_id(&self, id: ForgeId) -> Option<&Forge> {
        self.handle_of(id).and_then(|h| self.forges.get(h))
    }

    pub fn handle_for_kind(&self, kind: OreKind) -> Option<ForgeHandle> {
        self.cache[kind.index()].map(|e| e.handle)
    }

    pub fn handle_of(&self, id: ForgeId) -> Option<ForgeHandle> {
        self.cache
            .iter()
            .flatten()
            .find(|e| e.id == id)
            .map(|e| e.handle)
    }

    // ── Graphics forge registration ────────────────────────────────────────
    //
    // GraphicsForge is bytecode-only — no Vulkan handles to destroy here. The
    // pipeline (GraphicsMold) is built on demand by Window via
    // `GraphicsForge::compile()` and the window owns/destroys it. So
    // re-inserting the same kind just evicts the old bytecode entry.

    pub fn add_graphics_forge(&mut self, forge: GraphicsForge) -> GraphicsForgeHandle {
        let kind_idx = forge.kind.index();
        if let Some(stale) = self.graphics_cache[kind_idx].take() {
            self.graphics_forges.remove(stale.handle);
        }
        let id = forge.id;
        let handle = self.graphics_forges.insert(forge);
        self.graphics_cache[kind_idx] = Some(GraphicsForgeCacheEntry { id, handle });
        handle
    }

    pub fn add_graphics_forge_from_spirv_bytes(
        &mut self,
        id: GraphicsForgeId,
        kind: GraphicsOreKind,
        vert_spv: &[u8],
        frag_spv: &[u8],
    ) -> ForgeResult<GraphicsForgeHandle> {
        let forge = GraphicsForge::from_spirv_bytes(id, kind, vert_spv, frag_spv)?;
        Ok(self.add_graphics_forge(forge))
    }

    pub fn graphics_forge(&self, kind: GraphicsOreKind) -> Option<&GraphicsForge> {
        self.graphics_handle_for_kind(kind)
            .and_then(|h| self.graphics_forges.get(h))
    }

    pub fn graphics_forge_by_id(&self, id: GraphicsForgeId) -> Option<&GraphicsForge> {
        self.graphics_handle_of(id)
            .and_then(|h| self.graphics_forges.get(h))
    }

    pub fn graphics_handle_for_kind(
        &self,
        kind: GraphicsOreKind,
    ) -> Option<GraphicsForgeHandle> {
        self.graphics_cache[kind.index()].map(|e| e.handle)
    }

    pub fn graphics_handle_of(&self, id: GraphicsForgeId) -> Option<GraphicsForgeHandle> {
        self.graphics_cache
            .iter()
            .flatten()
            .find(|e| e.id == id)
            .map(|e| e.handle)
    }

    pub fn refine(&mut self, ore: Ore) -> ForgeResult<Ingot> {
        // The sync path's `reset_descriptor_pool` at the bottom would
        // invalidate any descriptor sets still owned by pending async
        // batches. Drain them all here so the two modes can coexist.
        self.await_pending()?;
        let forge_handle = self
            .handle_for_kind(ore.kind)
            .ok_or(ForgeError::MissingForge(ore.kind))?;

        let mut staged = ore.stage(&self.device, &self.memory_properties)?;
        let mut ingot =
            Ingot::create(ore.kind, &ore.output, &self.device, &self.memory_properties)?;

        let descriptor_set =
            self.allocate_descriptor_set(self.forges[forge_handle].descriptor_layout())?;
        write_forge_descriptors(&self.device, descriptor_set, &staged, &ingot);

        let command_buffer = self.allocate_command_buffer()?;

        unsafe {
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device.begin_command_buffer(command_buffer, &begin)?;

            staged.record_upload(&self.device, command_buffer);
            ingot.record_prepare_for_compute(&self.device, command_buffer);
            self.forges[forge_handle].record_dispatch(
                &self.device,
                command_buffer,
                descriptor_set,
                ore.workgroups,
            );
            ingot.record_readback(&self.device, command_buffer);

            self.device.end_command_buffer(command_buffer)?;
            self.submit_and_wait(command_buffer)?;
            self.device
                .free_command_buffers(self.command_pool, &[command_buffer]);
            self.device.reset_descriptor_pool(
                self.descriptor_pool,
                vk::DescriptorPoolResetFlags::empty(),
            )?;
            staged.destroy(&self.device);
        }

        ingot.finish_readback(&self.device)?;
        Ok(ingot)
    }

    pub fn refine_to_spec(
        &mut self,
        kind: OreKind,
        input: super::ore::OreInput,
        output: IngotSpec,
        workgroups: [u32; 3],
    ) -> ForgeResult<Ingot> {
        self.refine(Ore::new(kind, input, output, workgroups))
    }

    /// Async batched compute: records every Ore in `ores` into a SINGLE
    /// command buffer, submits it with a freshly-allocated signal
    /// semaphore, and returns `(ingots, signal_semaphore)` without
    /// waiting on the GPU. The returned semaphore is owned by the
    /// `PendingBatch` tracked internally — callers MUST consume it on
    /// the wait-side of a subsequent submission (typically the per-frame
    /// graphics submit) within the next `FRAMES_IN_FLIGHT` frames so the
    /// pending sweep can free it.
    ///
    /// Cleanup happens automatically on the next `refine_batch_async`
    /// call OR `await_pending()` — any pending batches whose fence has
    /// signalled are reclaimed (command buffer freed, descriptor sets
    /// returned to the pool, staged inputs destroyed).
    ///
    /// Ingots returned here are GPU-resident handles. Reading them back
    /// to the CPU (`finish_readback`) before the semaphore signals is
    /// undefined; the caller should treat them as opaque until the
    /// semaphore's downstream wait completes.
    pub fn refine_batch_async(
        &mut self,
        ores: Vec<Ore>,
    ) -> ForgeResult<(Vec<Ingot>, vk::Semaphore)> {
        self.sweep_pending()?;

        // Per-batch resources.
        let fence = unsafe {
            self.device.create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?
        };
        let semaphore = unsafe {
            self.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?
        };
        let command_buffer = self.allocate_command_buffer()?;

        let mut ingots:          Vec<Ingot>             = Vec::with_capacity(ores.len());
        let mut descriptor_sets: Vec<vk::DescriptorSet> = Vec::with_capacity(ores.len());
        let mut staged_inputs:   Vec<super::ore::StagedOre> = Vec::with_capacity(ores.len());

        // Stage every ore's input and pre-allocate every ore's output.
        // Any failure here cleanly tears down only the resources we've
        // already allocated.
        let prepared: Result<
            Vec<(ForgeHandle, super::ore::StagedOre, Ingot, vk::DescriptorSet, [u32; 3])>,
            ForgeError,
        > = ores
            .into_iter()
            .map(|ore| {
                let forge_handle = self
                    .handle_for_kind(ore.kind)
                    .ok_or(ForgeError::MissingForge(ore.kind))?;
                let staged = ore.stage(&self.device, &self.memory_properties)?;
                let ingot = Ingot::create(ore.kind, &ore.output, &self.device, &self.memory_properties)?;
                let descriptor_set =
                    self.allocate_descriptor_set(self.forges[forge_handle].descriptor_layout())?;
                write_forge_descriptors(&self.device, descriptor_set, &staged, &ingot);
                Ok((forge_handle, staged, ingot, descriptor_set, ore.workgroups))
            })
            .collect();
        let prepared = match prepared {
            Ok(v) => v,
            Err(e) => {
                unsafe {
                    self.device.destroy_fence(fence, None);
                    self.device.destroy_semaphore(semaphore, None);
                    self.device.free_command_buffers(self.command_pool, &[command_buffer]);
                }
                return Err(e);
            }
        };

        // Record every dispatch into the single CB. Each ore gets
        // upload → prepare → dispatch (NO per-ore readback — the
        // semaphore models a GPU-internal handoff; CPU readback would
        // require a different completion model).
        unsafe {
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device.begin_command_buffer(command_buffer, &begin)
                .map_err(ForgeError::Vk)?;

            for (forge_handle, staged, ingot, descriptor_set, workgroups) in &prepared {
                staged.record_upload(&self.device, command_buffer);
                ingot.record_prepare_for_compute(&self.device, command_buffer);
                self.forges[*forge_handle].record_dispatch(
                    &self.device,
                    command_buffer,
                    *descriptor_set,
                    *workgroups,
                );
            }

            self.device.end_command_buffer(command_buffer)
                .map_err(ForgeError::Vk)?;

            // Submit via Sync2: signal the semaphore at COMPUTE_SHADER
            // stage so any downstream graphics wait at
            // VERTEX_ATTRIBUTE_INPUT|VERTEX_SHADER gets correct ordering.
            let cb_infos = [vk::CommandBufferSubmitInfo::default()
                .command_buffer(command_buffer)];
            let sig_infos = [vk::SemaphoreSubmitInfo::default()
                .semaphore(semaphore)
                .stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .value(0)];
            let submits = [vk::SubmitInfo2::default()
                .command_buffer_infos(&cb_infos)
                .signal_semaphore_infos(&sig_infos)];
            self.device
                .queue_submit2(self.queue, &submits, fence)
                .map_err(ForgeError::Vk)?;
        }

        // Decompose prepared into the per-batch tracking lists +
        // user-facing ingots.
        for (_, staged, ingot, descriptor_set, _) in prepared {
            staged_inputs.push(staged);
            descriptor_sets.push(descriptor_set);
            ingots.push(ingot);
        }

        self.pending.push_back(PendingBatch {
            fence,
            semaphore,
            command_buffer,
            descriptor_sets,
            staged_inputs,
        });

        Ok((ingots, semaphore))
    }

    /// Drain every pending batch whose fence has already signalled,
    /// returning its resources to the pool. Called automatically at the
    /// start of each `refine_batch_async` and on `await_pending` /
    /// `destroy`.
    fn sweep_pending(&mut self) -> ForgeResult<()> {
        while let Some(front) = self.pending.front() {
            let signalled = unsafe { self.device.get_fence_status(front.fence) };
            match signalled {
                Ok(true) => {
                    let batch = self.pending.pop_front().unwrap();
                    unsafe { self.destroy_pending_batch(batch); }
                }
                Ok(false) => break,
                Err(e) => return Err(ForgeError::Vk(e)),
            }
        }
        Ok(())
    }

    /// Block until every in-flight batch has completed and every
    /// resource has been freed. Used at shutdown and from synchronous
    /// fence-mode `refine()` so the legacy path doesn't observe stale
    /// async state.
    pub fn await_pending(&mut self) -> ForgeResult<()> {
        if self.pending.is_empty() { return Ok(()); }
        let fences: Vec<vk::Fence> = self.pending.iter().map(|p| p.fence).collect();
        unsafe { self.device.wait_for_fences(&fences, true, u64::MAX).map_err(ForgeError::Vk)? };
        while let Some(batch) = self.pending.pop_front() {
            unsafe { self.destroy_pending_batch(batch); }
        }
        Ok(())
    }

    unsafe fn destroy_pending_batch(&self, mut batch: PendingBatch) {
        unsafe {
            self.device.free_command_buffers(self.command_pool, &[batch.command_buffer]);
            if !batch.descriptor_sets.is_empty() {
                let _ = self.device.free_descriptor_sets(self.descriptor_pool, &batch.descriptor_sets);
            }
            for staged in batch.staged_inputs.iter_mut() {
                staged.destroy(&self.device);
            }
            if batch.semaphore != vk::Semaphore::null() {
                self.device.destroy_semaphore(batch.semaphore, None);
            }
            if batch.fence != vk::Fence::null() {
                self.device.destroy_fence(batch.fence, None);
            }
        }
    }

    unsafe fn submit_and_wait(&self, command_buffer: vk::CommandBuffer) -> ForgeResult<()> {
        unsafe { self.device.reset_fences(&[self.fence])? };
        let command_buffers = [command_buffer];
        let submit = [vk::SubmitInfo::default().command_buffers(&command_buffers)];
        unsafe {
            self.device.queue_submit(self.queue, &submit, self.fence)?;
            self.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
        }
        Ok(())
    }

    fn allocate_descriptor_set(
        &self,
        layout: vk::DescriptorSetLayout,
    ) -> ForgeResult<vk::DescriptorSet> {
        let layouts = [layout];
        let alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);
        let sets = unsafe { self.device.allocate_descriptor_sets(&alloc)? };
        Ok(sets[0])
    }

    fn allocate_command_buffer(&self) -> ForgeResult<vk::CommandBuffer> {
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let buffers = unsafe { self.device.allocate_command_buffers(&alloc)? };
        Ok(buffers[0])
    }

    pub unsafe fn destroy(&mut self) {
        // Drain any in-flight async batches BEFORE tearing down the
        // device-owned resources they reference. await_pending blocks
        // on every batch's fence and frees the per-batch state.
        let _ = self.await_pending();
        for forge in self.forges.values_mut() {
            unsafe { forge.destroy(&self.device); }
        }
        self.cache = [None; OreKind::COMPUTE_COUNT];
        unsafe {
            if self.fence != vk::Fence::null() {
                self.device.destroy_fence(self.fence, None);
                self.fence = vk::Fence::null();
            }
            if self.descriptor_pool != vk::DescriptorPool::null() {
                self.device
                    .destroy_descriptor_pool(self.descriptor_pool, None);
                self.descriptor_pool = vk::DescriptorPool::null();
            }
        }
    }
}

impl Drop for ForgeMaster {
    fn drop(&mut self) {
        unsafe { self.destroy() };
    }
}

fn create_descriptor_pool(device: &ash::Device, max_sets: u32) -> ForgeResult<vk::DescriptorPool> {
    let pool_sizes = [
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_BUFFER,
            descriptor_count: max_sets.saturating_mul(3),
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_IMAGE,
            descriptor_count: max_sets,
        },
    ];
    // FREE_DESCRIPTOR_SET so individual sets can be returned to the pool
    // once their owning async batch's fence has signalled, without
    // resetting the whole pool (which would invalidate in-flight sets
    // from concurrently-pending batches).
    let info = vk::DescriptorPoolCreateInfo::default()
        .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
        .max_sets(max_sets)
        .pool_sizes(&pool_sizes);
    Ok(unsafe { device.create_descriptor_pool(&info, None)? })
}
