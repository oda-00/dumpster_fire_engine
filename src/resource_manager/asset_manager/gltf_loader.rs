use std::path::Path;

use thin_vec::ThinVec;

use crate::forge_master::ore::{ForgeVertex, MeshOre};

// ── Error ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum GltfError {
    Io(gltf::Error),
    NoPrimitives,
    NoPositions,
}

impl std::fmt::Display for GltfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GltfError::Io(e)        => write!(f, "gltf I/O error: {e}"),
            GltfError::NoPrimitives => write!(f, "glTF file has no mesh primitives"),
            GltfError::NoPositions  => write!(f, "glTF primitive has no POSITION accessor"),
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

// ── Public loaders ─────────────────────────────────────────────────────────

/// Load the first mesh primitive from a `.glb` / `.gltf` file.
pub fn load_first_mesh(path: impl AsRef<Path>) -> Result<MeshOre, GltfError> {
    let (doc, buffers, _) = gltf::import(path)?;
    extract_first(&doc, &buffers)
}

/// Load every mesh primitive from a `.glb` / `.gltf` file.
pub fn load_all_meshes(path: impl AsRef<Path>) -> Result<ThinVec<MeshOre>, GltfError> {
    let (doc, buffers, _) = gltf::import(path)?;
    extract_all(&doc, &buffers)
}

/// Load the first mesh primitive from an in-memory GLB/glTF blob.
pub fn load_first_mesh_from_slice(bytes: &[u8]) -> Result<MeshOre, GltfError> {
    let (doc, buffers, _) = gltf::import_slice(bytes)?;
    extract_first(&doc, &buffers)
}

/// Load every mesh primitive from an in-memory GLB/glTF blob.
pub fn load_all_meshes_from_slice(bytes: &[u8]) -> Result<ThinVec<MeshOre>, GltfError> {
    let (doc, buffers, _) = gltf::import_slice(bytes)?;
    extract_all(&doc, &buffers)
}

// ── Extraction (shared by file and slice paths) ────────────────────────────

fn extract_first(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<MeshOre, GltfError> {
    let mut all = extract_all(doc, buffers)?;
    if all.is_empty() { Err(GltfError::NoPrimitives) } else { Ok(all.swap_remove(0)) }
}

fn extract_all(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<ThinVec<MeshOre>, GltfError> {
    let mut out = ThinVec::new();
    for mesh in doc.meshes() {
        for prim in mesh.primitives() {
            out.push(extract_primitive(&prim, buffers)?);
        }
    }
    Ok(out)
}

fn extract_primitive(
    prim:    &gltf::Primitive<'_>,
    buffers: &[gltf::buffer::Data],
) -> Result<MeshOre, GltfError> {
    let reader = prim.reader(|buf| Some(&*buffers[buf.index()]));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or(GltfError::NoPositions)?
        .collect();
    let n = positions.len();

    let normals: Vec<[f32; 3]> = reader.read_normals()
        .map(|it| it.collect())
        .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; n]);

    let tangents: Vec<[f32; 4]> = reader.read_tangents()
        .map(|it| it.collect())
        .unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 1.0]; n]);

    let uvs: Vec<[f32; 2]> = reader.read_tex_coords(0)
        .map(|tc| tc.into_f32().collect())
        .unwrap_or_else(|| vec![[0.0, 0.0]; n]);

    let vertices: ThinVec<ForgeVertex> = (0..n)
        .map(|i| ForgeVertex::new(positions[i], normals[i], tangents[i], uvs[i]))
        .collect();

    let indices: ThinVec<u32> = match reader.read_indices() {
        Some(it) => it.into_u32().collect(),
        None     => (0..n as u32).collect(),
    };

    Ok(MeshOre::new(vertices, indices))
}

// ── Test helpers ───────────────────────────────────────────────────────────

