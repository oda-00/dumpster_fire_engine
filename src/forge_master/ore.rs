use ash::vk;
use std::mem::size_of;
use std::path::PathBuf;
use thin_vec::ThinVec;

use super::master::{ForgeError, ForgeResult};

// ── Graphics pipeline sub-kind ─────────────────────────────────────────────
//
// Lives inside `OreKind::Graphics(...)`. Each variant maps 1-to-1 with a
// registered GraphicsForge and drives the rasterization pipeline (dispatched
// during the render pass, not before it). Kept as its own enum so graphics
// forges can index a tight `[_; GraphicsOreKind::COUNT]` cache without
// scanning past compute slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GraphicsOreKind {
    /// Opaque geometry rendered with the forward-lit pipeline.
    ForwardLit,
    /// Skinned variant of ForwardLit — same fragment shader, but the
    /// vertex shader reads JOINTS_0 / WEIGHTS_0 from a second vertex
    /// binding and a `mat4[]` skin palette from set 2 binding 0 (SSBO,
    /// fed by the SkinPalette compute Ore).
    SkinnedForwardLit,
    /// Immediate-mode UI overlay rendered on top of the scene.
    Ui,
    /// KHR_gaussian_splatting raster. Reads pre-computed splat billboard
    /// vertices (clip-space pos + per-vertex ellipse_uv + colour) from a
    /// compute-shader output and alpha-blends them with depth test on,
    /// depth write off. Back-to-front order is enforced by the
    /// SplatSort compute Ore that runs ahead.
    GaussianSplat,
}

impl GraphicsOreKind {
    pub const ALL: [GraphicsOreKind; 4] = [
        GraphicsOreKind::ForwardLit,
        GraphicsOreKind::SkinnedForwardLit,
        GraphicsOreKind::Ui,
        GraphicsOreKind::GaussianSplat,
    ];

    pub const COUNT: usize = Self::ALL.len();

    pub const fn index(self) -> usize {
        self as usize
    }
}

// `OreKind` carries every dispatchable kind — compute variants are flat;
// rasterization is namespaced under `Graphics(GraphicsOreKind)` so the type
// system makes the compute/graphics split impossible to mix up. The compute
// forge cache only needs `COMPUTE_COUNT` slots; the full `ALL` / `COUNT`
// include the graphics variants so callers can enumerate everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OreKind {
    // Ray tracing.
    RayTrace,
    Denoise,

    // Compute / analysis.
    SignedDistanceField,
    SdfVoxelization,
    LightClustering,
    OcclusionCulling,

    // Graphics baking (still compute — produces data the rasterizer reads).
    MaterialFlattening,
    AmbientOcclusion,
    VisibilityPass,

    // Skinning / morph-blend (compute; run before graphics draws each frame).
    /// Builds a per-joint `[world × IBM]` palette from per-frame world matrices.
    SkinPalette,
    /// Applies weighted morph-target deltas to a rest-pose vertex buffer.
    MorphBlend,

    /// KHR_gaussian_splatting: bitonic sort of N splats by view-z. Run
    /// log²(N) times per frame to fully sort the splat list back-to-front
    /// before the GaussianSplat raster reads them.
    SplatSort,
    /// KHR_gaussian_splatting: per-splat projection + 2D covariance +
    /// 6-vertex billboard quad emission. Reads sorted splat indices from
    /// SplatSort output, emits one quad per splat into a vertex buffer
    /// the GaussianSplat raster pipeline consumes.
    SplatBillboard,

    // Rasterization — sub-kind selects the draw pipeline.
    Graphics(GraphicsOreKind),
}

impl OreKind {
    /// Compute-only kinds, in `index()` order. Use this to size the compute
    /// forge cache — graphics variants live in their own arena.
    pub const COMPUTE_ALL: [OreKind; 13] = [
        OreKind::RayTrace,
        OreKind::Denoise,
        OreKind::SignedDistanceField,
        OreKind::SdfVoxelization,
        OreKind::LightClustering,
        OreKind::OcclusionCulling,
        OreKind::MaterialFlattening,
        OreKind::AmbientOcclusion,
        OreKind::VisibilityPass,
        OreKind::SkinPalette,
        OreKind::MorphBlend,
        OreKind::SplatSort,
        OreKind::SplatBillboard,
    ];

    pub const COMPUTE_COUNT: usize = Self::COMPUTE_ALL.len();

