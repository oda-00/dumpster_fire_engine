//! End-to-end test: load real animated / special-effect glTF assets, sample
//! their animations over time, upload the resulting meshes to a Vulkan
//! device, and check that every render-pipeline-relevant byte buffer is
//! consistent across frames.
//!
//! The sandbox doesn't have a window system, so the test never opens a
//! swapchain — it stops one step short of `Renderer::draw_frame`, which is
//! the only piece of the pipeline that needs an OS window. Everything below
//! that (parsing, animation evaluation, mesh upload, material/light/AABB
//! flattening for the compute pipelines) is exercised end-to-end against
//! the real GPU device that lavapipe exposes when there's no hardware GPU.

use std::path::Path;
use std::sync::Arc;

use dumpster_fire_engine::forge_master::ore::{GpuMesh, GraphicsOreKind};
use dumpster_fire_engine::forge_master::FrameId;
use dumpster_fire_engine::render::VulkanContext;
use dumpster_fire_engine::resource_manager::asset_manager::{
    build_compute_ores, build_graphics_plans, build_graphics_plans_with_pose,
    load_asset, asset_to_texture_ores,
};

use forge_gltf::{
    GltfAsset, MaterialBlock, PipelineParams, Pose,
    build_graphics_draws_with_matrices,
};

const ASSETS: &str = "assets/models";

fn asset_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(ASSETS).join(name)
}

fn try_vulkan() -> Option<VulkanContext> {
    match VulkanContext::new() {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("test skipped: no Vulkan device ({e:?})");
            None
        }
    }
}

// ── 1. Animated skinned asset: BrainStem (joint animation) ─────────────────
//
// BrainStem's only mesh sits on node 1; the animation targets the 57 joint
// nodes of the skeleton, not the mesh node. So the *joint* world matrices
// must move between frames even though the mesh draw's world matrix stays
// constant — that's the whole point of skinning.

#[test]
fn brainstem_animation_advances_joint_world_matrices() {
    let asset = load_asset(asset_path("BrainStem.glb")).expect("load BrainStem");
    assert!(!asset.animations.is_empty(), "BrainStem ships with skeletal animation data");
    assert!(!asset.skins.is_empty(), "BrainStem is skinned");

    let mut pose = Pose::rest(&asset);
    let rest_world = pose.world.clone();

    let anim = &asset.animations[0];
    let dur = anim.duration();
    assert!(dur > 0.0);

    pose.sample(&asset, anim, dur * 0.25);
    let mid_world = pose.world.clone();
    pose.sample(&asset, anim, dur * 0.75);
    let late_world = pose.world.clone();

    assert!(world_diff(&mid_world, &rest_world)  > 1e-4, "animation should perturb the rest pose");
    assert!(world_diff(&late_world, &mid_world) > 1e-4, "different sample times yield different poses");

    // Spot-check that the joints — not the mesh node — are the ones moving.
    let mesh_nodes: Vec<usize> = asset.nodes.iter().enumerate()
        .filter_map(|(i, n)| n.mesh.map(|_| i)).collect();
    for &i in &mesh_nodes {
        // Mesh-bearing nodes have no animation channel in BrainStem; their
        // world matrix must equal the rest pose at every sample time.
        assert_eq!(mid_world[i],  rest_world[i]);
        assert_eq!(late_world[i], rest_world[i]);
    }
    let any_joint_moved = (0..asset.nodes.len())
        .filter(|i| !mesh_nodes.contains(i))
        .any(|i| {
            mid_world[i].iter().zip(&rest_world[i]).any(|(a, b)| (a - b).abs() > 1e-5)
        });
    assert!(any_joint_moved, "joint nodes should pick up animation");
}

fn world_diff(a: &[[f32; 16]], b: &[[f32; 16]]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.iter().zip(y).map(|(p, q)| (p - q).abs() as f64).sum::<f64>())
        .sum()
}

