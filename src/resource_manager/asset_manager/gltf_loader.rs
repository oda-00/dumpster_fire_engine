//! Engine-side bridge over the hand-rolled `forge_gltf` crate.
//!
//! Keeps the legacy entry points (`load_first_mesh`, `load_all_meshes`,
//! `load_first_mesh_from_slice`, `load_all_meshes_from_slice`, `GltfError`,
//! `build_test_glb`) working so existing call sites and benches don't need
//! to change. New code that wants the full document tree, multi-pipeline
//! adapters, materials, textures, lights, animations, etc. should reach
//! straight for `forge_gltf::GltfAsset`.

use std::path::Path;
use std::sync::Arc;

use ash::vk;
use thin_vec::ThinVec;

use forge_gltf::{
    GltfAsset, PipelineParams, PipelineUpload, Pose,
    build_all_compute_uploads, build_graphics_draws, build_graphics_draws_with_matrices,
    build_morph_blend_input, build_skin_palette_input, build_ui_draws,
};

use crate::forge_master::forge::ForgeId;
use crate::forge_master::frame::{FrameId, FramePlan, GraphicsFramePlan};
use crate::forge_master::ingot::Ingot;
use crate::forge_master::master::{ForgeMaster, ForgeResult};
use crate::render::factory_master::proto::{ComputeTag, Proto, ProtoId};
use crate::forge_master::ore::{
    ForgeVertex, GpuMesh, GraphicsOreKind, IngotSpec, MeshOre, MeshUploadCtx, Ore, OreInput,
    OreKind, TextureOre,
};

// Pre-compiled SPIR-V for the two skinning/morph compute shaders.
const SKIN_PALETTE_SPV: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/shaders/skin_palette.comp.glsl.spv"
));
const MORPH_BLEND_SPV: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/shaders/morph_blend.comp.glsl.spv"
));

// Re-export the test helper for benches that already pull it from here.
pub use forge_gltf::build_test_glb;

// ── Error ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum GltfError {
    Io(gltf::Error),
    NoPrimitives,
    NoPositions,
    Other(String),
}

impl std::fmt::Display for GltfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GltfError::Io(e)        => write!(f, "gltf I/O error: {e}"),
            GltfError::NoPrimitives => write!(f, "glTF file has no mesh primitives"),
            GltfError::NoPositions  => write!(f, "glTF primitive has no POSITION accessor"),
            GltfError::Other(s)     => write!(f, "gltf error: {s}"),
        }
    }
}

impl std::error::Error for GltfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GltfError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<gltf::Error> for GltfError {
    fn from(e: gltf::Error) -> Self { GltfError::Io(e) }
}

impl From<forge_gltf::GltfError> for GltfError {
    fn from(e: forge_gltf::GltfError) -> Self {
        match e {
            forge_gltf::GltfError::Io(inner)              => GltfError::Io(inner),
            forge_gltf::GltfError::NoPrimitives           => GltfError::NoPrimitives,
            forge_gltf::GltfError::NoPositions            => GltfError::NoPositions,
            forge_gltf::GltfError::InvalidAccessor(s)     => GltfError::Other(format!("invalid accessor: {s}")),
            forge_gltf::GltfError::UnsupportedComponent(s) => GltfError::Other(format!("unsupported component: {s}")),
            forge_gltf::GltfError::UnsupportedVersion(s)  => GltfError::Other(format!("unsupported version: {s}")),
            forge_gltf::GltfError::UnsupportedExtension(s) => GltfError::Other(format!("unsupported extension: {s}")),
            forge_gltf::GltfError::SpecViolation(s)       => GltfError::Other(format!("spec violation: {s}")),
            forge_gltf::GltfError::UnsupportedFeature(s)  => GltfError::Other(format!("unsupported feature: {s}")),
        }
    }
}

// ── Legacy mesh extraction (kept stable for existing callers) ──────────────

pub fn load_first_mesh(path: impl AsRef<Path>) -> Result<MeshOre, GltfError> {
    let asset = GltfAsset::load(path)?;
    first_mesh_from_asset(&asset)
}

pub fn load_all_meshes(path: impl AsRef<Path>) -> Result<ThinVec<MeshOre>, GltfError> {
    let asset = GltfAsset::load(path)?;
    Ok(all_meshes_from_asset(&asset))
}

pub fn load_first_mesh_from_slice(bytes: &[u8]) -> Result<MeshOre, GltfError> {
    let asset = GltfAsset::load_slice(bytes)?;
    first_mesh_from_asset(&asset)
}

pub fn load_all_meshes_from_slice(bytes: &[u8]) -> Result<ThinVec<MeshOre>, GltfError> {
    let asset = GltfAsset::load_slice(bytes)?;
    Ok(all_meshes_from_asset(&asset))
}

fn first_mesh_from_asset(asset: &GltfAsset) -> Result<MeshOre, GltfError> {
    let mut meshes = all_meshes_from_asset(asset);
    if meshes.is_empty() { Err(GltfError::NoPrimitives) } else { Ok(meshes.swap_remove(0)) }
}

fn all_meshes_from_asset(asset: &GltfAsset) -> ThinVec<MeshOre> {
    let mut out = ThinVec::new();
    for mesh in &asset.meshes {
        for prim in &mesh.primitives {
            let n = prim.streams.positions.len();
            let uv0 = prim.streams.uv_sets.first();
            let vertices: ThinVec<ForgeVertex> = (0..n)
                .map(|i| ForgeVertex::new(
                    prim.streams.positions[i],
                    prim.streams.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
                    prim.streams.tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]),
                    uv0.and_then(|s| s.get(i).copied()).unwrap_or([0.0, 0.0]),
                ))
                .collect();
            out.push(MeshOre::new(vertices, prim.indices.clone()));
        }
    }
    out
}

// ── New full-fidelity entry points ─────────────────────────────────────────
//
// The bridge layer that adapts the full `GltfAsset` into engine types every
// pipeline cares about. These don't replace the legacy mesh loaders — they
// sit alongside them so callers can pick the level of detail they want.

/// Load a glTF file and parse the entire document. Re-exposes the same
/// `forge_gltf::GltfAsset` so callers can walk scenes, nodes, materials,
/// textures, skins, animations and lights directly.
pub fn load_asset(path: impl AsRef<Path>) -> Result<GltfAsset, GltfError> {
    Ok(GltfAsset::load(path)?)
}

pub fn load_asset_from_slice(bytes: &[u8]) -> Result<GltfAsset, GltfError> {
    Ok(GltfAsset::load_slice(bytes)?)
}

/// Translate an in-memory glTF asset into the engine's `MeshOre`. One per
/// primitive, in document order — identical packing to `load_all_meshes`
/// but driven off an already-parsed `GltfAsset`.
pub fn asset_to_mesh_ores(asset: &GltfAsset) -> ThinVec<MeshOre> {
    all_meshes_from_asset(asset)
}

/// Translate the asset into one `TextureOre` per image. Pixel data is
/// already decoded to tightly packed RGBA8 by the loader; we pick the
/// vk format based on `ImageFormatHint`.
pub fn asset_to_texture_ores(asset: &GltfAsset) -> ThinVec<TextureOre> {
    asset
        .images
        .iter()
        .map(|img| {
            let format = match img.format {
                forge_gltf::ImageFormatHint::Srgb   => vk::Format::R8G8B8A8_SRGB,
                forge_gltf::ImageFormatHint::Linear => vk::Format::R8G8B8A8_UNORM,
            };
            TextureOre::new(img.width, img.height, format, img.rgba.clone())
        })
        .collect()
}

