use std::path::Path;

use thin_vec::ThinVec;

use crate::forge_master::ore::{ForgeVertex, MeshOre};

// ── Error ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum GltfError {
    /// Underlying gltf crate error (file not found, parse failure, …).
    Io(gltf::Error),
    /// The file contained no mesh primitives.
    NoPrimitives,
    /// A primitive had no POSITION accessor.
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
    fn from(e: gltf::Error) -> Self {
        GltfError::Io(e)
    }
}

// ── Public loaders ─────────────────────────────────────────────────────────

/// Load the **first** mesh primitive found in a `.glb` / `.gltf` file.
pub fn load_first_mesh(path: impl AsRef<Path>) -> Result<MeshOre, GltfError> {
    let (doc, buffers, _) = gltf::import(path)?;
    extract_first(&doc, &buffers)
}

/// Load **every** mesh primitive in a `.glb` / `.gltf` file.
pub fn load_all_meshes(path: impl AsRef<Path>) -> Result<ThinVec<MeshOre>, GltfError> {
    let (doc, buffers, _) = gltf::import(path)?;
    extract_all(&doc, &buffers)
}

/// Load the first mesh primitive from an in-memory GLB/glTF blob.
///
/// Useful for tests, tooling, and asset packs stored as embedded bytes.
pub fn load_first_mesh_from_slice(bytes: &[u8]) -> Result<MeshOre, GltfError> {
    let (doc, buffers, _) = gltf::import_slice(bytes)?;
    extract_first(&doc, &buffers)
}

/// Load every mesh primitive from an in-memory GLB/glTF blob.
pub fn load_all_meshes_from_slice(bytes: &[u8]) -> Result<ThinVec<MeshOre>, GltfError> {
    let (doc, buffers, _) = gltf::import_slice(bytes)?;
    extract_all(&doc, &buffers)
}

// ── Inner extraction (shared by file and slice paths) ─────────────────────

