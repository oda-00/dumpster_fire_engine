//! Phase-1 scoreboard: branchless, rayon-parallel box-select + count at the scale
//! where Blender's (single-threaded) selection falls over (~20M elements).

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use forge_mesh::select::{box_select_into, count_selected};

fn bench(c: &mut Criterion) {
    let n = 20_000_000usize;
    let pos: Vec<[f32; 3]> = (0..n)
        .map(|i| {
            let f = i as f32;
            [f % 1000.0, (f * 0.000_1) % 1000.0, 0.0]
        })
        .collect();
    let mut flags = vec![0u8; n];

    c.bench_function("box_select 20M verts (rayon)", |b| {
        b.iter(|| {
            box_select_into(
                black_box(&pos),
                black_box(&mut flags),
                [0.0, 0.0, -1.0],
                [500.0, 500.0, 1.0],
                false,
            );
        });
    });

    c.bench_function("count_selected 20M (rayon)", |b| {
        b.iter(|| black_box(count_selected(black_box(&flags))));
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