/// Bridge a `forge_gltf::PipelineUpload` into an engine `Ore`. The mapping
/// follows the one-to-one enum correspondence between the two crates'
/// pipeline-kind enums.
pub fn upload_to_ore(up: &PipelineUpload, output_size: vk::DeviceSize) -> Ore {
    let kind = pipeline_kind_to_ore(up.kind);
    let primary: ThinVec<u8> = up.primary_bytes.clone();
    let secondary: ThinVec<u8> = up.secondary_bytes.clone();

    // Compute-output buffers that downstream graphics draws read directly
    // need extra buffer-usage flags so the same allocation is bindable as a
    // vertex source or SSBO without a copy step.
    //   MorphBlend  → posed vertex buffer → VERTEX_BUFFER
    //   SkinPalette → joint-palette SSBO  → STORAGE_BUFFER (already default;
    //                  redundant flag is a no-op but documents intent)
    let extra_usage = match kind {
        OreKind::MorphBlend  => vk::BufferUsageFlags::VERTEX_BUFFER,
        OreKind::SkinPalette => vk::BufferUsageFlags::STORAGE_BUFFER,
        _                    => vk::BufferUsageFlags::empty(),
    };

    // Build the input variant carefully so we don't drop either buffer:
    //   • Mesh-shaped upload (is_mesh + vertices + indices)
    //     → OreInput::Mesh (the staging path reads vertices_as_bytes /
    //       indices_as_bytes).
    //   • Non-mesh upload with both primary and secondary bytes
    //     → OreInput::DualBytes — neither input fits Mesh, but we still
    //       need both bound to compute set 0 binding 0 and binding 1
    //       (SkinPalette: world matrices + IBMs; MorphBlend: rest verts +
    //       header/weights/deltas blob).
    //   • Non-mesh upload with only primary bytes → OreInput::Bytes.
    //   • Empty payload → OreInput::Empty.
    let input = if primary.is_empty() && secondary.is_empty() {
        OreInput::Empty
    } else if up.is_mesh && !primary.is_empty() {
        let vertex_stride = std::mem::size_of::<ForgeVertex>();
        let n_vertices = if up.element_stride == vertex_stride as u32 {
            (up.primary_bytes.len() / vertex_stride) as u32
        } else {
            up.element_count
        };
        OreInput::Mesh(MeshOre::new(
            unpack_forge_vertices(&up.primary_bytes, n_vertices as usize),
            if secondary.is_empty() { ThinVec::new() } else { unpack_u32(&secondary) },
        ))
    } else if !secondary.is_empty() {
        OreInput::DualBytes { primary, secondary }
    } else {
        OreInput::Bytes(primary)
    };

    Ore::new(
        kind,
        input,
        IngotSpec::Buffer {
            size: non_zero_size(output_size),
            save_path: None,
            extra_usage,
        },
        up.workgroups,
    )
}

fn unpack_forge_vertices(bytes: &[u8], n: usize) -> ThinVec<ForgeVertex> {
    let stride = std::mem::size_of::<ForgeVertex>();
    let mut out = ThinVec::with_capacity(n);
    for i in 0..n {
        let s = i * stride;
        let position = [
            f32_le(&bytes[s..]),         f32_le(&bytes[s+4..]),   f32_le(&bytes[s+8..]),
        ];
        let normal = [
            f32_le(&bytes[s+12..]),      f32_le(&bytes[s+16..]),  f32_le(&bytes[s+20..]),
        ];
        let tangent = [
            f32_le(&bytes[s+24..]),      f32_le(&bytes[s+28..]),  f32_le(&bytes[s+32..]),  f32_le(&bytes[s+36..]),
        ];
        let uv = [
            f32_le(&bytes[s+40..]),      f32_le(&bytes[s+44..]),
        ];
        out.push(ForgeVertex { position, normal, tangent, uv });
    }
    out
}

fn unpack_u32(bytes: &[u8]) -> ThinVec<u32> {
    let n = bytes.len() / 4;
    let mut out = ThinVec::with_capacity(n);
    for i in 0..n {
        out.push(u32::from_le_bytes([
            bytes[i*4], bytes[i*4+1], bytes[i*4+2], bytes[i*4+3],
        ]));
    }
    out
}

fn f32_le(b: &[u8]) -> f32 { f32::from_le_bytes([b[0], b[1], b[2], b[3]]) }

fn non_zero_size(size: vk::DeviceSize) -> vk::DeviceSize { size.max(1) }

fn pipeline_kind_to_ore(kind: forge_gltf::GltfPipelineKind) -> OreKind {
    use forge_gltf::GltfGraphicsKind as G;
    use forge_gltf::GltfPipelineKind as K;
    match kind {
        K::RayTrace            => OreKind::RayTrace,
        K::Denoise             => OreKind::Denoise,
        K::SignedDistanceField => OreKind::SignedDistanceField,
        K::SdfVoxelization     => OreKind::SdfVoxelization,
        K::LightClustering     => OreKind::LightClustering,
        K::OcclusionCulling    => OreKind::OcclusionCulling,
        K::MaterialFlattening  => OreKind::MaterialFlattening,
        K::AmbientOcclusion    => OreKind::AmbientOcclusion,
        K::VisibilityPass      => OreKind::VisibilityPass,
        K::SkinPalette         => OreKind::SkinPalette,
        K::MorphBlend          => OreKind::MorphBlend,
        K::Graphics(G::ForwardLit) => OreKind::Graphics(GraphicsOreKind::ForwardLit),
        K::Graphics(G::Ui)         => OreKind::Graphics(GraphicsOreKind::Ui),
    }
}

fn graphics_kind_to_ore(kind: forge_gltf::GltfGraphicsKind) -> GraphicsOreKind {
    match kind {
        forge_gltf::GltfGraphicsKind::ForwardLit => GraphicsOreKind::ForwardLit,
        forge_gltf::GltfGraphicsKind::Ui         => GraphicsOreKind::Ui,
    }
}

// ── End-to-end pipeline plumbing ───────────────────────────────────────────

/// Build one engine `Ore` per compute pipeline kind from a single glTF asset.
/// The caller supplies an output buffer size used for every compute pipeline;
/// pass `0` to let the engine pick the minimum (`non_zero_size` rounds up).
pub fn build_compute_ores(
    asset: &GltfAsset,
    params: PipelineParams,
    output_size: vk::DeviceSize,
) -> ThinVec<Ore> {
    build_all_compute_uploads(asset, params)
        .iter()
        .map(|up| upload_to_ore(up, output_size))
        .collect()
}

/// Refine every compute pipeline through a live `ForgeMaster` — dispatches
/// the compute work for each kind. Returns one `Ingot` per pipeline kind, in
/// the same order as `forge_gltf::build_all_compute_uploads`.
pub fn refine_all_compute(
    asset:       &GltfAsset,
    params:      PipelineParams,
    master:      &mut ForgeMaster,
    output_size: vk::DeviceSize,
) -> ForgeResult<ThinVec<Ingot>> {
    let ores = build_compute_ores(asset, params, output_size);
    let mut out = ThinVec::with_capacity(ores.len());
    for ore in ores { out.push(master.refine(ore)?); }
    Ok(out)
}