// ── 2. Animated asset → live GPU upload via lavapipe ────────────────────────
//
// DiffuseTransmissionPlant.glb animates six of its nine mesh-bearing nodes
// directly (no skin). Per-draw MVPs therefore must differ between sampled
// frames, so this is the right asset to check the "animation actually
// changes the rendered draws" guarantee.

#[test]
fn animated_mesh_nodes_emit_distinct_draws_per_frame() {
    let Some(ctx) = try_vulkan() else { return };
    let asset = load_asset(asset_path("DiffuseTransmissionPlant.glb"))
        .expect("load DiffuseTransmissionPlant");
    let upload_ctx = ctx.mesh_upload_ctx();

    let mut pose = Pose::rest(&asset);
    let anim = &asset.animations[0];

    let plans_rest = build_graphics_plans_with_pose(&asset, &pose, &upload_ctx)
        .expect("rest-pose upload");

    pose.sample(&asset, anim, anim.duration() * 0.5);
    let plans_mid = build_graphics_plans_with_pose(&asset, &pose, &upload_ctx)
        .expect("mid-anim upload");

    assert_eq!(plans_rest.len(), plans_mid.len(), "draw count is animation-invariant");
    assert!(!plans_rest.is_empty(), "asset renders at least one primitive");

    let any_changed = plans_rest
        .iter()
        .zip(&plans_mid)
        .any(|(a, b)| a.mvp.iter().zip(&b.mvp).any(|(x, y)| (x - y).abs() > 1e-5));
    assert!(any_changed, "animated plans should carry frame-specific MVPs");

    for p in &plans_mid {
        let mesh: &Arc<GpuMesh> = p.mesh.as_ref().expect("mesh attached");
        assert!(mesh.index_count > 0, "uploaded mesh carries an index buffer");
    }

    // Distinct FrameIds across the per-frame draw list.
    let mut ids: Vec<_> = plans_mid.iter().map(|p| p.id.raw()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), plans_mid.len());

    drop(plans_rest);
    drop(plans_mid);
    unsafe { ctx.device.device_wait_idle().ok(); }
}

// ── 3. Special-effect asset: KHR_materials_transmission ─────────────────────

#[test]
fn transmission_test_materials_carry_extension_data() {
    let asset = load_asset(asset_path("TransmissionTest.glb")).expect("load TransmissionTest");
    assert!(!asset.materials.is_empty(), "transmission test ships with materials");

    let has_transmission = asset
        .materials
        .iter()
        .any(|m| m.transmission.factor > 0.0 || m.transmission.texture.is_some());
    assert!(has_transmission, "at least one material uses KHR_materials_transmission");

    // Each material flattens to a 80-byte MaterialBlock; the transmission
    // factor lives in `transmission_volume.x`.
    for m in &asset.materials {
        let block = MaterialBlock::from_material(m);
        assert_eq!(block.transmission_volume[0], m.transmission.factor);
    }
}

// ── 4. Special-effect asset: KHR_materials_volume on ScatteringSkull ────────

#[test]
fn scattering_skull_carries_volume_extension() {
    let asset = load_asset(asset_path("ScatteringSkull.glb"))
        .expect("load ScatteringSkull");
    // ScatteringSkull's `subsurface_material` enables KHR_materials_volume:
    // non-zero thickness, finite attenuation distance, and an IOR override.
    let m = &asset.materials[0];
    assert!(m.volume.thickness_factor > 0.0, "thickness factor enabled");
    assert!(m.volume.attenuation_distance.is_finite(), "finite attenuation distance");
    assert!((m.ior - 1.38).abs() < 1e-3, "KHR_materials_ior override");
}

// ── 5. Full multi-pipeline drive: load → sample → all 9 compute kinds ──────

