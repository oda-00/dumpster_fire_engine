//! `GltfScene` — self-contained glTF asset renderer for use with `AppRunner`.
//!
//! Encapsulates everything that `hello_gltf` wired by hand:
//!   * background asset loading (`AsyncGltfLoader`)
//!   * one-shot GPU upload of meshes / textures / materials / skin attributes
//!   * per-frame animation sampling + compute dispatch (skin palette, morph blend)
//!   * graphics plan submission via the renderer's factory system
//!
//! Typical usage:
//! ```ignore
//! struct MyApp { scene: Option<GltfScene>, win: AppHandle }
//! impl AppLogic for MyApp {
//!     fn on_start(&mut self, ctx: &mut AppCtx, ev: &ActiveEventLoop) -> ForgeResult<()> {
//!         self.win = ctx.spawn_window(ev, "demo", 1024, 768)?;
//!         let mut scene = ctx.new_gltf_scene(self.win)?;
//!         scene.load("model.glb");
//!         self.scene = Some(scene);
//!         Ok(())
//!     }
//!     fn update(&mut self, ctx: &mut AppCtx, app: AppHandle, _dt: f32) -> bool {
//!         let elapsed = ctx.elapsed().as_secs_f32();
//!         let vp = ctx.camera_vp(app);
//!         if let Some(scene) = &mut self.scene {
//!             if scene.is_loaded() && !self.cam_fitted {
//!                 scene.fit_camera(ctx.camera_mut(app).unwrap());
//!                 self.cam_fitted = true;
//!             }
//!             if let Ok(Some(sem)) = ctx.gltf_update(scene, app, elapsed, &vp) {
//!                 ctx.push_compute_wait(app, sem);
//!             }
//!         }
//!         true
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use ash::Device;

use ash::vk;

use crate::forge_master::ore::{GpuMesh, GpuSkinBuffer};
use crate::forge_master::master::{ForgeError, ForgeResult};
use crate::render::camera::Camera;
use crate::render::{GraphicsTag, Proto, ProtoId, Renderer, WindowHandle};
use crate::render::vulkan::VulkanContext;
use crate::resource_manager::asset_manager::{
    SkinningFrame,
    build_graphics_plans_maximal_with_meshes_vp,
    build_skin_morph_proto,
    collect_morph_output_buffers,
    collect_skin_palette_buffers,
    compute_asset_aabb,
    forge_gltf::{self, GltfAsset, ImageFormatHint, Pose},
    pack_primitive_skin_attrs,
    primitive_is_skinned,
    upload_all_primitive_meshes,
};
use crate::resource_manager::gltf_driver::{
    AsyncGltfLoader,
    GltfCache,
    GltfSampler,
    GltfUploadCtx,
    MaterialHandle,
    TEXTURE_SLOT_COUNT,
    allocate_skin_palette_set,
    create_instance_pool,
    create_instance_set_layout,
    create_material,
    create_material_pool,
    create_skin_palette_pool,
    create_skin_palette_set_layout,
    gltf_sampler_to_vk,
    upload_texture_rgba,
};

// Keep TEXTURE_SLOT_COUNT referenced to avoid dead_code lint in this module.
const _: usize = TEXTURE_SLOT_COUNT;

// ── Internal loaded state ─────────────────────────────────────────────────────

struct SceneState {
    asset:               GltfAsset,
    pose:                Pose,
    material_sets:       Vec<Option<vk::DescriptorSet>>,
    meshes:              HashMap<(usize, usize), Arc<GpuMesh>>,
    skin_vertex_buffers: HashMap<(usize, usize), GpuSkinBuffer>,
    cache:               GltfCache,
    last_anim_time:      f32,
    rest_aabb:           ([f32; 3], [f32; 3]),
}

// ── GltfScene — public API ────────────────────────────────────────────────────

/// A complete glTF asset bound to a Vulkan device, ready to animate and render.
///
/// Create via [`AppCtx::new_gltf_scene`], kick off loading with [`GltfScene::load`],
/// then call [`AppCtx::gltf_update`] every frame.
pub struct GltfScene {
    loader:          Option<AsyncGltfLoader>,
    state:           Option<SceneState>,
    material_pool:   vk::DescriptorPool,
    material_layout: vk::DescriptorSetLayout,
    skin_pool:       vk::DescriptorPool,
    skin_set_layout: vk::DescriptorSetLayout,
    instance_pool:   vk::DescriptorPool,
    instance_layout: vk::DescriptorSetLayout,
    device:          Device,
}

