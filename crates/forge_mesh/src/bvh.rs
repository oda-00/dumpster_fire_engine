//! A triangle BVH for ray-triangle picking and spatial queries — the foundation
//! for vertex/edge/face hit-testing (extending the editor's actor-level ray-AABB
//! to element level), box/lasso, and snapping. Median-split build; iterative
//! traversal with Möller–Trumbore intersection.

use thin_vec::ThinVec;

use crate::half_edge::HalfEdgeMesh;

#[derive(Clone, Copy, Debug)]
struct Node {
    min: [f32; 3],
    max: [f32; 3],
    /// Internal: index of the left child (right = left+1), `count == 0`.
    /// Leaf: `start` = first triangle in `tri`, `count` > 0.
    left_or_start: u32,
    count: u32,
}

pub struct Bvh {
    nodes: Vec<Node>,
    tri: Vec<u32>,         // triangle ids in leaf order
    verts: Vec<[u32; 3]>,  // triangle → vertex ids
    pos: Vec<[f32; 3]>,    // vertex positions (snapshot)
}

#[inline]
fn tri_bounds(p: &[[f32; 3]], v: [u32; 3]) -> ([f32; 3], [f32; 3]) {
    let (a, b, c) = (p[v[0] as usize], p[v[1] as usize], p[v[2] as usize]);
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for q in [a, b, c] {
        for i in 0..3 {
            lo[i] = lo[i].min(q[i]);
            hi[i] = hi[i].max(q[i]);
        }
    }
    (lo, hi)
}

impl Bvh {
    pub fn build(pos: &[[f32; 3]], verts: &[[u32; 3]]) -> Bvh {
        let n = verts.len();
        let mut tri: Vec<u32> = (0..n as u32).collect();
        let centroid: Vec<[f32; 3]> = verts
            .iter()
            .map(|&v| {
                let (lo, hi) = tri_bounds(pos, v);
                [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5]
            })
            .collect();

        let mut nodes: Vec<Node> = Vec::new();
        if n == 0 {
            nodes.push(Node { min: [0.0; 3], max: [0.0; 3], left_or_start: 0, count: 0 });
            return Bvh { nodes, tri, verts: verts.to_vec(), pos: pos.to_vec() };
        }

        // Iterative median split. Stack holds (node_index, tri_start, tri_count).
        nodes.push(Node { min: [0.0; 3], max: [0.0; 3], left_or_start: 0, count: 0 });
        let mut stack = vec![(0usize, 0usize, n)];
        const LEAF: usize = 4;
        while let Some((ni, start, count)) = stack.pop() {
            // Compute node bounds over [start, start+count).
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for &t in &tri[start..start + count] {
                let (tl, th) = tri_bounds(pos, verts[t as usize]);
                for i in 0..3 {
                    lo[i] = lo[i].min(tl[i]);
                    hi[i] = hi[i].max(th[i]);
                }
            }
            nodes[ni].min = lo;
            nodes[ni].max = hi;

            if count <= LEAF {
                nodes[ni].left_or_start = start as u32;
                nodes[ni].count = count as u32;
                continue;
            }
            // Split on the longest centroid axis at the median.
            let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
            let axis = if ext[0] >= ext[1] && ext[0] >= ext[2] {
                0
            } else if ext[1] >= ext[2] {
                1
            } else {
                2
            };
            let s = &mut tri[start..start + count];
            s.sort_unstable_by(|&a, &b| {
                centroid[a as usize][axis]
                    .partial_cmp(&centroid[b as usize][axis])
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mid = count / 2;
            let left = nodes.len();
            nodes.push(Node { min: [0.0; 3], max: [0.0; 3], left_or_start: 0, count: 0 });
            nodes.push(Node { min: [0.0; 3], max: [0.0; 3], left_or_start: 0, count: 0 });
            nodes[ni].left_or_start = left as u32;
            nodes[ni].count = 0;
            stack.push((left, start, mid));
            stack.push((left + 1, start + mid, count - mid));
        }

        Bvh { nodes, tri, verts: verts.to_vec(), pos: pos.to_vec() }
    }

    /// Closest ray hit: returns `(t, face_id)` for the nearest triangle, if any.
    pub fn raycast(&self, origin: [f32; 3], dir: [f32; 3]) -> Option<(f32, u32)> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = [
            1.0 / dir[0],
            1.0 / dir[1],
            1.0 / dir[2],
        ];
        let mut best_t = f32::INFINITY;
        let mut best_f = u32::MAX;
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let node = self.nodes[ni as usize];
            if !ray_aabb(origin, inv, node.min, node.max, best_t) {
                continue;
            }
            if node.count > 0 {
                for k in 0..node.count {
                    let t = self.tri[(node.left_or_start + k) as usize];
                    let v = self.verts[t as usize];
                    if let Some(hit) = ray_triangle(
                        origin,
                        dir,
                        self.pos[v[0] as usize],
                        self.pos[v[1] as usize],
                        self.pos[v[2] as usize],
                    ) {
                        if hit < best_t {
                            best_t = hit;
                            best_f = t;
                        }
                    }
                }
            } else {
                stack.push(node.left_or_start);
                stack.push(node.left_or_start + 1);
            }
        }
        (best_f != u32::MAX).then_some((best_t, best_f))
    }
}

