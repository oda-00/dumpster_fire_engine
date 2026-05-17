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
        let flags = (mat.double_sided as u32)
            | match mat.alpha_mode {
                AlphaMode::Opaque => 0,
                AlphaMode::Mask   => 2,
                AlphaMode::Blend  => 4,
            };
        Self {
            base_color_factor: mat.pbr.base_color_factor,
            metallic_factor:   mat.pbr.metallic_factor,
            roughness_factor:  mat.pbr.roughness_factor,
            _pad0:             [0.0; 2],
            emissive_factor:   mat.emissive_factor,
            alpha_cutoff:      mat.alpha_cutoff,
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

// ─── Cache ───────────────────────────────────────────────────────────────────

pub struct GltfCache {
    pub materials:   Arena<MaterialTag, GpuMaterial>,
    pub textures:    Arena<TextureTag,  GpuTexture>,
    dummy_tex:       Option<TextureHandle>,
    /// Owned GpuTexture list for cleanup (arenas don't own by value yet).
    owned_textures:  Vec<GpuTexture>,
    owned_materials: Vec<GpuMaterial>,
}

impl GltfCache {
    pub fn new() -> Self {
        Self {
            materials:       Arena::new(),
            textures:        Arena::new(),
            dummy_tex:       None,
            owned_textures:  Vec::new(),
            owned_materials: Vec::new(),
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
        let tex = upload_texture_rgba(ctx, 1, 1, &white, &GltfSampler::default())?;
        let h = self.textures.insert(tex);
        self.dummy_tex = Some(h);
        Ok(h)
    }

    pub fn texture(&self, h: TextureHandle) -> Option<&GpuTexture> {
        self.textures.get(h)
    }

    pub fn material(&self, h: MaterialHandle) -> Option<&GpuMaterial> {
        self.materials.get(h)
    }
}

// ─── Texture upload ───────────────────────────────────────────────────────────

pub fn upload_texture_rgba(
    ctx:     &GltfUploadCtx<'_>,
    width:   u32,
    height:  u32,
    rgba:    &[u8],
    sampler: &GltfSampler,
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

    let image = ForgeImage::create_2d(
        device, mp, w, h,
        vk::Format::R8G8B8A8_UNORM,
        vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
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

        // Transition UNDEFINED → TRANSFER_DST.
        device.cmd_pipeline_barrier(cb,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(), &[], &[],
            &[image_layout_barrier(image.handle,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::AccessFlags::empty(),
                vk::AccessFlags::TRANSFER_WRITE)],
        );

        let region = vk::BufferImageCopy::default()
            .buffer_offset(0).buffer_row_length(0).buffer_image_height(0)
            .image_subresource(vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .mip_level(0).base_array_layer(0).layer_count(1))
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D { width: w, height: h, depth: 1 });
        device.cmd_copy_buffer_to_image(cb, staging.handle, image.handle,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[region]);

        // Transition TRANSFER_DST → SHADER_READ.
        device.cmd_pipeline_barrier(cb,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(), &[], &[],
            &[image_layout_barrier(image.handle,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::SHADER_READ)],
        );

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
        .mip_lod_bias(0.0).min_lod(0.0).max_lod(0.0);
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
    let mut ubo = ForgeBuffer::create(
        device, mp, ubo_size,
        vk::BufferUsageFlags::UNIFORM_BUFFER,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    ubo.write_bytes(device, uniform.as_bytes())?;

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
    let img_handles: Vec<Option<TextureHandle>> = asset.images
        .iter()
        .enumerate()
        .map(|(i, img)| {
            upload_texture_rgba(ctx, img.width, img.height, &img.rgba, &img_samplers[i])
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
    device.allocate_command_buffers(
        &vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1),
    ).map(|mut v| v.remove(0)).map_err(ForgeError::Vk)
}

fn image_layout_barrier(
    image:      vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
    src_access: vk::AccessFlags,
    dst_access: vk::AccessFlags,
) -> vk::ImageMemoryBarrier<'static> {
    vk::ImageMemoryBarrier::default()
        .old_layout(old_layout)
        .new_layout(new_layout)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0).level_count(1)
            .base_array_layer(0).layer_count(1))
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
}