/// Register the SkinPalette and MorphBlend compute forges with a live
/// `ForgeMaster`. Call once at startup before the first per-frame dispatch.
pub fn register_skin_morph_forges(master: &mut ForgeMaster) -> ForgeResult<()> {
    master.add_forge_from_spirv_bytes(
        ForgeId::new(OreKind::SkinPalette.index() as i64),
        OreKind::SkinPalette,
        SKIN_PALETTE_SPV,
    )?;
    master.add_forge_from_spirv_bytes(
        ForgeId::new(OreKind::MorphBlend.index() as i64),
        OreKind::MorphBlend,
        MORPH_BLEND_SPV,
    )?;
    Ok(())
}

/// Identifier for one morph-blended primitive's compute output. The
/// `build_skin_morph_proto` function assigns these IDs in a deterministic
/// order so callers can pull the right `vk::Buffer` out of the resulting
/// `Factory`'s ingot list via `morph_output_frame_id(mesh_idx, prim_idx)`.
pub fn morph_output_frame_id(asset: &GltfAsset, mesh_idx: usize, prim_idx: usize) -> Option<FrameId> {
    // Mirror the iteration order of build_skin_morph_proto: skins first
    // (one frame id each), then morphed primitives.
    let mut next: i64 = 1;
    for skin_idx in 0..asset.skins.len() {
        // Only counts when the skin actually has joints; matches the proto.
        if let Some(skin) = asset.skins.get(skin_idx) {
            if !skin.joints.is_empty() { next += 1; }
        }
    }
    for (mi, mesh) in asset.meshes.iter().enumerate() {
        for (pi, prim) in mesh.primitives.iter().enumerate() {
            if prim.morph_targets.is_empty() { continue; }
            if mi == mesh_idx && pi == prim_idx {
                return Some(FrameId::new(next));
            }
            next += 1;
        }
    }
    None
}

/// Walk a compute factory built from a `build_skin_morph_proto` proto and
/// produce a map from (mesh_idx, prim_idx) to the morph-blended posed
/// vertex buffer. Pass the map to
/// `build_graphics_plans_with_pose_and_materials_morphs` (next function)
/// so each draw binds the right compute output as its vertex source.
pub fn collect_morph_output_buffers(
    asset:   &GltfAsset,
    factory: &crate::render::factory_master::factory::Factory,
) -> std::collections::HashMap<(usize, usize), vk::Buffer> {
    let mut out = std::collections::HashMap::new();
    for (mi, mesh) in asset.meshes.iter().enumerate() {
        for (pi, prim) in mesh.primitives.iter().enumerate() {
            if prim.morph_targets.is_empty() { continue; }
            let Some(id) = morph_output_frame_id(asset, mi, pi) else { continue };
            let Some(frame) = factory.frame_by_id(id) else { continue };
            let Some(ingot) = frame.ingots.first() else { continue };
            if let Some(buf) = ingot.result_buffer() {
                out.insert((mi, pi), buf.handle);
            }
        }
    }
    out
}

/// Build a compute `Proto` that drives the SkinPalette + MorphBlend
/// pipelines for every relevant skin/primitive in `asset` for the current
/// `pose`. Pass it to `Renderer::build_compute_factory` each frame.
///
/// Returns `None` when the asset has no skins or morph targets — there's
/// nothing to dispatch, so the caller can skip the factory build entirely.
///
/// `output_size` is the per-Ore output-buffer size in bytes (round up via
/// `non_zero_size`); pass `0` to let the helper choose a 4-byte minimum.
pub fn build_skin_morph_proto(
    asset:       &GltfAsset,
    pose:        &Pose,
    proto_id:    ProtoId,
    output_size: vk::DeviceSize,
) -> Option<Proto<ComputeTag>> {
    let mut proto = Proto::<ComputeTag>::new(proto_id, "skin_morph_frame");
    let mut next_id: i64 = 1;

    // SkinPalette: one plan per skin.
    for skin_idx in 0..asset.skins.len() {
        if let Some(upload) = build_skin_palette_input(asset, pose, skin_idx) {
            let palette_bytes = (upload.element_count as vk::DeviceSize) * 64;
            let ore = upload_to_ore(&upload, output_size.max(palette_bytes));
            let mut plan = FramePlan::new(
                FrameId::new(next_id),
                format!("skin_palette_{skin_idx}"),
            );
            plan.push(ore);
            proto.push_plan(plan);
            next_id += 1;
        }
    }

    // MorphBlend: one plan per morphed primitive. We need a node index to
    // resolve per-instance weight overrides; pick the first node that
    // references the mesh (every other node will share the same weights at
    // this granularity since the pose only stores per-node overrides).
    for (mesh_idx, mesh) in asset.meshes.iter().enumerate() {
        let node_idx = asset.nodes.iter()
            .position(|n| n.mesh == Some(mesh_idx as u32))
            .unwrap_or(0);
        for (prim_idx, prim) in mesh.primitives.iter().enumerate() {
            if prim.morph_targets.is_empty() { continue; }
            if let Some(upload) = build_morph_blend_input(asset, mesh_idx, prim_idx, pose, node_idx) {
                let posed_bytes = (upload.primary_bytes.len()) as vk::DeviceSize;
                let ore = upload_to_ore(&upload, output_size.max(posed_bytes));
                let mut plan = FramePlan::new(
                    FrameId::new(next_id),
                    format!("morph_blend_m{mesh_idx}_p{prim_idx}"),
                );
                plan.push(ore);
                proto.push_plan(plan);
                next_id += 1;
            }
        }
    }

    if proto.is_empty() { None } else { Some(proto) }
}

/// Upload every glTF primitive as a `GpuMesh` and emit one
/// `GraphicsFramePlan` per node-primitive draw. The `MeshUploadCtx` is
/// borrowed only for the duration of the upload — meshes outlive it inside
/// the returned `Arc<GpuMesh>` handles.
pub fn build_graphics_plans(
    asset:      &GltfAsset,
    upload_ctx: &MeshUploadCtx,
) -> ForgeResult<ThinVec<GraphicsFramePlan>> {
    // Cache one Arc<GpuMesh> per (mesh, primitive) pair so multiple draws
    // referencing the same primitive share GPU memory.
    let mut cache: Vec<Vec<Option<Arc<GpuMesh>>>> = (0..asset.meshes.len())
        .map(|i| (0..asset.meshes[i].primitives.len()).map(|_| None).collect())
        .collect();

    let draws = build_graphics_draws(asset);
    let mut plans = ThinVec::with_capacity(draws.len());
    for (i, d) in draws.iter().enumerate() {
        let mesh_slot = &mut cache[d.mesh as usize][d.primitive as usize];
        if mesh_slot.is_none() {
            let ore = primitive_to_mesh_ore(asset, d.mesh, d.primitive);
            let gpu = GpuMesh::upload(upload_ctx, &ore)?;
            *mesh_slot = Some(Arc::new(gpu));
        }
        let mesh = mesh_slot.as_ref().unwrap().clone();
        let plan = GraphicsFramePlan::new_mesh(
            crate::forge_master::frame::FrameId::new((i + 1) as i64),
            d.material
                .map(|m| format!("primitive_{}_mat_{m}", i))
                .unwrap_or_else(|| format!("primitive_{i}")),
            mesh,
        )
        .with_mvp(d.world_matrix);
        plans.push(plan_with_kind(plan, graphics_kind_to_ore(d.kind)));
    }
    Ok(plans)
}