fn extract_first(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<MeshOre, GltfError> {
    let mut all = extract_all(doc, buffers)?;
    if all.is_empty() {
        Err(GltfError::NoPrimitives)
    } else {
        Ok(all.swap_remove(0))
    }
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

    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .map(|it| it.collect())
        .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; n]);

    let tangents: Vec<[f32; 4]> = reader
        .read_tangents()
        .map(|it| it.collect())
        .unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 1.0]; n]);

    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // ── Minimal GLB builder ────────────────────────────────────────────────
    //
    // Produces a valid GLB 2.0 blob entirely in memory so tests never touch
    // disk. Layout:
    //   [GLB header 12 B]
    //   [JSON chunk header 8 B][JSON padded to 4 B with spaces]
    //   [BIN  chunk header 8 B][BIN  padded to 4 B with zeros ]
    //
    // The JSON describes one mesh with one primitive. Optional accessors for
    // NORMAL, TANGENT, TEXCOORD_0, and INDICES can be toggled via the flags.

    pub struct GlbBuilder {
        positions: Vec<[f32; 3]>,
        normals:   Option<Vec<[f32; 3]>>,
        tangents:  Option<Vec<[f32; 4]>>,
        uvs:       Option<Vec<[f32; 2]>>,
        indices:   Option<Vec<u32>>,
    }

    impl GlbBuilder {
        pub fn triangle() -> Self {
            Self {
                positions: vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]],
                normals:   None,
                tangents:  None,
                uvs:       None,
                indices:   None,
            }
        }

        pub fn with_normals(mut self) -> Self {
            let n = self.positions.len();
            self.normals = Some(vec![[0.0, 0.0, 1.0]; n]);
            self
        }

        pub fn with_uvs(mut self) -> Self {
            let n = self.positions.len();
            self.uvs = Some(vec![[0.5, 0.5]; n]);
            self
        }

        pub fn with_indices(mut self) -> Self {
            let n = self.positions.len();
            self.indices = Some((0..n as u32).collect());
            self
        }

        /// Construct the raw GLB bytes.
        pub fn build(self) -> Vec<u8> {
            // --- BIN section ------------------------------------------------
            let mut bin: Vec<u8> = Vec::new();

            // Track accessors so we can build the JSON later.
            struct Acc { buf_view: usize, component: u32, count: usize, ty: &'static str }
            let mut accessors: Vec<Acc> = Vec::new();
            let mut buf_views: Vec<(usize, usize)> = Vec::new(); // (byte_offset, byte_len)

            macro_rules! push_acc {
                ($data:expr, $comp:expr, $ty:expr) => {{
                    let start = align4(&mut bin);
                    for item in $data {
                        for b in item { bin.extend_from_slice(&b.to_le_bytes()); }
                    }
                    let len = bin.len() - start;
                    let bv = buf_views.len();
                    buf_views.push((start, len));
                    accessors.push(Acc { buf_view: bv, component: $comp, count: $data.len(), ty: $ty });
                }};
            }

            // Positions (5126 = FLOAT, VEC3)
            let positions = &self.positions;
            {
                let start = bin.len();
                for p in positions { for &f in p { bin.extend_from_slice(&f.to_le_bytes()); } }
                let len = bin.len() - start;
                let bv = buf_views.len();
                buf_views.push((start, len));
                accessors.push(Acc { buf_view: bv, component: 5126, count: positions.len(), ty: "VEC3" });
            }

            let pos_acc = 0usize;
            let mut next_acc = 1usize;

            let norm_acc = self.normals.as_ref().map(|ns| {
                let i = next_acc; next_acc += 1;
                let start = pad4_len(bin.len());
                while bin.len() < start { bin.push(0); }
                let start = bin.len();
                for n in ns { for &f in n { bin.extend_from_slice(&f.to_le_bytes()); } }
                let len = bin.len() - start;
                buf_views.push((start, len));
                accessors.push(Acc { buf_view: buf_views.len()-1, component: 5126, count: ns.len(), ty: "VEC3" });
                i
            });

            let uv_acc = self.uvs.as_ref().map(|us| {
                let i = next_acc; next_acc += 1;
                let start = bin.len();
                for u in us { for &f in u { bin.extend_from_slice(&f.to_le_bytes()); } }
                let len = bin.len() - start;
                buf_views.push((start, len));
                accessors.push(Acc { buf_view: buf_views.len()-1, component: 5126, count: us.len(), ty: "VEC2" });
                i
            });

            let idx_acc = self.indices.as_ref().map(|ids| {
                let i = next_acc;
                let start = bin.len();
                for &v in ids { bin.extend_from_slice(&v.to_le_bytes()); }
                let len = bin.len() - start;
                buf_views.push((start, len));
                accessors.push(Acc { buf_view: buf_views.len()-1, component: 5125, count: ids.len(), ty: "SCALAR" });
                i
            });

            // Pad BIN to 4 bytes.
            while bin.len() % 4 != 0 { bin.push(0); }
            let bin_len = bin.len();

            // --- JSON section -----------------------------------------------
            let mut attrs = format!(r#""POSITION":{pos_acc}"#);
            if let Some(i) = norm_acc { attrs.push_str(&format!(r#","NORMAL":{i}"#)); }
            if let Some(i) = uv_acc   { attrs.push_str(&format!(r#","TEXCOORD_0":{i}"#)); }

            let prim_indices = match idx_acc {
                Some(i) => format!(r#","indices":{i}"#),
                None    => String::new(),
            };

            let accs_json: String = accessors.iter().enumerate().map(|(i, a)| {
                let comma = if i > 0 { "," } else { "" };
                format!(
                    r#"{comma}{{"bufferView":{bv},"componentType":{comp},"count":{cnt},"type":"{ty}"}}"#,
                    bv = a.buf_view, comp = a.component, cnt = a.count, ty = a.ty
                )
            }).collect();

            let bvs_json: String = buf_views.iter().enumerate().map(|(i, (off, len))| {
                let comma = if i > 0 { "," } else { "" };
                format!(r#"{comma}{{"buffer":0,"byteOffset":{off},"byteLength":{len}}}"#)
            }).collect();

            let json = format!(
                r#"{{"asset":{{"version":"2.0"}},"meshes":[{{"primitives":[{{"attributes":{{{attrs}}}{prim_indices}}}]}}],"accessors":[{accs_json}],"bufferViews":[{bvs_json}],"buffers":[{{"byteLength":{bin_len}}}]}}"#
            );

            let mut json_bytes = json.into_bytes();
            while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }

            // --- Assemble GLB -----------------------------------------------
            let total = 12 + 8 + json_bytes.len() + 8 + bin_len;
            let mut glb: Vec<u8> = Vec::with_capacity(total);

            let push_u32 = |v: Vec<u8>, n: u32| -> Vec<u8> {
                let mut v = v; v.extend_from_slice(&n.to_le_bytes()); v
            };

            let mut glb = push_u32(glb, 0x46546C67); // magic "glTF"
            glb = push_u32(glb, 2);                   // version
            glb = push_u32(glb, total as u32);

            glb = push_u32(glb, json_bytes.len() as u32);
            glb = push_u32(glb, 0x4E4F534A); // "JSON"
            glb.extend_from_slice(&json_bytes);

            glb = push_u32(glb, bin_len as u32);
            glb = push_u32(glb, 0x004E4942); // "BIN\0"
            glb.extend_from_slice(&bin);

            glb
        }
    }

    fn pad4_len(n: usize) -> usize {
        (n + 3) & !3
    }

    fn align4(buf: &mut Vec<u8>) -> usize {
        while buf.len() % 4 != 0 { buf.push(0); }
        buf.len()
    }

    // ── Unit tests ─────────────────────────────────────────────────────────

    #[test]
    fn triangle_positions_and_generated_indices() {
        let glb = GlbBuilder::triangle().build();
        let ore = load_first_mesh_from_slice(&glb).expect("parse");
        assert_eq!(ore.vertices.len(), 3, "should have 3 vertices");
        // No index accessor → sequential 0,1,2 generated
        assert_eq!(ore.indices.as_slice(), &[0u32, 1, 2]);
    }

    #[test]
    fn triangle_with_explicit_indices() {
        let glb = GlbBuilder::triangle().with_indices().build();
        let ore = load_first_mesh_from_slice(&glb).expect("parse");
        assert_eq!(ore.indices.as_slice(), &[0u32, 1, 2]);
    }

    #[test]
    fn missing_normals_fall_back_to_up() {
        let glb = GlbBuilder::triangle().build(); // no normals in GLB
        let ore = load_first_mesh_from_slice(&glb).expect("parse");
        for v in &ore.vertices {
            assert_eq!(v.normal, [0.0, 1.0, 0.0], "default normal should be (0,1,0)");
        }
    }

    #[test]
    fn explicit_normals_are_preserved() {
        let glb = GlbBuilder::triangle().with_normals().build();
        let ore = load_first_mesh_from_slice(&glb).expect("parse");
        for v in &ore.vertices {
            assert_eq!(v.normal, [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn missing_uvs_fall_back_to_zero() {
        let glb = GlbBuilder::triangle().build();
        let ore = load_first_mesh_from_slice(&glb).expect("parse");
        for v in &ore.vertices {
            assert_eq!(v.uv, [0.0, 0.0]);
        }
    }

    #[test]
    fn explicit_uvs_are_preserved() {
        let glb = GlbBuilder::triangle().with_uvs().build();
        let ore = load_first_mesh_from_slice(&glb).expect("parse");
        for v in &ore.vertices {
            assert_eq!(v.uv, [0.5, 0.5]);
        }
    }

    #[test]
    fn empty_file_returns_no_primitives() {
        // A valid GLB with no meshes → empty Ok(ThinVec)
        let json = br#"{"asset":{"version":"2.0"}}"#;
        let mut json_padded = json.to_vec();
        while json_padded.len() % 4 != 0 { json_padded.push(b' '); }
        let total = 12u32 + 8 + json_padded.len() as u32;
        let mut glb: Vec<u8> = Vec::new();
        glb.extend_from_slice(&0x46546C67u32.to_le_bytes());
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&total.to_le_bytes());
        glb.extend_from_slice(&(json_padded.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        glb.extend_from_slice(&json_padded);

        let result = load_all_meshes_from_slice(&glb).expect("parse");
        assert!(result.is_empty());
    }

    #[test]
    fn load_first_on_empty_returns_no_primitives_error() {
        let json = br#"{"asset":{"version":"2.0"}}"#;
        let mut json_padded = json.to_vec();
        while json_padded.len() % 4 != 0 { json_padded.push(b' '); }
        let total = 12u32 + 8 + json_padded.len() as u32;
        let mut glb: Vec<u8> = Vec::new();
        glb.extend_from_slice(&0x46546C67u32.to_le_bytes());
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&total.to_le_bytes());
        glb.extend_from_slice(&(json_padded.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        glb.extend_from_slice(&json_padded);

        assert!(matches!(
            load_first_mesh_from_slice(&glb),
            Err(GltfError::NoPrimitives)
        ));
    }

    #[test]
    fn vertex_count_scales_linearly() {
        // Build a quad (4 verts, 2 triangles = 6 indices).
        use crate::forge_master::ore::ForgeVertex;
        let positions = vec![
            [-1.0_f32, -1.0, 0.0], [1.0, -1.0, 0.0],
            [1.0,  1.0, 0.0], [-1.0,  1.0, 0.0],
        ];
        let mut builder = GlbBuilder {
            positions,
            normals:  None,
            tangents: None,
            uvs:      None,
            indices:  Some(vec![0, 1, 2, 2, 3, 0]),
        };
        let glb = builder.build();
        let ore = load_first_mesh_from_slice(&glb).expect("parse");
        assert_eq!(ore.vertices.len(), 4);
        assert_eq!(ore.indices.len(), 6);
    }
}
