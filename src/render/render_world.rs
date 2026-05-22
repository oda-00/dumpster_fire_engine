//! World-to-render bridge.
//!
//! `collect_and_submit` walks the World, culls by camera frustum and light
//! range, samples animations, dispatches skinning compute, and submits a
//! combined graphics plan.

use ash::vk;
use glam::{Affine3A, Mat4, Vec4};
use thin_vec::ThinVec;

use crate::forge_master::master::ForgeResult;
use crate::render::factory_master::proto::{GraphicsTag, Proto, ProtoId};
use crate::render::gltf_assets::{GltfAssetCache, transform_aabb};
use crate::render::vulkan::VulkanContext;
use crate::render::{Renderer, WindowHandle};
use crate::resource_manager::asset_manager::{
    SkinningFrame,
    build_graphics_plans_maximal_with_meshes_vp,
    build_skin_morph_proto,
    collect_morph_output_buffers,
    collect_skin_palette_buffers,
    forge_gltf::Pose,
};
use crate::resource_manager::component::{Component, ComponentType, GltfHandle, LightKind};
use crate::resource_manager::gltf_driver::allocate_skin_palette_set;
use crate::resource_manager::world_manager::World;

// ── View structs ──────────────────────────────────────────────────────────────

pub struct CameraView {
    pub frustum:   [Vec4; 6],
    pub vp_matrix: [f32; 16],
}

pub struct LightView {
    pub position:  [f32; 3],
    pub color:     [f32; 3],
    pub intensity: f32,
    pub range:     f32,
    pub kind:      LightKind,
}

// ── Renderable (per-frame scratch) ────────────────────────────────────────────

struct Renderable {
    asset_handle: GltfHandle,
    actor_world:  [f32; 16],
    aabb_world:   ([f32; 3], [f32; 3]),
    anim_index:   Option<usize>,
    anim_time:    f32,
}

// ── collect_and_submit ────────────────────────────────────────────────────────

pub fn collect_and_submit(
    world:        &World,
    asset_cache:  &GltfAssetCache,
    renderer:     &mut Renderer,
    window_h:     WindowHandle,
    vulkan:       &VulkanContext,
    camera_views: &[CameraView],
    elapsed:      f32,
) -> ForgeResult<Option<vk::Semaphore>> {
    let mut renderables: ThinVec<Renderable> = ThinVec::new();
    let mut lights:      ThinVec<LightView>  = ThinVec::new();

    // Step 1 — collect renderables and lights from the World.
    for level in world.levels.values() {
        for stage in level.stages.values() {
            for (actor_h, actor) in stage.actors.entries() {
                let world_affine: Affine3A = stage.worlds[actor_h.idx as usize];
                let actor_world: [f32; 16] = Mat4::from(world_affine).to_cols_array();

                for sub_entity in actor.sub_entities.iter().flatten() {
                    let (visible, mesh_opt) = sub_entity.actor_type.visibility_and_mesh();
                    if !visible { continue; }

                    if let Some(mesh_ref) = mesh_opt {
                        if let Some(loaded) = asset_cache.get(mesh_ref.asset) {
                            let aabb_world = transform_aabb(&loaded.rest_aabb, &world_affine);
                            let (anim_index, anim_time) =
                                if let Some(Component::Utility(uc)) =
                                    sub_entity.component(ComponentType::Utility)
                                {
                                    uc.render.as_ref()
                                        .map(|rs| (rs.anim_index, rs.anim_time))
                                        .unwrap_or((None, 0.0))
                                } else {
                                    (None, 0.0)
                                };
                            renderables.push(Renderable {
                                asset_handle: mesh_ref.asset,
                                actor_world,
                                aabb_world,
                                anim_index,
                                anim_time,
                            });
                        }
                    }

                    // Collect lights from Utility sub-entities.
                    if let Some(Component::Utility(uc)) =
                        sub_entity.component(ComponentType::Utility)
                    {
                        if let Some(ld) = &uc.light {
                            let pos = world_affine.translation;
                            lights.push(LightView {
                                position:  [pos.x, pos.y, pos.z],
                                color:     ld.color,
                                intensity: ld.intensity,
                                range:     ld.range,
                                kind:      ld.kind,
                            });
                        }
                    }
                }
            }
        }
    }

    // Step 2 — visibility cull.
    renderables.retain(|r| {
        (camera_views.is_empty() || visible_to_any_camera(&r.aabb_world, camera_views))
            && (lights.is_empty() || lit_by_any_light(&r.aabb_world, &lights))
    });

    if renderables.is_empty() {
        return Ok(None);
    }

    // Step 3 — reset skin pools before any per-frame allocations.
    asset_cache.reset_all_skin_pools();

    // Wait for previous frame before resetting.
    if let Err(e) = renderer.wait_for_last_submission(window_h) {
        eprintln!("render_world: wait_for_last_submission: {e:?}");
    }

    let mut compute_signal: Option<vk::Semaphore> = None;
    let mut all_plans: ThinVec<crate::forge_master::frame::GraphicsFramePlan> = ThinVec::new();

    // Step 4-6 — per-renderable: animate, compute, build plans.
    for (r_idx, r) in renderables.iter().enumerate() {
        let Some(loaded) = asset_cache.get(r.asset_handle) else { continue };

        // Sample animation or use rest pose.
        let mut pose = Pose::rest(&loaded.asset);
        if let Some(anim_idx) = r.anim_index {
            if let Some(anim) = loaded.asset.animations.get(anim_idx) {
                let dur = anim.duration().max(1e-3);
                pose.sample(&loaded.asset, anim, (elapsed - r.anim_time).rem_euclid(dur));
            }
        } else if let Some(anim) = loaded.asset.animations.first() {
            let dur = anim.duration().max(1e-3);
            pose.sample(&loaded.asset, anim, elapsed.rem_euclid(dur));
        }

        // Async compute: skin palette + morph blend.
        let morph_buffers:   ThinVec<(u32, u32, vk::Buffer)>;
        let palette_buffers: ThinVec<(u32, vk::Buffer)>;

        let proto_id = ProtoId::new(100 + r_idx as i64);
        if let Some(cp) = build_skin_morph_proto(&loaded.asset, &pose, proto_id, 0) {
            match renderer.build_compute_factory_async(window_h, cp) {
                Ok((handle, sem)) => {
                    compute_signal = Some(sem);
                    let factory = renderer.window(window_h)
                        .and_then(|w| w.factory_master.get(handle));
                    match factory {
                        Some(f) => {
                            morph_buffers   = collect_morph_output_buffers(&loaded.asset, f);
                            palette_buffers = collect_skin_palette_buffers(&loaded.asset, f);
                        }
                        None => {
                            morph_buffers   = ThinVec::new();
                            palette_buffers = ThinVec::new();
                        }
                    }
                }
                Err(e) => {
                    eprintln!("render_world: compute dispatch: {e:?}");
                    morph_buffers   = ThinVec::new();
                    palette_buffers = ThinVec::new();
                }
            }
        } else {
            morph_buffers   = ThinVec::new();
            palette_buffers = ThinVec::new();
        }

        // Build skin palette descriptor sets for this renderable.
        let mut skinning = SkinningFrame::default();
        for (mi, mesh) in loaded.asset.meshes.iter().enumerate() {
            for pi in 0..mesh.primitives.len() {
                if let Some(svb) = loaded.skin_vb(mi, pi) {
                    skinning.skin_vertex_buffers.push((mi as u32, pi as u32, svb.buffer.handle));
                }
            }
        }
        for (node_idx, node) in loaded.asset.nodes.iter().enumerate() {
            let Some(skin_idx) = node.skin else { continue };
            let Some(&(_, buf)) = palette_buffers.iter()
                .find(|(idx, _)| *idx == skin_idx) else { continue };
            let range = (loaded.asset.skins[skin_idx as usize].joints.len() as vk::DeviceSize) * 64;
            match allocate_skin_palette_set(
                &loaded.device,
                loaded.skin_pool,
                loaded.skin_set_layout,
                buf,
                range.max(64),
            ) {
                Ok(set) => skinning.palette_sets_by_node.push((node_idx as u32, set)),
                Err(e)  => eprintln!("render_world: skin palette alloc: {e:?}"),
            }
        }

        let plans = build_graphics_plans_maximal_with_meshes_vp(
            &loaded.asset,
            &pose,
            &loaded.meshes,
            &loaded.material_sets,
            &morph_buffers,
            &skinning,
            &r.actor_world,
            loaded.cache.dummy_material_set(),
            &[],
            loaded.cache.dummy_instance_set(),
        );
        all_plans.extend(plans);
    }

    // Step 7 — submit combined graphics proto.
    let mut proto = Proto::<GraphicsTag>::new(ProtoId::new(2), "render_world");
    for plan in all_plans { proto.push_call(plan); }
    renderer.build_graphics_factory(window_h, proto);

    Ok(compute_signal)
}

