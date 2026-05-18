//! GPU-side glTF resource driver (Phase 7).
//!
//! Bridges `forge_gltf::GltfAsset` (CPU-parsed, RGBA decoded) to Vulkan
//! GPU resources: textures, materials (descriptor sets), and meshes.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use ash::vk;

use forge_gltf::{GltfAsset, GltfError, Sampler as GltfSamplerDef};
use forge_gltf::texture::{MagFilter, MinFilter, WrapMode};
use forge_gltf::material::{AlphaMode, Material, TextureRef};

use crate::forge_master::ore::{ForgeBuffer, ForgeImage};
use crate::forge_master::master::ForgeError;
use crate::resource_manager::manager::{Arena, Handle};

// ─── Arena tag types (ZSTs — make Handle<Tag> distinct at compile time) ───────

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MaterialTag;
pub type MaterialHandle = Handle<MaterialTag>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TextureTag;
pub type TextureHandle = Handle<TextureTag>;

// ─── Sampler ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct GltfSampler {
    pub mag_filter:     vk::Filter,
    pub min_filter:     vk::Filter,
    pub mipmap_mode:    vk::SamplerMipmapMode,
    pub address_mode_u: vk::SamplerAddressMode,
    pub address_mode_v: vk::SamplerAddressMode,
}

impl Default for GltfSampler {
    fn default() -> Self {
        Self {
            mag_filter:     vk::Filter::LINEAR,
            min_filter:     vk::Filter::LINEAR,
            mipmap_mode:    vk::SamplerMipmapMode::LINEAR,
            address_mode_u: vk::SamplerAddressMode::REPEAT,
            address_mode_v: vk::SamplerAddressMode::REPEAT,
        }
    }
}

pub fn gltf_sampler_to_vk(s: &GltfSamplerDef) -> GltfSampler {
    let mag = match s.mag_filter {
        MagFilter::Linear  => vk::Filter::LINEAR,
        MagFilter::Nearest => vk::Filter::NEAREST,
    };
    let (min, mip) = match s.min_filter {
        MinFilter::Nearest              => (vk::Filter::NEAREST, vk::SamplerMipmapMode::NEAREST),
        MinFilter::Linear               => (vk::Filter::LINEAR,  vk::SamplerMipmapMode::NEAREST),
        MinFilter::NearestMipmapNearest => (vk::Filter::NEAREST, vk::SamplerMipmapMode::NEAREST),
        MinFilter::LinearMipmapNearest  => (vk::Filter::LINEAR,  vk::SamplerMipmapMode::NEAREST),
        MinFilter::NearestMipmapLinear  => (vk::Filter::NEAREST, vk::SamplerMipmapMode::LINEAR),
        MinFilter::LinearMipmapLinear   => (vk::Filter::LINEAR,  vk::SamplerMipmapMode::LINEAR),
    };
    let addr = |m: &WrapMode| match m {
        WrapMode::ClampToEdge    => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        WrapMode::MirroredRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
        WrapMode::Repeat         => vk::SamplerAddressMode::REPEAT,
    };
    GltfSampler {
        mag_filter:     mag,
        min_filter:     min,
        mipmap_mode:    mip,
        address_mode_u: addr(&s.wrap_s),
        address_mode_v: addr(&s.wrap_t),
    }
}

// ─── GPU texture ─────────────────────────────────────────────────────────────

pub struct GpuTexture {
    pub image:   ForgeImage,
    pub sampler: vk::Sampler,
}

impl GpuTexture {
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.sampler != vk::Sampler::null() {
                device.destroy_sampler(self.sampler, None);
                self.sampler = vk::Sampler::null();
            }
            self.image.destroy(device);
        }
    }
}

// ─── Material uniform ─────────────────────────────────────────────────────────

pub const TEXTURE_SLOT_COUNT: usize = 5;

/// Matches the std140 layout of `MaterialUbo` in
/// `assets/shaders/forward_lit.frag` exactly: 64 bytes total, 16-byte aligned.
/// The two `_pad` fields exist so `emissive_factor` lands at the 16-byte
/// boundary `vec3` requires in std140, and so the trailing `flags` rounds
/// the struct up to the next vec4 boundary.
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug)]
pub struct MaterialUniform {
    pub base_color_factor: [f32; 4], // offset 0
    pub metallic_factor:   f32,       // offset 16
    pub roughness_factor:  f32,       // offset 20
    pub _pad0:             [f32; 2],  // 24..32 — vec3 alignment
    pub emissive_factor:   [f32; 3],  // offset 32
    pub alpha_cutoff:      f32,       // offset 44
    /// bit0=doubleSided, bits1-2=alphaMode (Opaque=0, Mask=2, Blend=4)
    pub flags:             u32,       // offset 48
    pub _pad1:             [u32; 3],  // 52..64 — round struct to 64
}

impl Default for MaterialUniform {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0; 4],
            metallic_factor:   1.0,
            roughness_factor:  1.0,
            _pad0:             [0.0; 2],
            emissive_factor:   [0.0; 3],
            alpha_cutoff:      0.5,
            flags:             0,
            _pad1:             [0; 3],
        }
    }
}