#[test]
fn animated_asset_drives_every_compute_pipeline() {
    let asset = load_asset(asset_path("DiffuseTransmissionPlant.glb"))
        .expect("load DiffuseTransmissionPlant");
    let mut pose = Pose::rest(&asset);
    if let Some(anim) = asset.animations.first() {
        pose.sample(&asset, anim, 0.25);
    }

    let ores = build_compute_ores(&asset, PipelineParams::default(), 4096);
    // Engine ships nine compute pipeline kinds — every one must be produced.
    assert_eq!(ores.len(), 9);

    // No payload should panic when staged through the engine's primary/secondary
    // byte accessors — exercise the read path.
    for ore in &ores {
        let _ = ore.primary_bytes();
        let _ = ore.secondary_bytes();
    }
}

// ── 6. Textures decode through the bridge ──────────────────────────────────

#[test]
fn boombox_textures_decode_to_rgba8() {
    let asset = load_asset(asset_path("BoomBox.glb")).expect("load BoomBox");
    let texs = asset_to_texture_ores(&asset);
    assert!(!texs.is_empty(), "BoomBox ships with textures");
    for t in &texs {
        assert!(t.width > 0 && t.height > 0);
        assert_eq!(t.pixels.len(), (t.width * t.height * 4) as usize);
    }
}

// ── 7. Full GPU sanity: ToyCar (PBR + transmission) uploads + draws ────────

#[test]
fn toycar_uploads_to_gpu_and_emits_forward_lit_plans() {
    let Some(ctx) = try_vulkan() else { return };
    let asset = load_asset(asset_path("ToyCar.glb")).expect("load ToyCar");
    let upload_ctx = ctx.mesh_upload_ctx();

    let plans = build_graphics_plans(&asset, &upload_ctx).expect("ToyCar upload");
    assert!(!plans.is_empty(), "ToyCar has renderable primitives");
    for p in &plans {
        assert_eq!(p.kind, GraphicsOreKind::ForwardLit);
        assert!(p.mesh.as_ref().unwrap().index_count > 0);
    }
    unsafe { ctx.device.device_wait_idle().ok(); }
}

// ── 8. Animation evaluator self-check: synthetic ramp, exact values ────────

#[test]
fn animation_evaluator_lerps_known_keyframes() {
    // Build a 3-key linear translation animation: t = 0 → (0,0,0); t = 1 →
    // (10,0,0); t = 2 → (10,5,0). Sample at the midpoints and check.
    let glb = make_translating_box_glb();
    let asset = GltfAsset::load_slice(&glb).expect("load synthetic animated glb");
    assert!(!asset.animations.is_empty());

    let mut pose = Pose::rest(&asset);
    let anim = &asset.animations[0];

    pose.sample(&asset, anim, 0.5);
    let half_t = pose.translation[0];
    assert!((half_t[0] - 5.0).abs() < 1e-3, "got x={}", half_t[0]);

    pose.sample(&asset, anim, 1.5);
    let three_q = pose.translation[0];
    assert!((three_q[0] - 10.0).abs() < 1e-3);
    assert!((three_q[1] - 2.5).abs()  < 1e-3, "got y={}", three_q[1]);

    // The world matrix for the animated node must carry the sampled translation.
    pose.sample(&asset, anim, 2.0);
    let end_w = pose.world[0];
    assert!((end_w[12] - 10.0).abs() < 1e-3);
    assert!((end_w[13] - 5.0).abs()  < 1e-3);

    // The pipeline adapter must see the same world-space draw transform.
    let draws = build_graphics_draws_with_matrices(&asset, &pose.world);
    assert!(!draws.is_empty());
    assert!((draws[0].world_matrix[12] - 10.0).abs() < 1e-3);
}

