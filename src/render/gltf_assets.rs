//! Per-asset GPU state for glTF models.
//!
//! `GltfAssetCache` owns all loaded assets keyed by `GltfHandle`.
//! Multiple actors referencing the same path share one `LoadedGltf`; the
//! asset is uploaded to the GPU exactly once.

use std::path::PathBuf;
use std::sync::Arc;

use ash::vk;
use thin_vec::ThinVec;

use crate::forge_master::master::{ForgeError, ForgeResult};
use crate::forge_master::ore::GpuSkinBuffer;
use crate::render::camera::Camera;
use crate::render::vulkan::VulkanContext;
use crate::resource_manager::asset_manager::{
    MeshTable, compute_asset_aabb,
    forge_gltf::{GltfAsset, ImageFormatHint, Pose},
    pack_primitive_skin_attrs, primitive_is_skinned, upload_all_primitive_meshes,
};
use crate::resource_manager::component::{GltfHandle, GltfTag};
use crate::resource_manager::gltf_driver::{
    AsyncGltfLoader, GltfCache, GltfSampler, GltfUploadCtx, MaterialHandle, create_instance_pool,
    create_instance_set_layout, create_material, create_material_pool, create_skin_palette_pool,
    create_skin_palette_set_layout, gltf_sampler_to_vk, upload_texture_rgba,
};
use crate::resource_manager::manager::Arena;

// ── LoadedGltf ────────────────────────────────────────────────────────────────

pub struct LoadedGltf {
    pub asset: GltfAsset,
    pub meshes: MeshTable,
    pub material_sets: ThinVec<Option<vk::DescriptorSet>>,
    pub skin_vbs: ThinVec<Option<GpuSkinBuffer>>,
    pub skin_vb_offsets: ThinVec<u32>,
    pub cache: GltfCache,
    pub rest_aabb: ([f32; 3], [f32; 3]),
    pub material_pool: vk::DescriptorPool,
    pub skin_pool: vk::DescriptorPool,
    pub instance_pool: vk::DescriptorPool,
    pub material_layout: vk::DescriptorSetLayout,
    pub skin_set_layout: vk::DescriptorSetLayout,
    pub instance_layout: vk::DescriptorSetLayout,
    pub path: Arc<str>,
    pub device: ash::Device,

    /// Bottom-Level Acceleration Structures, one per loaded primitive.
    /// Built when `VulkanContext::has_ray_tracing` is true; otherwise empty.
    /// Lifetime: owned by this `LoadedGltf` and destroyed in `Drop`.
    pub blas: ThinVec<vk::AccelerationStructureKHR>,
    /// Backing buffers for the BLAS structures (one per BLAS). Each buffer is
    /// kept alive for the BLAS's lifetime; freed alongside the AS handle.
    pub blas_buffers: ThinVec<(vk::Buffer, vk::DeviceMemory)>,
}

impl LoadedGltf {
    #[inline]
    pub fn skin_vb(&self, mesh_idx: usize, prim_idx: usize) -> Option<&GpuSkinBuffer> {
        let off = *self.skin_vb_offsets.get(mesh_idx)? as usize;
        self.skin_vbs.get(off + prim_idx)?.as_ref()
    }
}

impl Drop for LoadedGltf {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            // BLAS handles must be destroyed via the acceleration_structure
            // extension loader; we cached the device but the loader lives on
            // the VulkanContext, not here. Since BLAS destruction requires the
            // KHR extension loader, the engine's shutdown path is responsible
            // for calling `release_blas()` before dropping LoadedGltf. We free
            // the backing buffers/memory here regardless (raw buffer destroy
            // works without the extension loader).
            for (buf, mem) in self.blas_buffers.drain(..) {
                if buf != vk::Buffer::null() {
                    self.device.destroy_buffer(buf, None);
                }
                if mem != vk::DeviceMemory::null() {
                    self.device.free_memory(mem, None);
                }
            }
            self.blas.clear();
            // Drop GPU resources before the pools that back them.
            let _ = std::mem::replace(&mut self.cache, GltfCache::new(self.device.clone()));
            self.skin_vbs.clear();
            macro_rules! destroy_pool {
                ($pool:expr) => {
                    if $pool != vk::DescriptorPool::null() {
                        self.device.destroy_descriptor_pool($pool, None);
                    }
                };
            }
            macro_rules! destroy_layout {
                ($layout:expr) => {
                    if $layout != vk::DescriptorSetLayout::null() {
                        self.device.destroy_descriptor_set_layout($layout, None);
                    }
                };
            }
            destroy_pool!(self.skin_pool);
            destroy_layout!(self.skin_set_layout);
            destroy_pool!(self.material_pool);
            destroy_pool!(self.instance_pool);
            destroy_layout!(self.instance_layout);
            // material_layout belongs to the window's mold — do NOT destroy.
        }
    }
}

