//! Top-level asset container + the loaders that populate it.
//!
//! `GltfAsset::load` (and its slice/bytes counterparts) walks a full glTF
//! document and copies every dispatchable bit of state — scenes, nodes,
//! meshes (with every vertex stream), materials with KHR_* extension data,
//! textures with decoded RGBA pixels, samplers, skins, animations, cameras,
//! lights — into engine-neutral structs.
//!
//! The loader never returns partial state: a single error bubbles up and
//! the caller gets nothing. This keeps downstream pipeline adapters from
//! having to second-guess what's present.

use std::path::Path;

use thin_vec::ThinVec;

use crate::animation::*;
use crate::camera::*;
use crate::error::*;
use crate::light::*;
use crate::material::*;
use crate::mesh::*;
use crate::scene::*;
use crate::skin::*;
use crate::texture::*;

#[derive(Debug, Clone)]
pub struct GltfAsset {
    pub scenes:        ThinVec<Scene>,
    pub default_scene: Option<u32>,
    pub nodes:         ThinVec<Node>,
    pub meshes:        ThinVec<Mesh>,
    pub materials:     ThinVec<Material>,
    pub textures:      ThinVec<Texture>,
    pub images:        ThinVec<Image>,
    pub samplers:      ThinVec<Sampler>,
    pub skins:         ThinVec<Skin>,
    pub animations:    ThinVec<Animation>,
    pub cameras:       ThinVec<Camera>,
    pub lights:        ThinVec<Light>,
}

impl GltfAsset {
    pub fn load(path: impl AsRef<Path>) -> GltfResult<Self> {
        let (doc, buffers, images) = gltf::import(path)?;
        extract_asset(&doc, &buffers, &images)
    }

    pub fn load_slice(bytes: &[u8]) -> GltfResult<Self> {
        let (doc, buffers, images) = gltf::import_slice(bytes)?;
        extract_asset(&doc, &buffers, &images)
    }

    /// Default scene if specified, else the first scene, else `None`.
    pub fn primary_scene(&self) -> Option<&Scene> {
        self.default_scene
            .map(|i| &self.scenes[i as usize])
            .or_else(|| self.scenes.first())
    }

    /// World matrices for every node, composed from the primary scene's roots.
    /// Returns an empty vec if the document has no scenes.
    pub fn world_matrices(&self) -> ThinVec<[f32; 16]> {
        match self.primary_scene() {
            Some(scene) => compose_world_matrices(&scene.roots, &self.nodes),
            None => ThinVec::new(),
        }
    }
}

fn extract_asset(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
    images:  &[gltf::image::Data],
) -> GltfResult<GltfAsset> {
    Ok(GltfAsset {
        scenes:        extract_scenes(doc),
        default_scene: doc.default_scene().map(|s| s.index() as u32),
        nodes:         extract_nodes(doc),
        meshes:        extract_meshes(doc, buffers)?,
        materials:     extract_materials(doc),
        textures:      extract_textures(doc),
        images:        extract_images(doc, images),
        samplers:      extract_samplers(doc),
        skins:         extract_skins(doc, buffers),
        animations:    extract_animations(doc, buffers)?,
        cameras:       extract_cameras(doc),
        lights:        extract_lights(doc),
    })
}

// ── Scenes / nodes ──────────────────────────────────────────────────────────

fn extract_scenes(doc: &gltf::Document) -> ThinVec<Scene> {
    doc.scenes()
        .map(|s| Scene {
            name:  s.name().map(str::to_owned),
            roots: s.nodes().map(|n| n.index() as u32).collect(),
        })
        .collect()
}

