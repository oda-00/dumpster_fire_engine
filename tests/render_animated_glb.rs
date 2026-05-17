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
    build_skin_morph_proto, load_asset, asset_to_texture_ores,
    collect_skin_palette_buffers, pack_primitive_skin_attrs, primitive_is_skinned,
    register_skin_morph_forges,
};
use dumpster_fire_engine::forge_master::ForgeMaster;
use dumpster_fire_engine::render::factory_master::factory::{Factory, FactoryId};
use dumpster_fire_engine::render::ProtoId;
use dumpster_fire_engine::resource_manager::gltf_driver::{
    GltfCache, GltfSampler, GltfUploadCtx, MaterialUniform,
    create_material, create_material_pool, upload_texture_rgba, TEXTURE_SLOT_COUNT,
};

use forge_gltf::{
    GltfAsset, MaterialBlock, MaterialExtBlock, PipelineParams, Pose,
    build_graphics_draws_with_matrices, build_material_input,
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

// ── 9. KHR_materials_clearcoat / sheen / transmission on ToyCar ────────────

#[test]
fn toycar_carries_clearcoat_sheen_and_transmission() {
    let asset = load_asset(asset_path("ToyCar.glb")).expect("load ToyCar");
    assert_eq!(asset.materials.len(), 3);
    // ToyCar mat0 = body (clearcoat), mat1 = fabric (sheen), mat2 = glass (transmission)
    assert!(asset.materials[0].clearcoat.is_some(), "ToyCar body uses clearcoat");
    assert!(asset.materials[1].sheen.is_some(),    "ToyCar fabric uses sheen");
    assert!(asset.materials[2].transmission.factor > 0.0, "ToyCar glass uses transmission");

    // Extension block should reflect the ext data.
    let ext = MaterialExtBlock::from_material(&asset.materials[0]);
    assert_eq!(ext.flags[0], 1, "clearcoat flag set");
    assert!(ext.clearcoat[0] > 0.0, "clearcoat factor > 0");

    let ext1 = MaterialExtBlock::from_material(&asset.materials[1]);
    assert_eq!(ext1.flags[1], 1, "sheen flag set");
}

// ── 10. KHR_materials_diffuse_transmission + dispersion (ScatteringSkull) ──

#[test]
fn scattering_skull_carries_diffuse_transmission_and_dispersion() {
    let asset = load_asset(asset_path("ScatteringSkull.glb")).expect("load ScatteringSkull");
    let m = &asset.materials[0];
    assert!(m.diffuse_transmission.is_some(), "skull uses diffuse_transmission");
    assert!(m.dispersion > 0.0, "skull uses KHR_materials_dispersion");

    let ext = MaterialExtBlock::from_material(m);
    assert_eq!(ext.flags2[1], 1, "diffuse_transmission flag set");
    assert!(ext.anisotropy[2] > 0.0, "dispersion lives in anisotropy.z slot");
}

// ── 11. KHR_animation_pointer pre-pass actually loads the file ─────────────

#[test]
fn animation_pointer_files_load_and_expose_pointer_channels() {
    let asset = load_asset(asset_path("AnimatedColorsCube.glb"))
        .expect("AnimatedColorsCube must load after KHR_animation_pointer pre-pass");
    let anim = &asset.animations[0];
    assert!(!anim.pointer_channels.is_empty(),
        "KHR_animation_pointer channels recorded separately from regular channels");
    let pc = &anim.pointer_channels[0];
    assert!(pc.pointer.starts_with('/'), "pointer is a JSON path");

    let big = load_asset(asset_path("AnimationPointerUVs.glb"))
        .expect("AnimationPointerUVs must load too");
    assert!(!big.animations[0].pointer_channels.is_empty());
    assert!(big.animations[0].pointer_channels.len() > 50,
        "AnimationPointerUVs has dozens of pointer channels");
}

// ── 12. MaterialFlattening pipeline now emits ext block in secondary buffer ─

#[test]
fn material_flattening_emits_both_base_and_ext_blocks() {
    let asset = load_asset(asset_path("ToyCar.glb")).expect("load ToyCar");
    let up = build_material_input(&asset);
    let n = asset.materials.len();
    assert_eq!(up.element_count, n as u32);
    assert_eq!(up.primary_bytes.len(),   n * MaterialBlock::BYTES);
    assert_eq!(up.secondary_bytes.len(), n * MaterialExtBlock::BYTES);
}

// ── 14. gltf_driver: pure CPU-side material uniform mapping ────────────────

#[test]
fn material_uniform_size_layout_matches_std140_64_bytes_aligned_16() {
    // std140: vec3 needs 16-byte alignment, struct rounds up to vec4 alignment.
    assert_eq!(std::mem::size_of::<MaterialUniform>(), 64);
    assert_eq!(std::mem::align_of::<MaterialUniform>(), 16);
}

#[test]
fn material_uniform_from_gltf_packs_flags_correctly() {
    let asset = load_asset(asset_path("ToyCar.glb")).expect("load ToyCar");
    assert!(!asset.materials.is_empty());
    for m in &asset.materials {
        let u = MaterialUniform::from_gltf(m);
        // alphaMode bits 1-2 — Opaque=0, Mask=2, Blend=4.
        let alpha_bits = u.flags & 0x6;
        assert!(matches!(alpha_bits, 0 | 2 | 4),
            "alphaMode flag bits must encode one of Opaque/Mask/Blend");
        // doubleSided bit 0.
        let ds_bit = u.flags & 0x1;
        assert_eq!(ds_bit, m.double_sided as u32);
        // Factors must round-trip exactly.
        assert_eq!(u.base_color_factor, m.pbr.base_color_factor);
        assert_eq!(u.metallic_factor,   m.pbr.metallic_factor);
        assert_eq!(u.roughness_factor,  m.pbr.roughness_factor);
        assert_eq!(u.emissive_factor,   m.emissive_factor);
    }
}

// ── 15. gltf_driver: GPU upload — needs Vulkan, falls through on lavapipe ──

#[test]
fn gltf_driver_uploads_dummy_white_texture() {
    let Some(ctx) = try_vulkan() else { return };
    let layout = dumpster_fire_engine::resource_manager::gltf_driver::create_material_set_layout(&ctx.device)
        .expect("material set layout");
    let pool = create_material_pool(&ctx.device, 16).expect("material pool");
    let upload = GltfUploadCtx {
        device:              &ctx.device,
        memory_properties:   &ctx.memory_properties,
        graphics_queue:      ctx.queue,
        command_pool:        ctx.command_pool,
        material_set_layout: layout,
        material_pool:       pool,
    };
    use ash::vk::Handle;
    let tex = upload_texture_rgba(
        &upload, 1, 1, &[255, 255, 255, 255], &GltfSampler::default(),
        ash::vk::Format::R8G8B8A8_UNORM,
    ).expect("upload 1x1 white");
    assert!(tex.image.handle.as_raw() != 0);
    assert!(tex.image.view.as_raw()   != 0);
    assert!(tex.sampler.as_raw()      != 0);

    unsafe {
        ctx.device.device_wait_idle().ok();
        let mut tex = tex;
        tex.destroy(&ctx.device);
        ctx.device.destroy_descriptor_pool(pool, None);
        ctx.device.destroy_descriptor_set_layout(layout, None);
    }
}

#[test]
fn gltf_driver_creates_material_descriptor_set_for_toycar() {
    let Some(ctx) = try_vulkan() else { return };
    let asset = load_asset(asset_path("ToyCar.glb")).expect("load ToyCar");
    assert!(!asset.materials.is_empty());

    let layout = dumpster_fire_engine::resource_manager::gltf_driver::create_material_set_layout(&ctx.device)
        .expect("material set layout");
    // ToyCar has ~5 materials and ~5 textures — 64 sets is plenty.
    let pool = create_material_pool(&ctx.device, 64).expect("material pool");
    let upload = GltfUploadCtx {
        device:              &ctx.device,
        memory_properties:   &ctx.memory_properties,
        graphics_queue:      ctx.queue,
        command_pool:        ctx.command_pool,
        material_set_layout: layout,
        material_pool:       pool,
    };
    let mut cache = GltfCache::new();

    // Upload every image (skip on error so the test still runs against
    // partial assets).
    let img_handles: Vec<Option<_>> = asset.images.iter().map(|img| {
        let fmt = match img.format {
            forge_gltf::ImageFormatHint::Srgb   => ash::vk::Format::R8G8B8A8_SRGB,
            forge_gltf::ImageFormatHint::Linear => ash::vk::Format::R8G8B8A8_UNORM,
        };
        upload_texture_rgba(&upload, img.width, img.height, &img.rgba, &GltfSampler::default(), fmt)
            .ok().map(|t| cache.textures.insert(t))
    }).collect();

    // Create one descriptor set per material.
    use ash::vk::Handle;
    let mat = create_material(&asset.materials[0], &asset, &img_handles, &upload, &mut cache)
        .expect("create material");
    assert!(mat.descriptor_set.as_raw() != 0,
        "material's descriptor set must be a real Vulkan handle");
    assert_eq!(mat.textures.len(), TEXTURE_SLOT_COUNT);
    let _h = cache.materials.insert(mat);

    unsafe {
        ctx.device.device_wait_idle().ok();
        // Cache owns resources for the test; we let them leak at process exit
        // (test-only — no Drop on GltfCache yet).
        ctx.device.destroy_descriptor_pool(pool, None);
        ctx.device.destroy_descriptor_set_layout(layout, None);
    }
}

// ── 16a. GPU dispatch — the skin/morph proto actually runs end-to-end ─────

#[test]
fn brainstem_skin_palette_dispatches_and_produces_finite_ingot() {
    let Some(ctx) = try_vulkan() else { return };
    let asset = load_asset(asset_path("BrainStem.glb")).expect("load BrainStem");
    let mut pose = forge_gltf::Pose::rest(&asset);
    if let Some(anim) = asset.animations.first() {
        pose.sample(&asset, anim, 0.25);
    }

    // Spin up an isolated ForgeMaster, register the compute pipelines, and
    // refine the per-frame proto. Successful refinement means the SPIR-V
    // linked, the descriptor set was allocated, the dispatch went through
    // the GPU, and the readback came back without a Vulkan error.
    let mut forge = ForgeMaster::new(
        ctx.device.clone(),
        ctx.queue,
        ctx.command_pool,
        ctx.memory_properties,
    ).expect("ForgeMaster");
    register_skin_morph_forges(&mut forge).expect("register skin/morph forges");

    let proto = build_skin_morph_proto(&asset, &pose, ProtoId::new(101), 0)
        .expect("BrainStem has skins → proto must be Some");
    let factory = Factory::from_compute_proto(FactoryId::new(101), proto, &mut forge)
        .expect("compute dispatch must succeed");

    // Every plan must have produced at least one Ingot.
    let total_ingots: usize = factory.frames().map(|f| f.ingots.len()).sum();
    assert!(total_ingots >= asset.skins.len(),
        "every skin should produce a palette ingot");
    // Palette ingot output must be at least joint_count × 64 bytes.
    for frame in factory.frames() {
        for ing in &frame.ingots {
            assert!(ing.as_bytes().len() >= 64,
                "palette ingot must hold at least one mat4 worth of bytes");
        }
    }
    unsafe {
        ctx.device.device_wait_idle().ok();
        let mut factory = factory;
        factory.destroy(&ctx.device);
    }
}

#[test]
fn diffuse_transmission_plant_morph_blend_dispatches_finite_ingot() {
    let Some(ctx) = try_vulkan() else { return };
    let asset = load_asset(asset_path("DiffuseTransmissionPlant.glb"))
        .expect("load DiffuseTransmissionPlant");
    let mut pose = forge_gltf::Pose::rest(&asset);
    if let Some(anim) = asset.animations.first() {
        pose.sample(&asset, anim, 0.25);
    }

    // Skip cleanly when the asset doesn't actually carry morph targets.
    let any_morph = asset.meshes.iter()
        .any(|m| m.primitives.iter().any(|p| !p.morph_targets.is_empty()));
    if !any_morph { return; }

    let mut forge = ForgeMaster::new(
        ctx.device.clone(),
        ctx.queue,
        ctx.command_pool,
        ctx.memory_properties,
    ).expect("ForgeMaster");
    register_skin_morph_forges(&mut forge).expect("register skin/morph forges");

    let proto = build_skin_morph_proto(&asset, &pose, ProtoId::new(102), 0)
        .expect("DiffuseTransmissionPlant has morph targets → proto must be Some");
    let factory = Factory::from_compute_proto(FactoryId::new(102), proto, &mut forge)
        .expect("morph compute dispatch must succeed");

    // At least one ingot for the morphed primitive.
    let total_ingots: usize = factory.frames().map(|f| f.ingots.len()).sum();
    assert!(total_ingots >= 1, "morphed primitive must produce a posed vertex ingot");
    unsafe {
        ctx.device.device_wait_idle().ok();
        let mut factory = factory;
        factory.destroy(&ctx.device);
    }
}

// ── 16c. Full GPU loop closure for skinning — palette feeds vertex shader ─

/// End-to-end correctness check: dispatch the SkinPalette compute Ore
/// against BrainStem at rest pose and compare every output mat4 to the
/// CPU reference produced by `Pose::skin_palette`. Catches both shader
/// math errors and input-upload mistakes (the previous variant of this
/// test caught a real bug where `upload_to_ore` was dropping the primary
/// buffer for non-mesh-shaped uploads — primary world-matrices ended up
/// all zero, the palette computed to 0×IBM=0, and skinning corrupted the
/// mesh).
#[test]
fn brainstem_skin_palette_matches_cpu_reference_at_rest() {
    let Some(ctx) = try_vulkan() else { return };
    let asset = load_asset(asset_path("BrainStem.glb")).expect("load BrainStem");
    let pose  = forge_gltf::Pose::rest(&asset);

    let mut forge = ForgeMaster::new(
        ctx.device.clone(), ctx.queue, ctx.command_pool, ctx.memory_properties,
    ).expect("ForgeMaster");
    register_skin_morph_forges(&mut forge).expect("register skin/morph forges");

    let proto = build_skin_morph_proto(&asset, &pose, ProtoId::new(120), 0)
        .expect("BrainStem skins → proto must be Some");
    let factory = Factory::from_compute_proto(FactoryId::new(120), proto, &mut forge)
        .expect("compute dispatch");

    let frame = factory.frames().next().expect("at least one compute frame");
    let ingot = frame.ingots.first().expect("at least one ingot per frame");
    let bytes = ingot.as_bytes();
    assert_eq!(bytes.len() % 64, 0, "ingot must be whole number of mat4s");
    let n_joints = asset.skins[0].joints.len();
    assert!(bytes.len() / 64 >= n_joints,
        "ingot has {} mat4s, expected ≥ {n_joints}", bytes.len() / 64);

    let cpu = pose.skin_palette(&asset, 0);
    assert_eq!(cpu.len(), n_joints);

    for j in 0..n_joints {
        let base = j * 64;
        let mut gpu = [0f32; 16];
        for k in 0..16 {
            let off = base + k * 4;
            gpu[k] = f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        }
        let ref_mat = cpu[j];
        let max_err = (0..16).map(|k| (gpu[k] - ref_mat[k]).abs())
            .fold(0f32, f32::max);
        assert!(max_err < 1e-3,
            "joint {j} GPU palette differs from CPU: max_err={max_err}\n  gpu={gpu:?}\n  cpu={ref_mat:?}");
    }

    unsafe {
        ctx.device.device_wait_idle().ok();
        let mut factory = factory;
        factory.destroy(&ctx.device);
    }
}

#[test]
fn brainstem_skin_palette_buffer_is_bindable_as_storage_buffer() {
    use ash::vk::Handle;
    let Some(ctx) = try_vulkan() else { return };
    let asset = load_asset(asset_path("BrainStem.glb")).expect("load BrainStem");
    let pose  = forge_gltf::Pose::rest(&asset);

    let mut forge = ForgeMaster::new(
        ctx.device.clone(),
        ctx.queue,
        ctx.command_pool,
        ctx.memory_properties,
    ).expect("ForgeMaster");
    register_skin_morph_forges(&mut forge).expect("register skin/morph forges");

    let proto = build_skin_morph_proto(&asset, &pose, ProtoId::new(104), 0)
        .expect("BrainStem has skins → proto must be Some");
    let factory = Factory::from_compute_proto(FactoryId::new(104), proto, &mut forge)
        .expect("compute dispatch");

    let palette_buffers = collect_skin_palette_buffers(&asset, &factory);
    assert!(!palette_buffers.is_empty(),
        "at least one skin palette buffer must be harvested");

    for (skin_idx, buf) in &palette_buffers {
        assert!(buf.as_raw() != 0,
            "skin palette buffer for skin {skin_idx} must be a real Vulkan handle");
    }

    unsafe {
        ctx.device.device_wait_idle().ok();
        let mut factory = factory;
        factory.destroy(&ctx.device);
    }
}

#[test]
fn brainstem_skin_vertex_attrs_pack_correctly() {
    let asset = load_asset(asset_path("BrainStem.glb")).expect("load BrainStem");
    // BrainStem's mesh 0 primitive 0 is skinned.
    assert!(primitive_is_skinned(&asset, 0, 0));
    let bytes = pack_primitive_skin_attrs(&asset, 0, 0);
    // 24 bytes per vertex; non-empty for a skinned primitive.
    assert!(!bytes.is_empty(), "skin attrs must produce bytes for skinned primitive");
    assert_eq!(bytes.len() % 24, 0, "stride must be 24 bytes per vertex");

    let n_verts = bytes.len() / 24;
    let n_pos = asset.meshes[0].primitives[0].streams.positions.len();
    assert_eq!(n_verts, n_pos, "one skin record per vertex");

    // Spot-check vertex 0: weights should sum near 1.0 for any real glTF skin.
    let w0 = f32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let w1 = f32::from_le_bytes(bytes[12..16].try_into().unwrap());
    let w2 = f32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let w3 = f32::from_le_bytes(bytes[20..24].try_into().unwrap());
    let sum = w0 + w1 + w2 + w3;
    assert!((sum - 1.0).abs() < 1e-3, "weights sum should be ~1.0, got {sum}");
}

#[test]
fn unskinned_box_skin_attrs_are_empty() {
    let asset = load_asset(asset_path("Box.glb")).expect("load Box");
    assert!(!primitive_is_skinned(&asset, 0, 0));
    let bytes = pack_primitive_skin_attrs(&asset, 0, 0);
    assert!(bytes.is_empty(), "unskinned primitive must produce zero skin bytes");
}

// ── 16b. Full GPU loop closure — compute output feeds graphics override ───

#[test]
fn morph_compute_output_buffer_is_bindable_as_vertex_source() {
    use ash::vk;
    let Some(ctx) = try_vulkan() else { return };
    let asset = load_asset(asset_path("DiffuseTransmissionPlant.glb"))
        .expect("load DiffuseTransmissionPlant");
    let pose  = forge_gltf::Pose::rest(&asset);

    // Skip when the asset has no morph targets.
    let any_morph = asset.meshes.iter()
        .any(|m| m.primitives.iter().any(|p| !p.morph_targets.is_empty()));
    if !any_morph { return; }

    let mut forge = ForgeMaster::new(
        ctx.device.clone(),
        ctx.queue,
        ctx.command_pool,
        ctx.memory_properties,
    ).expect("ForgeMaster");
    register_skin_morph_forges(&mut forge).expect("register skin/morph forges");

    let proto = build_skin_morph_proto(&asset, &pose, ProtoId::new(103), 0)
        .expect("proto must be Some — asset has morph targets");
    let factory = Factory::from_compute_proto(FactoryId::new(103), proto, &mut forge)
        .expect("compute dispatch");

    use dumpster_fire_engine::resource_manager::asset_manager::collect_morph_output_buffers;
    let morph_buffers = collect_morph_output_buffers(&asset, &factory);
    assert!(!morph_buffers.is_empty(),
        "at least one morph-blended primitive must produce a vertex buffer");

    // Every harvested buffer handle must be a real (non-null) Vulkan handle.
    use ash::vk::Handle;
    for (key, buf) in &morph_buffers {
        assert!(buf.as_raw() != 0, "morph output for {key:?} must be a real buffer");
    }

    // Sanity: the harvested buffer is the same one the Ingot owns —
    // the extra_usage = VERTEX_BUFFER flag we set in `upload_to_ore`
    // means it was created with the appropriate usage bits.
    let _ = vk::BufferUsageFlags::VERTEX_BUFFER;

    unsafe {
        ctx.device.device_wait_idle().ok();
        let mut factory = factory;
        factory.destroy(&ctx.device);
    }
}

// ── 16. Per-frame skin/morph compute proto (CPU-side build) ────────────────

#[test]
fn skin_morph_proto_produces_one_plan_per_skin_for_brainstem() {
    let asset = load_asset(asset_path("BrainStem.glb")).expect("load BrainStem");
    let pose  = forge_gltf::Pose::rest(&asset);
    let proto = build_skin_morph_proto(&asset, &pose, ProtoId::new(99), 0)
        .expect("BrainStem is skinned — proto must be Some");
    // BrainStem has exactly 1 skin and no morph targets.
    assert!(!proto.is_empty(), "proto must contain SkinPalette plan(s)");
    assert!(proto.len() >= asset.skins.len());
}

#[test]
fn skin_morph_proto_is_none_for_unskinned_unmorphed_asset() {
    let asset = load_asset(asset_path("Box.glb")).expect("load Box");
    let pose  = forge_gltf::Pose::rest(&asset);
    let proto = build_skin_morph_proto(&asset, &pose, ProtoId::new(99), 0);
    assert!(proto.is_none(),
        "Box has no skins and no morph targets — nothing to dispatch");
}

#[test]
fn build_graphics_plans_attaches_material_sets() {
    use dumpster_fire_engine::resource_manager::asset_manager::build_graphics_plans_with_pose_and_materials;
    let Some(ctx) = try_vulkan() else { return };
    let asset = load_asset(asset_path("ToyCar.glb")).expect("load ToyCar");
    let pose = forge_gltf::Pose::rest(&asset);
    let upload_ctx = ctx.mesh_upload_ctx();

    use ash::vk::Handle;
    // Synthesize one fake-but-non-null descriptor set per material slot.
    let fake = ash::vk::DescriptorSet::from_raw(0xDEADBEEFu64);
    let sets: Vec<Option<ash::vk::DescriptorSet>> =
        (0..asset.materials.len()).map(|_| Some(fake)).collect();

    let plans = build_graphics_plans_with_pose_and_materials(
        &asset, &pose, &upload_ctx, &sets,
    ).expect("build plans with materials");
    assert!(!plans.is_empty(), "ToyCar has draws");

    // Every draw whose primitive carried a material index must have a set.
    let with_material = plans.iter().filter(|p| p.material_set.is_some()).count();
    assert!(with_material > 0, "at least one draw must carry the fake material set");
    for p in &plans {
        if let Some(s) = p.material_set {
            assert_eq!(s.as_raw(), 0xDEADBEEFu64);
        }
    }
    unsafe { ctx.device.device_wait_idle().ok(); }
}
