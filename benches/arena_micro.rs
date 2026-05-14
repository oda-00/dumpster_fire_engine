// Microbenchmarks for the project's `Arena<Tag, T>` (manager.rs).
// Hot ops: insert/get/get_mut/remove. Cold: full iteration.
//
//   cargo bench --bench arena_micro

use divan::{black_box, Bencher};
use dumpster_fire_engine::resource_manager::*;

fn main() { divan::main(); }

const N: usize = 10_000;

fn build_full() -> (Arena<ActorTag, u64>, Vec<Handle<ActorTag>>) {
    let mut a: Arena<ActorTag, u64> = Arena::with_capacity(N);
    let h: ThinVec<_> = (0..N).map(|i| a.insert(i as u64)).collect();
    (a, h)
}

fn build_with_freelist() -> (Arena<ActorTag, u64>, Vec<Handle<ActorTag>>) {
    let (mut a, handles) = build_full();
    for (i, h) in handles.iter().enumerate() {
        if i % 2 == 0 { a.remove(*h); }
    }
    (a, handles)
}

#[divan::bench]
fn insert_fresh(b: Bencher) {
    b.bench_local(|| {
        let mut a: Arena<ActorTag, u64> = Arena::with_capacity(N);
        for i in 0..N { black_box(a.insert(i as u64)); }
        a
    });
}

#[divan::bench]
fn insert_freelist(b: Bencher) {
    b.with_inputs(build_with_freelist).bench_local_values(|(mut a, _)| {
        for i in 0..(N / 2) { black_box(a.insert(i as u64)); }
        a
    });
}

#[divan::bench]
fn remove_live(b: Bencher) {
    b.with_inputs(build_full).bench_local_values(|(mut a, h)| {
        for handle in h { black_box(a.remove(handle)); }
        a
    });
}

#[divan::bench]
fn remove_stale(b: Bencher) {
    b.with_inputs(|| {
        let (mut a, h) = build_full();
        for &handle in &h { a.remove(handle); }
        (a, h)
    }).bench_local_values(|(mut a, h)| {
        for handle in h { black_box(a.remove(handle)); }
        a
    });
}

#[divan::bench]
fn get_hit(b: Bencher) {
    let (a, handles) = build_full();
    b.bench_local(|| {
        let mut sum = 0u64;
        for h in &handles { sum = sum.wrapping_add(*a.get(*h).unwrap()); }
        sum
    });
}

#[divan::bench]
fn get_miss(b: Bencher) {
    let (mut a, handles) = build_full();
    for &h in &handles { a.remove(h); }
    b.bench_local(|| {
        let mut hits = 0u64;
        for h in &handles {
            if a.get(*h).is_some() { hits += 1; }
        }
        hits
    });
}

#[divan::bench]
fn values_iter_full(b: Bencher) {
    let (a, _) = build_full();
    b.bench_local(|| {
        let mut sum = 0u64;
        for &v in a.values() { sum = sum.wrapping_add(v); }
        sum
    });
}

#[divan::bench]
fn values_iter_sparse_50pct(b: Bencher) {
    let (a, _) = build_with_freelist();
    b.bench_local(|| {
        let mut sum = 0u64;
        for &v in a.values() { sum = sum.wrapping_add(v); }
        sum
    });
}