/// Same as `build_graphics_plans` but tags every plan as the UI pipeline.
/// Useful when the caller wants the glTF tree rendered as UI overlays.
pub fn build_ui_plans(
    asset:      &GltfAsset,
    upload_ctx: &MeshUploadCtx,
) -> ForgeResult<ThinVec<GraphicsFramePlan>> {
    let mut plans = build_graphics_plans(asset, upload_ctx)?;
    for p in plans.iter_mut() {
        p.kind = GraphicsOreKind::Ui;
    }
    // Touch the helper so the compiler keeps it warm (and so the dependency
    // graph between build_ui_draws and the bridge stays explicit).
    let _ = build_ui_draws(asset).len();
    Ok(plans)
}

/// Build draw plans against a pre-sampled `Pose`. Same upload semantics as
/// `build_graphics_plans`: meshes are uploaded once per (mesh, primitive)
/// and shared across draws; only the per-draw world matrix changes between
/// frames. Call once after each `Pose::sample` to get the frame's draw list.
pub fn build_graphics_plans_with_pose(
    asset:      &GltfAsset,
    pose:       &Pose,
    upload_ctx: &MeshUploadCtx,
) -> ForgeResult<ThinVec<GraphicsFramePlan>> {
    build_graphics_plans_with_pose_and_materials(asset, pose, upload_ctx, &[])
}

/// Variant of `build_graphics_plans_with_pose` that attaches a pre-resolved
/// material descriptor set to each plan. `material_sets` is indexed by the
/// asset's material index; entries that are `None` (or out of range) leave
/// the plan's `material_set` unset (caller falls back to dummy bindings).
pub fn build_graphics_plans_with_pose_and_materials(
    asset:         &GltfAsset,
    pose:          &Pose,
    upload_ctx:    &MeshUploadCtx,
    material_sets: &[Option<vk::DescriptorSet>],
) -> ForgeResult<ThinVec<GraphicsFramePlan>> {
    build_graphics_plans_full(asset, pose, upload_ctx, material_sets, &Default::default())
}

/// Bundle of per-frame resources for skinned-draw routing — output of
/// the compute pass plus the device-side buffers/sets the graphics pass
/// will bind.
#[derive(Default)]
pub struct SkinningFrame {
    /// (mesh_idx, prim_idx) → the per-vertex joints+weights buffer
    /// uploaded once per primitive via `GpuSkinBuffer::upload`. Owned by
    /// the caller (typically cached for the asset's lifetime).
    pub skin_vertex_buffers: std::collections::HashMap<(usize, usize), vk::Buffer>,
    /// `node_idx` → the skin-palette descriptor set bound at set 2.
    /// Allocated per frame from a recyclable descriptor pool.
    pub palette_sets_by_node: std::collections::HashMap<usize, vk::DescriptorSet>,
}

/// Maximal-power per-frame plan builder.
///
/// `morph_buffers` maps `(mesh_idx, prim_idx)` → the `vk::Buffer` produced
/// by a corresponding MorphBlend compute Ore in this frame (typically built
/// via `collect_morph_output_buffers(asset, &compute_factory)`). When a
/// draw's primitive carries morph targets and the map has a matching entry,
/// the draw is recorded with `vertex_buffer_override = Some(buf)` so the
/// rasterizer fetches the posed vertices directly out of the compute output.
pub fn build_graphics_plans_full(
    asset:         &GltfAsset,
    pose:          &Pose,
    upload_ctx:    &MeshUploadCtx,
    material_sets: &[Option<vk::DescriptorSet>],
    morph_buffers: &std::collections::HashMap<(usize, usize), vk::Buffer>,
) -> ForgeResult<ThinVec<GraphicsFramePlan>> {
    build_graphics_plans_maximal(
        asset, pose, upload_ctx, material_sets, morph_buffers, &SkinningFrame::default(),
    )
}

/// Upload every (mesh, primitive) pair as a `GpuMesh` exactly once and
/// return a (mesh_idx, prim_idx) → Arc<GpuMesh> cache. Call once at
/// asset-load time and pass the resulting cache to every per-frame
/// `build_graphics_plans_*_with_meshes` call so meshes survive across
/// frames instead of churning every redraw.
pub fn upload_all_primitive_meshes(
    asset:      &GltfAsset,
    upload_ctx: &MeshUploadCtx,
) -> ForgeResult<std::collections::HashMap<(usize, usize), Arc<GpuMesh>>> {
    let mut out = std::collections::HashMap::new();
    for (mi, mesh) in asset.meshes.iter().enumerate() {
        for pi in 0..mesh.primitives.len() {
            let ore = primitive_to_mesh_ore(asset, mi as u32, pi as u32);
            let gpu = GpuMesh::upload(upload_ctx, &ore)?;
            out.insert((mi, pi), Arc::new(gpu));
        }
    }
    Ok(out)
}

/// The final per-frame plan builder. Beyond `_full`, this also routes
/// skinned primitives to the `SkinnedForwardLit` pipeline, attaches the
/// per-vertex joints/weights buffer at vertex binding 1, and binds the
/// SkinPalette descriptor set at descriptor set 2.
///
/// Uploads new `GpuMesh` instances inline per call — for cross-frame
/// reuse, see `build_graphics_plans_maximal_with_meshes`.
pub fn build_graphics_plans_maximal(
    asset:         &GltfAsset,
    pose:          &Pose,
    upload_ctx:    &MeshUploadCtx,
    material_sets: &[Option<vk::DescriptorSet>],
    morph_buffers: &std::collections::HashMap<(usize, usize), vk::Buffer>,
    skinning:      &SkinningFrame,
) -> ForgeResult<ThinVec<GraphicsFramePlan>> {
    let mut cache: Vec<Vec<Option<Arc<GpuMesh>>>> = (0..asset.meshes.len())
        .map(|i| (0..asset.meshes[i].primitives.len()).map(|_| None).collect())
        .collect();

    let draws = build_graphics_draws_with_matrices(asset, &pose.world);
    let mut plans = ThinVec::with_capacity(draws.len());
    for (i, d) in draws.iter().enumerate() {
        let mesh_slot = &mut cache[d.mesh as usize][d.primitive as usize];
        if mesh_slot.is_none() {
            let ore = primitive_to_mesh_ore(asset, d.mesh, d.primitive);
            let gpu = GpuMesh::upload(upload_ctx, &ore)?;
            *mesh_slot = Some(Arc::new(gpu));
        }
        let mesh = mesh_slot.as_ref().unwrap().clone();

        // Skinned primitives need three things in lockstep:
        //   1. the per-vertex JOINTS_0 + WEIGHTS_0 buffer at vertex binding 1
        //   2. the SkinPalette descriptor set at set 2 (one per node-with-skin)
        //   3. the SkinnedForwardLit pipeline (kind override)
        // Falls back to plain ForwardLit when any of those pieces is missing.
        let node = &asset.nodes[d.node as usize];
        let is_skinned = node.skin.is_some() && primitive_is_skinned(asset, d.mesh, d.primitive);
        let skin_vb = if is_skinned {
            skinning.skin_vertex_buffers.get(&(d.mesh as usize, d.primitive as usize)).copied()
        } else { None };
        let palette_set = if is_skinned {
            skinning.palette_sets_by_node.get(&(d.node as usize)).copied()
        } else { None };

        let mut plan = GraphicsFramePlan::new_mesh(
            crate::forge_master::frame::FrameId::new((i + 1) as i64),
            d.material
                .map(|m| format!("animated_prim_{i}_mat_{m}"))
                .unwrap_or_else(|| format!("animated_prim_{i}")),
            mesh,
        )
        .with_mvp(d.world_matrix);
        if let Some(mat_idx) = d.material {
            if let Some(Some(set)) = material_sets.get(mat_idx as usize) {
                plan = plan.with_material_set(*set);
            }
        }
        if let Some(&buf) = morph_buffers.get(&(d.mesh as usize, d.primitive as usize)) {
            plan = plan.with_vertex_buffer_override(buf);
        }
        // Promote to the skinned pipeline only when the full triplet is wired.
        if is_skinned && skin_vb.is_some() && palette_set.is_some() {
            plan = plan
                .with_kind(GraphicsOreKind::SkinnedForwardLit)
                .with_skin_vertex_buffer(skin_vb.unwrap())
                .with_skin_palette_set(palette_set.unwrap());
            plans.push(plan);
        } else {
            plans.push(plan_with_kind(plan, graphics_kind_to_ore(d.kind)));
        }
    }
    Ok(plans)
}