impl LoadedGltf {
    /// Destroys BLAS handles via the KHR acceleration_structure loader.
    /// Must be called before this `LoadedGltf` is dropped when RT is enabled,
    /// otherwise the AS handle leaks (the backing buffer is still freed).
    pub fn release_blas(&mut self, vulkan: &VulkanContext) {
        if let Some(accel) = vulkan.rt_accel.as_ref() {
            unsafe {
                for h in self.blas.drain(..) {
                    if h != vk::AccelerationStructureKHR::null() {
                        accel.destroy_acceleration_structure(h, None);
                    }
                }
            }
        } else {
            self.blas.clear();
        }
    }
}

// ── PendingLoad ───────────────────────────────────────────────────────────────

struct PendingLoad {
    handle: GltfHandle,
    loader: AsyncGltfLoader,
    path: Arc<str>,
    device: ash::Device,
    material_pool: vk::DescriptorPool,
    material_layout: vk::DescriptorSetLayout,
    skin_pool: vk::DescriptorPool,
    skin_set_layout: vk::DescriptorSetLayout,
    instance_pool: vk::DescriptorPool,
    instance_layout: vk::DescriptorSetLayout,
}

impl PendingLoad {
    unsafe fn cleanup_vk(&self) {
        macro_rules! destroy_pool {
            ($pool:expr) => {
                if $pool != vk::DescriptorPool::null() {
                    unsafe {
                        self.device.destroy_descriptor_pool($pool, None);
                    }
                }
            };
        }
        macro_rules! destroy_layout {
            ($layout:expr) => {
                if $layout != vk::DescriptorSetLayout::null() {
                    unsafe {
                        self.device.destroy_descriptor_set_layout($layout, None);
                    }
                }
            };
        }
        destroy_pool!(self.skin_pool);
        destroy_layout!(self.skin_set_layout);
        destroy_pool!(self.material_pool);
        destroy_pool!(self.instance_pool);
        destroy_layout!(self.instance_layout);
    }
}

// ── GltfAssetCache ────────────────────────────────────────────────────────────

pub struct GltfAssetCache {
    slots: Arena<GltfTag, Option<LoadedGltf>>,
    pending: ThinVec<PendingLoad>,
    by_path: ThinVec<(Arc<str>, GltfHandle)>,
}

impl Default for GltfAssetCache {
    fn default() -> Self {
        Self::new()
    }
}

impl GltfAssetCache {
    pub fn new() -> Self {
        Self {
            slots: Arena::new(),
            pending: ThinVec::new(),
            by_path: ThinVec::new(),
        }
    }

    /// Register a glTF asset for loading; deduplicated by path.
    /// Returns a handle immediately; call `poll` each frame to finalize.
    pub fn load(
        &mut self,
        path: Arc<str>,
        device: ash::Device,
        material_layout: vk::DescriptorSetLayout,
    ) -> ForgeResult<GltfHandle> {
        if let Some(&(_, h)) = self
            .by_path
            .iter()
            .find(|(p, _)| p.as_ref() == path.as_ref())
        {
            return Ok(h);
        }
        let material_pool = create_material_pool(&device, 4096)?;
        let skin_set_layout = create_skin_palette_set_layout(&device)?;
        let skin_pool = create_skin_palette_pool(&device, 256)?;
        let instance_layout = create_instance_set_layout(&device)?;
        let instance_pool = create_instance_pool(&device, 4096)?;
        let loader = AsyncGltfLoader::spawn(PathBuf::from(path.as_ref()));
        let handle = self.slots.insert(None);
        self.by_path.push((path.clone(), handle));
        self.pending.push(PendingLoad {
            handle,
            loader,
            path,
            device,
            material_pool,
            material_layout,
            skin_pool,
            skin_set_layout,
            instance_pool,
            instance_layout,
        });
        Ok(handle)
    }