/// Build a minimal GLB 2.0 byte blob from raw mesh data.
///
/// Exposed as `pub(crate)` so the bench file can reuse it without touching disk.
pub fn build_test_glb(
    positions: &[[f32; 3]],
    normals:   Option<&[[f32; 3]]>,
    uvs:       Option<&[[f32; 2]]>,
    indices:   Option<&[u32]>,
) -> Vec<u8> {
    // ── BIN section ──────────────────────────────────────────────────────
    // Append each attribute run to one flat buffer, 4-byte aligned.

    let mut bin: Vec<u8> = Vec::new();

    // Returns (byte_offset, byte_length) of the appended block.
    fn append_f32s(bin: &mut Vec<u8>, floats: &[f32]) -> (usize, usize) {
        while bin.len() % 4 != 0 { bin.push(0); }
        let off = bin.len();
        for &f in floats { bin.extend_from_slice(&f.to_le_bytes()); }
        (off, bin.len() - off)
    }
    fn append_u32s(bin: &mut Vec<u8>, ints: &[u32]) -> (usize, usize) {
        while bin.len() % 4 != 0 { bin.push(0); }
        let off = bin.len();
        for &v in ints { bin.extend_from_slice(&v.to_le_bytes()); }
        (off, bin.len() - off)
    }

    // Flatten each slice-of-arrays into a flat f32 slice before appending.
    let pos_flat: Vec<f32> = positions.iter().flat_map(|p| p.iter().copied()).collect();
    let (pos_off, pos_len) = append_f32s(&mut bin, &pos_flat);

    let norm_bv = normals.map(|ns| {
        let flat: Vec<f32> = ns.iter().flat_map(|n| n.iter().copied()).collect();
        append_f32s(&mut bin, &flat)
    });
    let uv_bv = uvs.map(|us| {
        let flat: Vec<f32> = us.iter().flat_map(|u| u.iter().copied()).collect();
        append_f32s(&mut bin, &flat)
    });
    let idx_bv = indices.map(|ids| append_u32s(&mut bin, ids));

    while bin.len() % 4 != 0 { bin.push(0); }
    let bin_total = bin.len();

    // ── JSON section ─────────────────────────────────────────────────────
    // Each buffer view and accessor is pushed in the same order as the BIN
    // appends above. Indices are tracked as plain usizes.

    let mut bvs:  Vec<String> = Vec::new();
    let mut accs: Vec<String> = Vec::new();

    // pos  → bv 0, acc 0  (POSITION requires min/max per glTF spec)
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in positions {
        for i in 0..3 {
            mn[i] = mn[i].min(p[i]);
            mx[i] = mx[i].max(p[i]);
        }
    }
    bvs.push(format!(r#"{{"buffer":0,"byteOffset":{pos_off},"byteLength":{pos_len}}}"#));
    accs.push(format!(
        r#"{{"bufferView":0,"componentType":5126,"count":{cnt},"type":"VEC3","min":[{},{},{}],"max":[{},{},{}]}}"#,
        mn[0], mn[1], mn[2], mx[0], mx[1], mx[2], cnt = positions.len()
    ));

    let norm_acc: Option<usize> = norm_bv.map(|(off, len)| {
        let bv = bvs.len();
        bvs.push(format!(r#"{{"buffer":0,"byteOffset":{off},"byteLength":{len}}}"#));
        let ac = accs.len();
        accs.push(format!(r#"{{"bufferView":{bv},"componentType":5126,"count":{cnt},"type":"VEC3"}}"#,
            cnt = normals.unwrap().len()));
        ac
    });

    let uv_acc: Option<usize> = uv_bv.map(|(off, len)| {
        let bv = bvs.len();
        bvs.push(format!(r#"{{"buffer":0,"byteOffset":{off},"byteLength":{len}}}"#));
        let ac = accs.len();
        accs.push(format!(r#"{{"bufferView":{bv},"componentType":5126,"count":{cnt},"type":"VEC2"}}"#,
            cnt = uvs.unwrap().len()));
        ac
    });

    let idx_acc: Option<usize> = idx_bv.map(|(off, len)| {
        let bv = bvs.len();
        bvs.push(format!(r#"{{"buffer":0,"byteOffset":{off},"byteLength":{len}}}"#));
        let ac = accs.len();
        accs.push(format!(r#"{{"bufferView":{bv},"componentType":5125,"count":{cnt},"type":"SCALAR"}}"#,
            cnt = indices.unwrap().len()));
        ac
    });

    let mut attrs = format!(r#""POSITION":0"#);
    if let Some(i) = norm_acc { attrs.push_str(&format!(r#","NORMAL":{i}"#)); }
    if let Some(i) = uv_acc   { attrs.push_str(&format!(r#","TEXCOORD_0":{i}"#)); }
    let prim_idx = match idx_acc {
        Some(i) => format!(r#","indices":{i}"#),
        None    => String::new(),
    };

    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"meshes":[{{"primitives":[{{"attributes":{{{attrs}}}{prim_idx}}}]}}],"accessors":[{accs}],"bufferViews":[{bvs}],"buffers":[{{"byteLength":{bin_total}}}]}}"#,
        accs = accs.join(","), bvs = bvs.join(",")
    );
    let mut json_bytes = json.into_bytes();
    while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }

    // ── GLB assembly ──────────────────────────────────────────────────────
    let total = 12 + 8 + json_bytes.len() + 8 + bin_total;
    let mut glb: Vec<u8> = Vec::with_capacity(total);
    let p = |g: &mut Vec<u8>, v: u32| g.extend_from_slice(&v.to_le_bytes());
    p(&mut glb, 0x46546C67); // magic
    p(&mut glb, 2);           // version
    p(&mut glb, total as u32);
    p(&mut glb, json_bytes.len() as u32);
    p(&mut glb, 0x4E4F534A); // "JSON"
    glb.extend_from_slice(&json_bytes);
    p(&mut glb, bin_total as u32);
    p(&mut glb, 0x004E4942); // "BIN\0"
    glb.extend_from_slice(&bin);
    glb
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

    #[test]
    fn missing_normals_default_to_up() {
        let glb = build_test_glb(&triangle_pos(), None, None, None);
        let ore = load_first_mesh_from_slice(&glb).unwrap();
        for v in &ore.vertices {
            assert_eq!(v.normal, [0.0, 1.0, 0.0]);
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
        // Build a valid GLB that contains no meshes.
        let json = br#"{"asset":{"version":"2.0"}}"#;
        let mut jp = json.to_vec();
        while jp.len() % 4 != 0 { jp.push(b' '); }
        let total = (12 + 8 + jp.len()) as u32;
        let mut glb: Vec<u8> = Vec::new();
        for v in [0x46546C67u32, 2, total, jp.len() as u32, 0x4E4F534A] {
            glb.extend_from_slice(&v.to_le_bytes());
        }
        glb.extend_from_slice(&jp);
        assert!(matches!(
            load_first_mesh_from_slice(&glb),
            Err(GltfError::NoPrimitives)
        ));
    }

    #[test]
    fn all_meshes_from_slice_returns_empty_for_no_meshes() {
        let json = br#"{"asset":{"version":"2.0"}}"#;
        let mut jp = json.to_vec();
        while jp.len() % 4 != 0 { jp.push(b' '); }
        let total = (12 + 8 + jp.len()) as u32;
        let mut glb: Vec<u8> = Vec::new();
        for v in [0x46546C67u32, 2, total, jp.len() as u32, 0x4E4F534A] {
            glb.extend_from_slice(&v.to_le_bytes());
        }
        glb.extend_from_slice(&jp);
        let result = load_all_meshes_from_slice(&glb).unwrap();
        assert!(result.is_empty());
    }
}
