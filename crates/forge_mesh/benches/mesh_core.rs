//! Phase-0 baselines: half-edge build, one-ring traversal, and round-trip on a
//! ~1M-triangle grid. These are the numbers the EDITOR_research.md §9 scoreboard
//! measures future phases (and Blender) against.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use forge_mesh::HalfEdgeMesh;

/// An n×n quad grid (2·n² triangles), consistent CCW winding, open boundary.
fn grid(n: u32) -> (Vec<[f32; 3]>, Vec<u32>) {
    let w = n + 1;
    let mut pos = Vec::with_capacity((w * w) as usize);
    for y in 0..w {
        for x in 0..w {
            pos.push([x as f32, y as f32, 0.0]);
        }
    }
    let mut idx = Vec::with_capacity((n * n * 6) as usize);
    for y in 0..n {
        for x in 0..n {
            let i = y * w + x;
            let (a, b, c, d) = (i, i + 1, i + w + 1, i + w);
            idx.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    (pos, idx)
}

fn bench(c: &mut Criterion) {
    let (pos, idx) = grid(720); // ~1.04M triangles
    let tris = idx.len() / 3;

    c.bench_function(&format!("build_from_indexed ({tris} tris)"), |b| {
        b.iter(|| {
            black_box(HalfEdgeMesh::build_from_indexed(black_box(&pos), black_box(&idx)).unwrap());
        });
    });

    let m = HalfEdgeMesh::build_from_indexed(&pos, &idx).unwrap();

    c.bench_function("one_ring traversal (all verts)", |b| {
        b.iter(|| {
            let mut acc = 0u64;
            for v in 0..m.vertex_count() as u32 {
                m.for_each_outgoing(v, |h| acc += m.he_vert[h as usize] as u64);
            }
            black_box(acc)
        });
    });

    c.bench_function("to_indexed round-trip", |b| {
        b.iter(|| black_box(m.to_indexed()));
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