impl MaterialUniform {
    pub fn from_gltf(mat: &Material) -> Self {
        // bit 0   doubleSided
        // bits 1-2 alphaMode (Opaque=0, Mask=2, Blend=4)
        // bit 3   KHR_materials_unlit
        // bit 4   KHR_materials_ior present (then alpha_cutoff doubles as IOR)
        let mut flags = (mat.double_sided as u32)
            | match mat.alpha_mode {
                AlphaMode::Opaque => 0,
                AlphaMode::Mask   => 2,
                AlphaMode::Blend  => 4,
            };
        if mat.unlit { flags |= 1 << 3; }
        // We only flag IOR when it deviates meaningfully from the default
        // 1.5 dielectric value — otherwise the shader can use its
        // hard-coded F0 = 0.04 and skip the alpha_cutoff override.
        let ior_active = (mat.ior - 1.5).abs() > 1e-3;
        if ior_active { flags |= 1 << 4; }

        // When the unlit flag isn't set AND IOR is active, the shader
        // borrows the alpha_cutoff slot for the IOR value (the only place
        // we currently have an unused float in this 64-byte UBO).
        let alpha_cutoff = if ior_active && !matches!(mat.alpha_mode, AlphaMode::Mask) {
            mat.ior
        } else {
            mat.alpha_cutoff
        };

        Self {
            base_color_factor: mat.pbr.base_color_factor,
            metallic_factor:   mat.pbr.metallic_factor,
            roughness_factor:  mat.pbr.roughness_factor,
            _pad0:             [0.0; 2],
            emissive_factor:   mat.emissive_factor,
            alpha_cutoff,
            flags,
            _pad1:             [0; 3],
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self).cast::<u8>(),
                std::mem::size_of::<Self>(),
            )
        }
    }
}

// ─── GPU material ─────────────────────────────────────────────────────────────

pub struct GpuMaterial {
    pub descriptor_set: vk::DescriptorSet,
    pub ubo_buffer:     ForgeBuffer,
    pub uniform:        MaterialUniform,
    pub textures:       [Option<TextureHandle>; TEXTURE_SLOT_COUNT],
}

impl GpuMaterial {
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe { self.ubo_buffer.destroy(device); }
    }
}

// ─── Upload context ───────────────────────────────────────────────────────────

pub struct GltfUploadCtx<'a> {
    pub device:              &'a ash::Device,
    pub memory_properties:   &'a vk::PhysicalDeviceMemoryProperties,
    pub graphics_queue:      vk::Queue,
    pub command_pool:        vk::CommandPool,
    pub material_set_layout: vk::DescriptorSetLayout,
    pub material_pool:       vk::DescriptorPool,
    /// Layout for set 3 — per-instance mat4 SSBO. One binding (binding 0 =
    /// STORAGE_BUFFER at vertex stage). Reused for both the dummy
    /// identity set and every per-draw instance set.
    pub instance_set_layout: vk::DescriptorSetLayout,
    /// Pool from which both the dummy identity set and per-frame
    /// per-draw instance sets are allocated. Sized for ~4096 sets +
    /// 4096 storage buffers — re-growable via
    /// FREE_DESCRIPTOR_SET_BIT if the engine needs more.
    pub instance_pool:       vk::DescriptorPool,
}

/// Build the descriptor-set layout for material set 1 — must match what the
/// ForwardLit fragment shader expects (binding 0 = MaterialUbo,
/// bindings 1–5 = COMBINED_IMAGE_SAMPLER for the 5 PBR texture slots).
pub fn create_material_set_layout(
    device: &ash::Device,
) -> Result<vk::DescriptorSetLayout, ForgeError> {
    let mut bindings = Vec::with_capacity(1 + TEXTURE_SLOT_COUNT);
    bindings.push(
        vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
    );
    for i in 0..TEXTURE_SLOT_COUNT {
        bindings.push(
            vk::DescriptorSetLayoutBinding::default()
                .binding((i + 1) as u32)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        );
    }
    let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    unsafe { device.create_descriptor_set_layout(&info, None).map_err(ForgeError::Vk) }
}

/// Single-binding descriptor set layout for the skin palette (set 2 on
/// the `SkinnedForwardLit` pipeline). Mirrors what `forge.rs` builds, so
/// callers that allocate sets from a pool can use either source — the
/// layouts compare structurally equal.
pub fn create_skin_palette_set_layout(
    device: &ash::Device,
) -> Result<vk::DescriptorSetLayout, ForgeError> {
    let bindings = [vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::VERTEX)];
    let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    unsafe { device.create_descriptor_set_layout(&info, None).map_err(ForgeError::Vk) }
}

/// Descriptor pool sized for `max_palettes` per-frame skin palette sets.
/// Created with `FREE_DESCRIPTOR_SET` so the per-frame churn doesn't leak
/// — call `vkResetDescriptorPool` at the top of each frame to recycle.
pub fn create_skin_palette_pool(
    device:       &ash::Device,
    max_palettes: u32,
) -> Result<vk::DescriptorPool, ForgeError> {
    let pool_sizes = [vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(max_palettes)];
    let info = vk::DescriptorPoolCreateInfo::default()
        .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
        .max_sets(max_palettes)
        .pool_sizes(&pool_sizes);
    unsafe { device.create_descriptor_pool(&info, None).map_err(ForgeError::Vk) }
}

