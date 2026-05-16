//! Pipeline-targeted adapters.
//!
//! Each adapter takes a `GltfAsset` and produces a `PipelineUpload` —
//! a primary + (optional) secondary byte buffer, an element count, and a
//! suggested compute workgroup count. The engine bridge wraps these into
//! `Ore` values keyed by the matching `OreKind`/`GraphicsOreKind`.
//!
//! The kind enums here mirror the engine's enums one-for-one; the engine
//! bridges between them so this crate stays decoupled from Vulkan.

use thin_vec::ThinVec;

use crate::asset::GltfAsset;
use crate::light::LightBlock;
use crate::material::{MaterialBlock, MaterialExtBlock};
use crate::mesh::{Mesh, Primitive, PrimitiveTopology};

pub const IDENTITY_M4: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
];

/// Mirrors `crate::forge_master::ore::GraphicsOreKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GltfGraphicsKind {
    ForwardLit,
    Ui,
}

/// Mirrors `crate::forge_master::ore::OreKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GltfPipelineKind {
    RayTrace,
    Denoise,
    SignedDistanceField,
    SdfVoxelization,
    LightClustering,
    OcclusionCulling,
    MaterialFlattening,
    AmbientOcclusion,
    VisibilityPass,
    Graphics(GltfGraphicsKind),
}

/// Compute-side dispatch payload. `primary_bytes` is the main storage-buffer
/// input; `secondary_bytes` is the optional second input (index data for
/// mesh-shaped pipelines, otherwise empty). `workgroups` is the (x,y,z)
/// dispatch count we recommend — callers can override.
#[derive(Debug, Clone)]
pub struct PipelineUpload {
    pub kind:            GltfPipelineKind,
    pub primary_bytes:   ThinVec<u8>,
    pub secondary_bytes: ThinVec<u8>,
    pub element_count:   u32,
    pub element_stride:  u32,
    pub workgroups:      [u32; 3],
    /// True when this payload describes a triangle mesh (vertex+index pair).
    pub is_mesh:         bool,
}

impl PipelineUpload {
    /// Zero-sized payload — useful as a placeholder when a pipeline has no
    /// glTF-side input for the current asset.
    pub fn empty(kind: GltfPipelineKind) -> Self {
        Self {
            kind,
            primary_bytes:   ThinVec::new(),
            secondary_bytes: ThinVec::new(),
            element_count:   0,
            element_stride:  0,
            workgroups:      [1, 1, 1],
            is_mesh:         false,
        }
    }
}

/// Graphics-side draw description. One per primitive-instance pair: an
/// instanced glTF primitive (mesh primitive + node world matrix) plus the
/// graphics-pipeline kind that should bind it.
#[derive(Debug, Clone)]
pub struct GraphicsDraw {
    pub kind:           GltfGraphicsKind,
    /// Index into `GltfAsset.meshes`.
    pub mesh:           u32,
    /// Index into `mesh.primitives`.
    pub primitive:      u32,
    /// Index into the node array — the source of this draw's world matrix.
    pub node:           u32,
    /// Column-major world matrix to push as model-or-MVP constant.
    pub world_matrix:   [f32; 16],
    pub topology:       PrimitiveTopology,
    pub material:       Option<u32>,
    pub vertex_count:   u32,
    pub index_count:    u32,
}

// ── Vertex layout the ForwardLit pipeline reads ─────────────────────────────
//
// 48 bytes per vertex, identical to the engine's `ForgeVertex` so the bridge
// can blit the buffer with `core::mem::transmute` instead of repacking.

const VERT_BYTES: usize = 4 * (3 + 3 + 4 + 2);

fn encode_vertex(
    pos:     [f32; 3],
    normal:  [f32; 3],
    tangent: [f32; 4],
    uv:      [f32; 2],
    out:     &mut Vec<u8>,
) {
    for v in pos     { out.extend_from_slice(&v.to_le_bytes()); }
    for v in normal  { out.extend_from_slice(&v.to_le_bytes()); }
    for v in tangent { out.extend_from_slice(&v.to_le_bytes()); }
    for v in uv      { out.extend_from_slice(&v.to_le_bytes()); }
}

