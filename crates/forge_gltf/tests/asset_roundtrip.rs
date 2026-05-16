//! End-to-end smoke tests: build a GLB in memory, load it through the
//! full extractor, and run every pipeline adapter over the result. Covers
//! the "no meshes" case too.

use forge_gltf::*;

fn triangle_pos() -> Vec<[f32; 3]> {
    vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]]
}

#[test]
fn load_triangle_has_one_mesh_one_primitive() {
    let glb = build_test_glb(&triangle_pos(), None, None, None);
    let asset = GltfAsset::load_slice(&glb).unwrap();
    assert_eq!(asset.meshes.len(), 1);
    assert_eq!(asset.meshes[0].primitives.len(), 1);
    assert_eq!(asset.meshes[0].primitives[0].streams.positions.len(), 3);
    assert_eq!(asset.meshes[0].primitives[0].indices.as_slice(), &[0, 1, 2]);
}

#[test]
fn load_quad_with_explicit_indices() {
    let pos = vec![
        [-1.0_f32, -1.0, 0.0], [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],       [-1.0, 1.0, 0.0],
    ];
    let glb = build_test_glb(&pos, None, None, Some(&[0, 1, 2, 2, 3, 0]));
    let asset = GltfAsset::load_slice(&glb).unwrap();
    let prim = &asset.meshes[0].primitives[0];
    assert_eq!(prim.streams.positions.len(), 4);
    assert_eq!(prim.indices.as_slice(), &[0, 1, 2, 2, 3, 0]);
}

#[test]
fn empty_doc_has_no_meshes() {
    let glb = build_empty_glb();
    let asset = GltfAsset::load_slice(&glb).unwrap();
    assert!(asset.meshes.is_empty());
}

#[test]
fn aabb_from_positions_matches_bounds() {
    let glb = build_test_glb(&triangle_pos(), None, None, None);
    let asset = GltfAsset::load_slice(&glb).unwrap();
    let b = asset.meshes[0].primitives[0].bounds;
    assert_eq!(b.min, [-1.0, -1.0, 0.0]);
    assert_eq!(b.max, [1.0, 1.0, 0.0]);
}

#[test]
fn all_pipeline_adapters_produce_consistent_sizes() {
    let pos = vec![
        [-1.0_f32, -1.0, 0.0], [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],       [-1.0, 1.0, 0.0],
    ];
    let glb = build_test_glb(&pos, None, None, Some(&[0, 1, 2, 2, 3, 0]));
    let asset = GltfAsset::load_slice(&glb).unwrap();

    let rt = build_raytrace_input(&asset);
    assert!(rt.is_mesh);
    assert_eq!(rt.element_count, 2);
    assert_eq!(rt.primary_bytes.len(), 4 * 48); // 4 verts × 48 bytes
    assert_eq!(rt.secondary_bytes.len(), 6 * 4); // 6 indices × 4 bytes

    let sdf = build_sdf_input(&asset);
    assert_eq!(sdf.element_count, 2);

    let vox = build_sdf_voxel_input(&asset, 8);
    assert_eq!(vox.workgroups, [2, 2, 2]);

    let mats = build_material_input(&asset);
    assert_eq!(mats.element_count, 0); // no materials in test glb
    assert!(mats.primary_bytes.is_empty());

    let occ = build_occlusion_input(&asset);
    // no node references this mesh in our minimal glb → 0 occluders
    assert_eq!(occ.element_count, 0);

    let denoise = build_denoise_input(&asset, [64, 64]);
    assert_eq!(denoise.workgroups, [8, 8, 1]);

    let vis = build_visibility_input(&asset);
    assert_eq!(vis.element_count, 2);

    let ao = build_ao_input(&asset);
    assert_eq!(ao.element_count, 4);

    let lc = build_light_cluster_input(&asset);
    assert_eq!(lc.element_count, 0);
}

#[test]
fn build_all_compute_uploads_returns_nine_payloads() {
    let glb = build_test_glb(&triangle_pos(), None, None, None);
    let asset = GltfAsset::load_slice(&glb).unwrap();
    let uploads = build_all_compute_uploads(&asset, PipelineParams::default());
    assert_eq!(uploads.len(), 9);
}

#[test]
fn pipeline_kind_build_dispatches_correctly() {
    let glb = build_test_glb(&triangle_pos(), None, None, None);
    let asset = GltfAsset::load_slice(&glb).unwrap();
    let params = PipelineParams::default();

    for kind in [
        GltfPipelineKind::RayTrace,
        GltfPipelineKind::Denoise,
        GltfPipelineKind::SignedDistanceField,
        GltfPipelineKind::SdfVoxelization,
        GltfPipelineKind::LightClustering,
        GltfPipelineKind::OcclusionCulling,
        GltfPipelineKind::MaterialFlattening,
        GltfPipelineKind::AmbientOcclusion,
        GltfPipelineKind::VisibilityPass,
    ] {
        let up = kind.build(&asset, params).expect("compute pipelines return Some");
        assert_eq!(up.kind, kind);
    }
    assert!(GltfPipelineKind::Graphics(GltfGraphicsKind::ForwardLit).build(&asset, params).is_none());
}

#[test]
fn graphics_draws_empty_when_no_nodes_reference_mesh() {
    let glb = build_test_glb(&triangle_pos(), None, None, None);
    let asset = GltfAsset::load_slice(&glb).unwrap();
    let draws = build_graphics_draws(&asset);
    assert!(draws.is_empty()); // the minimal GLB has no scene/nodes
}

#[test]
fn material_block_size_is_80_bytes() {
    assert_eq!(MaterialBlock::BYTES, 80);
}

#[test]
fn light_block_size_is_48_bytes() {
    assert_eq!(LightBlock::BYTES, 48);
}
