//! glTF bridge (`gltf` feature): `forge_gltf` primitive ↔ `HalfEdgeMesh`.
//!
//! Phase 0 carries positions + triangle topology (the editable core). Normal/UV/
//! attribute-layer round-trip is a documented later-phase addition.

use forge_gltf::mesh::{Aabb, Primitive, PrimitiveTopology, VertexStreams};
use thin_vec::ThinVec;

use crate::half_edge::{HalfEdgeMesh, MeshError};

/// Build an editable half-edge mesh from a triangle-topology glTF primitive.
pub fn from_primitive(p: &Primitive) -> Result<HalfEdgeMesh, MeshError> {
    if p.topology != PrimitiveTopology::Triangles {
        return Err(MeshError::Invalid("only Triangles topology is supported"));
    }
    HalfEdgeMesh::build_from_indexed(&p.streams.positions, &p.indices)
}

impl HalfEdgeMesh {
    /// Emit positions as a `VertexStreams` plus the triangle index buffer.
    pub fn to_vertex_streams(&self) -> (VertexStreams, ThinVec<u32>) {
        let (positions, indices) = self.to_indexed();
        let mut streams = VertexStreams::new();
        streams.positions = positions;
        (streams, indices)
    }

    /// Emit a triangle-topology glTF primitive (positions + recomputed AABB).
    pub fn to_primitive(&self) -> Primitive {
        let (streams, indices) = self.to_vertex_streams();
        let bounds = Aabb::from_positions(&streams.positions);
        Primitive {
            topology: PrimitiveTopology::Triangles,
            streams,
            indices,
            material: None,
            morph_targets: ThinVec::new(),
            bounds,
            custom_attrs: ThinVec::new(),
            variant_mappings: ThinVec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_primitive() -> Primitive {
        let mut streams = VertexStreams::new();
        streams.positions = ThinVec::from(vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]);
        Primitive {
            topology: PrimitiveTopology::Triangles,
            streams,
            indices: ThinVec::from(vec![0u32, 1, 2, 0, 2, 3]),
            material: None,
            morph_targets: ThinVec::new(),
            bounds: Aabb::from_positions(&[]),
            custom_attrs: ThinVec::new(),
            variant_mappings: ThinVec::new(),
        }
    }

    #[test]
    fn primitive_round_trip_preserves_positions_and_indices() {
        let p = quad_primitive();
        let m = from_primitive(&p).unwrap();
        m.validate().unwrap();
        let out = m.to_primitive();
        assert_eq!(out.streams.positions, p.streams.positions);
        assert_eq!(out.indices, p.indices);
        // AABB recomputed from positions: unit quad in z=0.
        assert_eq!(out.bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(out.bounds.max, [1.0, 1.0, 0.0]);
    }

    #[test]
    fn non_triangle_topology_rejected() {
        let mut p = quad_primitive();
        p.topology = PrimitiveTopology::Points;
        assert!(from_primitive(&p).is_err());
    }
}