// ── Culling helpers ───────────────────────────────────────────────────────────

pub fn extract_frustum_planes(vp: &[f32; 16]) -> [Vec4; 6] {
    let m  = Mat4::from_cols_array(vp);
    let r0 = m.row(0);
    let r1 = m.row(1);
    let r2 = m.row(2);
    let r3 = m.row(3);
    [
        (r3 + r0).normalize(),
        (r3 - r0).normalize(),
        (r3 + r1).normalize(),
        (r3 - r1).normalize(),
        (r3 + r2).normalize(),
        (r3 - r2).normalize(),
    ]
}

fn aabb_inside_frustum(aabb: &([f32; 3], [f32; 3]), planes: &[Vec4; 6]) -> bool {
    let (mn, mx) = aabb;
    for plane in planes {
        let px = if plane.x >= 0.0 { mx[0] } else { mn[0] };
        let py = if plane.y >= 0.0 { mx[1] } else { mn[1] };
        let pz = if plane.z >= 0.0 { mx[2] } else { mn[2] };
        if plane.x * px + plane.y * py + plane.z * pz + plane.w < 0.0 {
            return false;
        }
    }
    true
}

fn visible_to_any_camera(aabb: &([f32; 3], [f32; 3]), cameras: &[CameraView]) -> bool {
    cameras.iter().any(|c| aabb_inside_frustum(aabb, &c.frustum))
}

fn light_overlaps_aabb(light: &LightView, aabb: &([f32; 3], [f32; 3])) -> bool {
    match &light.kind {
        LightKind::Directional { .. } => true,
        LightKind::Point | LightKind::Spot { .. } => {
            let (mn, mx) = aabb;
            let mut dist_sq = 0.0f32;
            for i in 0..3 {
                let lp = light.position[i];
                if lp < mn[i]      { dist_sq += (lp - mn[i]) * (lp - mn[i]); }
                else if lp > mx[i] { dist_sq += (lp - mx[i]) * (lp - mx[i]); }
            }
            dist_sq <= light.range * light.range
        }
    }
}

fn lit_by_any_light(aabb: &([f32; 3], [f32; 3]), lights: &[LightView]) -> bool {
    lights.iter().any(|l| light_overlaps_aabb(l, aabb))
}
