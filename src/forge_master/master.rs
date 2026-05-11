use ash::vk;
use std::error::Error;
use std::fmt;

use super::forge::{Forge, write_forge_descriptors};
use super::ingot::Ingot;
use super::ore::{IngotSpec, Ore, OreKind};

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
        }
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

pub struct ForgeMaster {
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub command_pool: vk::CommandPool,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    descriptor_pool: vk::DescriptorPool,
    fence: vk::Fence,
    forges: Vec<Forge>,
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
            forges: Vec::new(),
        })
    }

    pub fn add_forge(&mut self, forge: Forge) {
        self.forges.retain(|existing| existing.kind != forge.kind);
        self.forges.push(forge);
    }

    pub fn add_forge_from_spirv_bytes(&mut self, kind: OreKind, spirv: &[u8]) -> ForgeResult<()> {
        let forge = Forge::from_spirv_bytes(&self.device, kind, spirv)?;
        self.add_forge(forge);
        Ok(())
    }

    pub fn add_forge_from_spirv_words(&mut self, kind: OreKind, spirv: &[u32]) -> ForgeResult<()> {
        let forge = Forge::from_spirv_words(&self.device, kind, spirv)?;
        self.add_forge(forge);
        Ok(())
    }

    pub fn forge(&self, kind: OreKind) -> Option<&Forge> {
        self.forges.iter().find(|forge| forge.kind == kind)
    }

    pub fn refine(&mut self, ore: Ore) -> ForgeResult<Ingot> {
        let forge_idx = self
            .forges
            .iter()
            .position(|forge| forge.kind == ore.kind)
            .ok_or(ForgeError::MissingForge(ore.kind))?;

        let mut staged = ore.stage(&self.device, &self.memory_properties)?;
        let mut ingot =
            Ingot::create(ore.kind, &ore.output, &self.device, &self.memory_properties)?;

        let descriptor_set =
            self.allocate_descriptor_set(self.forges[forge_idx].descriptor_layout())?;
        write_forge_descriptors(&self.device, descriptor_set, &staged, &ingot);

        let command_buffer = self.allocate_command_buffer()?;

        unsafe {
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device.begin_command_buffer(command_buffer, &begin)?;

            staged.record_upload(&self.device, command_buffer);
            ingot.record_prepare_for_compute(&self.device, command_buffer);
            self.forges[forge_idx].record_dispatch(
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
        for forge in &mut self.forges {
            unsafe { forge.destroy(&self.device) };
        }
        self.forges.clear();
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