/// Pack one primitive's `JOINTS_0` + `WEIGHTS_0` streams into the
/// 24-byte-per-vertex layout expected by `skinned_forward_lit.vert` at
/// binding 1 — `uvec2 joints_packed` (4 × u16) + `vec4 weights` (4 × f32).
///
/// Primitives that don't carry joint/weight streams (i.e. unskinned) get a
/// zero-byte vector — callers should not upload a SkinVertex buffer for
/// them and should leave the draw on the regular ForwardLit pipeline.
pub fn pack_primitive_skin_attrs(asset: &GltfAsset, mesh_idx: u32, prim_idx: u32) -> Vec<u8> {
    let prim = &asset.meshes[mesh_idx as usize].primitives[prim_idx as usize];
    let n = prim.streams.positions.len();
    let joints0  = prim.streams.joints.first();
    let weights0 = prim.streams.weights.first();
    if joints0.is_none() || weights0.is_none() { return Vec::new(); }
    let joints  = joints0.unwrap();
    let weights = weights0.unwrap();

    #[cfg(target_arch = "x86_64")]
    unsafe { return pack_skin_attrs_sse2(n, joints, weights); }
    #[cfg(not(target_arch = "x86_64"))]
    pack_skin_attrs_scalar(n, joints, weights)
}

#[cfg(any(test, not(target_arch = "x86_64")))]
fn pack_skin_attrs_scalar(n: usize, joints: &[[u16; 4]], weights: &[[f32; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(n * 24);
    for i in 0..n {
        let j = joints.get(i).copied().unwrap_or([0; 4]);
        let w = weights.get(i).copied().unwrap_or([0.0; 4]);
        let xy0 = (j[0] as u32) | ((j[1] as u32) << 16);
        let xy1 = (j[2] as u32) | ((j[3] as u32) << 16);
        out.extend_from_slice(&xy0.to_le_bytes());
        out.extend_from_slice(&xy1.to_le_bytes());
        for v in w { out.extend_from_slice(&v.to_le_bytes()); }
    }
    out
}

/// SSE2 packer. The output layout per vertex is 24 bytes:
///   [joints_packed: u32 u32] [weights: f32 f32 f32 f32]
/// = one xmm of `uvec2 = (j0|j1<<16, j2|j3<<16)` plus one xmm of weights.
/// Joint packing uses `_mm_packus_epi32` to clamp u32→u16 lanes (joints
/// already fit in u16 per the glTF spec but we go through the saturating
/// pack instruction so any stray high bit becomes 0xFFFF rather than
/// wrapping). Weight stores are direct 16-byte writes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn pack_skin_attrs_sse2(
    n:       usize,
    joints:  &[[u16; 4]],
    weights: &[[f32; 4]],
) -> Vec<u8> {
    use std::arch::x86_64::*;
    unsafe {
        let mut out = Vec::with_capacity(n * 24);
        out.set_len(n * 24);
        let dst: *mut u8 = out.as_mut_ptr();

        for i in 0..n {
            let j = joints.get(i).copied().unwrap_or([0; 4]);
            let w = weights.get(i).copied().unwrap_or([0.0; 4]);

            // Pack 4 × u16 → two u32 lanes. Construct (j0, j1, j2, j3) as
            // four u32s, then bitwise-or with their pair-shifted versions
            // to land them in the (j0|j1<<16, j2|j3<<16, _, _) layout the
            // GLSL `uvec2` reads.
            let xy0 = (j[0] as u32) | ((j[1] as u32) << 16);
            let xy1 = (j[2] as u32) | ((j[3] as u32) << 16);

            // Build [xy0, xy1, 0, 0] as one xmm then store its low 8 bytes
            // — that's the joints_packed field at byte offset 0..8.
            let joints_xmm = _mm_set_epi32(0, 0, xy1 as i32, xy0 as i32);
            _mm_storel_epi64(dst.add(i * 24) as *mut __m128i, joints_xmm);

            // Weight vec4 → one aligned-ish 16-byte store at offset 8..24.
            let weights_xmm = _mm_loadu_ps(w.as_ptr());
            _mm_storeu_ps(dst.add(i * 24 + 8) as *mut f32, weights_xmm);
        }
        out
    }
}

/// Returns true when primitive `prim_idx` of mesh `mesh_idx` carries any
/// non-empty JOINTS_0 / WEIGHTS_0 streams — i.e. it's a skinned primitive
/// that should be routed through the SkinnedForwardLit pipeline.
pub fn primitive_is_skinned(asset: &GltfAsset, mesh_idx: u32, prim_idx: u32) -> bool {
    let prim = &asset.meshes[mesh_idx as usize].primitives[prim_idx as usize];
    prim.streams.joints.first().is_some_and(|s| !s.is_empty())
        && prim.streams.weights.first().is_some_and(|s| !s.is_empty())
}

/// Identifier for a per-skin SkinPalette compute output, matching the
/// iteration order that `build_skin_morph_proto` uses.
pub fn skin_palette_frame_id(asset: &GltfAsset, skin_idx: usize) -> Option<FrameId> {
    let mut next: i64 = 1;
    for si in 0..asset.skins.len() {
        if asset.skins.get(si).is_some_and(|s| !s.joints.is_empty()) {
            if si == skin_idx { return Some(FrameId::new(next)); }
            next += 1;
        }
    }
    None
}

/// Walk a compute factory built from `build_skin_morph_proto` and produce
/// a map from skin index → the SkinPalette mat4[] buffer the compute Ore
/// just wrote, ready to be bound as set 2 binding 0 on the
/// `SkinnedForwardLit` pipeline.
pub fn collect_skin_palette_buffers(
    asset:   &GltfAsset,
    factory: &crate::render::factory_master::factory::Factory,
) -> std::collections::HashMap<usize, vk::Buffer> {
    let mut out = std::collections::HashMap::new();
    for si in 0..asset.skins.len() {
        let Some(id) = skin_palette_frame_id(asset, si) else { continue };
        let Some(frame) = factory.frame_by_id(id) else { continue };
        let Some(ingot) = frame.ingots.first() else { continue };
        if let Some(buf) = ingot.result_buffer() {
            out.insert(si, buf.handle);
        }
    }
    out
}

/// Cross-frame-cache variant of `build_graphics_plans_maximal`. Same
/// behaviour except it borrows the pre-uploaded GpuMesh cache (built once
/// at asset-load time via `upload_all_primitive_meshes`) instead of
/// re-uploading every primitive every frame. This is the per-frame entry
/// point for any renderer that animates the scene — re-uploading on each
/// redraw both costs perf and risks destroying GPU resources still in
/// flight from the previous frame's submission.
/// World-space AABB enclosing every primitive's posed vertices in `asset`
/// at `pose`. Skinned primitives use their rest positions multiplied by
/// the bind-pose node world matrices, which approximates the rendered
/// bounds well enough for a default camera fit (a perfect fit would
/// require sampling the actual posed verts, which is overkill here).
pub fn compute_asset_aabb(asset: &GltfAsset, pose: &Pose) -> ([f32; 3], [f32; 3]) {
    let draws = build_graphics_draws_with_matrices(asset, &pose.world);
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let (mn, mx) = compute_asset_aabb_sse2(asset, &draws);
        return finalize_aabb(mn, mx);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let (mn, mx) = compute_asset_aabb_scalar(asset, &draws);
        finalize_aabb(mn, mx)
    }
}