impl GltfScene {
    /// Create a new, not-yet-loading scene.  Call [`load`] to start fetching.
    ///
    /// `material_layout` must come from the target window's graphics mold
    /// (available via `AppCtx::new_gltf_scene`).
    pub fn new(
        device:          Device,
        material_layout: vk::DescriptorSetLayout,
    ) -> ForgeResult<Self> {
        let material_pool = create_material_pool(&device, 4096)?;
        let skin_set_layout = create_skin_palette_set_layout(&device)?;
        let skin_pool = create_skin_palette_pool(&device, 256)?;
        let instance_layout = create_instance_set_layout(&device)?;
        let instance_pool = create_instance_pool(&device, 4096)?;
        Ok(Self {
            loader: None,
            state:  None,
            material_pool,
            material_layout,
            skin_pool,
            skin_set_layout,
            instance_pool,
            instance_layout,
            device,
        })
    }

    /// Start loading a glTF/GLB file in the background.
    pub fn load(&mut self, path: impl Into<PathBuf>) {
        self.loader = Some(AsyncGltfLoader::spawn(path.into()));
    }

    /// Returns true once the asset has been received and uploaded to the GPU.
    pub fn is_loaded(&self) -> bool {
        self.state.is_some()
    }

    /// Cached rest-pose AABB `(min, max)` — available after `is_loaded()`.
    pub fn rest_aabb(&self) -> Option<&([f32; 3], [f32; 3])> {
        self.state.as_ref().map(|s| &s.rest_aabb)
    }

    /// Fit a camera to frame this scene's rest-pose AABB.
    ///
    /// Positions the camera using the same eye-point math as the built-in
    /// static viewer, then points it at the asset center.  Also scales
    /// `near`/`far` and `fov` to the asset's size so clipping is correct.
    pub fn fit_camera(&self, camera: &mut Camera) {
        if let Some(s) = &self.state {
            fit_camera_to_aabb(camera, &s.rest_aabb);
        }
    }