fn extract_nodes(doc: &gltf::Document) -> ThinVec<Node> {
    // First pass: per-node data without parents.
    let mut nodes: ThinVec<Node> = doc
        .nodes()
        .map(|n| {
            let (t, r, s) = n.transform().decomposed();
            let local_matrix = compose_trs(t, r, s);
            Node {
                name:        n.name().map(str::to_owned),
                parent:      None,
                children:    n.children().map(|c| c.index() as u32).collect(),
                translation: t,
                rotation:    r,
                scale:       s,
                local_matrix,
                mesh:        n.mesh().map(|m| m.index() as u32),
                camera:      n.camera().map(|c| c.index() as u32),
                skin:        n.skin().map(|s| s.index() as u32),
                light:       n.light().map(|l| l.index() as u32),
                weights:     n.weights().map(|ws| ws.iter().copied().collect()).unwrap_or_default(),
            }
        })
        .collect();

    // Second pass: wire parents from the children we already collected.
    let kids: ThinVec<ThinVec<u32>> = nodes.iter().map(|n| n.children.clone()).collect();
    for (parent_idx, kids) in kids.iter().enumerate() {
        for &child in kids {
            nodes[child as usize].parent = Some(parent_idx as u32);
        }
    }
    nodes
}

// ── Meshes ──────────────────────────────────────────────────────────────────