/// Allocate + write one skin-palette descriptor set pointing at `buffer`.
/// Caller owns the returned set; reset the pool to free it.
pub fn allocate_skin_palette_set(
    device:  &ash::Device,
    pool:    vk::DescriptorPool,
    layout:  vk::DescriptorSetLayout,
    buffer:  vk::Buffer,
    range:   vk::DeviceSize,
) -> Result<vk::DescriptorSet, ForgeError> {
    let info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(std::slice::from_ref(&layout));
    let set = unsafe {
        device.allocate_descriptor_sets(&info).map_err(ForgeError::Vk)?.remove(0)
    };
    let buf_info = [vk::DescriptorBufferInfo::default()
        .buffer(buffer).offset(0).range(range)];
    let writes = [vk::WriteDescriptorSet::default()
        .dst_set(set).dst_binding(0)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .buffer_info(&buf_info)];
    unsafe { device.update_descriptor_sets(&writes, &[]); }
    Ok(set)
}

/// Create a descriptor pool sized for `max_materials` material slots.
/// Each slot consumes 1 uniform-buffer descriptor + `TEXTURE_SLOT_COUNT`
/// combined-image-sampler descriptors. `FREE_DESCRIPTOR_SET_BIT` is enabled
/// so individual sets can be freed on cache eviction.
pub fn create_material_pool(
    device:        &ash::Device,
    max_materials: u32,
) -> Result<vk::DescriptorPool, ForgeError> {
    let pool_sizes = [
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(max_materials),
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(max_materials * TEXTURE_SLOT_COUNT as u32),
    ];
    let info = vk::DescriptorPoolCreateInfo::default()
        .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
        .max_sets(max_materials)
        .pool_sizes(&pool_sizes);
    unsafe { device.create_descriptor_pool(&info, None).map_err(ForgeError::Vk) }
}

/// Build the descriptor-set layout for set 3 — per-instance mat4 SSBO
/// at binding 0 (vertex stage). Used by ForwardLit and
/// SkinnedForwardLit. Shared between the dummy identity set and every
/// per-draw instance set.
pub fn create_instance_set_layout(
    device: &ash::Device,
) -> Result<vk::DescriptorSetLayout, ForgeError> {
    let bindings = [vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::VERTEX)];
    let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    unsafe { device.create_descriptor_set_layout(&info, None).map_err(ForgeError::Vk) }
}

/// Pool sized for `max_sets` per-instance descriptor sets. Each set has
/// one STORAGE_BUFFER descriptor. Sets can be individually freed
/// (FREE_DESCRIPTOR_SET) so per-frame allocations don't leak.
pub fn create_instance_pool(
    device:   &ash::Device,
    max_sets: u32,
) -> Result<vk::DescriptorPool, ForgeError> {
    let pool_sizes = [vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(max_sets)];
    let info = vk::DescriptorPoolCreateInfo::default()
        .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
        .max_sets(max_sets)
        .pool_sizes(&pool_sizes);
    unsafe { device.create_descriptor_pool(&info, None).map_err(ForgeError::Vk) }
}

// ─── Cache ───────────────────────────────────────────────────────────────────

pub struct GltfCache {
    pub materials:    Arena<MaterialTag, GpuMaterial>,
    pub textures:     Arena<TextureTag,  GpuTexture>,
    dummy_tex:        Option<TextureHandle>,
    /// Sensible-defaults material used by any draw whose primitive has
    /// `material = None` or whose asset has zero materials. Without this
    /// the ForwardLit fragment shader's set 1 reads are undefined.
    dummy_material:   Option<vk::DescriptorSet>,
    /// Backing UBO for `dummy_material`; we own it to free it on drop.
    dummy_material_ubo: Option<ForgeBuffer>,
    /// Dummy single-element identity matrix SSBO + descriptor set for
    /// set 3. The vertex shader always reads
    /// `instances.m[gl_InstanceIndex]`, so non-instanced draws need
    /// this bound at slot 3 — index 0 reads identity and the
    /// multiplication is a no-op.
    dummy_instance_set:    Option<vk::DescriptorSet>,
    dummy_instance_buffer: Option<ForgeBuffer>,
    /// Per-frame instance buffers + descriptor sets allocated by
    /// `create_instance_matrices_set`. Tracked here so they can be
    /// freed (FREE_DESCRIPTOR_SET) on `flush_per_frame_instances`
    /// — typically called at the start of each frame before
    /// re-uploading.
    per_frame_instance_sets:    Vec<(vk::DescriptorSet, ForgeBuffer)>,
    /// Device handle stored so `Drop` can destroy every GPU resource the
    /// cache owns. `ash::Device` clones bump an inner Arc — cheap.
    device:           Option<ash::Device>,
}

impl GltfCache {
    /// Create an empty cache that will release every GPU resource it owns
    /// (textures, materials) when dropped.
    pub fn new(device: ash::Device) -> Self {
        Self {
            materials:          Arena::new(),
            textures:           Arena::new(),
            dummy_tex:          None,
            dummy_material:     None,
            dummy_material_ubo: None,
            dummy_instance_set:    None,
            dummy_instance_buffer: None,
            per_frame_instance_sets: Vec::new(),
            device:             Some(device),
        }
    }