    /// Per-frame update: tries to receive the asset on first call, drives
    /// animation, dispatches skin/morph compute, and builds the graphics plan.
    ///
    /// Returns `Ok(Some(sem))` when a compute command buffer was submitted;
    /// the caller should pass that semaphore to
    /// `AppCtx::push_compute_wait(app, sem)` so the graphics submit waits
    /// for compute output.
    pub fn update(
        &mut self,
        vulkan:        &VulkanContext,
        renderer:      &mut Renderer,
        window_handle: WindowHandle,
        camera_vp:     &[f32; 16],
        elapsed_secs:  f32,
    ) -> ForgeResult<Option<vk::Semaphore>> {
        // Try to receive the asset on first call after load() finishes.
        if self.state.is_none() {
            if let Some(loader) = self.loader.as_mut() {
                if let Some(result) = loader.try_recv() {
                    let asset = result.map_err(|e| {
                        ForgeError::Io(std::io::Error::other(format!("gltf load: {e:?}")))
                    })?;
                    println!(
                        "gltf_scene: loaded {} meshes, {} nodes, {} animations, {} materials",
                        asset.meshes.len(), asset.nodes.len(),
                        asset.animations.len(), asset.materials.len(),
                    );
                    let pose = Pose::rest(&asset);
                    let mut cache = GltfCache::new(vulkan.device.clone());
                    let upload_ctx = self.upload_ctx(vulkan);
                    let material_sets = upload_materials(&asset, &upload_ctx, &mut cache);
                    let skin_vertex_buffers = upload_skin_vbs(vulkan, &asset);
                    let meshes = upload_all_primitive_meshes(&asset, &vulkan.mesh_upload_ctx())
                        .map_err(|e| {
                            ForgeError::Io(std::io::Error::other(format!("mesh upload: {e:?}")))
                        })?;
                    let rest_aabb = compute_asset_aabb(&asset, &pose);
                    self.state = Some(SceneState {
                        asset, pose, material_sets, meshes,
                        skin_vertex_buffers, cache,
                        last_anim_time: -1.0, rest_aabb,
                    });
                }
            }
        }

        let Some(state) = self.state.as_mut() else {
            return Ok(None);
        };

        let t = elapsed_secs;
        let advanced = if let Some(anim) = state.asset.animations.first() {
            let dur = anim.duration().max(1e-3);
            state.pose.sample(&state.asset, anim, t.rem_euclid(dur));
            true
        } else {
            false
        };

        let needs_rebuild = advanced || state.last_anim_time < 0.0;
        if !needs_rebuild {
            return Ok(None);
        }

        // Wait for the previous frame's fence before resetting the skin pool.
        if let Err(e) = renderer.wait_for_last_submission(window_handle) {
            eprintln!("gltf_scene: wait_for_last_submission: {e:?}");
        }
        unsafe {
            let _ = self.device.reset_descriptor_pool(
                self.skin_pool,
                vk::DescriptorPoolResetFlags::empty(),
            );
        }

        // Compute pass: skin palette + morph blend.
        let mut compute_signal: Option<vk::Semaphore> = None;
        let (morph_buffers, palette_buffers): (HashMap<_, _>, HashMap<_, _>) =
            if let Some(cp) = build_skin_morph_proto(&state.asset, &state.pose, ProtoId::new(2), 0)
            {
                match renderer.build_compute_factory_async(window_handle, cp) {
                    Ok((handle, sem)) => {
                        compute_signal = Some(sem);
                        let win = renderer.window(window_handle);
                        let factory = win.and_then(|w| w.factory_master.get(handle));
                        match factory {
                            Some(f) => (
                                collect_morph_output_buffers(&state.asset, f),
                                collect_skin_palette_buffers(&state.asset, f),
                            ),
                            None => Default::default(),
                        }
                    }
                    Err(e) => {
                        eprintln!("gltf_scene: compute dispatch: {e:?}");
                        Default::default()
                    }
                }
            } else {
                Default::default()
            };

        // Allocate per-frame skin palette descriptor sets.
        let mut palette_sets_by_node: HashMap<usize, vk::DescriptorSet> = HashMap::new();
        for (node_idx, node) in state.asset.nodes.iter().enumerate() {
            let Some(skin_idx) = node.skin else { continue };
            let Some(&buf) = palette_buffers.get(&(skin_idx as usize)) else { continue };
            let range = (state.asset.skins[skin_idx as usize].joints.len() as vk::DeviceSize)
                * 64;
            match allocate_skin_palette_set(
                &self.device,
                self.skin_pool,
                self.skin_set_layout,
                buf,
                range.max(64),
            ) {
                Ok(set) => { palette_sets_by_node.insert(node_idx, set); }
                Err(e) => eprintln!("gltf_scene: skin palette alloc: {e:?}"),
            }
        }

        let skin_vbs: HashMap<(usize, usize), vk::Buffer> = state
            .skin_vertex_buffers
            .iter()
            .map(|(k, gpu)| (*k, gpu.buffer.handle))
            .collect();
        let skinning = SkinningFrame { skin_vertex_buffers: skin_vbs, palette_sets_by_node };

        // Dummy material / instance sets for primitives that have none.
        // Build upload_ctx from raw fields to avoid a second &self borrow while
        // `state` (from self.state.as_mut()) already holds &mut self.
        let upload_ctx = GltfUploadCtx {
            device:              &vulkan.device,
            memory_properties:   &vulkan.memory_properties,
            graphics_queue:      vulkan.queue,
            command_pool:        vulkan.command_pool,
            material_set_layout: self.material_layout,
            material_pool:       self.material_pool,
            instance_set_layout: self.instance_layout,
            instance_pool:       self.instance_pool,
        };
        let fallback_mat = state.cache.ensure_dummy_material(&upload_ctx).ok();
        let dummy_inst = state.cache.ensure_dummy_instance_matrices(&upload_ctx).ok();

        // Per-draw instance matrix sets (EXT_mesh_gpu_instancing).
        let mut instance_sets: HashMap<(usize, usize), vk::DescriptorSet> = HashMap::new();
        let draws_for_instances =
            forge_gltf::build_graphics_draws_with_matrices(&state.asset, &state.pose.world);
        for d in &draws_for_instances {
            if d.instance_matrices.is_empty() { continue; }
            let key = (d.mesh as usize, d.primitive as usize);
            if instance_sets.contains_key(&key) { continue; }
            if let Ok(set) = state.cache.create_instance_matrices_set(&upload_ctx, &d.instance_matrices) {
                instance_sets.insert(key, set);
            }
        }

        let plans = build_graphics_plans_maximal_with_meshes_vp(
            &state.asset, &state.pose, &state.meshes,
            &state.material_sets, &morph_buffers, &skinning,
            camera_vp, fallback_mat, &instance_sets, dummy_inst,
        );
        let mut proto = Proto::<GraphicsTag>::new(ProtoId::new(1), "gltf_scene");
        for plan in plans { proto.push_call(plan); }
        renderer.build_graphics_factory(window_handle, proto);
        state.last_anim_time = t;
        Ok(compute_signal)
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn upload_ctx<'a>(&'a self, vulkan: &'a VulkanContext) -> GltfUploadCtx<'a> {
        GltfUploadCtx {
            device:              &vulkan.device,
            memory_properties:   &vulkan.memory_properties,
            graphics_queue:      vulkan.queue,
            command_pool:        vulkan.command_pool,
            material_set_layout: self.material_layout,
            material_pool:       self.material_pool,
            instance_set_layout: self.instance_layout,
            instance_pool:       self.instance_pool,
        }
    }
}

impl Drop for GltfScene {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
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
            // Drop state (GpuMesh / GpuSkinBuffer / GltfCache) before pools.
            drop(self.state.take());
            destroy_pool!(self.skin_pool);
            destroy_layout!(self.skin_set_layout);
            destroy_pool!(self.material_pool);
            destroy_pool!(self.instance_pool);
            destroy_layout!(self.instance_layout);
            // material_layout is owned by the window's mold — do NOT destroy here.
        }
    }
}

