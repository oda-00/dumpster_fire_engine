use ash::vk;
use std::mem::size_of;
use std::path::PathBuf;

use super::master::{ForgeError, ForgeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OreKind {
    // Ray tracing.
    RayTrace,
    Denoise,

    // Compute / analysis.
    SignedDistanceField,
    SdfVoxelization,
    LightClustering,
    OcclusionCulling,

    // Graphics baking.
    MaterialFlattening,
    AmbientOcclusion,
    VisibilityPass,
}

impl OreKind {
    pub const ALL: [OreKind; 9] = [
        OreKind::RayTrace,
        OreKind::Denoise,
        OreKind::SignedDistanceField,
        OreKind::SdfVoxelization,
        OreKind::LightClustering,
        OreKind::OcclusionCulling,
        OreKind::MaterialFlattening,
        OreKind::AmbientOcclusion,
        OreKind::VisibilityPass,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ForgeVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv: [f32; 2],
}

impl ForgeVertex {
    pub const fn new(
        position: [f32; 3],
        normal: [f32; 3],
        tangent: [f32; 4],
        uv: [f32; 2],
    ) -> Self {
        Self {
            position,
            normal,
            tangent,
            uv,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MeshOre {
    pub vertices: Vec<ForgeVertex>,
    pub indices: Vec<u32>,
}

impl MeshOre {
    pub fn new(vertices: Vec<ForgeVertex>, indices: Vec<u32>) -> Self {
        Self { vertices, indices }
    }
}

#[derive(Debug, Clone)]
pub struct TextureOre {
    pub width: u32,
    pub height: u32,
    pub format: vk::Format,
    pub pixels: Vec<u8>,
}

impl TextureOre {
    pub fn new(width: u32, height: u32, format: vk::Format, pixels: Vec<u8>) -> Self {
        Self {
            width,
            height,
            format,
            pixels,
        }
    }
}

#[derive(Debug, Clone)]
pub enum OreInput {
    Bytes(Vec<u8>),
    Mesh(MeshOre),
    Texture(TextureOre),
    Empty,
}

#[derive(Debug, Clone)]
pub enum IngotSpec {
    Buffer {
        size: vk::DeviceSize,
        save_path: Option<PathBuf>,
    },
    Image2d {
        width: u32,
        height: u32,
        format: vk::Format,
        byte_size: vk::DeviceSize,
        save_path: Option<PathBuf>,
    },
}

#[derive(Debug, Clone)]
pub struct Ore {
    pub kind: OreKind,
    pub input: OreInput,
    pub output: IngotSpec,
    pub workgroups: [u32; 3],
}

impl Ore {
    pub fn new(kind: OreKind, input: OreInput, output: IngotSpec, workgroups: [u32; 3]) -> Self {
        Self {
            kind,
            input,
            output,
            workgroups,
        }
    }

    pub fn primary_bytes(&self) -> Vec<u8> {
        match &self.input {
            OreInput::Bytes(bytes) => bytes.clone(),
            OreInput::Mesh(mesh) => vertices_as_bytes(&mesh.vertices).to_vec(),
            OreInput::Texture(texture) => texture.pixels.clone(),
            OreInput::Empty => vec![0; 4],
        }
    }

    pub fn secondary_bytes(&self) -> Vec<u8> {
        match &self.input {
            OreInput::Mesh(mesh) if !mesh.indices.is_empty() => {
                indices_as_bytes(&mesh.indices).to_vec()
            }
            _ => vec![0; 4],
        }
    }

    pub fn stage(
        &self,
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
    ) -> ForgeResult<StagedOre> {
        let primary_bytes = self.primary_bytes();
        let secondary_bytes = self.secondary_bytes();

        let primary_size = non_zero_size(primary_bytes.len() as vk::DeviceSize);
        let secondary_size = non_zero_size(secondary_bytes.len() as vk::DeviceSize);

        let mut primary_staging = ForgeBuffer::create(
            device,
            memory_properties,
            primary_size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        primary_staging.write_bytes(device, &primary_bytes)?;

        let primary = ForgeBuffer::create(
            device,
            memory_properties,
            primary_size,
            vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::STORAGE_BUFFER,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;

        let mut secondary_staging = ForgeBuffer::create(
            device,
            memory_properties,
            secondary_size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        secondary_staging.write_bytes(device, &secondary_bytes)?;

        let secondary = ForgeBuffer::create(
            device,
            memory_properties,
            secondary_size,
            vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::STORAGE_BUFFER,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;

        Ok(StagedOre {
            kind: self.kind,
            primary_staging,
            primary,
            secondary_staging,
            secondary,
        })
    }
}

pub struct StagedOre {
    pub kind: OreKind,
    pub primary_staging: ForgeBuffer,
    pub primary: ForgeBuffer,
    pub secondary_staging: ForgeBuffer,
    pub secondary: ForgeBuffer,
}

impl StagedOre {
    pub unsafe fn record_upload(&self, device: &ash::Device, command_buffer: vk::CommandBuffer) {
        let copies = [vk::BufferCopy::default()
            .src_offset(0)
            .dst_offset(0)
            .size(self.primary.size)];
        unsafe {
            device.cmd_copy_buffer(
                command_buffer,
                self.primary_staging.handle,
                self.primary.handle,
                &copies,
            );
        }

        let secondary_copies = [vk::BufferCopy::default()
            .src_offset(0)
            .dst_offset(0)
            .size(self.secondary.size)];
        unsafe {
            device.cmd_copy_buffer(
                command_buffer,
                self.secondary_staging.handle,
                self.secondary.handle,
                &secondary_copies,
            );
        }

        let barriers = [
            storage_buffer_upload_barrier(self.primary.handle, self.primary.size),
            storage_buffer_upload_barrier(self.secondary.handle, self.secondary.size),
        ];
        unsafe {
            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &barriers,
                &[],
            );
        }
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            self.primary_staging.destroy(device);
            self.primary.destroy(device);
            self.secondary_staging.destroy(device);
            self.secondary.destroy(device);
        }
    }
}

#[derive(Debug)]
pub struct ForgeBuffer {
    pub handle: vk::Buffer,
    pub memory: vk::DeviceMemory,
    pub size: vk::DeviceSize,
}

impl ForgeBuffer {
    pub fn create(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        size: vk::DeviceSize,
        usage: vk::BufferUsageFlags,
        properties: vk::MemoryPropertyFlags,
    ) -> ForgeResult<Self> {
        let size = non_zero_size(size);
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let handle = unsafe { device.create_buffer(&info, None)? };
        let req = unsafe { device.get_buffer_memory_requirements(handle) };
        let memory_type_index =
            find_memory_type(memory_properties, req.memory_type_bits, properties)?;
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(memory_type_index);
        let memory = unsafe { device.allocate_memory(&alloc, None)? };
        unsafe { device.bind_buffer_memory(handle, memory, 0)? };

        Ok(Self {
            handle,
            memory,
            size,
        })
    }

    pub fn write_bytes(&mut self, device: &ash::Device, bytes: &[u8]) -> ForgeResult<()> {
        let len = bytes.len().min(self.size as usize);
        if len == 0 {
            return Ok(());
        }
        unsafe {
            let ptr = device.map_memory(
                self.memory,
                0,
                len as vk::DeviceSize,
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), len);
            device.unmap_memory(self.memory);
        }
        Ok(())
    }

    pub fn read_bytes(&self, device: &ash::Device, len: vk::DeviceSize) -> ForgeResult<Vec<u8>> {
        let len = len.min(self.size) as usize;
        if len == 0 {
            return Ok(Vec::new());
        }
        let mut bytes = vec![0u8; len];
        unsafe {
            let ptr = device.map_memory(
                self.memory,
                0,
                len as vk::DeviceSize,
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(ptr.cast::<u8>(), bytes.as_mut_ptr(), len);
            device.unmap_memory(self.memory);
        }
        Ok(bytes)
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.handle != vk::Buffer::null() {
                device.destroy_buffer(self.handle, None);
                self.handle = vk::Buffer::null();
            }
            if self.memory != vk::DeviceMemory::null() {
                device.free_memory(self.memory, None);
                self.memory = vk::DeviceMemory::null();
            }
        }
    }
}

#[derive(Debug)]
pub struct ForgeImage {
    pub handle: vk::Image,
    pub view: vk::ImageView,
    pub memory: vk::DeviceMemory,
    pub format: vk::Format,
    pub extent: vk::Extent3D,
}

impl ForgeImage {
    pub fn create_2d(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        properties: vk::MemoryPropertyFlags,
    ) -> ForgeResult<Self> {
        let extent = vk::Extent3D {
            width: width.max(1),
            height: height.max(1),
            depth: 1,
        };
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(extent)
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        let handle = unsafe { device.create_image(&info, None)? };
        let req = unsafe { device.get_image_memory_requirements(handle) };
        let memory_type_index =
            find_memory_type(memory_properties, req.memory_type_bits, properties)?;
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(memory_type_index);
        let memory = unsafe { device.allocate_memory(&alloc, None)? };
        unsafe { device.bind_image_memory(handle, memory, 0)? };

        let view_info = vk::ImageViewCreateInfo::default()
            .image(handle)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        let view = unsafe { device.create_image_view(&view_info, None)? };

        Ok(Self {
            handle,
            view,
            memory,
            format,
            extent,
        })
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.view != vk::ImageView::null() {
                device.destroy_image_view(self.view, None);
                self.view = vk::ImageView::null();
            }
            if self.handle != vk::Image::null() {
                device.destroy_image(self.handle, None);
                self.handle = vk::Image::null();
            }
            if self.memory != vk::DeviceMemory::null() {
                device.free_memory(self.memory, None);
                self.memory = vk::DeviceMemory::null();
            }
        }
    }
}

pub fn find_memory_type(
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    properties: vk::MemoryPropertyFlags,
) -> ForgeResult<u32> {
    for i in 0..memory_properties.memory_type_count {
        let supported = (type_bits & (1 << i)) != 0;
        let has_properties = memory_properties.memory_types[i as usize]
            .property_flags
            .contains(properties);
        if supported && has_properties {
            return Ok(i);
        }
    }

    Err(ForgeError::NoMemoryType {
        type_bits,
        properties,
    })
}

pub fn non_zero_size(size: vk::DeviceSize) -> vk::DeviceSize {
    size.max(1)
}

pub fn storage_buffer_upload_barrier(
    buffer: vk::Buffer,
    size: vk::DeviceSize,
) -> vk::BufferMemoryBarrier<'static> {
    vk::BufferMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(buffer)
        .offset(0)
        .size(size)
}

pub fn storage_buffer_readback_barrier(
    buffer: vk::Buffer,
    size: vk::DeviceSize,
) -> vk::BufferMemoryBarrier<'static> {
    vk::BufferMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(buffer)
        .offset(0)
        .size(size)
}

fn vertices_as_bytes(vertices: &[ForgeVertex]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            vertices.len() * size_of::<ForgeVertex>(),
        )
    }
}

fn indices_as_bytes(indices: &[u32]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            indices.as_ptr().cast::<u8>(),
            std::mem::size_of_val(indices),
        )
    }
}