/// Pack a single primitive's vertex streams to the engine `ForgeVertex` layout.
pub fn pack_primitive_vertices(prim: &Primitive) -> ThinVec<u8> {
    let n = prim.streams.positions.len();
    let mut out = Vec::with_capacity(n * VERT_BYTES);
    let uv0 = prim.streams.uv_sets.first();
    for i in 0..n {
        let p = prim.streams.positions[i];
        let no = prim.streams.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
        let ta = prim.streams.tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]);
        let uv = uv0.and_then(|s| s.get(i).copied()).unwrap_or([0.0, 0.0]);
        encode_vertex(p, no, ta, uv, &mut out);
    }
    out.into_iter().collect()
}

/// Pack a primitive's indices as u32 LE bytes.
pub fn pack_primitive_indices(prim: &Primitive) -> ThinVec<u8> {
    let mut out = Vec::with_capacity(prim.indices.len() * 4);
    for i in &prim.indices { out.extend_from_slice(&i.to_le_bytes()); }
    out.into_iter().collect()
}

// ── Public adapters ─────────────────────────────────────────────────────────

/// Every primitive flattened to engine vertex layout. Mostly useful for
/// triangle-shaped compute pipelines (raytrace BVH input, SDF, visibility).
fn flatten_all_triangle_data(asset: &GltfAsset) -> (ThinVec<u8>, ThinVec<u8>, u32) {
    let mut verts: Vec<u8> = Vec::new();
    let mut inds:  Vec<u8> = Vec::new();
    let mut base:  u32     = 0;
    let mut tri_count: u32 = 0;
    for mesh in &asset.meshes {
        for prim in &mesh.primitives {
            if !matches!(prim.topology, PrimitiveTopology::Triangles) {
                continue;
            }
            let vb = pack_primitive_vertices(prim);
            verts.extend_from_slice(&vb);
            for i in &prim.indices {
                inds.extend_from_slice(&(*i + base).to_le_bytes());
            }
            base += prim.streams.positions.len() as u32;
            tri_count += (prim.indices.len() / 3) as u32;
        }
    }
    (
        verts.into_iter().collect(),
        inds.into_iter().collect(),
        tri_count,
    )
}

/// Dispatch volumes: `(n_x, n_y, n_z)` where `n_x = ceil(elements/64)`.
fn workgroups_1d(elements: u32) -> [u32; 3] {
    [((elements + 63) / 64).max(1), 1, 1]
}

// ── ForwardLit / Ui — graphics ──────────────────────────────────────────────

/// One `GraphicsDraw` per primitive-instance pair in the primary scene.
/// All draws are tagged `ForwardLit` — the engine bridge can re-tag a UI
/// subset later if needed.
pub fn build_graphics_draws(asset: &GltfAsset) -> ThinVec<GraphicsDraw> {
    build_graphics_draws_with_matrices(asset, &asset.world_matrices())
}

/// Same as `build_graphics_draws` but with caller-supplied world matrices —
/// the typical use case is feeding a `Pose::world` from the animation
/// evaluator so each frame's draw list reflects the sampled animation.
pub fn build_graphics_draws_with_matrices(
    asset: &GltfAsset,
    world: &[[f32; 16]],
) -> ThinVec<GraphicsDraw> {
    let mut out = ThinVec::new();
    for (node_idx, node) in asset.nodes.iter().enumerate() {
        let Some(mesh_idx) = node.mesh else { continue };
        let mesh = &asset.meshes[mesh_idx as usize];
        let world_m = world.get(node_idx).copied().unwrap_or(IDENTITY_M4);
        for (prim_idx, prim) in mesh.primitives.iter().enumerate() {
            out.push(GraphicsDraw {
                kind:         GltfGraphicsKind::ForwardLit,
                mesh:         mesh_idx,
                primitive:    prim_idx as u32,
                node:         node_idx as u32,
                world_matrix: world_m,
                topology:     prim.topology,
                material:     prim.material,
                vertex_count: prim.streams.positions.len() as u32,
                index_count:  prim.indices.len() as u32,
            });
        }
    }
    out
}

/// Same shape as `build_graphics_draws` but tagged for the UI pipeline. The
/// pre-multiplied world matrix is preserved so screen-space layouts that
/// reuse the glTF node tree still get correct offsets.
pub fn build_ui_draws(asset: &GltfAsset) -> ThinVec<GraphicsDraw> {
    let mut draws = build_graphics_draws(asset);
    for d in &mut draws { d.kind = GltfGraphicsKind::Ui; }
    draws
}