    /// Every kind, compute + every graphics sub-kind, in `index()` order.
    pub const ALL: [OreKind; 17] = [
        OreKind::RayTrace,
        OreKind::Denoise,
        OreKind::SignedDistanceField,
        OreKind::SdfVoxelization,
        OreKind::LightClustering,
        OreKind::OcclusionCulling,
        OreKind::MaterialFlattening,
        OreKind::AmbientOcclusion,
        OreKind::VisibilityPass,
        OreKind::SkinPalette,
        OreKind::MorphBlend,
        OreKind::SplatSort,
        OreKind::SplatBillboard,
        OreKind::Graphics(GraphicsOreKind::ForwardLit),
        OreKind::Graphics(GraphicsOreKind::SkinnedForwardLit),
        OreKind::Graphics(GraphicsOreKind::Ui),
        OreKind::Graphics(GraphicsOreKind::GaussianSplat),
    ];

    pub const COUNT: usize = Self::ALL.len();

    /// Stable dense index across all variants. Compute variants occupy
    /// `0..COMPUTE_COUNT`; graphics sub-kinds extend the range past that.
    pub const fn index(self) -> usize {
        match self {
            OreKind::RayTrace            => 0,
            OreKind::Denoise             => 1,
            OreKind::SignedDistanceField => 2,
            OreKind::SdfVoxelization     => 3,
            OreKind::LightClustering     => 4,
            OreKind::OcclusionCulling    => 5,
            OreKind::MaterialFlattening  => 6,
            OreKind::AmbientOcclusion    => 7,
            OreKind::VisibilityPass      => 8,
            OreKind::SkinPalette         => 9,
            OreKind::MorphBlend          => 10,
            OreKind::SplatSort           => 11,
            OreKind::SplatBillboard      => 12,
            OreKind::Graphics(g)         => Self::COMPUTE_COUNT + g.index(),
        }
    }

    pub const fn is_graphics(self) -> bool {
        matches!(self, OreKind::Graphics(_))
    }