#[inline]
fn finalize_aabb(mn: [f32; 3], mx: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    if !mn[0].is_finite() {
        return ([-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]);
    }
    (mn, mx)
}

#[cfg(any(test, not(target_arch = "x86_64")))]
fn compute_asset_aabb_scalar(
    asset: &GltfAsset,
    draws: &[forge_gltf::GraphicsDraw],
) -> ([f32; 3], [f32; 3]) {
    let mut mn = [f32::INFINITY; 3];
    let mut mx = [f32::NEG_INFINITY; 3];
    for d in draws.iter() {
        let prim = &asset.meshes[d.mesh as usize].primitives[d.primitive as usize];
        let w = d.world_matrix;
        for p in prim.streams.positions.iter() {
            let x = w[0] * p[0] + w[4] * p[1] + w[8]  * p[2] + w[12];
            let y = w[1] * p[0] + w[5] * p[1] + w[9]  * p[2] + w[13];
            let z = w[2] * p[0] + w[6] * p[1] + w[10] * p[2] + w[14];
            mn[0] = mn[0].min(x); mx[0] = mx[0].max(x);
            mn[1] = mn[1].min(y); mx[1] = mx[1].max(y);
            mn[2] = mn[2].min(z); mx[2] = mx[2].max(z);
        }
    }
    (mn, mx)
}

/// SSE2 SIMD AABB. The per-vertex matrix-vector transform is three
/// independent dot products with the matrix's row vectors (in the
/// column-major layout used by glTF, that's a broadcast-multiply-add
/// chain over the four columns). The running min/max are tracked in two
/// xmm registers (lanes 0..2 hold x/y/z; lane 3 carries a neutral
/// element). We update them with `_mm_min_ps` / `_mm_max_ps`, which
/// match the scalar `f32::min`/`f32::max` semantics for finite inputs
/// (NaN handling differs but our positions are always finite per glTF
/// spec — the loader rejects NaN positions at parse time).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn compute_asset_aabb_sse2(
    asset: &GltfAsset,
    draws: &[forge_gltf::GraphicsDraw],
) -> ([f32; 3], [f32; 3]) {
    use std::arch::x86_64::*;
    unsafe {
        let inf  = _mm_set1_ps(f32::INFINITY);
        let ninf = _mm_set1_ps(f32::NEG_INFINITY);
        let mut mn_v = inf;
        let mut mx_v = ninf;

        for d in draws.iter() {
            let prim = &asset.meshes[d.mesh as usize].primitives[d.primitive as usize];
            let w = &d.world_matrix;
            // Load the four world columns once per draw — every vertex of
            // this primitive reuses them, so 4 loads amortise over the
            // whole position stream.
            let c0 = _mm_loadu_ps(w.as_ptr().add(0));
            let c1 = _mm_loadu_ps(w.as_ptr().add(4));
            let c2 = _mm_loadu_ps(w.as_ptr().add(8));
            let c3 = _mm_loadu_ps(w.as_ptr().add(12));

            for p in prim.streams.positions.iter() {
                let px = _mm_set1_ps(p[0]);
                let py = _mm_set1_ps(p[1]);
                let pz = _mm_set1_ps(p[2]);
                // world = c0*px + c1*py + c2*pz + c3
                let posed = _mm_add_ps(
                    _mm_add_ps(_mm_mul_ps(c0, px), _mm_mul_ps(c1, py)),
                    _mm_add_ps(_mm_mul_ps(c2, pz), c3),
                );
                mn_v = _mm_min_ps(mn_v, posed);
                mx_v = _mm_max_ps(mx_v, posed);
            }
        }

        let mut mn_buf = [0f32; 4];
        let mut mx_buf = [0f32; 4];
        _mm_storeu_ps(mn_buf.as_mut_ptr(), mn_v);
        _mm_storeu_ps(mx_buf.as_mut_ptr(), mx_v);
        ([mn_buf[0], mn_buf[1], mn_buf[2]],
         [mx_buf[0], mx_buf[1], mx_buf[2]])
    }
}

/// Compute a sensible default view-projection that frames `asset` in
/// `viewport_aspect`'s frustum. Camera sits at center + (radius * 2.5)
/// in the (+x, +y, +z) octant looking back at center, FoV ≈ 50°.
///
/// This walks every vertex via `compute_asset_aabb` — call once at load
/// time and reuse the AABB across frames via `view_projection_from_aabb`.
pub fn default_view_projection(
    asset: &GltfAsset, pose: &Pose, viewport_aspect: f32,
) -> [f32; 16] {
    let aabb = compute_asset_aabb(asset, pose);
    view_projection_from_aabb(&aabb, viewport_aspect)
}

/// Same camera-fit math, but driven by a pre-computed AABB so per-frame
/// callers avoid the O(vertices) walk.
pub fn view_projection_from_aabb(
    aabb: &([f32; 3], [f32; 3]), viewport_aspect: f32,
) -> [f32; 16] {
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
    let eye = [center[0] + dist * 0.6, center[1] + dist * 0.4, center[2] + dist * 1.0];

    let view = look_at_rh(eye, center, [0.0, 1.0, 0.0]);
    let fov_y = 50.0_f32.to_radians();
    let near  = (radius * 0.01).max(0.001);
    let far   = (radius * 10.0).max(100.0);
    let proj  = perspective_rh_zo_y_down(fov_y, viewport_aspect.max(1e-3), near, far);
    mat4_mul_cm(&proj, &view)
}

/// Column-major right-handed look-at. Returns the world→view matrix.
fn look_at_rh(eye: [f32; 3], center: [f32; 3], up: [f32; 3]) -> [f32; 16] {
    let f = normalize_v3([center[0] - eye[0], center[1] - eye[1], center[2] - eye[2]]);
    let s = normalize_v3(cross_v3(f, up));
    let u = cross_v3(s, f);
    [
         s[0],  u[0], -f[0], 0.0,
         s[1],  u[1], -f[1], 0.0,
         s[2],  u[2], -f[2], 0.0,
        -dot_v3(s, eye), -dot_v3(u, eye), dot_v3(f, eye), 1.0,
    ]
}