    /// Legacy constructor kept for callers that bring their own destruction
    /// path (e.g. integration tests that use `device_wait_idle` then leak
    /// at process exit). New code should use `new(device)` instead.
    pub fn detached() -> Self {
        Self {
            materials:          Arena::new(),
            textures:           Arena::new(),
            dummy_tex:          None,
            dummy_material:     None,
            dummy_material_ubo: None,
            dummy_instance_set:    None,
            dummy_instance_buffer: None,
            per_frame_instance_sets: Vec::new(),
            device:          None,
        }
    }

    /// Ensure the 1×1 white dummy texture exists and return its handle.
    pub fn ensure_dummy_texture(
        &mut self,
        ctx: &GltfUploadCtx<'_>,
    ) -> Result<TextureHandle, ForgeError> {
        if let Some(h) = self.dummy_tex {
            return Ok(h);
        }
        let white = [255u8, 255, 255, 255];
        // Dummy is a constant white texel — linear UNORM is fine (no gamma
        // ambiguity in pure white).
        let tex = upload_texture_rgba(
            ctx, 1, 1, &white, &GltfSampler::default(),
            vk::Format::R8G8B8A8_UNORM,
        )?;
        let h = self.textures.insert(tex);
        self.dummy_tex = Some(h);
        Ok(h)
    }

    /// Ensure a default ForwardLit material descriptor set exists (white
    /// 1×1 albedo, default factors) and return its `vk::DescriptorSet`.
    /// Used as the fallback for any draw whose primitive lacks a material
    /// or whose asset has zero materials — without it, validation flags
    /// the shader's set 1 reads as accessing unbound descriptors.
    pub fn ensure_dummy_material(
        &mut self,
        ctx: &GltfUploadCtx<'_>,
    ) -> Result<vk::DescriptorSet, ForgeError> {
        if let Some(set) = self.dummy_material {
            return Ok(set);
        }
        let dummy_tex = self.ensure_dummy_texture(ctx)?;
        let uniform   = MaterialUniform::default();
        let device    = ctx.device;
        let mp        = ctx.memory_properties;
        let ubo_size  = std::mem::size_of::<MaterialUniform>() as vk::DeviceSize;

        // DEVICE_LOCAL UBO with one-shot staging — mirrors `create_material`.
        let mut staging = ForgeBuffer::create(
            device, mp, ubo_size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        staging.write_bytes(device, uniform.as_bytes())?;
        let ubo = ForgeBuffer::create(
            device, mp, ubo_size,
            vk::BufferUsageFlags::UNIFORM_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        unsafe {
            let cb = alloc_single_cmd(device, ctx.command_pool)?;
            let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?;
            device.begin_command_buffer(cb,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            ).map_err(ForgeError::Vk)?;
            let copy = vk::BufferCopy::default().size(ubo_size);
            device.cmd_copy_buffer(cb, staging.handle, ubo.handle, &[copy]);
            device.end_command_buffer(cb).map_err(ForgeError::Vk)?;
            device.queue_submit(
                ctx.graphics_queue,
                &[vk::SubmitInfo::default().command_buffers(&[cb])],
                fence,
            ).map_err(ForgeError::Vk)?;
            device.wait_for_fences(&[fence], true, u64::MAX).map_err(ForgeError::Vk)?;
            device.destroy_fence(fence, None);
            device.free_command_buffers(ctx.command_pool, &[cb]);
            staging.destroy(device);
        }

        // Allocate one set + bind dummy texture into all five slots.
        let set = unsafe {
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(ctx.material_pool)
                    .set_layouts(&[ctx.material_set_layout]),
            ).map_err(ForgeError::Vk)?.remove(0)
        };
        let buf_info = [vk::DescriptorBufferInfo::default()
            .buffer(ubo.handle).offset(0).range(ubo_size)];
        let dummy = self.textures.get(dummy_tex)
            .expect("dummy_tex just inserted above");
        let img_info = [vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image_view(dummy.image.view)
            .sampler(dummy.sampler)];
        let mut writes = vec![
            vk::WriteDescriptorSet::default()
                .dst_set(set).dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buf_info),
        ];
        for i in 0..TEXTURE_SLOT_COUNT {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_set(set).dst_binding((i + 1) as u32)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&img_info),
            );
        }
        unsafe { device.update_descriptor_sets(&writes, &[]); }

        self.dummy_material_ubo = Some(ubo);
        self.dummy_material     = Some(set);
        Ok(set)
    }

    pub fn dummy_material_set(&self) -> Option<vk::DescriptorSet> {
        self.dummy_material
    }

    /// Ensure the dummy single-identity per-instance SSBO + descriptor
    /// set exists. Bound at set 3 for every non-instanced draw so the
    /// vertex shader's `instances.m[gl_InstanceIndex]` read returns
    /// identity for `gl_InstanceIndex == 0`.
    pub fn ensure_dummy_instance_matrices(
        &mut self,
        ctx: &GltfUploadCtx<'_>,
    ) -> Result<vk::DescriptorSet, ForgeError> {
        if let Some(set) = self.dummy_instance_set {
            return Ok(set);
        }
        let identity: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(identity.as_ptr() as *const u8, 64)
        };
        let (set, buf) = create_instance_matrices_set_impl(ctx, bytes)?;
        self.dummy_instance_buffer = Some(buf);
        self.dummy_instance_set    = Some(set);
        Ok(set)
    }

    pub fn dummy_instance_set(&self) -> Option<vk::DescriptorSet> {
        self.dummy_instance_set
    }

    /// Allocate a per-draw instance descriptor set + DEVICE_LOCAL SSBO,
    /// upload `matrices`, and stash both for cleanup on the next
    /// `flush_per_frame_instances` call. Returns the descriptor set
    /// handle the caller binds at set 3.
    pub fn create_instance_matrices_set(
        &mut self,
        ctx:      &GltfUploadCtx<'_>,
        matrices: &[[f32; 16]],
    ) -> Result<vk::DescriptorSet, ForgeError> {
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                matrices.as_ptr() as *const u8,
                matrices.len() * 64,
            )
        };
        let (set, buf) = create_instance_matrices_set_impl(ctx, bytes)?;
        self.per_frame_instance_sets.push((set, buf));
        Ok(set)
    }

    /// Free every per-frame instance descriptor set + SSBO. Caller MUST
    /// have ensured the GPU is finished with last frame's draws (fence
    /// wait) before invoking — otherwise the device-local buffer
    /// destruction races with in-flight reads.
    pub fn flush_per_frame_instances(&mut self, device: &ash::Device, pool: vk::DescriptorPool) {
        let mut sets_to_free: Vec<vk::DescriptorSet> = Vec::new();
        for (set, mut buf) in self.per_frame_instance_sets.drain(..) {
            sets_to_free.push(set);
            unsafe { buf.destroy(device); }
        }
        if !sets_to_free.is_empty() {
            unsafe {
                let _ = device.free_descriptor_sets(pool, &sets_to_free);
            }
        }
    }

    pub fn texture(&self, h: TextureHandle) -> Option<&GpuTexture> {
        self.textures.get(h)
    }

    pub fn material(&self, h: MaterialHandle) -> Option<&GpuMaterial> {
        self.materials.get(h)
    }

    /// Explicitly destroy every GPU resource the cache owns. Called
    /// automatically from `Drop` when the cache was built via `new(device)`,
    /// but tests / advanced callers can invoke it manually before
    /// `device_wait_idle` returns, to be sure resources are gone before
    /// the device is destroyed.
    pub fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            for mat in self.materials.values_mut() { mat.destroy(device); }
            for tex in self.textures.values_mut()  { tex.destroy(device); }
            if let Some(mut ubo) = self.dummy_material_ubo.take() {
                ubo.destroy(device);
            }
            if let Some(mut buf) = self.dummy_instance_buffer.take() {
                buf.destroy(device);
            }
            for (_, mut buf) in self.per_frame_instance_sets.drain(..) {
                buf.destroy(device);
            }
        }
    }
}