// ── Camera fit helper ─────────────────────────────────────────────────────────

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

// ── GPU upload helpers ────────────────────────────────────────────────────────

fn upload_materials(
    asset:      &GltfAsset,
    upload_ctx: &GltfUploadCtx<'_>,
    cache:      &mut GltfCache,
) -> Vec<Option<vk::DescriptorSet>> {
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
    let img_handles: Vec<Option<_>> = asset.images.iter().enumerate().map(|(i, img)| {
        let fmt = match img.format {
            ImageFormatHint::Srgb   => vk::Format::R8G8B8A8_SRGB,
            ImageFormatHint::Linear => vk::Format::R8G8B8A8_UNORM,
        };
        match upload_texture_rgba(upload_ctx, img.width, img.height, &img.rgba, &img_samplers[i], fmt) {
            Ok(tex) => Some(cache.textures.insert(tex)),
            Err(e)  => { eprintln!("texture upload failed (image {i}): {e:?}"); None }
        }
    }).collect();

    asset.materials.iter().map(|mat| {
        match create_material(mat, asset, &img_handles, upload_ctx, cache) {
            Ok(gm) => {
                let set = gm.descriptor_set;
                let _h: MaterialHandle = cache.materials.insert(gm);
                Some(set)
            }
            Err(e) => { eprintln!("material upload failed: {e:?}"); None }
        }
    }).collect()
}

fn upload_skin_vbs(
    ctx:   &VulkanContext,
    asset: &GltfAsset,
) -> HashMap<(usize, usize), GpuSkinBuffer> {
    let mut out = HashMap::new();
    let mesh_ctx = ctx.mesh_upload_ctx();
    for (mi, mesh) in asset.meshes.iter().enumerate() {
        for (pi, _prim) in mesh.primitives.iter().enumerate() {
            if !primitive_is_skinned(asset, mi as u32, pi as u32) { continue; }
            let bytes = pack_primitive_skin_attrs(asset, mi as u32, pi as u32);
            let vcount = (bytes.len() / 24) as u32;
            if vcount == 0 { continue; }
            match GpuSkinBuffer::upload(&mesh_ctx, &bytes, vcount) {
                Ok(b)  => { out.insert((mi, pi), b); }
                Err(e) => eprintln!("skin vb upload failed: {e:?}"),
            }
        }
    }
    out
}
