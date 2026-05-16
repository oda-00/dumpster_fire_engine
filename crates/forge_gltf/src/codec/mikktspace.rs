//! Thin wrapper around the `mikktspace` crate for per-spec tangent generation.
//!
//! When a glTF primitive has normals and at least one UV set but no TANGENT
//! attribute, this module computes MikkTSpace-compatible tangents exactly as
//! the spec mandates (section 3.7.2.1).

use thin_vec::ThinVec;

struct MikkMesh<'a> {
    positions: &'a [[f32; 3]],
    normals:   &'a [[f32; 3]],
    uvs:       &'a [[f32; 2]],
    indices:   &'a [u32],
    tangents:  &'a mut Vec<[f32; 4]>,
}

impl<'a> mikktspace::Geometry for MikkMesh<'a> {
    fn num_faces(&self) -> usize {
        self.indices.len() / 3
    }

    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }

    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.positions[self.indices[face * 3 + vert] as usize]
    }

    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.normals[self.indices[face * 3 + vert] as usize]
    }

    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.uvs[self.indices[face * 3 + vert] as usize]
    }

    fn set_tangent_encoded(&mut self, tangent: [f32; 4], face: usize, vert: usize) {
        let idx = self.indices[face * 3 + vert] as usize;
        if idx < self.tangents.len() {
            self.tangents[idx] = tangent;
        }
    }
}

/// Generate MikkTSpace tangents for a triangle-list primitive.
///
/// `positions`, `normals`, and `uvs` must all have length `vertex_count`.
/// `indices` must be a valid triangle list (length divisible by 3).
/// Returns a `ThinVec` of `[tx, ty, tz, sign]` with the same length as
/// `positions`.
pub fn generate_tangents(
    positions: &[[f32; 3]],
    normals:   &[[f32; 3]],
    uvs:       &[[f32; 2]],
    indices:   &[u32],
) -> ThinVec<[f32; 4]> {
    let n = positions.len();
    let mut tans: Vec<[f32; 4]> = vec![[1.0, 0.0, 0.0, 1.0]; n];

    if indices.len() < 3 || normals.len() != n || uvs.len() != n {
        return tans.into_iter().collect();
    }

    let mut mesh = MikkMesh {
        positions,
        normals,
        uvs,
        indices,
        tangents: &mut tans,
    };
    mikktspace::generate_tangents(&mut mesh);
    tans.into_iter().collect()
}
