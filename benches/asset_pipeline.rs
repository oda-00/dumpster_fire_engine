// Benchmarks for the asset_manager: AssetArena fetch/evict/get + Pipeline
// BinaryHeap queue ordering and throughput.
//
//   cargo bench --bench asset_pipeline

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use dumpster_fire_engine::resource_manager::asset_manager::{
    AssetArena, AssetHandle, AssetId, AssetKind, AssetSource, AssetType, Audio, Fetcher, Mesh, Pipeline,
    QueueEntry, Texture, TitleText, Visual,
};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

// ── AssetArena ─────────────────────────────────────────────────────────────

fn make_kind(i: usize) -> AssetKind {
    let path: Arc<str> = "p".into();
    match i % 5 {
        0 => AssetKind::Texture(Texture { path }),
        1 => AssetKind::TitleText(TitleText { text: path }),
        2 => AssetKind::Visual(Visual { path }),
        3 => AssetKind::Audio(Audio { path }),
        _ => AssetKind::Mesh(Mesh { path }),
    }
}

fn bench_asset_fetch(c: &mut Criterion) {
    let mut g = c.benchmark_group("asset_fetch");
    for &n in &[64usize, 1024, 10_000] {
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut arena = Fetcher::new(AssetArena::new());
                for i in 0..n {
                    arena.fetch(AssetId::new(i as i64 + 1), make_kind(i));
                }
                black_box(arena);
            });
        });
    }
    g.finish();
}

fn bench_asset_evict(c: &mut Criterion) {
    let mut g = c.benchmark_group("asset_evict");

    // Mid-list eviction triggers cache_slot.retain() over the whole list.
    g.bench_function("mid_list_1024", |b| {
        b.iter(|| {
            let mut arena = Fetcher::new(AssetArena::new());
            let mut handles = ThinVec::with_capacity(1024);
            for i in 0..1024 {
                handles.push(arena.fetch(AssetId::new(i as i64 + 1), make_kind(i)));
            }
            // Evict the middle 256 entries — each retain scans the whole bucket.
            for h in handles.iter().skip(384).take(256) {
                arena.evict(*h);
            }
            black_box(arena);
        });
    });

    g.bench_function("tail_1024", |b| {
        b.iter(|| {
            let mut arena = Fetcher::new(AssetArena::new());
            let mut handles = ThinVec::with_capacity(1024);
            for i in 0..1024 {
                handles.push(arena.fetch(AssetId::new(i as i64 + 1), make_kind(i)));
            }
            // Evict the tail — retain scan still sees the whole bucket per evict.
            for h in handles.iter().rev().take(256) {
                arena.evict(*h);
            }
            black_box(arena);
        });
    });

    g.finish();
}

fn bench_asset_of_type(c: &mut Criterion) {
    let mut g = c.benchmark_group("asset_of_type");
    for &n in &[64usize, 1024, 10_000] {
        let mut arena = Fetcher::new(AssetArena::new());
        for i in 0..n {
            arena.fetch(AssetId::new(i as i64 + 1), make_kind(i));
        }
        g.throughput(Throughput::Elements((n / 5 + 1) as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &arena, |b, arena| {
            b.iter(|| {
                let slice = arena.of_type(black_box(AssetType::Texture));
                black_box(slice.len());
            });
        });
    }
    g.finish();
}

// ── Pipeline queue: BinaryHeap priority order + throughput ─────────────────

fn bench_pipeline_queue(c: &mut Criterion) {
    let mut g = c.benchmark_group("pipeline_queue");

    // Verify max-heap ordering: priority 5,3,9,1,7 → pops 9,7,5,3,1.
    g.bench_function("priority_order_correctness", |b| {
        b.iter(|| {
            let mut p = Pipeline::new();
            for &pri in &[5u32, 3, 9, 1, 7] {
                p.push_queue(QueueEntry::new(
                    pri,
                    AssetId::new(pri as i64),
                    AssetSource::Fetcher(0),
                    AssetHandle {
                        idx: 0,
                        generation: std::num::NonZeroU32::new(1).unwrap(),
                        _tag: std::marker::PhantomData,
                    },
                ));
            }
            let mut order: ThinVec<u32> = ThinVec::with_capacity(5);
            while let Some(e) = p.pop_queue() {
                order.push(e.priority);
            }
            assert_eq!(order, vec![9, 7, 5, 3, 1]);
            black_box(order);
        });
    });

    // Throughput: push N, pop all.
    for &n in &[16usize, 256, 4096] {
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::new("push_n_pop_all", n), &n, |b, &n| {
            b.iter(|| {
                let mut p = Pipeline::with_capacity(n);
                for i in 0..n {
                    p.push_queue(QueueEntry::new(
                        ((i * 2654435761usize) & 0xFFFF_FFFF) as u32,
                        AssetId::new(i as i64),
                        AssetSource::Fetcher(0),
                        AssetHandle {
                            idx: i as u32,
                            generation: std::num::NonZeroU32::new(1).unwrap(),
                            _tag: std::marker::PhantomData,
                        },
                    ));
                }
                let mut count = 0u64;
                while p.pop_queue().is_some() {
                    count += 1;
                }
                black_box(count);
            });
        });
    }

    g.finish();
}

// Control benchmark: same N items, sorted Vec rebuild on every pop (worst case).
// Demonstrates why BinaryHeap is the right tool.
fn bench_baseline_sorted_vec(c: &mut Criterion) {
    let mut g = c.benchmark_group("control_sorted_vec_vs_heap");

    for &n in &[256usize, 4096] {
        g.throughput(Throughput::Elements(n as u64));

        // BinaryHeap (the actual Pipeline approach).
        g.bench_with_input(BenchmarkId::new("binary_heap", n), &n, |b, &n| {
            b.iter(|| {
                let mut h: BinaryHeap<u32> = BinaryHeap::with_capacity(n);
                for i in 0..n {
                    h.push(((i * 2654435761usize) & 0xFFFF_FFFF) as u32);
                }
                let mut count = 0u64;
                while h.pop().is_some() {
                    count += 1;
                }
                black_box(count);
            });
        });

        // Min-heap variant via Reverse, for comparison.
        g.bench_with_input(BenchmarkId::new("min_heap_via_reverse", n), &n, |b, &n| {
            b.iter(|| {
                let mut h: BinaryHeap<Reverse<u32>> = BinaryHeap::with_capacity(n);
                for i in 0..n {
                    h.push(Reverse(((i * 2654435761usize) & 0xFFFF_FFFF) as u32));
                }
                let mut count = 0u64;
                while h.pop().is_some() {
                    count += 1;
                }
                black_box(count);
            });
        });

        // Sorted-ThinVec — push then sort, drain in order.
        g.bench_with_input(BenchmarkId::new("sorted_ThinVec_drain", n), &n, |b, &n| {
            b.iter(|| {
                let mut v: ThinThinVec<u32> = Vec::with_capacity(n);
                for i in 0..n {
                    v.push(((i * 2654435761usize) & 0xFFFF_FFFF) as u32);
                }
                v.sort_unstable_by(|a, b| b.cmp(a)); // max-first
                let mut count = 0u64;
                for _ in v.drain(..) {
                    count += 1;
                }
                black_box(count);
            });
        });
    }
    g.finish();
}

criterion_group!(
    benches,
    bench_asset_fetch,
    bench_asset_evict,
    bench_asset_of_type,
    bench_pipeline_queue,
    bench_baseline_sorted_vec,
);
criterion_main!(benches);