// ── RayTrace — compute ──────────────────────────────────────────────────────
//
// Flat triangle list in `primary_bytes`, index list in `secondary_bytes`,
// one workgroup per 64 triangles.

pub fn build_raytrace_input(asset: &GltfAsset) -> PipelineUpload {
    let (verts, inds, tri_count) = flatten_all_triangle_data(asset);
    PipelineUpload {
        kind:            GltfPipelineKind::RayTrace,
        primary_bytes:   verts,
        secondary_bytes: inds,
        element_count:   tri_count,
        element_stride:  VERT_BYTES as u32,
        workgroups:      workgroups_1d(tri_count),
        is_mesh:         true,
    }
}

// ── Denoise — compute ───────────────────────────────────────────────────────
//
// Denoise reads from a previous-frame storage image, so the glTF document
// itself contributes nothing. We still emit a payload so callers can wire
// the pipeline with a sized `Empty` ore — workgroups land on a default 8x8
// tile when no resolution is provided.

pub fn build_denoise_input(_asset: &GltfAsset, image_size: [u32; 2]) -> PipelineUpload {
    let [w, h] = image_size;
    PipelineUpload {
        kind:            GltfPipelineKind::Denoise,
        primary_bytes:   ThinVec::new(),
        secondary_bytes: ThinVec::new(),
        element_count:   w * h,
        element_stride:  0,
        workgroups:      [(w + 7) / 8, (h + 7) / 8, 1],
        is_mesh:         false,
    }
}

// ── SignedDistanceField — compute ───────────────────────────────────────────

pub fn build_sdf_input(asset: &GltfAsset) -> PipelineUpload {
    let (verts, inds, tri_count) = flatten_all_triangle_data(asset);
    PipelineUpload {
        kind:            GltfPipelineKind::SignedDistanceField,
        primary_bytes:   verts,
        secondary_bytes: inds,
        element_count:   tri_count,
        element_stride:  VERT_BYTES as u32,
        workgroups:      workgroups_1d(tri_count),
        is_mesh:         true,
    }
}

// ── SdfVoxelization — compute ───────────────────────────────────────────────
//
// Voxelizes the merged triangle soup into a `grid_size**3` volume.

pub fn build_sdf_voxel_input(asset: &GltfAsset, grid_size: u32) -> PipelineUpload {
    let (verts, inds, tri_count) = flatten_all_triangle_data(asset);
    let g = grid_size.max(1);
    PipelineUpload {
        kind:            GltfPipelineKind::SdfVoxelization,
        primary_bytes:   verts,
        secondary_bytes: inds,
        element_count:   tri_count,
        element_stride:  VERT_BYTES as u32,
        workgroups:      [(g + 3) / 4, (g + 3) / 4, (g + 3) / 4],
        is_mesh:         true,
    }
}

// ── LightClustering — compute ───────────────────────────────────────────────
//
// Walks every node that points at a light, baking world-space position +
// direction into a `LightBlock`. Directional lights have an implied
// direction of -Z in light-local space; we transform `(0,0,-1,0)`.

pub fn build_light_cluster_input(asset: &GltfAsset) -> PipelineUpload {
    let world = asset.world_matrices();
    let mut bytes: Vec<u8> = Vec::new();
    let mut count = 0u32;
    for (node_idx, node) in asset.nodes.iter().enumerate() {
        let Some(light_idx) = node.light else { continue };
        let light = &asset.lights[light_idx as usize];
        let m = world.get(node_idx).copied().unwrap_or(IDENTITY_M4);
        let pos = [m[12], m[13], m[14]];
        let dir = mat4_mul_dir(&m, [0.0, 0.0, -1.0]);
        let block = LightBlock::from_light(light, pos, dir);
        bytes.extend_from_slice(unsafe {
            core::slice::from_raw_parts(
                (&block as *const LightBlock).cast::<u8>(),
                LightBlock::BYTES,
            )
        });
        count += 1;
    }
    PipelineUpload {
        kind:            GltfPipelineKind::LightClustering,
        primary_bytes:   bytes.into_iter().collect(),
        secondary_bytes: ThinVec::new(),
        element_count:   count,
        element_stride:  LightBlock::BYTES as u32,
        workgroups:      workgroups_1d(count),
        is_mesh:         false,
    }
}

