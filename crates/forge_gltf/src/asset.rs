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
use crate::codec::extensions;
use crate::codec::meshopt::{MeshoptFilter, MeshoptMode};
use crate::error::*;
use crate::light::*;
use crate::material::*;
use crate::mesh::*;
use crate::scene::*;
use crate::skin::*;
use crate::texture::*;

/// KTX2 magic header bytes.
const KTX2_MAGIC: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// Metadata from the glTF `asset` property (spec §2.1).
#[derive(Debug, Clone)]
pub struct AssetMetadata {
    /// Required: must start with `"2."`.
    pub version:     String,
    /// Optional minimum required version (must be ≤ 2.0 to load).
    pub min_version: Option<String>,
    /// Name of tool that generated the file.
    pub generator:   Option<String>,
    /// Copyright notice.
    pub copyright:   Option<String>,
}

impl Default for AssetMetadata {
    fn default() -> Self {
        Self {
            version:     "2.0".to_owned(),
            min_version: None,
            generator:   None,
            copyright:   None,
        }
    }
}

/// One entry from the top-level `KHR_materials_variants.variants` array.
#[derive(Debug, Clone)]
pub struct MaterialVariant {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct GltfAsset {
    /// Parsed `asset` metadata block.
    pub asset_metadata:        AssetMetadata,
    /// `extensionsUsed` array.
    pub extensions_used:       ThinVec<String>,
    /// `extensionsRequired` array.
    pub extensions_required:   ThinVec<String>,
    /// Top-level `KHR_materials_variants.variants` list (may be empty).
    pub material_variants:     ThinVec<MaterialVariant>,
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
        // Read the bytes first so we can pre-process KHR_animation_pointer.
        let p = path.as_ref();
        let bytes = std::fs::read(p).map_err(|e| {
            GltfError::Io(gltf::Error::Io(e))
        })?;
        // Set the base directory so external .png / .bin URIs can resolve
        // relative to the file we just read. Threading this through every
        // call site cleanly is more invasive than we want here, so we stash
        // it in a per-thread cell that `load_images_custom` consults
        // when it sees a non-data:// URI.
        let parent = p.parent().unwrap_or_else(|| Path::new("."));
        let prev = set_image_base_dir(Some(parent.to_path_buf()));
        let result = Self::load_slice(&bytes);
        set_image_base_dir(prev);
        result
    }