#[inline]
fn ray_aabb(o: [f32; 3], inv: [f32; 3], lo: [f32; 3], hi: [f32; 3], tmax: f32) -> bool {
    let mut t0 = 0.0f32;
    let mut t1 = tmax;
    for i in 0..3 {
        let mut a = (lo[i] - o[i]) * inv[i];
        let mut b = (hi[i] - o[i]) * inv[i];
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t1 < t0 {
            return false;
        }
    }
    true
}

/// Möller–Trumbore; returns the ray parameter `t` of a front-or-back hit.
#[inline]
fn ray_triangle(o: [f32; 3], d: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<f32> {
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-8 {
        return None;
    }
    let inv = 1.0 / det;
    let tvec = [o[0] - a[0], o[1] - a[1], o[2] - a[2]];
    let u = dot(tvec, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(tvec, e1);
    let v = dot(d, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    (t > 1e-6).then_some(t)
}

#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl HalfEdgeMesh {
    /// Build a triangle BVH over the current faces.
    pub fn build_bvh(&self) -> Bvh {
        let verts: ThinVec<[u32; 3]> =
            (0..self.face_count() as u32).map(|f| self.face_verts(f)).collect();
        Bvh::build(&self.pos, &verts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> HalfEdgeMesh {
        let pos = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = vec![0, 1, 2, 0, 2, 3];
        HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap()
    }

    #[test]
    fn ray_hits_expected_face() {
        let m = quad();
        let bvh = m.build_bvh();
        // Ray straight down at (0.7,0.2): inside triangle 0 (0,1,2).
        let hit = bvh.raycast([0.7, 0.2, 1.0], [0.0, 0.0, -1.0]);
        let (t, f) = hit.expect("should hit");
        assert!((t - 1.0).abs() < 1e-4);
        assert_eq!(f, 0);
        // (0.2,0.7) is inside triangle 1 (0,2,3).
        let (_t2, f2) = bvh.raycast([0.2, 0.7, 1.0], [0.0, 0.0, -1.0]).expect("hit");
        assert_eq!(f2, 1);
    }

    #[test]
    fn ray_miss_returns_none() {
        let m = quad();
        let bvh = m.build_bvh();
        assert!(bvh.raycast([5.0, 5.0, 1.0], [0.0, 0.0, -1.0]).is_none());
        // Pointing away from the quad.
        assert!(bvh.raycast([0.5, 0.5, 1.0], [0.0, 0.0, 1.0]).is_none());
    }

    #[test]
    fn bvh_handles_many_triangles() {
        // 200-triangle grid; cast a ray at a known cell and expect a hit.
        let n = 10u32;
        let w = n + 1;
        let mut pos = Vec::new();
        for y in 0..w {
            for x in 0..w {
                pos.push([x as f32, y as f32, 0.0]);
            }
        }
        let mut idx = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let i = y * w + x;
                idx.extend_from_slice(&[i, i + 1, i + w + 1, i, i + w + 1, i + w]);
            }
        }
        let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();
        let bvh = m.build_bvh();
        let hit = bvh.raycast([5.5, 5.5, 10.0], [0.0, 0.0, -1.0]);
        assert!(hit.is_some());
        assert!((hit.unwrap().0 - 10.0).abs() < 1e-3);
    }
}