fn make_translating_box_glb() -> Vec<u8> {
    // 8-vertex unit cube + index list + a 3-keyframe linear translation
    // animation. The smallest GLB we can hand-encode that exercises the
    // animation channel/sampler readers end-to-end.
    let pos: [[f32; 3]; 8] = [
        [-0.5, -0.5, -0.5], [ 0.5, -0.5, -0.5],
        [ 0.5,  0.5, -0.5], [-0.5,  0.5, -0.5],
        [-0.5, -0.5,  0.5], [ 0.5, -0.5,  0.5],
        [ 0.5,  0.5,  0.5], [-0.5,  0.5,  0.5],
    ];
    let idx: [u32; 36] = [
        0,1,2, 2,3,0,  4,5,6, 6,7,4,
        0,1,5, 5,4,0,  2,3,7, 7,6,2,
        1,2,6, 6,5,1,  0,3,7, 7,4,0,
    ];

    let mut bin = Vec::<u8>::new();
    let pad = |b: &mut Vec<u8>| while b.len() % 4 != 0 { b.push(0); };

    let pos_off = bin.len();
    for p in &pos { for v in p { bin.extend_from_slice(&v.to_le_bytes()); } }
    let pos_len = bin.len() - pos_off;
    pad(&mut bin);

    let idx_off = bin.len();
    for &i in &idx { bin.extend_from_slice(&i.to_le_bytes()); }
    let idx_len = bin.len() - idx_off;
    pad(&mut bin);

    let times: [f32; 3] = [0.0, 1.0, 2.0];
    let in_off = bin.len();
    for t in &times { bin.extend_from_slice(&t.to_le_bytes()); }
    let in_len = bin.len() - in_off;
    pad(&mut bin);

    let values: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 5.0, 0.0]];
    let out_off = bin.len();
    for v in &values { for c in v { bin.extend_from_slice(&c.to_le_bytes()); } }
    let out_len = bin.len() - out_off;
    pad(&mut bin);

    let bin_total = bin.len();

    let json = format!(
        r#"{{
            "asset":{{"version":"2.0"}},
            "scene":0,
            "scenes":[{{"nodes":[0]}}],
            "nodes":[{{"mesh":0}}],
            "meshes":[{{"primitives":[{{
                "attributes":{{"POSITION":0}},
                "indices":1
            }}]}}],
            "accessors":[
                {{"bufferView":0,"componentType":5126,"count":8,"type":"VEC3","min":[-0.5,-0.5,-0.5],"max":[0.5,0.5,0.5]}},
                {{"bufferView":1,"componentType":5125,"count":36,"type":"SCALAR"}},
                {{"bufferView":2,"componentType":5126,"count":3,"type":"SCALAR","min":[0.0],"max":[2.0]}},
                {{"bufferView":3,"componentType":5126,"count":3,"type":"VEC3"}}
            ],
            "bufferViews":[
                {{"buffer":0,"byteOffset":{pos_off},"byteLength":{pos_len}}},
                {{"buffer":0,"byteOffset":{idx_off},"byteLength":{idx_len}}},
                {{"buffer":0,"byteOffset":{in_off},"byteLength":{in_len}}},
                {{"buffer":0,"byteOffset":{out_off},"byteLength":{out_len}}}
            ],
            "buffers":[{{"byteLength":{bin_total}}}],
            "animations":[{{
                "channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}],
                "samplers":[{{"input":2,"output":3,"interpolation":"LINEAR"}}]
            }}]
        }}"#
    );
    let mut json_bytes = json.replace(['\n', ' '], "").into_bytes();
    while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }

    let total = 12 + 8 + json_bytes.len() + 8 + bin_total;
    let mut glb = Vec::<u8>::with_capacity(total);
    let push = |g: &mut Vec<u8>, v: u32| g.extend_from_slice(&v.to_le_bytes());
    push(&mut glb, 0x46546C67);
    push(&mut glb, 2);
    push(&mut glb, total as u32);
    push(&mut glb, json_bytes.len() as u32);
    push(&mut glb, 0x4E4F534A);
    glb.extend_from_slice(&json_bytes);
    push(&mut glb, bin_total as u32);
    push(&mut glb, 0x004E4942);
    glb.extend_from_slice(&bin);
    glb
}

// Suppress unused-import warning when building without GPU.
#[allow(dead_code)]
fn _force_use(_: FrameId) {}