    pub fn load_slice(bytes: &[u8]) -> GltfResult<Self> {
        // Step 1: resolve any KHR_animation_pointer JSON patch needed.
        // We parse via Gltf::from_slice (structure-only, no image decode)
        // so we can control image loading ourselves.
        let (effective_bytes, per_anim_patches): (std::borrow::Cow<'_, [u8]>, Vec<crate::preprocess::PointerPatch>) =
            match gltf::Gltf::from_slice(bytes) {
                Ok(_) => (std::borrow::Cow::Borrowed(bytes), Vec::new()),
                Err(_) => {
                    if let Some((patched, patches)) = crate::preprocess::rewrite_animation_pointer(bytes) {
                        (std::borrow::Cow::Owned(patched), patches)
                    } else {
                        // Re-parse to get the actual error.
                        gltf::Gltf::from_slice(bytes).map_err(GltfError::Io)?;
                        unreachable!()
                    }
                }
            };

        // Step 2: Parse document and load buffer data.
        let gltf_obj = gltf::Gltf::from_slice(effective_bytes.as_ref())
            .map_err(GltfError::Io)?;
        // Pass the base directory (set by `GltfAsset::load`) so external
        // `.bin` URI references in plain `.gltf` files resolve against the
        // file's parent dir. `.glb` files have all buffers inline so the
        // base dir is irrelevant; `load_slice` callers without filesystem
        // context get `None` and external URIs error cleanly inside the
        // gltf crate.
        let base = image_base_dir();
        let mut buffer_data = gltf::import_buffers(
            &gltf_obj.document, base.as_deref(), gltf_obj.blob,
        ).map_err(GltfError::Io)?;

        // Step 3: Pre-decompress any EXT_meshopt_compression buffer views.
        preprocess_meshopt_buffer_views(effective_bytes.as_ref(), &mut buffer_data);

        // Step 4: Decode images with our custom decoders (WebP, KTX2/BasisU, PNG, JPEG).
        let image_data = load_images_custom(&gltf_obj.document, &buffer_data);

        let mut asset = extract_asset_with_patches(&gltf_obj.document, &buffer_data, &image_data, &per_anim_patches)?;
        // Attach pointer channels and drop the sentinel node-0 channels.
        for (i, patch) in per_anim_patches.into_iter().enumerate() {
            let Some(anim) = asset.animations.get_mut(i) else { continue };
            let skip = patch.patched_channel_indices;
            if !skip.is_empty() {
                let kept: thin_vec::ThinVec<crate::animation::AnimChannel> = anim
                    .channels
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| !skip.contains(&(*idx as u32)))
                    .map(|(_, c)| c.clone())
                    .collect();
                anim.channels = kept;
            }
            for p in patch.pointers { anim.pointer_channels.push(p); }
        }
        Ok(asset)
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

fn extract_asset_with_patches(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
    images:  &[gltf::image::Data],
    patches: &[crate::preprocess::PointerPatch],
) -> GltfResult<GltfAsset> {
    let asset_metadata = extract_asset_metadata(doc)?;
    let extensions_used     = extract_extensions_used(doc);
    let extensions_required = extract_extensions_required(doc);

    // Validate that every required extension is supported.
    for ext in &extensions_required {
        if !extensions::is_supported(ext) {
            return Err(GltfError::UnsupportedExtension(ext.clone()));
        }
    }

    let material_variants = extract_material_variants(doc);

    Ok(GltfAsset {
        asset_metadata,
        extensions_used,
        extensions_required,
        material_variants,
        scenes:        extract_scenes(doc),
        default_scene: doc.default_scene().map(|s| s.index() as u32),
        nodes:         extract_nodes(doc),
        meshes:        extract_meshes(doc, buffers)?,
        materials:     extract_materials(doc),
        textures:      extract_textures(doc),
        images:        extract_images(doc, images),
        samplers:      extract_samplers(doc),
        skins:         extract_skins(doc, buffers),
        animations:    extract_animations(doc, buffers, patches)?,
        cameras:       extract_cameras(doc),
        lights:        extract_lights(doc),
    })
}

fn extract_asset_metadata(doc: &gltf::Document) -> GltfResult<AssetMetadata> {
    let a = &doc.as_json().asset;
    let version = a.version.to_string();
    if !version.starts_with("2.") {
        return Err(GltfError::UnsupportedVersion(format!(
            "expected 2.x, got {version}"
        )));
    }
    let min_version = a.min_version.as_ref().map(|s| s.to_string());
    if let Some(ref mv) = min_version {
        // Parse X.Y — reject anything with major > 2 or major == 2 and minor > 0.
        if let Some((maj, min)) = mv.split_once('.') {
            let maj = maj.parse::<u32>().unwrap_or(99);
            let min = min.parse::<u32>().unwrap_or(99);
            if maj > 2 || (maj == 2 && min > 0) {
                return Err(GltfError::UnsupportedVersion(format!(
                    "minVersion {mv} is above 2.0"
                )));
            }
        }
    }
    Ok(AssetMetadata {
        version,
        min_version,
        generator: a.generator.as_ref().map(|s| s.to_string()),
        copyright: a.copyright.as_ref().map(|s| s.to_string()),
    })
}

fn extract_extensions_used(doc: &gltf::Document) -> ThinVec<String> {
    doc.as_json().extensions_used.iter().map(|s| s.to_string()).collect()
}

fn extract_extensions_required(doc: &gltf::Document) -> ThinVec<String> {
    doc.as_json().extensions_required.iter().map(|s| s.to_string()).collect()
}

fn extract_material_variants(doc: &gltf::Document) -> ThinVec<MaterialVariant> {
    // KHR_materials_variants stores its top-level data in doc.extensions.
    use serde_json::Value;
    let Some(exts) = doc.as_json().extensions.as_ref() else { return ThinVec::new(); };
    let Some(khr) = exts.others.get("KHR_materials_variants") else { return ThinVec::new(); };
    let Some(arr) = khr.get("variants").and_then(|v| v.as_array()) else { return ThinVec::new(); };
    arr.iter()
        .filter_map(|v: &Value| {
            let name = v.as_object()?.get("name")?.as_str()?.to_owned();
            Some(MaterialVariant { name })
        })
        .collect()
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
    use rayon::prelude::*;

    // gltf::Mesh<'_> wraps &'_ Root + a mesh index — Sync when Root: Sync.
    // Collect first so rayon can slice the Vec across threads.
    let meshes: Vec<gltf::Mesh<'_>> = doc.meshes().collect();
    let results: Vec<GltfResult<Mesh>> = meshes.par_iter().map(|mesh| {
        let mut primitives = ThinVec::with_capacity(mesh.primitives().count());
        for prim in mesh.primitives() {
            primitives.push(extract_primitive(&prim, buffers)?);
        }
        Ok(Mesh {
            name:     mesh.name().map(str::to_owned),
            primitives,
            weights:  mesh.weights().map(|ws| ws.iter().copied().collect()).unwrap_or_default(),
        })
    }).collect();

    results.into_iter().collect::<GltfResult<ThinVec<Mesh>>>()
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

    let indices: ThinVec<u32> = match reader.read_indices() {
        Some(it) => it.into_u32().collect(),
        None     => (0..n as u32).collect(),
    };

    // Per spec §3.7.2.1: when NORMAL is absent and topology is triangles,
    // compute flat normals. For non-triangle topologies fall back to Y-up.
    streams.normals = if let Some(it) = reader.read_normals() {
        it.collect()
    } else if matches!(prim.mode(), gltf::mesh::Mode::Triangles) {
        flat_normals(&streams.positions, &indices)
    } else {
        (0..n).map(|_| [0.0_f32, 1.0, 0.0]).collect()
    };

    // Multi-set UVs (TEXCOORD_n) — pull until None.
    let mut set = 0u32;
    while let Some(tc) = reader.read_tex_coords(set) {
        streams.uv_sets.push(tc.into_f32().collect());
        set += 1;
    }
    if streams.uv_sets.is_empty() {
        streams.uv_sets.push((0..n).map(|_| [0.0_f32, 0.0]).collect());
    }

    // Per spec §3.7.2.1: when TANGENT is absent but NORMAL + TEXCOORD_0 are
    // present and topology is triangles, generate MikkTSpace tangents.
    streams.tangents = if let Some(it) = reader.read_tangents() {
        it.collect()
    } else if matches!(prim.mode(), gltf::mesh::Mode::Triangles)
        && !streams.normals.is_empty()
        && !streams.uv_sets.is_empty()
    {
        crate::codec::mikktspace::generate_tangents(
            &streams.positions,
            &streams.normals,
            &streams.uv_sets[0],
            &indices,
        )
    } else {
        (0..n).map(|_| [1.0_f32, 0.0, 0.0, 1.0]).collect()
    };

    set = 0;
    while let Some(c) = reader.read_colors(set) {
        // Clamp colors to [0,1] per spec §3.7.2.2
        let clamped: ThinVec<[f32; 4]> = c.into_rgba_f32()
            .map(|[r, g, b, a]| [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0),
                                   b.clamp(0.0, 1.0), a.clamp(0.0, 1.0)])
            .collect();
        streams.colors.push(clamped);
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

    // Morph targets.
    let mut morph_targets = ThinVec::new();
    for morph in reader.read_morph_targets() {
        let (pos, norm, tan) = morph;
        let mut mt = MorphTarget::default();
        if let Some(p) = pos  { mt.positions = p.collect(); }
        if let Some(no) = norm { mt.normals  = no.collect(); }
        if let Some(t) = tan  { mt.tangents  = t.collect(); }
        morph_targets.push(mt);
    }

    // Custom _UNDERSCORED attributes (spec §3.7.2.1).
    let mut custom_attrs = ThinVec::new();
    for (semantic, accessor) in prim.attributes() {
        let name = match &semantic {
            gltf::mesh::Semantic::Extras(n) => n.as_str(),
            _ => continue,
        };
        if !name.starts_with('_') { continue; }
        match crate::codec::sparse::resolve_custom_attribute(name, &accessor, buffers) {
            Ok(ca) => custom_attrs.push(ca),
            Err(GltfError::SpecViolation(s)) => return Err(GltfError::SpecViolation(s)),
            Err(_) => {}
        }
    }

    // KHR_materials_variants per-primitive mappings
    let variant_mappings = extract_primitive_variant_mappings(prim);

    let bounds = {
        let bb = prim.bounding_box();
        Aabb { min: bb.min, max: bb.max }
    };

    Ok(Primitive {
        topology:        PrimitiveTopology::from_mode(prim.mode()),
        streams,
        indices,
        material:        prim.material().index().map(|i| i as u32),
        morph_targets,
        bounds,
        custom_attrs,
        variant_mappings,
    })
}

/// Compute one flat normal per triangle and duplicate it to all three vertices.
/// The result has the same length as `positions`.
fn flat_normals(positions: &[[f32; 3]], indices: &[u32]) -> ThinVec<[f32; 3]> {
    let n = positions.len();
    let mut out: ThinVec<[f32; 3]> = (0..n).map(|_| [0.0_f32, 1.0, 0.0]).collect();
    let tri_count = indices.len() / 3;
    for t in 0..tri_count {
        let i0 = indices[t * 3]     as usize;
        let i1 = indices[t * 3 + 1] as usize;
        let i2 = indices[t * 3 + 2] as usize;
        if i0 >= n || i1 >= n || i2 >= n { continue; }
        let p0 = positions[i0];
        let p1 = positions[i1];
        let p2 = positions[i2];
        let e1 = [p1[0]-p0[0], p1[1]-p0[1], p1[2]-p0[2]];
        let e2 = [p2[0]-p0[0], p2[1]-p0[1], p2[2]-p0[2]];
        let nx = e1[1]*e2[2] - e1[2]*e2[1];
        let ny = e1[2]*e2[0] - e1[0]*e2[2];
        let nz = e1[0]*e2[1] - e1[1]*e2[0];
        let len = (nx*nx + ny*ny + nz*nz).sqrt();
        let norm = if len > 1e-12 { [nx/len, ny/len, nz/len] } else { [0.0, 1.0, 0.0] };
        out[i0] = norm;
        out[i1] = norm;
        out[i2] = norm;
    }
    out
}

fn extract_primitive_variant_mappings(prim: &gltf::Primitive<'_>) -> ThinVec<VariantMapping> {
    let Some(khr) = prim.extension_value("KHR_materials_variants") else { return ThinVec::new(); };
    let Some(arr) = khr.get("mappings").and_then(|v: &serde_json::Value| v.as_array()) else { return ThinVec::new(); };
    arr.iter()
        .filter_map(|v: &serde_json::Value| {
            let obj = v.as_object()?;
            let material = obj.get("material")?.as_u64()? as u32;
            let variants: ThinVec<u32> = obj.get("variants")
                .and_then(|v: &serde_json::Value| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
                .unwrap_or_default();
            Some(VariantMapping { material, variants })
        })
        .collect()
}

// ── Materials ───────────────────────────────────────────────────────────────

fn apply_texture_transform_from_json(val: &serde_json::Value, tr: &mut TextureRef) {
    let Some(obj) = val.as_object() else { return };
    if let Some(arr) = obj.get("offset").and_then(|v| v.as_array()) {
        if let (Some(x), Some(y)) = (arr.first().and_then(|v| v.as_f64()),
                                      arr.get(1).and_then(|v| v.as_f64())) {
            tr.uv_offset = [x as f32, y as f32];
        }
    }
    if let Some(r) = obj.get("rotation").and_then(|v| v.as_f64()) {
        tr.uv_rotation = r as f32;
    }
    if let Some(arr) = obj.get("scale").and_then(|v| v.as_array()) {
        if let (Some(x), Some(y)) = (arr.first().and_then(|v| v.as_f64()),
                                      arr.get(1).and_then(|v| v.as_f64())) {
            tr.uv_scale = [x as f32, y as f32];
        }
    }
    if let Some(tc) = obj.get("texCoord").and_then(|v| v.as_u64()) {
        tr.tex_coord_set = tc as u32;
    }
}

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

            if let Some(n) = m.normal_texture() {
                let texture = n.texture().index() as u32;
                let tex_coord_set = n.tex_coord();
                let mut tr = TextureRef::identity(texture, tex_coord_set);
                // The typed gltf API exposes KHR_texture_transform via extensions().
                if let Some(ext_val) = n.extensions().and_then(|e| e.get("KHR_texture_transform")) {
                    apply_texture_transform_from_json(ext_val, &mut tr);
                }
                out.normal = NormalTexture { texture: Some(tr), scale: n.scale() };
            }

            if let Some(o) = m.occlusion_texture() {
                let texture = o.texture().index() as u32;
                let tex_coord_set = o.tex_coord();
                let mut tr = TextureRef::identity(texture, tex_coord_set);
                if let Some(ext_val) = o.extensions().and_then(|e| e.get("KHR_texture_transform")) {
                    apply_texture_transform_from_json(ext_val, &mut tr);
                }
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

            // Pluck the extension data the gltf crate doesn't model
            // (clearcoat, sheen, specular, iridescence, anisotropy,
            // diffuse_transmission, dispersion) out of the raw JSON map.
            if let Some(ext) = m.extensions() {
                parse_material_extensions(&mut out, ext);
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

    // Pre-extract names sequentially (gltf iterator types may not be Send).
    let names: Vec<Option<String>> = doc.images()
        .map(|im| im.name().map(str::to_owned))
        .collect();

    use rayon::prelude::*;
    let hints_ref: &[ImageFormatHint] = &hints;

    let decoded: Vec<Image> = images.par_iter().enumerate().map(|(i, data)| {
        let (w, h, rgba) = decode_to_rgba8(data);
        Image {
            name:   names.get(i).and_then(|n| n.clone()),
            width:  w,
            height: h,
            rgba,
            format: *hints_ref.get(i).unwrap_or(&ImageFormatHint::Linear),
        }
    }).collect();
    decoded.into_iter().collect()
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

// ── Phase 4 codec hooks ──────────────────────────────────────────────────────

/// Pre-process EXT_meshopt_compression: decompress any buffer views that
/// carry the extension and write the decompressed bytes into the fallback
/// range in the buffer data. This runs before the gltf crate's accessor
/// readers see the data, so regular `prim.reader(...)` calls get clean bytes.
fn preprocess_meshopt_buffer_views(
    raw_bytes: &[u8],
    buffer_data: &mut Vec<gltf::buffer::Data>,
) {
    let json_val = match extract_json_value(raw_bytes) {
        Some(v) => v,
        None    => return,
    };
    let bvs = match json_val["bufferViews"].as_array() {
        Some(a) => a,
        None    => return,
    };

    for bv in bvs {
        let Some(ext) = bv.get("extensions")
            .and_then(|e| e.get("EXT_meshopt_compression")) else { continue };

        let comp_buf_idx = ext["buffer"].as_u64().unwrap_or(0) as usize;
        let comp_off     = ext["byteOffset"].as_u64().unwrap_or(0) as usize;
        let comp_len     = ext["byteLength"].as_u64().unwrap_or(0) as usize;
        let stride       = ext["byteStride"].as_u64().unwrap_or(1) as usize;
        let count        = ext["count"].as_u64().unwrap_or(0) as usize;
        let mode_str     = ext["mode"].as_str().unwrap_or("ATTRIBUTES");
        let filter_str   = ext["filter"].as_str().unwrap_or("NONE");

        if comp_buf_idx >= buffer_data.len() || count == 0 || stride == 0 { continue; }

        let mode = match mode_str {
            "ATTRIBUTES" => MeshoptMode::Attributes,
            "TRIANGLES"  => MeshoptMode::Triangles,
            "INDICES"    => MeshoptMode::Indices,
            _            => continue,
        };
        let filter = match filter_str {
            "OCTAHEDRAL"  => MeshoptFilter::Octahedral,
            "QUATERNION"  => MeshoptFilter::Quaternion,
            "EXPONENTIAL" => MeshoptFilter::Exponential,
            _             => MeshoptFilter::None,
        };

        // Read compressed bytes into an owned Vec (avoids aliasing when we
        // write back below, in case src and dst are in the same buffer).
        let comp_end = comp_off.saturating_add(comp_len);
        if comp_end > buffer_data[comp_buf_idx].0.len() { continue; }
        let compressed: Vec<u8> = buffer_data[comp_buf_idx].0[comp_off..comp_end].to_vec();

        let decompressed = match crate::codec::meshopt::decompress_buffer_view(
            mode, filter, count, stride, &compressed,
        ) {
            Ok(d)  => d,
            Err(_) => continue,
        };

        // Write decompressed bytes into the fallback buffer-view location.
        let dst_buf_idx = bv["buffer"].as_u64().unwrap_or(0) as usize;
        let dst_off     = bv["byteOffset"].as_u64().unwrap_or(0) as usize;
        let dst_len     = bv["byteLength"].as_u64().unwrap_or(0) as usize;

        if dst_buf_idx >= buffer_data.len() { continue; }
        let dst_end = dst_off.saturating_add(dst_len);
        if dst_end > buffer_data[dst_buf_idx].0.len() { continue; }

        let copy_len = dst_len.min(decompressed.len());
        buffer_data[dst_buf_idx].0[dst_off..dst_off + copy_len]
            .copy_from_slice(&decompressed[..copy_len]);
    }
}

/// Load all images from the document, using our hand-rolled decoders for
/// WebP and KTX2/BasisU, and falling back to the gltf crate (image crate)
/// for PNG and JPEG. Images that fail to decode are replaced with a 1×1
/// black-transparent dummy so the rest of the asset loads normally.
/// Decoding is done in parallel (one rayon task per image).
/// Parallel-friendly owned form of a glTF image source. Built sequentially
/// from `gltf::image::Source<'_>` (which borrows the document and isn't
/// `Send`) and consumed in parallel by `load_single_image_custom` from a
/// rayon worker.
pub(crate) enum RawSource {
    /// Inline bytes pulled from a buffer view or a `data:` URI. `mime`
    /// disambiguates WebP / KTX2 / PNG / JPEG when the file's magic isn't
    /// authoritative.
    Bytes { bytes: Vec<u8>, mime: Option<String> },
    /// Relative path resolved against the per-thread `IMAGE_BASE_DIR`
    /// (set by `GltfAsset::load`). Falls back to an error if `load_slice`
    /// was called without a base dir set.
    ExternalUri { path_str: String, mime: Option<String> },
    /// The gltf crate exposed a source variant we don't know how to honour,
    /// or the bytes were truncated.
    Unsupported,
}

/// Decode one image source (post-extraction) into a CPU-side `image::Data`.
/// Dispatches between embedded-bytes, external-URI-on-disk, and the
/// unsupported error path.
fn load_single_image_custom(src: RawSource) -> GltfResult<gltf::image::Data> {
    match src {
        RawSource::Bytes { bytes, mime } => decode_image_bytes(&bytes, mime.as_deref()),
        RawSource::ExternalUri { path_str, mime } => {
            let path_str = path_str.strip_prefix("file://").unwrap_or(&path_str).to_owned();
            let Some(base) = image_base_dir() else {
                return Err(GltfError::UnsupportedFeature(
                    format!("external URI image without filesystem context: {path_str}")
                ));
            };
            let resolved = base.join(&path_str);
            let bytes = std::fs::read(&resolved).map_err(|e| {
                GltfError::Io(gltf::Error::Io(e))
            })?;
            decode_image_bytes(&bytes, mime.as_deref())
        }
        RawSource::Unsupported => Err(GltfError::UnsupportedFeature(
            "image source variant not understood (truncated bytes or unknown URI)".to_owned()
        )),
    }
}

/// Convert a borrowed `gltf::image::Source<'_>` into the owned, `Send`able
/// `RawSource` used by the parallel-decode path. Inline-bytes sources are
/// copied out of the document; external URIs are stashed as relative
/// paths for `load_single_image_custom` to resolve later.
fn extract_raw_source(
    src:         gltf::image::Source<'_>,
    buffer_data: &[gltf::buffer::Data],
) -> RawSource {
    match src {
        gltf::image::Source::View { view, mime_type } => {
            let buf = &buffer_data[view.buffer().index()].0;
            match buf.get(view.offset()..view.offset() + view.length()) {
                Some(slice) => RawSource::Bytes {
                    bytes: slice.to_vec(),
                    mime:  Some(mime_type.to_owned()),
                },
                None => RawSource::Unsupported,
            }
        }
        gltf::image::Source::Uri { uri, mime_type } => {
            if let Some(rest) = uri.strip_prefix("data:") {
                let mut parts = rest.splitn(2, ";base64,");
                let uri_mime = parts.next().map(str::to_owned);
                if let Some(b64) = parts.next() {
                    if let Some(bytes) = simple_base64_decode(b64) {
                        let mime = uri_mime.or_else(|| mime_type.map(str::to_owned));
                        return RawSource::Bytes { bytes, mime };
                    }
                }
                RawSource::Unsupported
            } else {
                RawSource::ExternalUri {
                    path_str: uri.to_owned(),
                    mime:     mime_type.map(str::to_owned),
                }
            }
        }
    }
}

fn load_images_custom(
    doc: &gltf::Document,
    buffer_data: &[gltf::buffer::Data],
) -> Vec<gltf::image::Data> {
    use rayon::prelude::*;
    // Pre-extract owned sources sequentially (gltf types aren't Send),
    // then decode in parallel through `load_single_image_custom`.
    let sources: Vec<RawSource> = doc.images()
        .map(|img| extract_raw_source(img.source(), buffer_data))
        .collect();
    let dummy = || gltf::image::Data {
        pixels: vec![0, 0, 0, 0],
        format: gltf::image::Format::R8G8B8A8,
        width: 1, height: 1,
    };
    sources.into_par_iter()
        .map(|src| load_single_image_custom(src).unwrap_or_else(|_| dummy()))
        .collect()
}

// Per-thread image-resolution base directory. Set by `GltfAsset::load` so
// `load_images_custom` can resolve external URI references to disk;
// `load_slice` callers leave this `None` and external URIs error cleanly.
thread_local! {
    static IMAGE_BASE_DIR: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

fn set_image_base_dir(p: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    IMAGE_BASE_DIR.with(|d| std::mem::replace(&mut *d.borrow_mut(), p))
}
fn image_base_dir() -> Option<std::path::PathBuf> {
    IMAGE_BASE_DIR.with(|d| d.borrow().clone())
}

fn decode_image_bytes(bytes: &[u8], mime_type: Option<&str>) -> GltfResult<gltf::image::Data> {
    // WebP detection
    let is_webp = mime_type == Some("image/webp")
        || (bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP");

    if is_webp {
        let (w, h, rgba) = crate::codec::webp::decode_to_rgba8(bytes)?;
        return Ok(gltf::image::Data {
            pixels: rgba.to_vec(),
            format: gltf::image::Format::R8G8B8A8,
            width:  w,
            height: h,
        });
    }

    // KTX2 detection
    let is_ktx2 = mime_type == Some("image/ktx2")
        || bytes.get(..12).map_or(false, |m| m == KTX2_MAGIC);

    if is_ktx2 {
        let (w, h, rgba) = decode_ktx2_to_rgba8(bytes)?;
        return Ok(gltf::image::Data {
            pixels: rgba,
            format: gltf::image::Format::R8G8B8A8,
            width:  w,
            height: h,
        });
    }

    // PNG / JPEG: use the gltf crate's image-crate decoder.
    // We reconstruct a minimal Source::View from the raw bytes by re-wrapping
    // them into the gltf::image::Data path using image_crate directly via
    // the approach in gltf's import.rs.
    decode_standard_image(bytes, mime_type)
}

/// Decode PNG / JPEG bytes via the `image` crate (re-exposed through gltf).
fn decode_standard_image(
    bytes:     &[u8],
    mime_type: Option<&str>,
) -> GltfResult<gltf::image::Data> {
    use image::GenericImageView;
    let fmt = if mime_type == Some("image/png") {
        Some(image::ImageFormat::Png)
    } else if mime_type == Some("image/jpeg") {
        Some(image::ImageFormat::Jpeg)
    } else {
        // Sniff from bytes.
        match bytes.get(..4) {
            Some([0x89, 0x50, 0x4E, 0x47]) => Some(image::ImageFormat::Png),
            Some([0xFF, 0xD8, ..])          => Some(image::ImageFormat::Jpeg),
            _                               => None,
        }
    };
    let dyn_img = match fmt {
        Some(f) => image::load_from_memory_with_format(bytes, f)
            .map_err(|_| GltfError::InvalidAccessor("image decode failed"))?,
        None    => image::load_from_memory(bytes)
            .map_err(|_| GltfError::InvalidAccessor("image decode failed, unknown format"))?,
    };
    let (w, h) = dyn_img.dimensions();
    let pixels = dyn_img.into_rgba8().into_raw();
    Ok(gltf::image::Data {
        pixels,
        format: gltf::image::Format::R8G8B8A8,
        width: w,
        height: h,
    })
}

/// Decode a KTX2 container to raw RGBA8 pixels.
/// Handles BasisLZ (ETC1S) and raw (UASTC) supercompression.
/// Decode an uncompressed KTX2 level (`SupercompressionScheme::None`)
/// into a tightly-packed RGBA8 buffer. Dispatches on the Vulkan format
/// enum to cover the formats glTF assets actually use — the BC-block
/// family (BC1/2/3/4/5/7), the common UNORM/SRGB byte layouts (R8 /
/// R8G8 / R8G8B8 / R8G8B8A8 / B8G8R8A8), and `R16G16B16A16_SFLOAT`. The
/// special "UASTC in a vkFormat=0 wrapper" case routes through the
/// BasisU decoder. Anything else surfaces as an explicit unsupported
/// error rather than silent black output.
fn decode_ktx2_uncompressed(vk_format: u32, level_data: &[u8], w: u32, h: u32) -> GltfResult<Vec<u8>> {
    use crate::codec::bc;
    let n = (w as usize) * (h as usize);
    Ok(match vk_format {
        0 => crate::codec::basisu_uastc::transcode_to_rgba8(level_data, w, h).to_vec(),

        // Single-channel UNORM/SRGB → splat the value into R, fill G=B=0, A=255.
        9 | 15 => {
            let take = level_data.len().min(n);
            let mut out = vec![0u8; n * 4];
            for i in 0..take {
                out[i * 4]     = level_data[i];
                out[i * 4 + 3] = 255;
            }
            out
        }

        // Two-channel UNORM/SRGB → R, G, 0, 255.
        16 | 22 => {
            let take = level_data.len().min(n * 2);
            let mut out = vec![0u8; n * 4];
            for i in 0..(take / 2) {
                out[i * 4]     = level_data[i * 2];
                out[i * 4 + 1] = level_data[i * 2 + 1];
                out[i * 4 + 3] = 255;
            }
            out
        }

        // Three-channel UNORM/SRGB → R, G, B, 255.
        23 | 29 => {
            let take = level_data.len().min(n * 3);
            let mut out = vec![0u8; n * 4];
            for i in 0..(take / 3) {
                out[i * 4]     = level_data[i * 3];
                out[i * 4 + 1] = level_data[i * 3 + 1];
                out[i * 4 + 2] = level_data[i * 3 + 2];
                out[i * 4 + 3] = 255;
            }
            out
        }

        // R8G8B8A8 UNORM/SRGB → direct passthrough (truncate to expected
        // length to defend against trailing padding).
        37 | 43 => {
            let take = level_data.len().min(n * 4);
            level_data[..take].to_vec()
        }

        // B8G8R8A8 UNORM/SRGB → swizzle BGRA → RGBA per texel.
        44 | 50 => {
            let take = level_data.len().min(n * 4);
            let mut out = vec![0u8; n * 4];
            for i in 0..(take / 4) {
                out[i * 4]     = level_data[i * 4 + 2];
                out[i * 4 + 1] = level_data[i * 4 + 1];
                out[i * 4 + 2] = level_data[i * 4];
                out[i * 4 + 3] = level_data[i * 4 + 3];
            }
            out
        }

        // R16G16B16A16_SFLOAT — convert each half-precision lane to a
        // clamped u8 (the LDR pipeline accepts this lossy cast for HDR
        // sources; a future tone-mapping pass would do better).
        97 => {
            let take = level_data.len().min(n * 8);
            let mut out = vec![0u8; n * 4];
            for i in 0..(take / 8) {
                for c in 0..4 {
                    let bytes = [level_data[i * 8 + c * 2], level_data[i * 8 + c * 2 + 1]];
                    let raw = u16::from_le_bytes(bytes);
                    out[i * 4 + c] = half_to_u8(raw);
                }
            }
            out
        }

        // ── Block-compressed (BC1-BC7). VK_FORMAT_BC* enums:
        //   131 BC1_RGB_UNORM,  132 BC1_RGB_SRGB,
        //   133 BC1_RGBA_UNORM, 134 BC1_RGBA_SRGB,
        //   135 BC2_UNORM, 136 BC2_SRGB, 137 BC3_UNORM, 138 BC3_SRGB,
        //   139 BC4_UNORM, 140 BC4_SNORM, 141 BC5_UNORM, 142 BC5_SNORM,
        //   143 BC6H_UFLOAT, 144 BC6H_SFLOAT, 145 BC7_UNORM, 146 BC7_SRGB.
        131 | 132 | 133 | 134 => bc::decode_bc1(level_data, w, h).to_vec(),
        135 | 136                => bc::decode_bc2(level_data, w, h).to_vec(),
        137 | 138                => bc::decode_bc3(level_data, w, h).to_vec(),
        139 | 140                => bc::decode_bc4(level_data, w, h).to_vec(),
        141 | 142                => bc::decode_bc5(level_data, w, h).to_vec(),
        145 | 146                => bc::decode_bc7(level_data, w, h).to_vec(),

        other => return Err(GltfError::UnsupportedFeature(
            format!("KTX2 uncompressed vkFormat {other}")
        )),
    })
}

/// IEEE-754 binary16 (half) → clamped u8. We bias to [0,1] before scaling
/// so HDR values land at 255 rather than wrapping; sub-zero clamps to 0.
fn half_to_u8(h: u16) -> u8 {
    let sign = (h >> 15) & 1;
    let exp  = ((h >> 10) & 0x1f) as i32;
    let mant = (h & 0x3ff) as u32;
    let f = if exp == 0 && mant == 0 { 0.0 }
            else if exp == 0          { (mant as f32) * (1.0 / (1u32 << 24) as f32) }
            else if exp == 0x1f       { if mant == 0 { f32::INFINITY } else { f32::NAN } }
            else {
                let m = (mant | 0x400) as f32;
                m * (1u32 << (exp - 25).max(-126) as u32) as f32
            };
    let f = if sign == 1 { -f } else { f };
    if !f.is_finite() || f <= 0.0 { 0 }
    else if f >= 1.0              { 255 }
    else                           { (f * 255.0 + 0.5) as u8 }
}

fn decode_ktx2_to_rgba8(bytes: &[u8]) -> GltfResult<(u32, u32, Vec<u8>)> {
    use crate::codec::ktx2::{Ktx2, SupercompressionScheme};

    let ktx = Ktx2::parse(bytes)?;
    let w = ktx.pixel_width;
    let h = ktx.pixel_height.max(1);

    let level_data = ktx.level_data(bytes, 0)
        .ok_or(GltfError::InvalidAccessor("KTX2 has no level 0 data"))?;

    let rgba: Vec<u8> = match ktx.supercompression {
        SupercompressionScheme::None => decode_ktx2_uncompressed(ktx.vk_format, level_data, w, h)?,
        SupercompressionScheme::BasisLZ => {
            // ETC1S via BasisLZ supercompression.
            crate::codec::basisu_etc1s::transcode_to_rgba8(
                &ktx.sgd, level_data, w, h,
            )?.to_vec()
        }
        _ => {
            return Err(GltfError::UnsupportedFeature(
                format!("KTX2 supercompression scheme {:?}", ktx.supercompression),
            ));
        }
    };

    Ok((w, h, rgba))
}

/// Parse the JSON portion of raw glTF/GLB bytes into a serde_json Value.
fn extract_json_value(bytes: &[u8]) -> Option<serde_json::Value> {
    const GLB_MAGIC: u32 = 0x46546C67;
    if bytes.len() >= 12
        && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == GLB_MAGIC
    {
        // GLB: JSON chunk is at bytes 12..12+json_chunk_length
        if bytes.len() < 20 { return None; }
        let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        // Chunk type at [16..20] should be 0x4E4F534A ("JSON")
        let json_bytes = bytes.get(20..20 + json_len)?;
        serde_json::from_slice(json_bytes).ok()
    } else {
        serde_json::from_slice(bytes).ok()
    }
}

/// Minimal base64 decoder (no external dep).
fn simple_base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: [u8; 128] = {
        let mut t = [255u8; 128];
        let src = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0usize;
        while i < src.len() { t[src[i] as usize] = i as u8; i += 1; }
        t
    };
    let input = input.trim().as_bytes();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in input {
        if c == b'=' { break; }
        if c as usize >= 128 { return None; }
        let v = TABLE[c as usize];
        if v == 255 { return None; }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
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
    patches: &[crate::preprocess::PointerPatch],
) -> GltfResult<ThinVec<Animation>> {
    let mut out = ThinVec::with_capacity(doc.animations().count());
    for (anim_idx, anim) in doc.animations().enumerate() {
        // Channel indices that were rewritten by the KHR_animation_pointer
        // pre-pass — their samplers point at non-TRS data that the gltf
        // crate's typed reader would assert on, so we route them through
        // `pointer_channels` separately.
        let patched_idx: &[u32] = patches
            .get(anim_idx)
            .map(|p| p.patched_channel_indices.as_slice())
            .unwrap_or(&[]);
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

        for (ch_idx, c) in anim.channels().enumerate() {
            // Skip pointer-patched channels — their samplers point at
            // non-TRS data; the typed reader would assert on stride.
            if patched_idx.contains(&(ch_idx as u32)) { continue; }
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
            pointer_channels: ThinVec::new(),
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
