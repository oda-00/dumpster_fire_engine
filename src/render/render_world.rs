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
    SkinningFrame, build_graphics_plans_maximal_with_meshes_vp, build_skin_morph_proto,
    collect_morph_output_buffers, collect_skin_palette_buffers, forge_gltf::Pose,
};
use crate::resource_manager::component::{
    Component, ComponentType, GltfHandle, LightKind, SkyModel,
};
use crate::resource_manager::gltf_driver::{
    LightGpu, LightsUBO, MAX_LIGHTS, allocate_skin_palette_set,
};
use crate::resource_manager::world_manager::World;

// ── View structs ──────────────────────────────────────────────────────────────

pub struct CameraView {
    pub frustum: [Vec4; 6],
    pub vp_matrix: [f32; 16],
}

pub struct LightView {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
    pub kind: LightKind,
}

// ── Renderable (per-frame scratch) ────────────────────────────────────────────

struct Renderable {
    asset_handle: GltfHandle,
    actor_world: [f32; 16],
    aabb_world: ([f32; 3], [f32; 3]),
    anim_index: Option<usize>,
    anim_time: f32,
}

// ── collect_and_submit ────────────────────────────────────────────────────────

pub fn collect_and_submit(
    world: &World,
    asset_cache: &GltfAssetCache,
    renderer: &mut Renderer,
    window_h: WindowHandle,
    vulkan: &VulkanContext,
    camera_views: &[CameraView],
    elapsed: f32,
) -> ForgeResult<Option<vk::Semaphore>> {
    let mut renderables: ThinVec<Renderable> = ThinVec::new();
    let mut lights: ThinVec<LightView> = ThinVec::new();

    // Step 1 — collect renderables and lights from the World.
    for level in world.levels.values() {
        for stage in level.stages.values() {
            for (actor_h, actor) in stage.actors.entries() {
                let world_affine: Affine3A = stage.worlds[actor_h.idx as usize];
                let actor_world: [f32; 16] = Mat4::from(world_affine).to_cols_array();

                for sub_entity in actor.sub_entities.iter().flatten() {
                    let (visible, mesh_opt) = sub_entity.actor_type.visibility_and_mesh();
                    if !visible {
                        continue;
                    }

                    if let Some(mesh_ref) = mesh_opt
                        && let Some(loaded) = asset_cache.get(mesh_ref.asset)
                    {
                        let aabb_world = transform_aabb(&loaded.rest_aabb, &world_affine);
                        let (anim_index, anim_time) = if let Some(Component::Utility(uc)) =
                            sub_entity.component(ComponentType::Utility)
                        {
                            uc.render
                                .as_ref()
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

                    // Collect lights from Utility sub-entities.
                    if let Some(Component::Utility(uc)) =
                        sub_entity.component(ComponentType::Utility)
                        && let Some(ld) = &uc.light
                    {
                        let pos = world_affine.translation;
                        lights.push(LightView {
                            position: [pos.x, pos.y, pos.z],
                            color: ld.color,
                            // Phase 1 note: LightKind now carries non-Copy data
                            // (Polygon.vertices is a ThinVec) — explicit clone.
                            intensity: ld.intensity,
                            range: ld.range,
                            kind: ld.kind.clone(),
                        });
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
        let Some(loaded) = asset_cache.get(r.asset_handle) else {
            continue;
        };

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
        let morph_buffers: ThinVec<(u32, u32, vk::Buffer)>;
        let palette_buffers: ThinVec<(u32, vk::Buffer)>;

        let proto_id = ProtoId::new(100 + r_idx as i64);
        if let Some(cp) = build_skin_morph_proto(&loaded.asset, &pose, proto_id, 0) {
            match renderer.build_compute_factory_async(window_h, cp) {
                Ok((handle, sem)) => {
                    compute_signal = Some(sem);
                    let factory = renderer
                        .window(window_h)
                        .and_then(|w| w.factory_master.get(handle));
                    match factory {
                        Some(f) => {
                            morph_buffers = collect_morph_output_buffers(&loaded.asset, f);
                            palette_buffers = collect_skin_palette_buffers(&loaded.asset, f);
                        }
                        None => {
                            morph_buffers = ThinVec::new();
                            palette_buffers = ThinVec::new();
                        }
                    }
                }
                Err(e) => {
                    eprintln!("render_world: compute dispatch: {e:?}");
                    morph_buffers = ThinVec::new();
                    palette_buffers = ThinVec::new();
                }
            }
        } else {
            morph_buffers = ThinVec::new();
            palette_buffers = ThinVec::new();
        }

        // Build skin palette descriptor sets for this renderable.
        let mut skinning = SkinningFrame::default();
        for (mi, mesh) in loaded.asset.meshes.iter().enumerate() {
            for pi in 0..mesh.primitives.len() {
                if let Some(svb) = loaded.skin_vb(mi, pi) {
                    skinning
                        .skin_vertex_buffers
                        .push((mi as u32, pi as u32, svb.buffer.handle));
                }
            }
        }
        for (node_idx, node) in loaded.asset.nodes.iter().enumerate() {
            let Some(skin_idx) = node.skin else { continue };
            let Some(&(_, buf)) = palette_buffers.iter().find(|(idx, _)| *idx == skin_idx) else {
                continue;
            };
            let range = (loaded.asset.skins[skin_idx as usize].joints.len() as vk::DeviceSize) * 64;
            match allocate_skin_palette_set(
                &loaded.device,
                loaded.skin_pool,
                loaded.skin_set_layout,
                buf,
                range.max(64),
            ) {
                Ok(set) => skinning.palette_sets_by_node.push((node_idx as u32, set)),
                Err(e) => eprintln!("render_world: skin palette alloc: {e:?}"),
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

    // Step 6.5 — RT upkeep: gather TLAS instances from the draw plans
    // (BLAS address + model→world transform), pack the lights UBO, and let
    // the window rebuild its TLAS / upload lights. No-op without RT support.
    if vulkan.has_ray_tracing {
        let mut rt_instances: ThinVec<vk::AccelerationStructureInstanceKHR> = ThinVec::new();
        for plan in &all_plans {
            let Some(mesh) = &plan.mesh else { continue };
            if mesh.blas_addr == 0 {
                continue;
            }
            // Column-major model→world → VkTransformMatrixKHR (row-major 3×4).
            let m = &plan.mvp;
            let transform = vk::TransformMatrixKHR {
                matrix: [
                    m[0], m[4], m[8], m[12], //
                    m[1], m[5], m[9], m[13], //
                    m[2], m[6], m[10], m[14],
                ],
            };
            rt_instances.push(vk::AccelerationStructureInstanceKHR {
                transform,
                instance_custom_index_and_mask: vk::Packed24_8::new(
                    rt_instances.len() as u32,
                    0xFF,
                ),
                instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                    0,
                    vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE.as_raw() as u8,
                ),
                acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                    device_handle: mesh.blas_addr,
                },
            });
        }
        let lights_ubo = pack_lights_ubo(&lights);
        if let Some(window) = renderer.window_mut(window_h)
            && let Err(e) = window.ensure_rt_frame(vulkan, &lights_ubo, &rt_instances)
        {
            eprintln!("render_world: ensure_rt_frame: {e:?}");
        }
    }

    // Step 7 — submit combined graphics proto.
    let mut proto = Proto::<GraphicsTag>::new(ProtoId::new(2), "render_world");
    for plan in all_plans {
        proto.push_call(plan);
    }
    renderer.build_graphics_factory(window_h, proto);

    Ok(compute_signal)
}

// ── Culling helpers ───────────────────────────────────────────────────────────

pub fn extract_frustum_planes(vp: &[f32; 16]) -> [Vec4; 6] {
    let m = Mat4::from_cols_array(vp);
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
    cameras
        .iter()
        .any(|c| aabb_inside_frustum(aabb, &c.frustum))
}

fn light_overlaps_aabb(light: &LightView, aabb: &([f32; 3], [f32; 3])) -> bool {
    match &light.kind {
        // Infinite / scene-wide kinds always overlap.
        LightKind::Directional { .. }
        | LightKind::Sun { .. }
        | LightKind::Environment { .. }
        | LightKind::AnalyticSky { .. }
        | LightKind::Ambient
        | LightKind::Mesh { .. } => true,

        // Positional kinds: cull by sphere of `range` around `position`.
        // `range == 0` (or `< 0`) is treated as unbounded per glTF KHR_lights_punctual.
        LightKind::Point
        | LightKind::Spot { .. }
        | LightKind::Sphere { .. }
        | LightKind::Disk { .. }
        | LightKind::Rectangle { .. }
        | LightKind::Polygon { .. }
        | LightKind::Linear { .. }
        | LightKind::Tube { .. }
        | LightKind::Volumetric { .. }
        | LightKind::VolumeBox { .. }
        | LightKind::VolumeCone { .. }
        | LightKind::VolumeCylinder { .. }
        | LightKind::VolumeMesh { .. }
        | LightKind::Ies { .. } => {
            if light.range <= 0.0 {
                return true;
            }
            let (mn, mx) = aabb;
            let mut dist_sq = 0.0f32;
            for i in 0..3 {
                let lp = light.position[i];
                if lp < mn[i] {
                    let d = lp - mn[i];
                    dist_sq += d * d;
                } else if lp > mx[i] {
                    let d = lp - mx[i];
                    dist_sq += d * d;
                }
            }
            dist_sq <= light.range * light.range
        }
    }
}

fn lit_by_any_light(aabb: &([f32; 3], [f32; 3]), lights: &[LightView]) -> bool {
    lights.iter().any(|l| light_overlaps_aabb(l, aabb))
}

// ─── Lights UBO packer ───────────────────────────────────────────────────────
//
// `pack_lights_ubo` is the host side that produces the GPU `LightsUBO` from
// the per-frame `LightView` collection. Every variant pre-computes the values
// the shader would otherwise have to derive per fragment / per hit:
//   • inv_range² (1 / range²) so the shader does one fma instead of a sqrt+divide.
//   • cos_outer / cos_inner for Spot (avoids two cosines per shaded fragment).
//   • normalised direction vectors.
//
// Variant encoding mirrors the table in the plan; the shader dispatch reads
// data[] by tag without recomputing geometry.

#[inline]
fn inv_range_sq(range: f32) -> f32 {
    // Per glTF KHR_lights_punctual: range == 0 means unbounded; encoded as 0
    // (the shader checks `inv_r2 == 0` for the unbounded branch).
    if range > 0.0 {
        1.0 / (range * range)
    } else {
        0.0
    }
}

#[inline]
fn norm3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1.0e-6 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        v
    }
}

#[inline]
fn vec4_xyz_w(xyz: [f32; 3], w: f32) -> [f32; 4] {
    [xyz[0], xyz[1], xyz[2], w]
}

/// Build the GPU-ready uniform-buffer payload from a slice of `LightView`s.
///
/// Truncates to `MAX_LIGHTS`; the remainder is silently dropped. Header
/// `env_idx` is set to the first Environment light's hdri handle (if any),
/// and `sky_present` is `1` when any AnalyticSky light is in the list — both
/// shortcuts the RT miss shader uses to skip a linear scan.
pub fn pack_lights_ubo(lights: &[LightView]) -> LightsUBO {
    let mut ubo = LightsUBO {
        count: (lights.len().min(MAX_LIGHTS)) as u32,
        env_idx: u32::MAX,
        sky_present: 0,
        ..LightsUBO::default()
    };

    for (i, lv) in lights.iter().take(MAX_LIGHTS).enumerate() {
        let pos = lv.position;
        let ir2 = inv_range_sq(lv.range);
        let mut gpu = LightGpu::default();
        let mut flags: u32 = 0;
        gpu.color_intensity = [lv.color[0], lv.color[1], lv.color[2], lv.intensity];
        gpu.kind = lv.kind.tag();

        match &lv.kind {
            // 0 Point — d[0] = (pos.xyz, inv_range²)
            LightKind::Point => {
                gpu.data[0] = vec4_xyz_w(pos, ir2);
            }
            // 1 Spot — d[0] = (pos.xyz, inv_range²); d[1] = (dir.xyz, cos_outer);
            //          d[2].x = cos_inner
            LightKind::Spot {
                cone_inner,
                cone_outer,
                direction,
            } => {
                let d = norm3(*direction);
                gpu.data[0] = vec4_xyz_w(pos, ir2);
                gpu.data[1] = vec4_xyz_w(d, cone_outer.cos());
                gpu.data[2] = [cone_inner.cos(), 0.0, 0.0, 0.0];
            }
            // 2 Directional — d[0] = (dir.xyz, 0)
            LightKind::Directional { direction } => {
                let d = norm3(*direction);
                gpu.data[0] = vec4_xyz_w(d, 0.0);
            }
            // 3 Sun — d[0] = (dir.xyz, angular_radius_rad)
            LightKind::Sun {
                direction,
                angular_radius,
            } => {
                let d = norm3(*direction);
                gpu.data[0] = vec4_xyz_w(d, *angular_radius);
            }
            // 4 Sphere — d[0] = (center.xyz, radius); d[1].x = inv_range²
            LightKind::Sphere { radius } => {
                gpu.data[0] = vec4_xyz_w(pos, *radius);
                gpu.data[1] = [ir2, 0.0, 0.0, 0.0];
            }
            // 5 Disk — d[0] = (center.xyz, radius); d[1] = (normal.xyz, inv_range²)
            LightKind::Disk {
                normal,
                radius,
                two_sided,
            } => {
                let n = norm3(*normal);
                gpu.data[0] = vec4_xyz_w(pos, *radius);
                gpu.data[1] = vec4_xyz_w(n, ir2);
                if *two_sided {
                    flags |= 1;
                }
            }
            // 6 Rectangle — d[0]=(center.xyz, w); d[1]=(tangent.xyz, h);
            //               d[2]=(bitangent.xyz, inv_range²); d[3]=(normal.xyz, 0)
            LightKind::Rectangle {
                normal,
                tangent,
                size,
                two_sided,
            } => {
                let n = norm3(*normal);
                let t = norm3(*tangent);
                // bitangent = normal × tangent (right-handed)
                let b = norm3([
                    n[1] * t[2] - n[2] * t[1],
                    n[2] * t[0] - n[0] * t[2],
                    n[0] * t[1] - n[1] * t[0],
                ]);
                gpu.data[0] = vec4_xyz_w(pos, size[0]);
                gpu.data[1] = vec4_xyz_w(t, size[1]);
                gpu.data[2] = vec4_xyz_w(b, ir2);
                gpu.data[3] = vec4_xyz_w(n, 0.0);
                if *two_sided {
                    flags |= 1;
                }
            }
            // 7 Polygon — d[0]=(center.xyz, vertex_count_f);
            //             d[1]=(tangent.xyz, side_ssbo_idx_f);
            //             d[2]=(bitangent.xyz, inv_range²);
            //             d[3]=(normal.xyz, 0);
            //             d[4]=(v0.xy, v1.xy); d[5]=(v2.xy, v3.xy)
            // Polygons with ≤4 verts are fully inline; ≥5 verts reference
            // the polygon-verts SSBO (Phase 5 wires the side buffer; for
            // now we encode side_ssbo_idx_f = u32::MAX as f32, meaning
            // "no side buffer — only inline verts are valid").
            LightKind::Polygon {
                normal,
                tangent,
                vertices,
                two_sided,
            } => {
                let n = norm3(*normal);
                let t = norm3(*tangent);
                let b = norm3([
                    n[1] * t[2] - n[2] * t[1],
                    n[2] * t[0] - n[0] * t[2],
                    n[0] * t[1] - n[1] * t[0],
                ]);
                let vc = vertices.len();
                gpu.data[0] = vec4_xyz_w(pos, vc as f32);
                gpu.data[1] = vec4_xyz_w(t, f32::from_bits(u32::MAX));
                gpu.data[2] = vec4_xyz_w(b, ir2);
                gpu.data[3] = vec4_xyz_w(n, 0.0);
                let g = |i: usize| -> [f32; 2] { vertices.get(i).copied().unwrap_or([0.0, 0.0]) };
                let v0 = g(0);
                let v1 = g(1);
                let v2 = g(2);
                let v3 = g(3);
                gpu.data[4] = [v0[0], v0[1], v1[0], v1[1]];
                gpu.data[5] = [v2[0], v2[1], v3[0], v3[1]];
                if *two_sided {
                    flags |= 1;
                }
            }
            // 8 Linear — d[0]=(pt_a.xyz, radius); d[1]=(pt_b.xyz, inv_range²)
            LightKind::Linear { point_b, radius } => {
                gpu.data[0] = vec4_xyz_w(pos, *radius);
                gpu.data[1] = vec4_xyz_w(*point_b, ir2);
            }
            // 9 Tube — d[0]=(pt_a.xyz, radius); d[1]=(pt_b.xyz, inv_range²);
            //          d[2]=(capped_flag, 0, 0, 0); flags bit1 mirrors capped.
            LightKind::Tube {
                point_b,
                radius,
                capped,
            } => {
                gpu.data[0] = vec4_xyz_w(pos, *radius);
                gpu.data[1] = vec4_xyz_w(*point_b, ir2);
                gpu.data[2] = [if *capped { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0];
                if *capped {
                    flags |= 2;
                }
            }
            // 10 Volumetric — d[0]=(center.xyz, radius); d[1]=(ext, g, 0, 0)
            LightKind::Volumetric {
                radius,
                extinction,
                anisotropy_g,
            } => {
                gpu.data[0] = vec4_xyz_w(pos, *radius);
                gpu.data[1] = [*extinction, *anisotropy_g, 0.0, 0.0];
            }
            // 11 VolumeBox — d[0]=(center.xyz, 0); d[1]=(half_ext.xyz, ext);
            //                d[2]=(g, 0, 0, 0)
            LightKind::VolumeBox {
                half_extents,
                extinction,
                anisotropy_g,
            } => {
                gpu.data[0] = vec4_xyz_w(pos, 0.0);
                gpu.data[1] = vec4_xyz_w(*half_extents, *extinction);
                gpu.data[2] = [*anisotropy_g, 0.0, 0.0, 0.0];
            }
            // 12 VolumeCone — d[0]=(apex.xyz, half_angle); d[1]=(dir.xyz, height);
            //                 d[2]=(ext, g, 0, 0)
            LightKind::VolumeCone {
                direction,
                half_angle,
                height,
                extinction,
                anisotropy_g,
            } => {
                let d = norm3(*direction);
                gpu.data[0] = vec4_xyz_w(pos, *half_angle);
                gpu.data[1] = vec4_xyz_w(d, *height);
                gpu.data[2] = [*extinction, *anisotropy_g, 0.0, 0.0];
            }
            // 13 VolumeCylinder — d[0]=(base.xyz, radius); d[1]=(dir.xyz, height);
            //                     d[2]=(ext, g, 0, 0)
            LightKind::VolumeCylinder {
                direction,
                height,
                radius,
                extinction,
                anisotropy_g,
            } => {
                let d = norm3(*direction);
                gpu.data[0] = vec4_xyz_w(pos, *radius);
                gpu.data[1] = vec4_xyz_w(d, *height);
                gpu.data[2] = [*extinction, *anisotropy_g, 0.0, 0.0];
            }
            // 14 VolumeMesh — d[0]=(mesh_record_idx_f, density_tex_idx_f, 0, 0);
            //                 d[1]=(ext, g, 0, 0);
            //                 d[2]=(aabb_min.xyz, 0); d[3]=(aabb_max.xyz, 0)
            // mesh_record_idx is resolved during Phase 5 wiring; here we
            // encode the ActorHandle's slot index as a placeholder.
            LightKind::VolumeMesh {
                mesh_actor,
                density_tex,
                extinction,
                anisotropy_g,
            } => {
                let mesh_idx_f = (mesh_actor.idx as f32).to_bits() as f32;
                let dens_idx_f = density_tex.map(|h| h.0).unwrap_or(u32::MAX) as f32;
                gpu.data[0] = [mesh_idx_f, dens_idx_f, 0.0, 0.0];
                gpu.data[1] = [*extinction, *anisotropy_g, 0.0, 0.0];
                // aabb min/max filled by render_world from the actor's world AABB
                // when available; left zero here (Phase 5 wires this).
            }
            // 15 Ies — d[0]=(pos.xyz, ies_profile_idx_f); d[1]=(dir.xyz, 0)
            LightKind::Ies { direction, profile } => {
                let d = norm3(*direction);
                gpu.data[0] = vec4_xyz_w(pos, profile.0 as f32);
                gpu.data[1] = vec4_xyz_w(d, 0.0);
            }
            // 16 Mesh — d[0]=(mesh_record_idx_f, 0, 0, 0); d[1]=(aabb_min.xyz, 0);
            //           d[2]=(aabb_max.xyz, 0); d[3]=(emissive_cdf_addr_lo_f,
            //                                          emissive_cdf_addr_hi_f,
            //                                          tri_count_f, 0)
            // Phase 5 fills the addresses and tri count; Phase 1 leaves them 0.
            LightKind::Mesh { mesh_actor } => {
                gpu.data[0] = [mesh_actor.idx as f32, 0.0, 0.0, 0.0];
            }
            // 17 Environment — d[0]=(hdri_idx_f, rotation, intensity_scale, 0)
            LightKind::Environment {
                hdri,
                rotation_rad,
                intensity_scale,
            } => {
                gpu.data[0] = [hdri.0 as f32, *rotation_rad, *intensity_scale, 0.0];
                ubo.env_idx = hdri.0;
            }
            // 18 AnalyticSky — d[0]=(sun_dir.xyz, turbidity);
            //                  d[1]=(ground.xyz, model_id_f)
            LightKind::AnalyticSky {
                sun_direction,
                turbidity,
                ground_albedo,
                model,
            } => {
                let s = norm3(*sun_direction);
                gpu.data[0] = vec4_xyz_w(s, *turbidity);
                gpu.data[1] = vec4_xyz_w(*ground_albedo, (*model as u8) as f32);
                ubo.sky_present = 1;
                // env_idx remains untouched — Environment and AnalyticSky are
                // independent of one another in the miss shader.
                let _ = SkyModel::HosekWilkie; // ensure use stays in scope for the enum import
            }
            // 19 Ambient — data slots unused; intensity / color carries everything
            LightKind::Ambient => { /* nothing to pack */ }
        }

        gpu.flags = flags;
        ubo.lights[i] = gpu;
    }

    ubo
}

// Pure-math tests; no GPU device, Vulkan instance, or window required.
#[cfg(test)]
mod lights_ubo_tests {
    use super::*;
    use crate::resource_manager::component::{HdriHandle, IesHandle, LightKind, SkyModel};

    fn lv(kind: LightKind) -> LightView {
        LightView {
            position: [1.0, 2.0, 3.0],
            color: [1.0, 1.0, 1.0],
            intensity: 100.0,
            range: 10.0,
            kind,
        }
    }

    #[test]
    fn pack_point_basic() {
        let u = pack_lights_ubo(&[lv(LightKind::Point)]);
        assert_eq!(u.count, 1);
        assert_eq!(u.lights[0].kind, 0);
        assert_eq!(u.lights[0].data[0][0], 1.0);
        assert_eq!(u.lights[0].data[0][3], 1.0 / 100.0);
        assert_eq!(u.lights[0].color_intensity, [1.0, 1.0, 1.0, 100.0]);
    }

    #[test]
    fn pack_spot_cosines() {
        let outer = 0.5_f32;
        let inner = 0.3_f32;
        let u = pack_lights_ubo(&[lv(LightKind::Spot {
            cone_inner: inner,
            cone_outer: outer,
            direction: [0.0, -1.0, 0.0],
        })]);
        assert_eq!(u.lights[0].kind, 1);
        assert!((u.lights[0].data[1][3] - outer.cos()).abs() < 1.0e-6);
        assert!((u.lights[0].data[2][0] - inner.cos()).abs() < 1.0e-6);
    }

    #[test]
    fn pack_directional_unbounded_range_encoded_zero() {
        let mut l = lv(LightKind::Directional {
            direction: [0.0, -1.0, 0.0],
        });
        l.range = 0.0;
        let u = pack_lights_ubo(&[l]);
        assert_eq!(u.lights[0].kind, 2);
        // direction is the only field; range is irrelevant for directional.
        assert!((u.lights[0].data[0][1] - -1.0).abs() < 1.0e-6);
    }

    #[test]
    fn pack_environment_header_shortcut() {
        let u = pack_lights_ubo(&[lv(LightKind::Environment {
            hdri: HdriHandle(7),
            rotation_rad: 1.5,
            intensity_scale: 2.0,
        })]);
        assert_eq!(u.env_idx, 7);
        assert_eq!(u.lights[0].kind, 17);
    }

    #[test]
    fn pack_analytic_sky_sets_sky_present() {
        let u = pack_lights_ubo(&[lv(LightKind::AnalyticSky {
            sun_direction: [0.0, -1.0, 0.0],
            turbidity: 3.0,
            ground_albedo: [0.3, 0.3, 0.3],
            model: SkyModel::HosekWilkie,
        })]);
        assert_eq!(u.sky_present, 1);
        assert_eq!(u.lights[0].kind, 18);
        assert!((u.lights[0].data[0][3] - 3.0).abs() < 1.0e-6);
    }

    #[test]
    fn pack_truncates_above_max_lights() {
        let mut v = Vec::new();
        for _ in 0..(MAX_LIGHTS + 5) {
            v.push(lv(LightKind::Point));
        }
        let u = pack_lights_ubo(&v);
        assert_eq!(u.count as usize, MAX_LIGHTS);
    }

    #[test]
    fn pack_ies_passes_handle_through() {
        let u = pack_lights_ubo(&[lv(LightKind::Ies {
            direction: [0.0, -1.0, 0.0],
            profile: IesHandle(42),
        })]);
        assert_eq!(u.lights[0].kind, 15);
        assert_eq!(u.lights[0].data[0][3] as u32, 42);
    }

    #[test]
    fn pack_two_sided_flag() {
        let u = pack_lights_ubo(&[lv(LightKind::Disk {
            normal: [0.0, 1.0, 0.0],
            radius: 1.0,
            two_sided: true,
        })]);
        assert_eq!(u.lights[0].kind, 5);
        assert_eq!(u.lights[0].flags & 1, 1);
    }

    #[test]
    fn pack_ambient_writes_only_color() {
        let u = pack_lights_ubo(&[lv(LightKind::Ambient)]);
        assert_eq!(u.lights[0].kind, 19);
        // Data slots stay zero; color/intensity is the carrier.
        for slot in u.lights[0].data.iter() {
            for v in slot.iter() {
                assert_eq!(*v, 0.0);
            }
        }
        assert_eq!(u.lights[0].color_intensity[3], 100.0);
    }
}