/// Allocate one descriptor set against the instance-set layout, upload
/// `bytes` into a fresh DEVICE_LOCAL STORAGE_BUFFER, and bind the
/// buffer at binding 0 of the set. Caller owns the returned buffer
/// until the next pool reset or explicit destroy.
fn create_instance_matrices_set_impl(
    ctx:   &GltfUploadCtx<'_>,
    bytes: &[u8],
) -> Result<(vk::DescriptorSet, ForgeBuffer), ForgeError> {
    let device = ctx.device;
    let mp     = ctx.memory_properties;
    let size   = bytes.len() as vk::DeviceSize;

    // Staging upload pattern shared with other GPU resource paths.
    let mut staging = ForgeBuffer::create(
        device, mp, size,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    staging.write_bytes(device, bytes)?;
    let buf = ForgeBuffer::create(
        device, mp, size,
        vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;
    unsafe {
        let cb = alloc_single_cmd(device, ctx.command_pool)?;
        let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)
            .map_err(ForgeError::Vk)?;
        device.begin_command_buffer(cb,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        ).map_err(ForgeError::Vk)?;
        device.cmd_copy_buffer(cb, staging.handle, buf.handle,
            &[vk::BufferCopy::default().size(size)]);
        device.end_command_buffer(cb).map_err(ForgeError::Vk)?;
        device.queue_submit(ctx.graphics_queue,
            &[vk::SubmitInfo::default().command_buffers(&[cb])], fence)
            .map_err(ForgeError::Vk)?;
        device.wait_for_fences(&[fence], true, u64::MAX).map_err(ForgeError::Vk)?;
        device.destroy_fence(fence, None);
        device.free_command_buffers(ctx.command_pool, &[cb]);
        staging.destroy(device);
    }

    let set = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(ctx.instance_pool)
                .set_layouts(&[ctx.instance_set_layout]),
        ).map_err(ForgeError::Vk)?.remove(0)
    };
    let buf_info = [vk::DescriptorBufferInfo::default()
        .buffer(buf.handle).offset(0).range(size)];
    let writes = [vk::WriteDescriptorSet::default()
        .dst_set(set).dst_binding(0)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .buffer_info(&buf_info)];
    unsafe { device.update_descriptor_sets(&writes, &[]); }
    Ok((set, buf))
}

impl Drop for GltfCache {
    fn drop(&mut self) {
        // Only auto-destroy when we own a device handle. Tests that opt out
        // (via `detached()`) are responsible for tearing down themselves.
        if let Some(device) = self.device.take() {
            // Drain the GPU before touching descriptor pools / image memory
            // — the cache may still be referenced by an in-flight draw.
            unsafe { let _ = device.device_wait_idle(); }
            self.destroy(&device);
        }
    }
}