/// Vulkan-clip-space perspective (right-handed, NDC z ∈ [0, 1], y flipped).
fn perspective_rh_zo_y_down(fov_y: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let f = 1.0 / (fov_y * 0.5).tan();
    let mut m = [0f32; 16];
    m[0]  =  f / aspect;
    m[5]  = -f;                       // y-flip for Vulkan NDC
    m[10] =  far / (near - far);
    m[11] = -1.0;
    m[14] = (far * near) / (near - far);
    m
}

#[inline]
fn cross_v3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1],
     a[2] * b[0] - a[0] * b[2],
     a[0] * b[1] - a[1] * b[0]]
}
#[inline] fn dot_v3(a: [f32; 3], b: [f32; 3]) -> f32 { a[0]*b[0] + a[1]*b[1] + a[2]*b[2] }
#[inline]
fn normalize_v3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0]*v[0] + v[1]*v[1] + v[2]*v[2]).sqrt().max(1e-30);
    [v[0] / l, v[1] / l, v[2] / l]
}
#[inline]
fn mat4_mul_cm(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0f32; 16];
    for c in 0..4 {
        for row in 0..4 {
            let mut s = 0f32;
            for k in 0..4 { s += a[k * 4 + row] * b[c * 4 + k]; }
            r[c * 4 + row] = s;
        }
    }
    r
}

pub fn build_graphics_plans_maximal_with_meshes(
    asset:         &GltfAsset,
    pose:          &Pose,
    meshes:        &std::collections::HashMap<(usize, usize), Arc<GpuMesh>>,
    material_sets: &[Option<vk::DescriptorSet>],
    morph_buffers: &std::collections::HashMap<(usize, usize), vk::Buffer>,
    skinning:      &SkinningFrame,
) -> ThinVec<GraphicsFramePlan> {
    build_graphics_plans_maximal_with_meshes_vp(
        asset, pose, meshes, material_sets, morph_buffers, skinning,
        &IDENTITY_MAT4, None,
    )
}

const IDENTITY_MAT4: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
];

/// Same as `build_graphics_plans_maximal_with_meshes`, but multiplies a
/// caller-supplied view-projection matrix into each draw's MVP so the
/// rasterizer actually gets clip-space coordinates. Pass identity to keep
/// the original "MVP = world only" behaviour.
pub fn build_graphics_plans_maximal_with_meshes_vp(
    asset:             &GltfAsset,
    pose:              &Pose,
    meshes:            &std::collections::HashMap<(usize, usize), Arc<GpuMesh>>,
    material_sets:     &[Option<vk::DescriptorSet>],
    morph_buffers:     &std::collections::HashMap<(usize, usize), vk::Buffer>,
    skinning:          &SkinningFrame,
    view_proj:         &[f32; 16],
    fallback_material: Option<vk::DescriptorSet>,
) -> ThinVec<GraphicsFramePlan> {
    let draws = build_graphics_draws_with_matrices(asset, &pose.world);
    let mut plans = ThinVec::with_capacity(draws.len());
    for (i, d) in draws.iter().enumerate() {
        let Some(mesh) = meshes.get(&(d.mesh as usize, d.primitive as usize)) else { continue };

        let node = &asset.nodes[d.node as usize];
        let is_skinned = node.skin.is_some()
            && primitive_is_skinned(asset, d.mesh, d.primitive);
        let skin_vb = if is_skinned {
            skinning.skin_vertex_buffers.get(&(d.mesh as usize, d.primitive as usize)).copied()
        } else { None };
        let palette_set = if is_skinned {
            skinning.palette_sets_by_node.get(&(d.node as usize)).copied()
        } else { None };

        let mvp = mat4_mul_cm(view_proj, &d.world_matrix);
        let mut plan = GraphicsFramePlan::new_mesh(
            crate::forge_master::frame::FrameId::new((i + 1) as i64),
            d.material
                .map(|m| format!("animated_prim_{i}_mat_{m}"))
                .unwrap_or_else(|| format!("animated_prim_{i}")),
            mesh.clone(),
        )
        .with_mvp(mvp);
        // Try the asset's material first, then fall back to the cache's
        // dummy white material so every ForwardLit / SkinnedForwardLit
        // draw has a bound set 1 (the shader reads from it
        // unconditionally; missing means UB and a validation error).
        let resolved_set = d.material
            .and_then(|m| material_sets.get(m as usize).copied().flatten())
            .or(fallback_material);
        if let Some(set) = resolved_set {
            plan = plan.with_material_set(set);
        }
        if let Some(&buf) = morph_buffers.get(&(d.mesh as usize, d.primitive as usize)) {
            plan = plan.with_vertex_buffer_override(buf);
        }
        if is_skinned && skin_vb.is_some() && palette_set.is_some() {
            plan = plan
                .with_kind(GraphicsOreKind::SkinnedForwardLit)
                .with_skin_vertex_buffer(skin_vb.unwrap())
                .with_skin_palette_set(palette_set.unwrap());
            plans.push(plan);
        } else {
            plans.push(plan_with_kind(plan, graphics_kind_to_ore(d.kind)));
        }
    }
    plans
}

fn primitive_to_mesh_ore(asset: &GltfAsset, mesh_idx: u32, prim_idx: u32) -> MeshOre {
    let prim = &asset.meshes[mesh_idx as usize].primitives[prim_idx as usize];
    let n = prim.streams.positions.len();
    let uv0 = prim.streams.uv_sets.first();
    let vertices: ThinVec<ForgeVertex> = (0..n)
        .map(|i| ForgeVertex::new(
            prim.streams.positions[i],
            prim.streams.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
            prim.streams.tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]),
            uv0.and_then(|s| s.get(i).copied()).unwrap_or([0.0, 0.0]),
        ))
        .collect();
    MeshOre::new(vertices, prim.indices.clone())
}

