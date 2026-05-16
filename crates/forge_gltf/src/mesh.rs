//! Mesh + primitive representation.
//!
//! A `Mesh` holds one or more `Primitive`s. Each primitive carries every
//! optional vertex stream the glTF spec defines — positions, normals,
//! tangents, an arbitrary number of UV / colour / joint / weight sets — plus
//! a primitive topology so downstream pipelines can choose the right vk
//! topology. Defaults are filled in (zero normals → up, missing UV → 0,0)
//! to keep pipeline adapters branch-free.

use thin_vec::ThinVec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PrimitiveTopology {
    Points,
    Lines,
    LineLoop,
    LineStrip,
    Triangles,
    TriangleStrip,
    TriangleFan,
}

impl PrimitiveTopology {
    pub fn from_mode(mode: gltf::mesh::Mode) -> Self {
        match mode {
            gltf::mesh::Mode::Points        => Self::Points,
            gltf::mesh::Mode::Lines         => Self::Lines,
            gltf::mesh::Mode::LineLoop      => Self::LineLoop,
            gltf::mesh::Mode::LineStrip     => Self::LineStrip,
            gltf::mesh::Mode::Triangles     => Self::Triangles,
            gltf::mesh::Mode::TriangleStrip => Self::TriangleStrip,
            gltf::mesh::Mode::TriangleFan   => Self::TriangleFan,
        }
    }
}

/// Per-vertex stream container. All inner vectors are either empty or have
/// exactly `vertex_count` entries — pipeline adapters can index them blindly.
#[derive(Debug, Clone)]
pub struct VertexStreams {
    pub positions: ThinVec<[f32; 3]>,
    pub normals:   ThinVec<[f32; 3]>,
    pub tangents:  ThinVec<[f32; 4]>,
    /// Multiple UV sets (TEXCOORD_n). Outer vec = set index; inner = per-vertex.
    pub uv_sets:   ThinVec<ThinVec<[f32; 2]>>,
    /// Vertex colours (COLOR_n) as RGBA float.
    pub colors:    ThinVec<ThinVec<[f32; 4]>>,
    /// Skinning joint indices (JOINTS_n) — up to 4 per set.
    pub joints:    ThinVec<ThinVec<[u16; 4]>>,
    /// Skinning joint weights (WEIGHTS_n).
    pub weights:   ThinVec<ThinVec<[f32; 4]>>,
}

impl VertexStreams {
    pub fn new() -> Self { Self::default() }
    pub fn vertex_count(&self) -> usize { self.positions.len() }
}

impl Default for VertexStreams {
    fn default() -> Self {
        Self {
            positions: ThinVec::new(),
            normals:   ThinVec::new(),
            tangents:  ThinVec::new(),
            uv_sets:   ThinVec::new(),
            colors:    ThinVec::new(),
            joints:    ThinVec::new(),
            weights:   ThinVec::new(),
        }
    }
}

/// Morph target — sparse-displacement attributes that get blended at runtime.
#[derive(Debug, Clone, Default)]
pub struct MorphTarget {
    pub positions: ThinVec<[f32; 3]>,
    pub normals:   ThinVec<[f32; 3]>,
    pub tangents:  ThinVec<[f32; 3]>,
}

/// Axis-aligned bounding box in primitive-local space, sourced from the
/// POSITION accessor's min/max when present, otherwise computed.
#[derive(Debug, Clone, Copy)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub fn from_positions(positions: &[[f32; 3]]) -> Self {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for p in positions {
            for i in 0..3 {
                if p[i] < min[i] { min[i] = p[i]; }
                if p[i] > max[i] { max[i] = p[i]; }
            }
        }
        if positions.is_empty() { min = [0.0; 3]; max = [0.0; 3]; }
        Self { min, max }
    }

    pub fn center(&self) -> [f32; 3] {
        [
            0.5 * (self.min[0] + self.max[0]),
            0.5 * (self.min[1] + self.max[1]),
            0.5 * (self.min[2] + self.max[2]),
        ]
    }

    pub fn half_extents(&self) -> [f32; 3] {
        [
            0.5 * (self.max[0] - self.min[0]),
            0.5 * (self.max[1] - self.min[1]),
            0.5 * (self.max[2] - self.min[2]),
        ]
    }
}

#[derive(Debug, Clone)]
pub struct Primitive {
    pub topology: PrimitiveTopology,
    pub streams:  VertexStreams,
    pub indices:  ThinVec<u32>,
    pub material: Option<u32>,
    pub morph_targets: ThinVec<MorphTarget>,
    pub bounds:   Aabb,
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub name:       Option<String>,
    pub primitives: ThinVec<Primitive>,
    /// Default morph-target weights (used when no node override).
    pub weights:    ThinVec<f32>,
}