fn extract_meshes(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> GltfResult<ThinVec<Mesh>> {
    let mut out = ThinVec::with_capacity(doc.meshes().count());
    for mesh in doc.meshes() {
        let mut primitives = ThinVec::new();
        for prim in mesh.primitives() {
            primitives.push(extract_primitive(&prim, buffers)?);
        }
        out.push(Mesh {
            name: mesh.name().map(str::to_owned),
            primitives,
            weights: mesh.weights().map(|ws| ws.iter().copied().collect()).unwrap_or_default(),
        });
    }
    Ok(out)
}

fn extract_primitive(
    prim:    &gltf::Primitive<'_>,
    buffers: &[gltf::buffer::Data],
) -> GltfResult<Primitive> {
    let reader = prim.reader(|buf| Some(&*buffers[buf.index()]));

    let positions: ThinVec<[f32; 3]> = reader
        .read_positions()
        .ok_or(GltfError::NoPositions)?
        .collect();
    let n = positions.len();

    let mut streams = VertexStreams::default();
    streams.positions = positions;

    streams.normals = reader.read_normals()
        .map(|it| it.collect())
        .unwrap_or_else(|| (0..n).map(|_| [0.0_f32, 1.0, 0.0]).collect());

    streams.tangents = reader.read_tangents()
        .map(|it| it.collect())
        .unwrap_or_else(|| (0..n).map(|_| [1.0_f32, 0.0, 0.0, 1.0]).collect());

    // Multi-set UVs (TEXCOORD_n) — pull until None.
    let mut set = 0u32;
    while let Some(tc) = reader.read_tex_coords(set) {
        streams.uv_sets.push(tc.into_f32().collect());
        set += 1;
    }
    if streams.uv_sets.is_empty() {
        streams.uv_sets.push((0..n).map(|_| [0.0_f32, 0.0]).collect());
    }

    set = 0;
    while let Some(c) = reader.read_colors(set) {
        streams.colors.push(c.into_rgba_f32().collect());
        set += 1;
    }

    set = 0;
    while let Some(j) = reader.read_joints(set) {
        streams.joints.push(j.into_u16().collect());
        set += 1;
    }
    set = 0;
    while let Some(w) = reader.read_weights(set) {
        streams.weights.push(w.into_f32().collect());
        set += 1;
    }

    let indices: ThinVec<u32> = match reader.read_indices() {
        Some(it) => it.into_u32().collect(),
        None     => (0..n as u32).collect(),
    };

    // Morph targets — each target shares the parent primitive's vertex count.
    let mut morph_targets = ThinVec::new();
    for morph in reader.read_morph_targets() {
        let (pos, norm, tan) = morph;
        let mut mt = MorphTarget::default();
        if let Some(p) = pos  { mt.positions = p.collect(); }
        if let Some(no) = norm { mt.normals  = no.collect(); }
        if let Some(t) = tan  { mt.tangents  = t.collect(); }
        morph_targets.push(mt);
    }

    let bounds = {
        // Prefer accessor min/max; fall back to a scan.
        let bb = prim.bounding_box();
        Aabb { min: bb.min, max: bb.max }
    };

    Ok(Primitive {
        topology: PrimitiveTopology::from_mode(prim.mode()),
        streams,
        indices,
        material: prim.material().index().map(|i| i as u32),
        morph_targets,
        bounds,
    })
}

// ── Materials ───────────────────────────────────────────────────────────────

fn texture_ref_from(info: gltf::texture::Info<'_>) -> TextureRef {
    let texture = info.texture().index() as u32;
    let tex_coord_set = info.tex_coord();
    let mut out = TextureRef::identity(texture, tex_coord_set);
    if let Some(tx) = info.texture_transform() {
        out.uv_offset   = tx.offset();
        out.uv_rotation = tx.rotation();
        out.uv_scale    = tx.scale();
        if let Some(ts) = tx.tex_coord() {
            out.tex_coord_set = ts;
        }
    }
    out
}

fn extract_materials(doc: &gltf::Document) -> ThinVec<Material> {
    doc.materials()
        .map(|m| {
            let mut out = Material::default();
            out.name = m.name().map(str::to_owned);
            out.alpha_mode = AlphaMode::from(m.alpha_mode());
            out.alpha_cutoff = m.alpha_cutoff().unwrap_or(0.5);
            out.double_sided = m.double_sided();
            out.unlit = m.unlit();
            out.ior = m.ior().unwrap_or(1.5);

            let pbr = m.pbr_metallic_roughness();
            out.pbr.base_color_factor = pbr.base_color_factor();
            out.pbr.metallic_factor = pbr.metallic_factor();
            out.pbr.roughness_factor = pbr.roughness_factor();
            out.pbr.base_color_texture = pbr.base_color_texture().map(texture_ref_from);
            out.pbr.metallic_roughness_texture =
                pbr.metallic_roughness_texture().map(texture_ref_from);

            // KHR_texture_transform is exposed on `gltf::texture::Info` only;
            // NormalTexture / OcclusionTexture don't surface it through the
            // public crate API, so we record the plain reference here.
            if let Some(n) = m.normal_texture() {
                let texture = n.texture().index() as u32;
                let tex_coord_set = n.tex_coord();
                let tr = TextureRef::identity(texture, tex_coord_set);
                out.normal = NormalTexture { texture: Some(tr), scale: n.scale() };
            }

            if let Some(o) = m.occlusion_texture() {
                let texture = o.texture().index() as u32;
                let tex_coord_set = o.tex_coord();
                let tr = TextureRef::identity(texture, tex_coord_set);
                out.occlusion = OcclusionTexture { texture: Some(tr), strength: o.strength() };
            }

            out.emissive_factor = m.emissive_factor();
            out.emissive_strength = m.emissive_strength().unwrap_or(1.0);
            out.emissive_texture = m.emissive_texture().map(texture_ref_from);

            if let Some(t) = m.transmission() {
                out.transmission = Transmission {
                    factor:  t.transmission_factor(),
                    texture: t.transmission_texture().map(texture_ref_from),
                };
            }
            if let Some(v) = m.volume() {
                out.volume = Volume {
                    thickness_factor:    v.thickness_factor(),
                    thickness_texture:   v.thickness_texture().map(texture_ref_from),
                    attenuation_distance:v.attenuation_distance(),
                    attenuation_color:   v.attenuation_color(),
                };
            }

            out
        })
        .collect()
}

// ── Textures / images / samplers ────────────────────────────────────────────

fn extract_textures(doc: &gltf::Document) -> ThinVec<Texture> {
    doc.textures()
        .map(|t| Texture {
            name:    t.name().map(str::to_owned),
            image:   t.source().index() as u32,
            sampler: t.sampler().index().map(|i| i as u32),
        })
        .collect()
}

fn extract_samplers(doc: &gltf::Document) -> ThinVec<Sampler> {
    doc.samplers()
        .map(|s| Sampler {
            name: s.name().map(str::to_owned),
            mag_filter: s.mag_filter().map(mag_filter_from).unwrap_or(MagFilter::Linear),
            min_filter: s.min_filter().map(min_filter_from).unwrap_or(MinFilter::LinearMipmapLinear),
            wrap_s: wrap_from(s.wrap_s()),
            wrap_t: wrap_from(s.wrap_t()),
        })
        .collect()
}

fn extract_images(doc: &gltf::Document, images: &[gltf::image::Data]) -> ThinVec<Image> {
    // Walk every material/texture to flag which texture indices want sRGB —
    // base-colour and emissive maps live in sRGB; everything else is linear.
    let mut srgb_textures: ThinVec<bool> = (0..doc.textures().count())
        .map(|_| false)
        .collect();
    for m in doc.materials() {
        if let Some(t) = m.pbr_metallic_roughness().base_color_texture() {
            srgb_textures[t.texture().index()] = true;
        }
        if let Some(t) = m.emissive_texture() {
            srgb_textures[t.texture().index()] = true;
        }
    }

    // Image format hint = sRGB if any texture pointing to this image is sRGB.
    let n_images = images.len();
    let mut hints: ThinVec<ImageFormatHint> = (0..n_images)
        .map(|_| ImageFormatHint::Linear)
        .collect();
    for t in doc.textures() {
        if srgb_textures[t.index()] {
            hints[t.source().index()] = ImageFormatHint::Srgb;
        }
    }

    images
        .iter()
        .enumerate()
        .map(|(i, data)| {
            let (w, h, rgba) = decode_to_rgba8(data);
            let name = doc.images().nth(i).and_then(|im| im.name().map(str::to_owned));
            Image {
                name,
                width:  w,
                height: h,
                rgba,
                format: hints[i],
            }
        })
        .collect()
}

fn decode_to_rgba8(data: &gltf::image::Data) -> (u32, u32, ThinVec<u8>) {
    use gltf::image::Format as F;
    let w = data.width;
    let h = data.height;
    let n = (w * h) as usize;
    let pixels = &data.pixels;
    // Re-pack everything to tightly-packed RGBA8. 16-bit and float sources
    // get a lossy down-convert here — fine for the engine's TextureOre,
    // which expects 8-bit pixels.
    let mut out: ThinVec<u8> = ThinVec::with_capacity(n * 4);
    out.resize(n * 4, 0);
    match data.format {
        F::R8 => {
            for i in 0..n {
                out[i * 4]     = pixels[i];
                out[i * 4 + 1] = pixels[i];
                out[i * 4 + 2] = pixels[i];
                out[i * 4 + 3] = 255;
            }
        }
        F::R8G8 => {
            for i in 0..n {
                out[i * 4]     = pixels[i * 2];
                out[i * 4 + 1] = pixels[i * 2 + 1];
                out[i * 4 + 2] = 0;
                out[i * 4 + 3] = 255;
            }
        }
        F::R8G8B8 => {
            for i in 0..n {
                out[i * 4]     = pixels[i * 3];
                out[i * 4 + 1] = pixels[i * 3 + 1];
                out[i * 4 + 2] = pixels[i * 3 + 2];
                out[i * 4 + 3] = 255;
            }
        }
        F::R8G8B8A8 => {
            out[..n * 4].copy_from_slice(&pixels[..n * 4]);
        }
        F::R16 => {
            for i in 0..n {
                let v = u16_at(pixels, i * 2);
                let b = (v >> 8) as u8;
                out[i * 4]     = b;
                out[i * 4 + 1] = b;
                out[i * 4 + 2] = b;
                out[i * 4 + 3] = 255;
            }
        }
        F::R16G16 => {
            for i in 0..n {
                out[i * 4]     = (u16_at(pixels, i * 4) >> 8) as u8;
                out[i * 4 + 1] = (u16_at(pixels, i * 4 + 2) >> 8) as u8;
                out[i * 4 + 2] = 0;
                out[i * 4 + 3] = 255;
            }
        }
        F::R16G16B16 => {
            for i in 0..n {
                out[i * 4]     = (u16_at(pixels, i * 6) >> 8) as u8;
                out[i * 4 + 1] = (u16_at(pixels, i * 6 + 2) >> 8) as u8;
                out[i * 4 + 2] = (u16_at(pixels, i * 6 + 4) >> 8) as u8;
                out[i * 4 + 3] = 255;
            }
        }
        F::R16G16B16A16 => {
            for i in 0..n {
                out[i * 4]     = (u16_at(pixels, i * 8) >> 8) as u8;
                out[i * 4 + 1] = (u16_at(pixels, i * 8 + 2) >> 8) as u8;
                out[i * 4 + 2] = (u16_at(pixels, i * 8 + 4) >> 8) as u8;
                out[i * 4 + 3] = (u16_at(pixels, i * 8 + 6) >> 8) as u8;
            }
        }
        F::R32G32B32FLOAT => {
            for i in 0..n {
                out[i * 4]     = f32_to_u8(f32_at(pixels, i * 12));
                out[i * 4 + 1] = f32_to_u8(f32_at(pixels, i * 12 + 4));
                out[i * 4 + 2] = f32_to_u8(f32_at(pixels, i * 12 + 8));
                out[i * 4 + 3] = 255;
            }
        }
        F::R32G32B32A32FLOAT => {
            for i in 0..n {
                out[i * 4]     = f32_to_u8(f32_at(pixels, i * 16));
                out[i * 4 + 1] = f32_to_u8(f32_at(pixels, i * 16 + 4));
                out[i * 4 + 2] = f32_to_u8(f32_at(pixels, i * 16 + 8));
                out[i * 4 + 3] = f32_to_u8(f32_at(pixels, i * 16 + 12));
            }
        }
    }
    (w, h, out)
}

#[inline] fn u16_at(buf: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([buf[i], buf[i + 1]])
}
#[inline] fn f32_at(buf: &[u8], i: usize) -> f32 {
    f32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]])
}
#[inline] fn f32_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

