read al of it
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use slotmap::{DenseSlotMap, SlotMap, new_key_type};

new_key_type! { struct NodeKey; }

const N: usize = 10_000;

#[derive(Clone, Copy)]
struct Node {
    pos:         (f32, f32, f32),
    rot:         (f32, f32, f32),
    scale:       (f32, f32, f32),
    world_pos:   (f32, f32, f32),
    world_rot:   (f32, f32, f32),
    world_scale: (f32, f32, f32),
    parent:      u64,
    id:          u64,
    dirty:       bool,
    _pad:        [u8; 55],
}

fn make_node(i: usize) -> Node {
    Node {
        pos:         (i as f32, 0.0, 0.0),
        rot:         (0.0, 0.0, 0.0),
        scale:       (1.0, 1.0, 1.0),
        world_pos:   (0.0, 0.0, 0.0),
        world_rot:   (0.0, 0.0, 0.0),
        world_scale: (1.0, 1.0, 1.0),
        parent:      i as u64,
        id:          i as u64,
        dirty:       i % 4 == 0,
        _pad:        [0; 55],
    }
}

fn build_full_slotmap() -> SlotMap<NodeKey, Node> {
    let mut m = SlotMap::with_key();
    for i in 0..N { m.insert(make_node(i)); }
    m
}

fn build_full_dense() -> DenseSlotMap<NodeKey, Node> {
    let mut m = DenseSlotMap::with_key();
    for i in 0..N { m.insert(make_node(i)); }
    m
}

fn build_half_deleted_slotmap() -> SlotMap<NodeKey, Node> {
    let mut m: SlotMap<NodeKey, Node> = SlotMap::with_key();
    let keys: ThinVec<_> = (0..N).map(|i| m.insert(make_node(i))).collect();
    for (i, k) in keys.iter().enumerate() {
        if i % 2 == 0 { m.remove(*k); }
    }
    m
}

fn build_half_deleted_dense() -> DenseSlotMap<NodeKey, Node> {
    let mut m: DenseSlotMap<NodeKey, Node> = DenseSlotMap::with_key();
    let keys: ThinVec<_> = (0..N).map(|i| m.insert(make_node(i))).collect();
    for (i, k) in keys.iter().enumerate() {
        if i % 2 == 0 { m.remove(*k); }
    }
    m
}

fn bench_iterate_full(c: &mut Criterion) {
    let sm = build_full_slotmap();
    let dsm = build_full_dense();

    let mut g = c.benchmark_group("iterate_full_10k");
    g.bench_function("slotmap/values", |b| {
        b.iter(|| {
            let mut acc = 0.0_f32;
            for n in sm.values() { acc += n.pos.0; }
            black_box(acc)
        })
    });
    g.bench_function("dense_slotmap/values", |b| {
        b.iter(|| {
            let mut acc = 0.0_f32;
            for n in dsm.values() { acc += n.pos.0; }
            black_box(acc)
        })
    });
    g.finish();
}

fn bench_iterate_sparse(c: &mut Criterion) {
    let sm = build_half_deleted_slotmap();
    let dsm = build_half_deleted_dense();

    let mut g = c.benchmark_group("iterate_half_deleted_10k");
    g.bench_function("slotmap/values", |b| {
        b.iter(|| {
            let mut acc = 0.0_f32;
            for n in sm.values() { acc += n.pos.0; }
            black_box(acc)
        })
    });
    g.bench_function("dense_slotmap/values", |b| {
        b.iter(|| {
            let mut acc = 0.0_f32;
            for n in dsm.values() { acc += n.pos.0; }
            black_box(acc)
        })
    });
    g.finish();
}

fn bench_iterate_mut(c: &mut Criterion) {
    let mut sm = build_full_slotmap();
    let mut dsm = build_full_dense();

    let mut g = c.benchmark_group("iterate_mut_10k_propagation");
    g.bench_function("slotmap/values_mut", |b| {
        b.iter(|| {
            for n in sm.values_mut() {
                n.world_pos = (
                    n.pos.0 * n.scale.0,
                    n.pos.1 * n.scale.1,
                    n.pos.2 * n.scale.2,
                );
            }
        })
    });
    g.bench_function("dense_slotmap/values_mut", |b| {
        b.iter(|| {
            for n in dsm.values_mut() {
                n.world_pos = (
                    n.pos.0 * n.scale.0,
                    n.pos.1 * n.scale.1,
                    n.pos.2 * n.scale.2,
                );
            }
        })
    });
    g.finish();
}

criterion_group!(benches, bench_iterate_full, bench_iterate_sparse, bench_iterate_mut);
criterion_main!(benches);