    /// Drive background loaders; GPU-upload any that have completed.
    pub fn poll(&mut self, vulkan: &VulkanContext) -> ForgeResult<()> {
        let mut i = 0;
        while i < self.pending.len() {
            let maybe = self.pending[i].loader.try_recv();
            match maybe {
                None => {
                    i += 1;
                }
                Some(load_result) => {
                    let pl = self.pending.swap_remove(i);
                    match load_result {
                        Ok(asset) => match finalize_load(asset, pl, vulkan) {
                            Ok((handle, loaded)) => {
                                if let Some(slot) = self.slots.get_mut(handle) {
                                    *slot = Some(loaded);
                                }
                            }
                            Err(e) => eprintln!("gltf finalize error: {e:?}"),
                        },
                        Err(e) => eprintln!("gltf load error: {e:?}"),
                    }
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, h: GltfHandle) -> Option<&LoadedGltf> {
        self.slots.get(h)?.as_ref()
    }

    pub fn is_loaded(&self, h: GltfHandle) -> bool {
        self.slots.get(h).is_some_and(|s| s.is_some())
    }

    /// Reset all skin descriptor pools. Call once per frame before allocating
    /// skin palette sets.
    pub fn reset_all_skin_pools(&self) {
        unsafe {
            for slot in self.slots.values() {
                if let Some(loaded) = slot
                    && !loaded.asset.skins.is_empty()
                    && loaded.skin_pool != vk::DescriptorPool::null()
                {
                    let _ = loaded.device.reset_descriptor_pool(
                        loaded.skin_pool,
                        vk::DescriptorPoolResetFlags::empty(),
                    );
                }
            }
        }
    }
}

impl Drop for GltfAssetCache {
    fn drop(&mut self) {
        for pl in self.pending.drain(..) {
            unsafe {
                pl.cleanup_vk();
            }
        }
    }
}

// ── finalize_load ─────────────────────────────────────────────────────────────

fn finalize_load(
    asset: GltfAsset,
    pl: PendingLoad,
    vulkan: &VulkanContext,
) -> ForgeResult<(GltfHandle, LoadedGltf)> {
    eprintln!("gltf_assets: loaded {}", pl.path);
    let mut cache = GltfCache::new(pl.device.clone());
    let upload_ctx = GltfUploadCtx {
        device: &vulkan.device,
        memory_properties: &vulkan.memory_properties,
        graphics_queue: vulkan.queue,
        command_pool: vulkan.command_pool,
        material_set_layout: pl.material_layout,
        material_pool: pl.material_pool,
        instance_set_layout: pl.instance_layout,
        instance_pool: pl.instance_pool,
    };
    let material_sets = upload_materials_flat(&asset, &upload_ctx, &mut cache);
    let (skin_vbs, skin_vb_offsets) = upload_skin_vbs_flat(vulkan, &asset);
    let meshes = upload_all_primitive_meshes(&asset, &vulkan.mesh_upload_ctx())
        .map_err(|e| ForgeError::Io(std::io::Error::other(format!("mesh upload: {e:?}"))))?;
    let rest_pose = Pose::rest(&asset);
    let rest_aabb = compute_asset_aabb(&asset, &rest_pose);
    // Eagerly init dummy sets so render_world can use them read-only.
    let _ = cache.ensure_dummy_material(&upload_ctx);
    let _ = cache.ensure_dummy_instance_matrices(&upload_ctx);

    let mut loaded = LoadedGltf {
        asset,
        meshes,
        material_sets,
        skin_vbs,
        skin_vb_offsets,
        cache,
        rest_aabb,
        material_pool: pl.material_pool,
        skin_pool: pl.skin_pool,
        instance_pool: pl.instance_pool,
        material_layout: pl.material_layout,
        skin_set_layout: pl.skin_set_layout,
        instance_layout: pl.instance_layout,
        path: pl.path,
        device: pl.device.clone(),
        blas: ThinVec::new(),
        blas_buffers: ThinVec::new(),
    };

    // Build per-primitive BLAS structures when ray tracing is available.
    if vulkan.has_ray_tracing
        && let Err(e) = build_blas_for_loaded(&mut loaded, vulkan)
    {
        // Non-fatal: RT BLAS construction failed; raster path still works.
        eprintln!("BLAS build failed for {}: {e:?}", loaded.path);
    }

    Ok((pl.handle, loaded))
}

/// Build a BLAS for every loaded mesh primitive that has indices.
/// One AS per primitive; on failure leaves `loaded.blas` empty (RT path
/// skips this asset cleanly).
fn build_blas_for_loaded(loaded: &mut LoadedGltf, vulkan: &VulkanContext) -> ForgeResult<()> {
    use crate::render::blas::{BlasBuildInputs, build_blas};

    let accel = vulkan
        .rt_accel
        .as_ref()
        .ok_or(ForgeError::NoPhysicalDevice)?;
    let device = &vulkan.device;

    // Walk every (mesh_idx, prim_idx) by trying sequential indices until
    // MeshTable::get returns None. The asset's `meshes` array gives the
    // primitive-per-mesh structure.
    let n_meshes = loaded.asset.meshes.len();
    for mesh_idx in 0..n_meshes {
        let n_prims = loaded.asset.meshes[mesh_idx].primitives.len();
        for prim_idx in 0..n_prims {
            let Some(gpu) = loaded.meshes.get(mesh_idx, prim_idx) else {
                continue;
            };
            let prim = &loaded.asset.meshes[mesh_idx].primitives[prim_idx];
            // Vertex count from the gltf position accessor.
            let vertex_count = prim.streams.positions.len() as u32;
            if vertex_count == 0 || gpu.index_count == 0 {
                continue;
            }

            let input = BlasBuildInputs {
                device,
                accel_ext: accel,
                memory_props: &vulkan.memory_properties,
                command_pool: vulkan.command_pool,
                queue: vulkan.queue,
                vertex_buffer: gpu.vertex_buffer.handle,
                vertex_offset: 0,
                vertex_count,
                vertex_stride: 24, // ForgeVertex = pos(12) + normal(12)
                vertex_format: vk::Format::R32G32B32_SFLOAT,
                index_buffer: gpu.index_buffer.handle,
                index_offset: 0,
                index_count: gpu.index_count,
                index_type: vk::IndexType::UINT32,
                transform: None,
            };

            match build_blas(&input) {
                Ok((handle, backing)) => {
                    loaded.blas.push(handle);
                    // Copy the raw Vulkan handles into the pair tuple.
                    let pair = (backing.handle, backing.memory);
                    loaded.blas_buffers.push(pair);
                }
                Err(e) => {
                    eprintln!("build_blas failed (mesh {mesh_idx} prim {prim_idx}): {e:?}");
                }
            }
        }
    }
    Ok(())
}

// ── upload helpers ────────────────────────────────────────────────────────────

fn upload_materials_flat(
    asset: &GltfAsset,
    upload_ctx: &GltfUploadCtx<'_>,
    cache: &mut GltfCache,
) -> ThinVec<Option<vk::DescriptorSet>> {
    let n_images = asset.images.len();
    let mut img_samplers: ThinVec<GltfSampler> =
        (0..n_images).map(|_| GltfSampler::default()).collect();
    for tex in &asset.textures {
        let idx = tex.image as usize;
        if idx < n_images
            && let Some(si) = tex.sampler
            && let Some(s) = asset.samplers.get(si as usize)
        {
            img_samplers[idx] = gltf_sampler_to_vk(s);
        }
    }
    let img_handles: ThinVec<Option<_>> = asset
        .images
        .iter()
        .enumerate()
        .map(|(i, img)| {
            let fmt = match img.format {
                ImageFormatHint::Srgb => vk::Format::R8G8B8A8_SRGB,
                ImageFormatHint::Linear => vk::Format::R8G8B8A8_UNORM,
            };
            match upload_texture_rgba(
                upload_ctx,
                img.width,
                img.height,
                &img.rgba,
                &img_samplers[i],
                fmt,
            ) {
                Ok(tex) => Some(cache.textures.insert(tex)),
                Err(e) => {
                    eprintln!("texture upload failed (image {i}): {e:?}");
                    None
                }
            }
        })
        .collect();

    asset
        .materials
        .iter()
        .map(
            |mat| match create_material(mat, asset, &img_handles, upload_ctx, cache) {
                Ok(gm) => {
                    let set = gm.descriptor_set;
                    let _h: MaterialHandle = cache.materials.insert(gm);
                    Some(set)
                }
                Err(e) => {
                    eprintln!("material upload failed: {e:?}");
                    None
                }
            },
        )
        .collect()
}

fn upload_skin_vbs_flat(
    ctx: &VulkanContext,
    asset: &GltfAsset,
) -> (ThinVec<Option<GpuSkinBuffer>>, ThinVec<u32>) {
    let mut vbs: ThinVec<Option<GpuSkinBuffer>> = ThinVec::new();
    let mut offsets: ThinVec<u32> = ThinVec::new();
    let mesh_ctx = ctx.mesh_upload_ctx();
    for (mi, mesh) in asset.meshes.iter().enumerate() {
        offsets.push(vbs.len() as u32);
        for pi in 0..mesh.primitives.len() {
            if primitive_is_skinned(asset, mi as u32, pi as u32) {
                let bytes = pack_primitive_skin_attrs(asset, mi as u32, pi as u32);
                let vcount = (bytes.len() / 24) as u32;
                if vcount > 0 {
                    match GpuSkinBuffer::upload(&mesh_ctx, &bytes, vcount) {
                        Ok(b) => vbs.push(Some(b)),
                        Err(e) => {
                            eprintln!("skin vb upload failed: {e:?}");
                            vbs.push(None);
                        }
                    }
                } else {
                    vbs.push(None);
                }
            } else {
                vbs.push(None);
            }
        }
    }
    (vbs, offsets)
}

// ── AABB helpers ──────────────────────────────────────────────────────────────

use glam::{Affine3A, Vec3};

pub fn transform_aabb(aabb: &([f32; 3], [f32; 3]), t: &Affine3A) -> ([f32; 3], [f32; 3]) {
    let (mn, mx) = aabb;
    let corners = [
        [mn[0], mn[1], mn[2]],
        [mx[0], mn[1], mn[2]],
        [mn[0], mx[1], mn[2]],
        [mx[0], mx[1], mn[2]],
        [mn[0], mn[1], mx[2]],
        [mx[0], mn[1], mx[2]],
        [mn[0], mx[1], mx[2]],
        [mx[0], mx[1], mx[2]],
    ];
    let mut out_mn = [f32::MAX; 3];
    let mut out_mx = [f32::MIN; 3];
    for c in &corners {
        let p = t.transform_point3(Vec3::new(c[0], c[1], c[2]));
        out_mn[0] = out_mn[0].min(p.x);
        out_mn[1] = out_mn[1].min(p.y);
        out_mn[2] = out_mn[2].min(p.z);
        out_mx[0] = out_mx[0].max(p.x);
        out_mx[1] = out_mx[1].max(p.y);
        out_mx[2] = out_mx[2].max(p.z);
    }
    (out_mn, out_mx)
}

pub fn fit_camera_to_aabb(camera: &mut Camera, aabb: &([f32; 3], [f32; 3])) {
    let (mn, mx) = aabb;
    let center = [
        0.5 * (mn[0] + mx[0]),
        0.5 * (mn[1] + mx[1]),
        0.5 * (mn[2] + mx[2]),
    ];
    let half = [
        0.5 * (mx[0] - mn[0]).max(1e-3),
        0.5 * (mx[1] - mn[1]).max(1e-3),
        0.5 * (mx[2] - mn[2]).max(1e-3),
    ];
    let radius = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
    let dist = radius * 2.5;
    let eye = [
        center[0] + dist * 0.6,
        center[1] + dist * 0.4,
        center[2] + dist * 1.0,
    ];
    camera.position = eye;
    let dx = center[0] - eye[0];
    let dy = center[1] - eye[1];
    let dz = center[2] - eye[2];
    let horiz = (dx * dx + dz * dz).sqrt();
    camera.pitch = dy.atan2(horiz);
    camera.yaw = dz.atan2(dx);
    camera.near = (radius * 0.01).max(0.001);
    camera.far = (radius * 10.0).max(100.0);
    camera.fov = 50.0_f32.to_radians();
}