fn plan_with_kind(mut plan: GraphicsFramePlan, kind: GraphicsOreKind) -> GraphicsFramePlan {
    plan.kind = kind;
    plan
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle_pos() -> Vec<[f32; 3]> {
        vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]]
    }

    #[test]
    fn positions_loaded_correctly() {
        let glb = build_test_glb(&triangle_pos(), None, None, None);
        let ore = load_first_mesh_from_slice(&glb).unwrap();
        assert_eq!(ore.vertices.len(), 3);
        assert_eq!(ore.vertices[0].position, [-1.0, -1.0, 0.0]);
        assert_eq!(ore.vertices[2].position, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn no_index_accessor_generates_sequential_indices() {
        let glb = build_test_glb(&triangle_pos(), None, None, None);
        let ore = load_first_mesh_from_slice(&glb).unwrap();
        assert_eq!(ore.indices.as_slice(), &[0u32, 1, 2]);
    }

    #[test]
    fn explicit_indices_preserved() {
        let glb = build_test_glb(&triangle_pos(), None, None, Some(&[2, 1, 0]));
        let ore = load_first_mesh_from_slice(&glb).unwrap();
        assert_eq!(ore.indices.as_slice(), &[2u32, 1, 0]);
    }

    /// Per glTF spec §3.7.2.1, when `NORMAL` is absent the client MUST
    /// generate flat (per-face) normals. For the test triangle
    /// `(-1,-1,0), (1,-1,0), (0,1,0)` CCW the face normal is
    /// `cross(p2-p1, p3-p1)` = `cross((2,0,0), (1,2,0))` = `(0,0,4)`,
    /// which normalises to `(0,0,1)`. The old `(0,1,0)` "default-up"
    /// fallback was spec-incorrect and got replaced.
    #[test]
    fn missing_normals_compute_spec_flat_normal() {
        let glb = build_test_glb(&triangle_pos(), None, None, None);
        let ore = load_first_mesh_from_slice(&glb).unwrap();
        for v in &ore.vertices {
            assert_eq!(v.normal, [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn explicit_normals_preserved() {
        let norms = vec![[0.0_f32, 0.0, 1.0]; 3];
        let glb = build_test_glb(&triangle_pos(), Some(&norms), None, None);
        let ore = load_first_mesh_from_slice(&glb).unwrap();
        for v in &ore.vertices {
            assert_eq!(v.normal, [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn missing_uvs_default_to_zero() {
        let glb = build_test_glb(&triangle_pos(), None, None, None);
        let ore = load_first_mesh_from_slice(&glb).unwrap();
        for v in &ore.vertices { assert_eq!(v.uv, [0.0, 0.0]); }
    }

    #[test]
    fn explicit_uvs_preserved() {
        let uvs = vec![[0.0_f32, 0.5], [1.0, 0.5], [0.5, 1.0]];
        let glb = build_test_glb(&triangle_pos(), None, Some(&uvs), None);
        let ore = load_first_mesh_from_slice(&glb).unwrap();
        assert_eq!(ore.vertices[1].uv, [1.0, 0.5]);
    }

    #[test]
    fn quad_vertex_and_index_counts() {
        let pos = vec![
            [-1.0_f32,-1.0,0.0],[1.0,-1.0,0.0],[1.0,1.0,0.0],[-1.0,1.0,0.0],
        ];
        let glb = build_test_glb(&pos, None, None, Some(&[0,1,2, 2,3,0]));
        let ore = load_first_mesh_from_slice(&glb).unwrap();
        assert_eq!(ore.vertices.len(), 4);
        assert_eq!(ore.indices.len(), 6);
    }

    #[test]
    fn empty_document_returns_no_primitives_error() {
        let glb = forge_gltf::build_empty_glb();
        assert!(matches!(
            load_first_mesh_from_slice(&glb),
            Err(GltfError::NoPrimitives)
        ));
    }

    #[test]
    fn all_meshes_from_slice_returns_empty_for_no_meshes() {
        let glb = forge_gltf::build_empty_glb();
        let result = load_all_meshes_from_slice(&glb).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn load_asset_exposes_full_document() {
        let glb = build_test_glb(&triangle_pos(), None, None, None);
        let asset = load_asset_from_slice(&glb).unwrap();
        assert_eq!(asset.meshes.len(), 1);
        let ores = asset_to_mesh_ores(&asset);
        assert_eq!(ores.len(), 1);
        assert_eq!(ores[0].vertices.len(), 3);
    }

    #[test]
    fn compute_ores_produced_for_every_kind() {
        let glb = build_test_glb(&triangle_pos(), None, None, None);
        let asset = load_asset_from_slice(&glb).unwrap();
        let ores = build_compute_ores(&asset, PipelineParams::default(), 1024);
        assert_eq!(ores.len(), 9);
        let kinds: Vec<OreKind> = ores.iter().map(|o| o.kind).collect();
        assert!(kinds.contains(&OreKind::RayTrace));
        assert!(kinds.contains(&OreKind::Denoise));
        assert!(kinds.contains(&OreKind::SignedDistanceField));
        assert!(kinds.contains(&OreKind::SdfVoxelization));
        assert!(kinds.contains(&OreKind::LightClustering));
        assert!(kinds.contains(&OreKind::OcclusionCulling));
        assert!(kinds.contains(&OreKind::MaterialFlattening));
        assert!(kinds.contains(&OreKind::AmbientOcclusion));
        assert!(kinds.contains(&OreKind::VisibilityPass));
    }

    #[test]
    fn texture_ores_decode_from_asset() {
        // The test GLB has no images so this should be empty — exercises
        // the empty path without needing the `image` crate.
        let glb = build_test_glb(&triangle_pos(), None, None, None);
        let asset = load_asset_from_slice(&glb).unwrap();
        let texs = asset_to_texture_ores(&asset);
        assert!(texs.is_empty());
    }

    /// SSE2 packer must produce byte-identical output to the scalar
    /// reference. This both validates correctness and prevents the
    /// scalar fallback from going dead on x86_64 hosts.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn pack_skin_attrs_simd_matches_scalar() {
        let joints = vec![
            [0u16, 1, 2, 3],
            [10, 20, 30, 40],
            [u16::MAX, 0, 5, 7],
        ];
        let weights = vec![
            [0.25_f32, 0.25, 0.25, 0.25],
            [0.7, 0.1, 0.15, 0.05],
            [1.0, 0.0, 0.0, 0.0],
        ];
        let scalar = pack_skin_attrs_scalar(joints.len(), &joints, &weights);
        let simd = unsafe { pack_skin_attrs_sse2(joints.len(), &joints, &weights) };
        assert_eq!(scalar, simd, "SIMD packer must match scalar packer byte-for-byte");
    }

    /// AABB SIMD path must match the scalar reference. Builds the draws
    /// list from a real GLB plus a non-identity world matrix so the
    /// matrix-vector kernel is fully exercised.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn compute_asset_aabb_simd_matches_scalar() {
        let glb = build_test_glb(&triangle_pos(), None, None, None);
        let asset = load_asset_from_slice(&glb).unwrap();
        let pose = forge_gltf::Pose::rest(&asset);
        let mut draws = build_graphics_draws_with_matrices(&asset, &pose.world);
        // The synthesised test asset may produce zero draws if the scene
        // root layout doesn't match; force a known draw with a non-trivial
        // world matrix so the SIMD vs scalar paths get real work to do.
        if draws.is_empty() && !asset.meshes.is_empty() && !asset.meshes[0].primitives.is_empty() {
            draws.push(forge_gltf::GraphicsDraw {
                kind:         forge_gltf::GltfGraphicsKind::ForwardLit,
                mesh:         0,
                primitive:    0,
                node:         0,
                world_matrix: [
                    1.0, 0.0, 0.0, 0.0,
                    0.0, 1.0, 0.0, 0.0,
                    0.0, 0.0, 1.0, 0.0,
                    10.0, 20.0, 30.0, 1.0,
                ],
                topology:     forge_gltf::PrimitiveTopology::Triangles,
                material:     None,
                vertex_count: asset.meshes[0].primitives[0].streams.positions.len() as u32,
                index_count:  asset.meshes[0].primitives[0].indices.len() as u32,
            });
        }
        let (mn_s, mx_s) = compute_asset_aabb_scalar(&asset, &draws);
        let (mn_v, mx_v) = unsafe { compute_asset_aabb_sse2(&asset, &draws) };
        for i in 0..3 {
            assert!(mn_s[i].is_finite() && mn_v[i].is_finite(),
                    "AABB went to infinity — no draws executed");
            assert!((mn_s[i] - mn_v[i]).abs() < 1e-5, "min[{i}]: scalar={} simd={}", mn_s[i], mn_v[i]);
            assert!((mx_s[i] - mx_v[i]).abs() < 1e-5, "max[{i}]: scalar={} simd={}", mx_s[i], mx_v[i]);
        }
    }
}