fn mat4_mul_dir(m: &[f32; 16], d: [f32; 3]) -> [f32; 3] {
    [
        m[0] * d[0] + m[4] * d[1] + m[8]  * d[2],
        m[1] * d[0] + m[5] * d[1] + m[9]  * d[2],
        m[2] * d[0] + m[6] * d[1] + m[10] * d[2],
    ]
}

// ── OcclusionCulling — compute ──────────────────────────────────────────────
//
// One AABB per node-with-mesh, transformed into world space (8-corner
// projection). 32 bytes per record: min.xyz | _ | max.xyz | _.

pub fn build_occlusion_input(asset: &GltfAsset) -> PipelineUpload {
    let world = asset.world_matrices();
    let mut bytes: Vec<u8> = Vec::new();
    let mut count = 0u32;
    for (node_idx, node) in asset.nodes.iter().enumerate() {
        let Some(mesh_idx) = node.mesh else { continue };
        let mesh: &Mesh = &asset.meshes[mesh_idx as usize];
        let m = world.get(node_idx).copied().unwrap_or(IDENTITY_M4);
        for prim in &mesh.primitives {
            let (mn, mx) = aabb_world(&m, prim.bounds.min, prim.bounds.max);
            for v in [mn[0], mn[1], mn[2], 0.0_f32, mx[0], mx[1], mx[2], 0.0_f32] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            count += 1;
        }
    }
    PipelineUpload {
        kind:            GltfPipelineKind::OcclusionCulling,
        primary_bytes:   bytes.into_iter().collect(),
        secondary_bytes: ThinVec::new(),
        element_count:   count,
        element_stride:  32,
        workgroups:      workgroups_1d(count),
        is_mesh:         false,
    }
}

fn aabb_world(m: &[f32; 16], mn: [f32; 3], mx: [f32; 3]) -> ([f32; 3], [f32; 3]) {
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
    let mut out_min = [f32::MAX; 3];
    let mut out_max = [f32::MIN; 3];
    for c in corners {
        let w = mat4_mul_point(m, c);
        for i in 0..3 {
            if w[i] < out_min[i] { out_min[i] = w[i]; }
            if w[i] > out_max[i] { out_max[i] = w[i]; }
        }
    }
    (out_min, out_max)
}

fn mat4_mul_point(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8]  * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9]  * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

// ── MaterialFlattening — compute ────────────────────────────────────────────
//
// Per material we write an 80-byte `MaterialBlock` (PBR + base extensions)
// in `primary_bytes` and a 128-byte `MaterialExtBlock` (clearcoat / sheen /
// specular / iridescence / anisotropy / diffuse_transmission / dispersion)
// in `secondary_bytes`. Both arrays are the same length and indexed by
// material id — shaders bind them as a pair.

pub fn build_material_input(asset: &GltfAsset) -> PipelineUpload {
    let n = asset.materials.len();
    let mut base: Vec<u8> = Vec::with_capacity(n * MaterialBlock::BYTES);
    let mut ext:  Vec<u8> = Vec::with_capacity(n * MaterialExtBlock::BYTES);
    for m in &asset.materials {
        let b = MaterialBlock::from_material(m);
        base.extend_from_slice(unsafe {
            core::slice::from_raw_parts(
                (&b as *const MaterialBlock).cast::<u8>(),
                MaterialBlock::BYTES,
            )
        });
        let e = MaterialExtBlock::from_material(m);
        ext.extend_from_slice(unsafe {
            core::slice::from_raw_parts(
                (&e as *const MaterialExtBlock).cast::<u8>(),
                MaterialExtBlock::BYTES,
            )
        });
    }
    PipelineUpload {
        kind:            GltfPipelineKind::MaterialFlattening,
        primary_bytes:   base.into_iter().collect(),
        secondary_bytes: ext.into_iter().collect(),
        element_count:   n as u32,
        element_stride:  MaterialBlock::BYTES as u32,
        workgroups:      workgroups_1d(n as u32),
        is_mesh:         false,
    }
}