// ─── Texture upload ───────────────────────────────────────────────────────────

pub fn upload_texture_rgba(
    ctx:     &GltfUploadCtx<'_>,
    width:   u32,
    height:  u32,
    rgba:    &[u8],
    sampler: &GltfSampler,
    format:  vk::Format,
) -> Result<GpuTexture, ForgeError> {
    let device = ctx.device;
    let mp     = ctx.memory_properties;
    let w = width.max(1);
    let h = height.max(1);
    let size = (w as usize) * (h as usize) * 4;

    let mut staging = ForgeBuffer::create(
        device, mp,
        size as vk::DeviceSize,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    let write_len = size.min(rgba.len());
    if write_len > 0 {
        staging.write_bytes(device, &rgba[..write_len])?;
    }

    // Full mip chain — needed for trilinear / minified sampling to look
    // right. `vkCmdBlitImage` halves the extent per level using linear
    // filtering, which approximates a 2×2 box filter cheaply on the GPU.
    let image = ForgeImage::create_2d_mip(
        device, mp, w, h,
        format,
        vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::TRANSFER_SRC
            | vk::ImageUsageFlags::TRANSFER_DST,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;
    let mip_levels = image.mip_levels;

    unsafe {
        let cb = alloc_single_cmd(device, ctx.command_pool)?;
        let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)
            .map_err(ForgeError::Vk)?;
        device.begin_command_buffer(cb,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        ).map_err(ForgeError::Vk)?;

        // ── Step 1: transition every level UNDEFINED → TRANSFER_DST so the
        //    buffer-to-image copy targets level 0 and the subsequent blits
        //    can target levels 1..N safely.
        let barriers_1 = [image_layout_barrier_mips(image.handle,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::PipelineStageFlags2::NONE,
            vk::AccessFlags2::NONE,
            vk::PipelineStageFlags2::COPY,
            vk::AccessFlags2::TRANSFER_WRITE,
            0, mip_levels)];
        device.cmd_pipeline_barrier2(cb,
            &vk::DependencyInfo::default().image_memory_barriers(&barriers_1));

        // ── Step 2: copy staging buffer into level 0.
        let region = vk::BufferImageCopy::default()
            .buffer_offset(0).buffer_row_length(0).buffer_image_height(0)
            .image_subresource(vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .mip_level(0).base_array_layer(0).layer_count(1))
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D { width: w, height: h, depth: 1 });
        device.cmd_copy_buffer_to_image(cb, staging.handle, image.handle,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[region]);

        // ── Step 3: build the rest of the mip chain by blitting level i-1
        //    (TRANSFER_SRC) → level i (TRANSFER_DST), one level at a time.
        let (mut mip_w, mut mip_h) = (w as i32, h as i32);
        for i in 1..mip_levels {
            // Transition the source level from TRANSFER_DST to TRANSFER_SRC.
            let barriers_blit = [image_layout_barrier_mips(image.handle,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::PipelineStageFlags2::COPY,
                vk::AccessFlags2::TRANSFER_WRITE,
                vk::PipelineStageFlags2::BLIT,
                vk::AccessFlags2::TRANSFER_READ,
                i - 1, 1)];
            device.cmd_pipeline_barrier2(cb,
                &vk::DependencyInfo::default().image_memory_barriers(&barriers_blit));

            let next_w = (mip_w / 2).max(1);
            let next_h = (mip_h / 2).max(1);
            let blit = vk::ImageBlit::default()
                .src_offsets([
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D { x: mip_w, y: mip_h, z: 1 },
                ])
                .src_subresource(vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(i - 1).base_array_layer(0).layer_count(1))
                .dst_offsets([
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D { x: next_w, y: next_h, z: 1 },
                ])
                .dst_subresource(vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(i).base_array_layer(0).layer_count(1));
            device.cmd_blit_image(cb,
                image.handle, vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                image.handle, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[blit], vk::Filter::LINEAR);

            mip_w = next_w;
            mip_h = next_h;
        }

        // ── Step 4: transition every level to SHADER_READ. Levels 0..mip-2
        //    are currently TRANSFER_SRC (we just blitted out of them);
        //    level mip-1 is still TRANSFER_DST (last blit destination).
        if mip_levels > 1 {
            let barriers_src = [image_layout_barrier_mips(image.handle,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags2::BLIT,
                vk::AccessFlags2::TRANSFER_READ,
                vk::PipelineStageFlags2::FRAGMENT_SHADER,
                vk::AccessFlags2::SHADER_SAMPLED_READ,
                0, mip_levels - 1)];
            device.cmd_pipeline_barrier2(cb,
                &vk::DependencyInfo::default().image_memory_barriers(&barriers_src));
        }
        let barriers_last = [image_layout_barrier_mips(image.handle,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags2::COPY,
            vk::AccessFlags2::TRANSFER_WRITE,
            vk::PipelineStageFlags2::FRAGMENT_SHADER,
            vk::AccessFlags2::SHADER_SAMPLED_READ,
            mip_levels - 1, 1)];
        device.cmd_pipeline_barrier2(cb,
            &vk::DependencyInfo::default().image_memory_barriers(&barriers_last));

        device.end_command_buffer(cb).map_err(ForgeError::Vk)?;
        device.queue_submit(ctx.graphics_queue,
            &[vk::SubmitInfo::default().command_buffers(&[cb])], fence)
            .map_err(ForgeError::Vk)?;
        device.wait_for_fences(&[fence], true, u64::MAX).map_err(ForgeError::Vk)?;
        device.destroy_fence(fence, None);
        device.free_command_buffers(ctx.command_pool, &[cb]);
        staging.destroy(device);
    }

    let sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(sampler.mag_filter)
        .min_filter(sampler.min_filter)
        .mipmap_mode(sampler.mipmap_mode)
        .address_mode_u(sampler.address_mode_u)
        .address_mode_v(sampler.address_mode_v)
        .address_mode_w(vk::SamplerAddressMode::REPEAT)
        .anisotropy_enable(false)
        .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
        .unnormalized_coordinates(false)
        .compare_enable(false)
        .mip_lod_bias(0.0)
        .min_lod(0.0)
        // Now that every level is populated, let the sampler walk the full
        // chain. The glTF sampler's mipmap mode + min_filter pick whether
        // we get nearest/linear interpolation between levels.
        .max_lod(mip_levels as f32);
    let vk_sampler = unsafe {
        device.create_sampler(&sampler_info, None).map_err(ForgeError::Vk)?
    };
    Ok(GpuTexture { image, sampler: vk_sampler })
}

