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

pub struct ForgeMaster {
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub command_pool: vk::CommandPool,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    descriptor_pool: vk::DescriptorPool,
    fence: vk::Fence,
    forges: Arena<ForgeTag, Forge>,
    cache: [Option<ForgeCacheEntry>; OreKind::COMPUTE_COUNT],
    graphics_forges: Arena<GraphicsForgeTag, GraphicsForge>,
    graphics_cache: [Option<GraphicsForgeCacheEntry>; GraphicsOreKind::COUNT],
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
    let info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(max_sets)
        .pool_sizes(&pool_sizes);
    Ok(unsafe { device.create_descriptor_pool(&info, None)? })
}