// ── AmbientOcclusion — compute ──────────────────────────────────────────────
//
// SSAO-style input: position + normal per vertex. We reuse the full
// `ForgeVertex` packing so it's a drop-in for any compute kernel that wants
// the full vertex set.

pub fn build_ao_input(asset: &GltfAsset) -> PipelineUpload {
    let (verts, inds, tri_count) = flatten_all_triangle_data(asset);
    let elements = (verts.len() / VERT_BYTES) as u32;
    PipelineUpload {
        kind:            GltfPipelineKind::AmbientOcclusion,
        primary_bytes:   verts,
        secondary_bytes: inds,
        element_count:   elements,
        element_stride:  VERT_BYTES as u32,
        workgroups:      workgroups_1d(elements.max(tri_count)),
        is_mesh:         true,
    }
}

// ── VisibilityPass — compute ────────────────────────────────────────────────

pub fn build_visibility_input(asset: &GltfAsset) -> PipelineUpload {
    let (verts, inds, tri_count) = flatten_all_triangle_data(asset);
    PipelineUpload {
        kind:            GltfPipelineKind::VisibilityPass,
        primary_bytes:   verts,
        secondary_bytes: inds,
        element_count:   tri_count,
        element_stride:  VERT_BYTES as u32,
        workgroups:      workgroups_1d(tri_count),
        is_mesh:         true,
    }
}

// ── Dispatch table ──────────────────────────────────────────────────────────

/// Build one `PipelineUpload` for every compute pipeline kind the engine
/// supports. Caller-supplied parameters (image size, voxel grid size) are
/// taken from `params`. Useful as a one-shot "prepare everything" call.
#[derive(Debug, Clone, Copy)]
pub struct PipelineParams {
    pub denoise_image_size: [u32; 2],
    pub sdf_voxel_grid:     u32,
}

impl Default for PipelineParams {
    fn default() -> Self { Self { denoise_image_size: [1, 1], sdf_voxel_grid: 16 } }
}

pub fn build_all_compute_uploads(
    asset:  &GltfAsset,
    params: PipelineParams,
) -> ThinVec<PipelineUpload> {
    let mut out = ThinVec::new();
    out.push(build_raytrace_input(asset));
    out.push(build_denoise_input(asset, params.denoise_image_size));
    out.push(build_sdf_input(asset));
    out.push(build_sdf_voxel_input(asset, params.sdf_voxel_grid));
    out.push(build_light_cluster_input(asset));
    out.push(build_occlusion_input(asset));
    out.push(build_material_input(asset));
    out.push(build_ao_input(asset));
    out.push(build_visibility_input(asset));
    out
}

/// Convenience: every graphics draw for both pipelines.
pub fn build_all_graphics_draws(asset: &GltfAsset) -> ThinVec<GraphicsDraw> {
    let mut out = build_graphics_draws(asset);
    // Don't duplicate every primitive — UI draws are opt-in via build_ui_draws.
    // Empty by default so the engine doesn't render the same mesh twice.
    out.extend(ThinVec::<GraphicsDraw>::new());
    out
}

impl GltfPipelineKind {
    pub fn build(self, asset: &GltfAsset, params: PipelineParams) -> Option<PipelineUpload> {
        match self {
            GltfPipelineKind::RayTrace            => Some(build_raytrace_input(asset)),
            GltfPipelineKind::Denoise             => Some(build_denoise_input(asset, params.denoise_image_size)),
            GltfPipelineKind::SignedDistanceField => Some(build_sdf_input(asset)),
            GltfPipelineKind::SdfVoxelization     => Some(build_sdf_voxel_input(asset, params.sdf_voxel_grid)),
            GltfPipelineKind::LightClustering     => Some(build_light_cluster_input(asset)),
            GltfPipelineKind::OcclusionCulling    => Some(build_occlusion_input(asset)),
            GltfPipelineKind::MaterialFlattening  => Some(build_material_input(asset)),
            GltfPipelineKind::AmbientOcclusion    => Some(build_ao_input(asset)),
            GltfPipelineKind::VisibilityPass      => Some(build_visibility_input(asset)),
            GltfPipelineKind::Graphics(_)         => None,
        }
    }
}