// ─── Material creation ────────────────────────────────────────────────────────

/// Resolve a `TextureRef` → the source image index it points to.
fn resolve_tex_index(opt: Option<&TextureRef>, asset: &GltfAsset) -> Option<usize> {
    let tref = opt?;
    let tex   = asset.textures.get(tref.texture as usize)?;
    Some(tex.image as usize)
}

pub fn create_material(
    mat:         &Material,
    asset:       &GltfAsset,
    img_handles: &[Option<TextureHandle>],
    ctx:         &GltfUploadCtx<'_>,
    cache:       &mut GltfCache,
) -> Result<GpuMaterial, ForgeError> {
    let device = ctx.device;
    let mp     = ctx.memory_properties;
    let uniform = MaterialUniform::from_gltf(mat);

    let ubo_size = std::mem::size_of::<MaterialUniform>() as vk::DeviceSize;
    // DEVICE_LOCAL UBO + one-shot staging copy: materials are written
    // exactly once at upload time and then read every fragment shader
    // invocation, so the cost of the staging copy is paid in full by
    // not having every fragment go through the PCIe-mapped HOST_VISIBLE
    // heap. The staging buffer is freed before this function returns.
    let mut staging = ForgeBuffer::create(
        device, mp, ubo_size,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    staging.write_bytes(device, uniform.as_bytes())?;
    let ubo = ForgeBuffer::create(
        device, mp, ubo_size,
        vk::BufferUsageFlags::UNIFORM_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;
    unsafe {
        let cb = alloc_single_cmd(device, ctx.command_pool)?;
        let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)
            .map_err(ForgeError::Vk)?;
        device.begin_command_buffer(cb,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        ).map_err(ForgeError::Vk)?;
        let copy = vk::BufferCopy::default().src_offset(0).dst_offset(0).size(ubo_size);
        device.cmd_copy_buffer(cb, staging.handle, ubo.handle, &[copy]);
        device.end_command_buffer(cb).map_err(ForgeError::Vk)?;
        device.queue_submit(
            ctx.graphics_queue,
            &[vk::SubmitInfo::default().command_buffers(&[cb])],
            fence,
        ).map_err(ForgeError::Vk)?;
        device.wait_for_fences(&[fence], true, u64::MAX).map_err(ForgeError::Vk)?;
        device.destroy_fence(fence, None);
        device.free_command_buffers(ctx.command_pool, &[cb]);
        staging.destroy(device);
    }

    let dummy = cache.ensure_dummy_texture(ctx)?;

    let resolve = |opt: Option<&TextureRef>| -> TextureHandle {
        resolve_tex_index(opt, asset)
            .and_then(|i| img_handles.get(i).copied().flatten())
            .unwrap_or(dummy)
    };

    let slots: [Option<TextureHandle>; TEXTURE_SLOT_COUNT] = [
        Some(resolve(mat.pbr.base_color_texture.as_ref())),
        Some(resolve(mat.pbr.metallic_roughness_texture.as_ref())),
        Some(resolve(mat.normal.texture.as_ref())),
        Some(resolve(mat.emissive_texture.as_ref())),
        Some(resolve(mat.occlusion.texture.as_ref())),
    ];

    let set = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(ctx.material_pool)
                .set_layouts(&[ctx.material_set_layout]),
        ).map_err(ForgeError::Vk)?.remove(0)
    };

    let buf_info = [vk::DescriptorBufferInfo::default()
        .buffer(ubo.handle).offset(0).range(ubo_size)];
    let mut writes = vec![
        vk::WriteDescriptorSet::default()
            .dst_set(set).dst_binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .buffer_info(&buf_info),
    ];
    // Build image infos separately to ensure they live long enough.
    let img_infos: Vec<[vk::DescriptorImageInfo; 1]> = slots.iter().map(|&s| {
        let h = s.unwrap_or(dummy);
        let (view, sampler) = cache.textures.get(h)
            .map(|t| (t.image.view, t.sampler))
            .unwrap_or((vk::ImageView::null(), vk::Sampler::null()));
        [vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image_view(view)
            .sampler(sampler)]
    }).collect();
    for (i, info) in img_infos.iter().enumerate() {
        writes.push(
            vk::WriteDescriptorSet::default()
                .dst_set(set).dst_binding((i + 1) as u32)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(info),
        );
    }
    unsafe { device.update_descriptor_sets(&writes, &[]); }

    Ok(GpuMaterial { descriptor_set: set, ubo_buffer: ubo, uniform, textures: slots })
}