// ── Skins ───────────────────────────────────────────────────────────────────

fn extract_skins(doc: &gltf::Document, buffers: &[gltf::buffer::Data]) -> ThinVec<Skin> {
    doc.skins()
        .map(|s| {
            let reader = s.reader(|buf| Some(&*buffers[buf.index()]));
            let inverse_bind_matrices: ThinVec<[f32; 16]> = reader
                .read_inverse_bind_matrices()
                .map(|it| it.map(flatten_mat4).collect())
                .unwrap_or_default();
            Skin {
                name:    s.name().map(str::to_owned),
                joints:  s.joints().map(|j| j.index() as u32).collect(),
                inverse_bind_matrices,
                skeleton_root: s.skeleton().map(|n| n.index() as u32),
            }
        })
        .collect()
}

#[inline] fn flatten_mat4(m: [[f32; 4]; 4]) -> [f32; 16] {
    [
        m[0][0], m[0][1], m[0][2], m[0][3],
        m[1][0], m[1][1], m[1][2], m[1][3],
        m[2][0], m[2][1], m[2][2], m[2][3],
        m[3][0], m[3][1], m[3][2], m[3][3],
    ]
}

// ── Animations ──────────────────────────────────────────────────────────────

fn extract_animations(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> GltfResult<ThinVec<Animation>> {
    let mut out = ThinVec::with_capacity(doc.animations().count());
    for anim in doc.animations() {
        // Sampler readers live on each Channel; the easiest robust path is
        // to walk channels and stash the (input, output) data keyed by the
        // sampler's index. Multiple channels may share a sampler — we keep
        // a sparse lookup so we only decode each sampler once.
        let sampler_count = anim.samplers().count();
        let mut decoded: ThinVec<Option<AnimSampler>> =
            (0..sampler_count).map(|_| None).collect();

        let channels: ThinVec<AnimChannel> = anim
            .channels()
            .map(|c| AnimChannel {
                target_node: c.target().node().index() as u32,
                target_path: anim_path_from(c.target().property()),
                sampler:     c.sampler().index() as u32,
            })
            .collect();

        for c in anim.channels() {
            let s_idx = c.sampler().index();
            if decoded[s_idx].is_some() { continue; }
            let reader = c.reader(|buf| Some(&*buffers[buf.index()]));
            let input: ThinVec<f32> = reader.read_inputs()
                .ok_or(GltfError::InvalidAccessor("animation input"))?
                .collect();
            let output = match reader.read_outputs()
                .ok_or(GltfError::InvalidAccessor("animation output"))?
            {
                gltf::animation::util::ReadOutputs::Translations(it) => SamplerOutput::Vec3(it.collect()),
                gltf::animation::util::ReadOutputs::Scales(it)       => SamplerOutput::Vec3(it.collect()),
                gltf::animation::util::ReadOutputs::Rotations(it)    => SamplerOutput::Vec4(it.into_f32().collect()),
                gltf::animation::util::ReadOutputs::MorphTargetWeights(it) => SamplerOutput::Scalars(it.into_f32().collect()),
            };
            decoded[s_idx] = Some(AnimSampler {
                interpolation: interp_from(c.sampler().interpolation()),
                input,
                output,
            });
        }

        // Any sampler not referenced by a channel is dead data — collapse to
        // an empty sampler so indices stay stable.
        let samplers: ThinVec<AnimSampler> = decoded
            .into_iter()
            .map(|opt| opt.unwrap_or(AnimSampler {
                interpolation: Interpolation::Linear,
                input:         ThinVec::new(),
                output:        SamplerOutput::Scalars(ThinVec::new()),
            }))
            .collect();

        out.push(Animation {
            name: anim.name().map(str::to_owned),
            samplers,
            channels,
        });
    }
    Ok(out)
}

fn interp_from(i: gltf::animation::Interpolation) -> Interpolation {
    match i {
        gltf::animation::Interpolation::Linear      => Interpolation::Linear,
        gltf::animation::Interpolation::Step        => Interpolation::Step,
        gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
    }
}

fn anim_path_from(p: gltf::animation::Property) -> AnimPath {
    match p {
        gltf::animation::Property::Translation  => AnimPath::Translation,
        gltf::animation::Property::Rotation     => AnimPath::Rotation,
        gltf::animation::Property::Scale        => AnimPath::Scale,
        gltf::animation::Property::MorphTargetWeights => AnimPath::MorphWeights,
    }
}

// ── Cameras / lights ────────────────────────────────────────────────────────

fn extract_cameras(doc: &gltf::Document) -> ThinVec<Camera> {
    doc.cameras()
        .map(|c| match c.projection() {
            gltf::camera::Projection::Perspective(p) => Camera::Perspective {
                name:         c.name().map(str::to_owned),
                aspect_ratio: p.aspect_ratio(),
                y_fov:        p.yfov(),
                z_near:       p.znear(),
                z_far:        p.zfar(),
            },
            gltf::camera::Projection::Orthographic(o) => Camera::Orthographic {
                name:   c.name().map(str::to_owned),
                x_mag:  o.xmag(),
                y_mag:  o.ymag(),
                z_near: o.znear(),
                z_far:  o.zfar(),
            },
        })
        .collect()
}

fn extract_lights(doc: &gltf::Document) -> ThinVec<Light> {
    let Some(lights) = doc.lights() else { return ThinVec::new(); };
    lights
        .map(|l| {
            use gltf::khr_lights_punctual::Kind;
            let (kind, inner, outer) = match l.kind() {
                Kind::Directional => (LightKind::Directional, 0.0, 0.0),
                Kind::Point       => (LightKind::Point, 0.0, 0.0),
                Kind::Spot { inner_cone_angle, outer_cone_angle } => {
                    (LightKind::Spot, inner_cone_angle, outer_cone_angle)
                }
            };
            Light {
                name: l.name().map(str::to_owned),
                kind,
                color: l.color(),
                intensity: l.intensity(),
                range: l.range().unwrap_or(0.0),
                inner_cone: inner,
                outer_cone: outer,
            }
        })
        .collect()
}