    pub const fn as_graphics(self) -> Option<GraphicsOreKind> {
        if let OreKind::Graphics(g) = self {
            Some(g)
        } else {
            None
        }
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
    pub vertices: ThinVec<ForgeVertex>,
    pub indices: ThinVec<u32>,
}

impl MeshOre {
    pub fn new(vertices: ThinVec<ForgeVertex>, indices: ThinVec<u32>) -> Self {
        Self { vertices, indices }
    }
}

#[derive(Debug, Clone)]
pub struct TextureOre {
    pub width: u32,
    pub height: u32,
    pub format: vk::Format,
    pub pixels: ThinVec<u8>,
}

impl TextureOre {
    pub fn new(width: u32, height: u32, format: vk::Format, pixels: ThinVec<u8>) -> Self {
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
    Bytes(ThinVec<u8>),
    Mesh(MeshOre),
    Texture(TextureOre),
    /// Two raw byte buffers — primary goes to binding 0, secondary to
    /// binding 1. Used by compute pipelines whose secondary input is
    /// neither an index list nor mesh-shaped (e.g. SkinPalette: primary =
    /// joint world matrices, secondary = inverse-bind matrices; both are
    /// `mat4[]` SSBOs, neither matches the `MeshOre` shape).
    DualBytes { primary: ThinVec<u8>, secondary: ThinVec<u8> },
    Empty,
}

#[derive(Debug, Clone)]
pub enum IngotSpec {
    Buffer {
        size: vk::DeviceSize,
        save_path: Option<PathBuf>,
        /// Extra `vk::BufferUsageFlags` to OR into the default
        /// `STORAGE_BUFFER | TRANSFER_SRC`. Compute pipelines whose output
        /// is consumed directly by a graphics draw (e.g. MorphBlend posed
        /// vertices, SkinPalette mat4[]) should pass `VERTEX_BUFFER` or
        /// `STORAGE_BUFFER` here so the same buffer is bindable downstream
        /// without an extra copy.
        extra_usage: vk::BufferUsageFlags,
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

    pub fn primary_bytes(&self) -> ThinVec<u8> {
        match &self.input {
            OreInput::DualBytes { primary, .. } => primary.clone(),
            OreInput::Bytes(bytes) => bytes.clone(),
            OreInput::Mesh(mesh) => vertices_as_bytes(&mesh.vertices).iter().copied().collect(),
            OreInput::Texture(texture) => texture.pixels.clone(),
            OreInput::Empty => thin_vec::thin_vec![0; 4],
        }
    }

    pub fn secondary_bytes(&self) -> ThinVec<u8> {
        match &self.input {
            OreInput::DualBytes { secondary, .. } if !secondary.is_empty() => secondary.clone(),
            OreInput::Mesh(mesh) if !mesh.indices.is_empty() => {
                indices_as_bytes(&mesh.indices).iter().copied().collect()
            }
            _ => thin_vec::thin_vec![0; 4],
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
        let dep_info = vk::DependencyInfo::default()
            .buffer_memory_barriers(&barriers);
        unsafe {
            device.cmd_pipeline_barrier2(command_buffer, &dep_info);
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

    pub fn read_bytes(&self, device: &ash::Device, len: vk::DeviceSize) -> ForgeResult<ThinVec<u8>> {
        let len = len.min(self.size) as usize;
        if len == 0 {
            return Ok(ThinVec::new());
        }
        let mut bytes: ThinVec<u8> = ThinVec::with_capacity(len);
        bytes.resize(len, 0u8);
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
    /// Mip level count baked into the image at creation time. `1` for
    /// single-mip / depth / storage images; > 1 for sampled textures that
    /// went through `create_2d_mip`.
    pub mip_levels: u32,
}

/// Infer the correct `ImageAspectFlags` from a format.
/// Depth-only formats → DEPTH; combined depth-stencil → DEPTH | STENCIL;
/// everything else (colour, storage) → COLOR.
pub fn aspect_mask_for_format(format: vk::Format) -> vk::ImageAspectFlags {
    match format {
        vk::Format::D16_UNORM
        | vk::Format::D32_SFLOAT
        | vk::Format::X8_D24_UNORM_PACK32 => vk::ImageAspectFlags::DEPTH,
        vk::Format::D16_UNORM_S8_UINT
        | vk::Format::D24_UNORM_S8_UINT
        | vk::Format::D32_SFLOAT_S8_UINT => {
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
        }
        _ => vk::ImageAspectFlags::COLOR,
    }
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

        let aspect_mask = aspect_mask_for_format(format);
        let view_info = vk::ImageViewCreateInfo::default()
            .image(handle)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(aspect_mask)
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
            mip_levels: 1,
        })
    }

    /// Same as `create_2d` but with a caller-chosen sample count. When
    /// `samples == TYPE_1` this is identical to `create_2d`; otherwise the
    /// resulting image is multi-sampled (no mips, one array layer). Used
    /// for MSAA colour + depth render-pass attachments.
    pub fn create_2d_msaa(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        properties: vk::MemoryPropertyFlags,
        samples: vk::SampleCountFlags,
    ) -> ForgeResult<Self> {
        let extent = vk::Extent3D { width: width.max(1), height: height.max(1), depth: 1 };
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(extent)
            .mip_levels(1)
            .array_layers(1)
            .samples(samples)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let handle = unsafe { device.create_image(&info, None)? };
        let req = unsafe { device.get_image_memory_requirements(handle) };
        let memory_type_index = find_memory_type(memory_properties, req.memory_type_bits, properties)?;
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(memory_type_index);
        let memory = unsafe { device.allocate_memory(&alloc, None)? };
        unsafe { device.bind_image_memory(handle, memory, 0)? };
        let aspect_mask = aspect_mask_for_format(format);
        let view_info = vk::ImageViewCreateInfo::default()
            .image(handle)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(aspect_mask)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        let view = unsafe { device.create_image_view(&view_info, None)? };
        Ok(Self { handle, view, memory, format, extent, mip_levels: 1 })
    }

    /// Same as `create_2d` but allocates the full mip chain
    /// (`1 + floor(log2(max(w,h)))` levels). The image view covers every
    /// level so the sampler can blend across them when `max_lod` permits.
    /// Caller is responsible for writing each level (`upload_texture_rgba`
    /// generates the lower levels via `vkCmdBlitImage` from level 0).
    pub fn create_2d_mip(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        properties: vk::MemoryPropertyFlags,
    ) -> ForgeResult<Self> {
        let w = width.max(1);
        let h = height.max(1);
        let mip_levels = 1 + (w.max(h) as f32).log2().floor() as u32;
        let extent = vk::Extent3D { width: w, height: h, depth: 1 };
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(extent)
            .mip_levels(mip_levels)
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

        let aspect_mask = aspect_mask_for_format(format);
        let view_info = vk::ImageViewCreateInfo::default()
            .image(handle)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(aspect_mask)
                    .base_mip_level(0)
                    .level_count(mip_levels)
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
            mip_levels,
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

// ── GpuMesh ────────────────────────────────────────────────────────────────
//
// A mesh uploaded to GPU-local memory as a vertex buffer + index buffer pair.
// GPU resources require explicit destruction via `destroy(device)`.
// Wrap in `Arc<GpuMesh>` when shared across draw calls; use `Arc::try_unwrap`
// in the owning arena's cleanup path to reclaim the allocation.

/// Column-major identity mat4 (64 bytes) for use as a push-constant default.
pub const MAT4_IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
];

/// Everything `GpuMesh::upload` needs in one bundle.
///
/// Best-practice version supports a **dedicated transfer queue** (a queue
/// family that exposes `TRANSFER` but not `GRAPHICS`/`COMPUTE` — typical on
/// discrete GPUs). When `transfer_family != graphics_family`, the upload
/// performs a queue-family **ownership release** on the transfer queue and
/// a matching **acquire** on the graphics queue so the buffer is legal to
/// read in the vertex shader stage.
///
/// When both families are equal (integrated GPU, fallback), the same code
/// path collapses to a normal single-queue submit — barriers degrade to a
/// pipeline barrier without an ownership transfer.
pub struct MeshUploadCtx<'a> {
    pub device:             &'a ash::Device,
    pub memory_properties:  &'a vk::PhysicalDeviceMemoryProperties,
    /// Queue used to perform the staging→device copy.
    pub transfer_queue:        vk::Queue,
    pub transfer_queue_family: u32,
    pub transfer_command_pool: vk::CommandPool,
    /// Queue family the buffer will eventually be read on (vertex shader).
    pub graphics_queue:        vk::Queue,
    pub graphics_queue_family: u32,
    /// Used to record the matching acquire barrier when families differ.
    /// May equal `transfer_command_pool` when families are equal.
    pub graphics_command_pool: vk::CommandPool,
}

#[derive(Debug)]
pub struct GpuMesh {
    pub vertex_buffer: ForgeBuffer,
    pub index_buffer:  ForgeBuffer,
    pub index_count:   u32,
}

impl GpuMesh {
    /// Upload `ore` to GPU-local vertex and index buffers using a dedicated
    /// transfer queue when available.
    ///
    /// Blocks until the transfer completes (one-shot path — fine for asset
    /// boot; for streaming, batch many meshes per submit and use a fence
    /// instead of `queue_wait_idle`).
    pub fn upload(ctx: &MeshUploadCtx, ore: &MeshOre) -> ForgeResult<Self> {
        let device            = ctx.device;
        let memory_properties = ctx.memory_properties;
        let vb_bytes = vertices_as_bytes(&ore.vertices);
        let ib_bytes = indices_as_bytes(&ore.indices);

        let vb_size = non_zero_size(vb_bytes.len() as vk::DeviceSize);
        let ib_size = non_zero_size(ib_bytes.len() as vk::DeviceSize);

        // Staging buffers (host-visible).
        let mut vb_stage = ForgeBuffer::create(
            device, memory_properties, vb_size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        vb_stage.write_bytes(device, vb_bytes)?;

        let mut ib_stage = ForgeBuffer::create(
            device, memory_properties, ib_size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        ib_stage.write_bytes(device, ib_bytes)?;

        // Device-local targets.
        let vertex_buffer = ForgeBuffer::create(
            device, memory_properties, vb_size,
            vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let index_buffer = ForgeBuffer::create(
            device, memory_properties, ib_size,
            vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;

        let need_ownership_xfer =
            ctx.transfer_queue_family != ctx.graphics_queue_family;

        // ── Transfer queue: copy + (optional) release barriers ──────────────
        unsafe {
            let transfer_cb = {
                let alloc = vk::CommandBufferAllocateInfo::default()
                    .command_pool(ctx.transfer_command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1);
                device.allocate_command_buffers(&alloc).map_err(ForgeError::Vk)?[0]
            };

            device.begin_command_buffer(
                transfer_cb,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            ).map_err(ForgeError::Vk)?;

            device.cmd_copy_buffer(
                transfer_cb, vb_stage.handle, vertex_buffer.handle,
                &[vk::BufferCopy::default().src_offset(0).dst_offset(0).size(vb_size)],
            );
            device.cmd_copy_buffer(
                transfer_cb, ib_stage.handle, index_buffer.handle,
                &[vk::BufferCopy::default().src_offset(0).dst_offset(0).size(ib_size)],
            );

            // Release ownership to graphics family if they differ.
            // Sync2: src_stage=COPY, dst_stage=NONE on the release half;
            // the acquire half on graphics queue restates the stages.
            if need_ownership_xfer {
                let release = [
                    vk::BufferMemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::COPY)
                        .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                        .dst_stage_mask(vk::PipelineStageFlags2::NONE)
                        .dst_access_mask(vk::AccessFlags2::NONE)
                        .src_queue_family_index(ctx.transfer_queue_family)
                        .dst_queue_family_index(ctx.graphics_queue_family)
                        .buffer(vertex_buffer.handle)
                        .offset(0).size(vb_size),
                    vk::BufferMemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::COPY)
                        .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                        .dst_stage_mask(vk::PipelineStageFlags2::NONE)
                        .dst_access_mask(vk::AccessFlags2::NONE)
                        .src_queue_family_index(ctx.transfer_queue_family)
                        .dst_queue_family_index(ctx.graphics_queue_family)
                        .buffer(index_buffer.handle)
                        .offset(0).size(ib_size),
                ];
                let dep_info = vk::DependencyInfo::default()
                    .buffer_memory_barriers(&release);
                device.cmd_pipeline_barrier2(transfer_cb, &dep_info);
            }

            device.end_command_buffer(transfer_cb).map_err(ForgeError::Vk)?;

            // Submit transfer; signal a semaphore if we need an acquire.
            let transfer_cbs = [transfer_cb];

            // Fence so we can free staging deterministically.
            let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?;

            // Semaphore only needed when the graphics queue must wait on
            // the transfer queue (i.e. different families OR different queues).
            let (sem, gfx_cb) = if need_ownership_xfer {
                let sem = device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                    .map_err(ForgeError::Vk)?;

                // Build graphics-side acquire command buffer.
                let alloc = vk::CommandBufferAllocateInfo::default()
                    .command_pool(ctx.graphics_command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1);
                let gfx_cb = device.allocate_command_buffers(&alloc).map_err(ForgeError::Vk)?[0];

                device.begin_command_buffer(
                    gfx_cb,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                ).map_err(ForgeError::Vk)?;

                let acquire = [
                    vk::BufferMemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::NONE)
                        .src_access_mask(vk::AccessFlags2::NONE)
                        .dst_stage_mask(vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT)
                        .dst_access_mask(vk::AccessFlags2::VERTEX_ATTRIBUTE_READ)
                        .src_queue_family_index(ctx.transfer_queue_family)
                        .dst_queue_family_index(ctx.graphics_queue_family)
                        .buffer(vertex_buffer.handle)
                        .offset(0).size(vb_size),
                    vk::BufferMemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::NONE)
                        .src_access_mask(vk::AccessFlags2::NONE)
                        .dst_stage_mask(vk::PipelineStageFlags2::INDEX_INPUT)
                        .dst_access_mask(vk::AccessFlags2::INDEX_READ)
                        .src_queue_family_index(ctx.transfer_queue_family)
                        .dst_queue_family_index(ctx.graphics_queue_family)
                        .buffer(index_buffer.handle)
                        .offset(0).size(ib_size),
                ];
                let dep_info = vk::DependencyInfo::default()
                    .buffer_memory_barriers(&acquire);
                device.cmd_pipeline_barrier2(gfx_cb, &dep_info);

                device.end_command_buffer(gfx_cb).map_err(ForgeError::Vk)?;

                (Some(sem), Some(gfx_cb))
            } else {
                (None, None)
            };

            if let Some(sem) = sem {
                let signal = [sem];
                let submit = vk::SubmitInfo::default()
                    .command_buffers(&transfer_cbs)
                    .signal_semaphores(&signal);
                device.queue_submit(ctx.transfer_queue, &[submit], vk::Fence::null())
                    .map_err(ForgeError::Vk)?;

                // Graphics-side acquire submit; signal the fence.
                let gfx_cbs = [gfx_cb.unwrap()];
                let wait_stages = [vk::PipelineStageFlags::VERTEX_INPUT];
                let wait = [sem];
                let submit_g = vk::SubmitInfo::default()
                    .wait_semaphores(&wait)
                    .wait_dst_stage_mask(&wait_stages)
                    .command_buffers(&gfx_cbs);
                device.queue_submit(ctx.graphics_queue, &[submit_g], fence)
                    .map_err(ForgeError::Vk)?;
            } else {
                let submit = vk::SubmitInfo::default().command_buffers(&transfer_cbs);
                device.queue_submit(ctx.transfer_queue, &[submit], fence)
                    .map_err(ForgeError::Vk)?;
            }

            // Wait for completion before freeing staging.
            device.wait_for_fences(&[fence], true, u64::MAX).map_err(ForgeError::Vk)?;
            device.destroy_fence(fence, None);
            if let Some(sem) = sem { device.destroy_semaphore(sem, None); }

            device.free_command_buffers(ctx.transfer_command_pool, &transfer_cbs);
            if let Some(gfx_cb) = gfx_cb {
                device.free_command_buffers(ctx.graphics_command_pool, &[gfx_cb]);
            }

            vb_stage.destroy(device);
            ib_stage.destroy(device);
        }

        Ok(Self { vertex_buffer, index_buffer, index_count: ore.indices.len() as u32 })
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            self.vertex_buffer.destroy(device);
            self.index_buffer.destroy(device);
        }
    }
}

/// Per-vertex skinning attributes uploaded as binding 1 on the
/// SkinnedForwardLit pipeline. Stride matches `SKIN_VERTEX_STRIDE` (24 B):
/// 4 × u16 joint indices packed into a uvec2, followed by 4 × f32 weights.
#[derive(Debug)]
pub struct GpuSkinBuffer {
    pub buffer:       ForgeBuffer,
    pub vertex_count: u32,
}

impl GpuSkinBuffer {
    /// `bytes` must be exactly `vertex_count * 24` bytes laid out as
    /// described above. The buffer is created with `VERTEX_BUFFER |
    /// TRANSFER_DST` usage and DEVICE_LOCAL memory; bytes are staged through
    /// a HOST_VISIBLE buffer and copied via a one-shot transfer cmd.
    pub fn upload(
        ctx:          &MeshUploadCtx,
        bytes:        &[u8],
        vertex_count: u32,
    ) -> ForgeResult<Self> {
        let size = bytes.len() as vk::DeviceSize;
        let size = if size == 0 { 24 } else { size };

        let mut staging = ForgeBuffer::create(
            ctx.device, ctx.memory_properties, size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        if !bytes.is_empty() {
            staging.write_bytes(ctx.device, bytes)?;
        }
        let buffer = ForgeBuffer::create(
            ctx.device, ctx.memory_properties, size,
            vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;

        unsafe {
            let info = vk::CommandBufferAllocateInfo::default()
                .command_pool(ctx.transfer_command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let cbs = ctx.device.allocate_command_buffers(&info)
                .map_err(ForgeError::Vk)?;
            let cb = cbs[0];
            ctx.device.begin_command_buffer(cb,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            ).map_err(ForgeError::Vk)?;
            let copy = vk::BufferCopy::default().src_offset(0).dst_offset(0).size(size);
            ctx.device.cmd_copy_buffer(cb, staging.handle, buffer.handle, &[copy]);
            ctx.device.end_command_buffer(cb).map_err(ForgeError::Vk)?;
            let fence = ctx.device.create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?;
            let submit = vk::SubmitInfo::default().command_buffers(&cbs);
            ctx.device.queue_submit(ctx.transfer_queue, &[submit], fence)
                .map_err(ForgeError::Vk)?;
            ctx.device.wait_for_fences(&[fence], true, u64::MAX).map_err(ForgeError::Vk)?;
            ctx.device.destroy_fence(fence, None);
            ctx.device.free_command_buffers(ctx.transfer_command_pool, &cbs);
            staging.destroy(ctx.device);
        }

        Ok(Self { buffer, vertex_count })
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe { self.buffer.destroy(device); }
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
) -> vk::BufferMemoryBarrier2<'static> {
    vk::BufferMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::COPY)
        .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
        .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
        .dst_access_mask(vk::AccessFlags2::SHADER_STORAGE_READ | vk::AccessFlags2::SHADER_STORAGE_WRITE)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(buffer)
        .offset(0)
        .size(size)
}

pub fn storage_buffer_readback_barrier(
    buffer: vk::Buffer,
    size: vk::DeviceSize,
) -> vk::BufferMemoryBarrier2<'static> {
    vk::BufferMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
        .src_access_mask(vk::AccessFlags2::SHADER_STORAGE_WRITE)
        .dst_stage_mask(vk::PipelineStageFlags2::COPY)
        .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
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