// ─── Full document load ───────────────────────────────────────────────────────

pub fn load_full_document(
    path:  &Path,
    ctx:   &GltfUploadCtx<'_>,
    cache: &mut GltfCache,
) -> Result<(GltfAsset, Vec<MaterialHandle>), GltfError> {
    let asset = GltfAsset::load(path)?;

    // For each image, look up which sampler its first referencing texture uses.
    let n_images = asset.images.len();
    let mut img_samplers: Vec<GltfSampler> = vec![GltfSampler::default(); n_images];
    for tex in &asset.textures {
        let idx = tex.image as usize;
        if idx < n_images {
            if let Some(sampler_idx) = tex.sampler {
                if let Some(s) = asset.samplers.get(sampler_idx as usize) {
                    img_samplers[idx] = gltf_sampler_to_vk(s);
                }
            }
        }
    }

    // Upload all images → TextureHandle per image index.
    //
    // The Vulkan format is chosen from the per-image `format` hint that
    // `forge_gltf` set during extraction: sRGB-encoded source data (albedo,
    // emissive) goes to R8G8B8A8_SRGB so the sampler does the de-gamma
    // automatically; everything else (normal, metallicRoughness, occlusion,
    // data) goes to R8G8B8A8_UNORM. Without this distinction sRGB textures
    // come out about 2× too bright in linear shading.
    let img_handles: Vec<Option<TextureHandle>> = asset.images
        .iter()
        .enumerate()
        .map(|(i, img)| {
            let fmt = match img.format {
                forge_gltf::ImageFormatHint::Srgb   => vk::Format::R8G8B8A8_SRGB,
                forge_gltf::ImageFormatHint::Linear => vk::Format::R8G8B8A8_UNORM,
            };
            upload_texture_rgba(ctx, img.width, img.height, &img.rgba, &img_samplers[i], fmt)
                .ok()
                .map(|t| cache.textures.insert(t))
        })
        .collect();

    // Create GPU materials.
    let mat_handles: Vec<MaterialHandle> = asset.materials
        .iter()
        .map(|m| {
            create_material(m, &asset, &img_handles, ctx, cache)
                .map(|gm| cache.materials.insert(gm))
                .unwrap_or_else(|_| {
                    cache.materials.insert(GpuMaterial {
                        descriptor_set: vk::DescriptorSet::null(),
                        ubo_buffer: unsafe { std::mem::zeroed() },
                        uniform:    MaterialUniform::default(),
                        textures:   [None; TEXTURE_SLOT_COUNT],
                    })
                })
        })
        .collect();

    Ok((asset, mat_handles))
}

// ─── Async loader ─────────────────────────────────────────────────────────────

pub struct AsyncGltfLoader {
    rx: mpsc::Receiver<Result<GltfAsset, GltfError>>,
}

impl AsyncGltfLoader {
    pub fn spawn(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            tx.send(GltfAsset::load(&path)).ok();
        });
        Self { rx }
    }

    pub fn try_recv(&mut self) -> Option<Result<GltfAsset, GltfError>> {
        self.rx.try_recv().ok()
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

unsafe fn alloc_single_cmd(
    device: &ash::Device,
    pool:   vk::CommandPool,
) -> Result<vk::CommandBuffer, ForgeError> {
    unsafe {
        device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        ).map(|mut v| v.remove(0)).map_err(ForgeError::Vk)
    }
}

/// Image-memory layout transition barrier for a specific mip range. Used
/// by the mipmap generation pipeline in `upload_texture_rgba` to flip
/// individual levels between TRANSFER_SRC and TRANSFER_DST while
/// building the chain; single-level callers pass `base_mip=0, level_count=1`.
fn image_layout_barrier_mips(
    image:        vk::Image,
    old_layout:   vk::ImageLayout,
    new_layout:   vk::ImageLayout,
    src_stage:    vk::PipelineStageFlags2,
    src_access:   vk::AccessFlags2,
    dst_stage:    vk::PipelineStageFlags2,
    dst_access:   vk::AccessFlags2,
    base_mip:     u32,
    level_count:  u32,
) -> vk::ImageMemoryBarrier2<'static> {
    vk::ImageMemoryBarrier2::default()
        .src_stage_mask(src_stage)
        .src_access_mask(src_access)
        .dst_stage_mask(dst_stage)
        .dst_access_mask(dst_access)
        .old_layout(old_layout)
        .new_layout(new_layout)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(base_mip).level_count(level_count)
            .base_array_layer(0).layer_count(1))
}
