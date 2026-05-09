// Microbenchmarks for component-cache and SubEntity component-array operations.
// - SubEntity::component / has_component (array indexing, hot)
// - Stage cache add/remove paths (cache hit-list maintenance)
// - Stage::cache slice scan (per-component-type query)
// - despawn cascades (Level/Stage cache eviction, cold)
//
//   cargo bench --bench cache_ops

use divan::{black_box, Bencher};
use glam::Affine3A;
use dumpster_fire_engine::resource_manager::*;

fn main() { divan::main(); }

const SIZES: &[usize] = &[64, 1024, 10_000];

fn build_world(n: usize) -> (World, LevelHandle, StageHandle, Vec<ActorHandle>) {
    let mut w = World::new(WorldId::new(1));
    let lh = w.spawn_level(LevelId::new(1), "L");
    let sh = w.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let actors: Vec<_> = (0..n).map(|i| {
        let aid = ActorId::new(i as i64 + 1);
        let ah = w.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();
        w.spawn_sub_entity(
            lh, sh, ah,
            ActorType::Character(Character {
                id: CharacterId::new(i as i64 + 1), name: "n".into(),
                visible: true, physical: true, playable: false,
            }),
            Affine3A::IDENTITY,
        );
        ah
    }).collect();
    w.propagate_transforms();
    (w, lh, sh, actors)
}

// ── SubEntity::component / has_component — fixed-array indexing ────────────

#[divan::bench]
fn subentity_component_lookup_hit(b: Bencher) {
    let (mut w, lh, sh, actors) = build_world(1024);
    for &ah in &actors {
        w.add_component(lh, sh, ah, 0, PhysicsComponent {
            mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
        });
    }
    b.bench_local(|| {
        let mut hits = 0u64;
        for &ah in &actors {
            let actor = &w.levels[lh].stages[sh].actors[ah];
            for sub in actor.sub_entities.iter().flatten() {
                if sub.component(black_box(ComponentType::Physics)).is_some() { hits += 1; }
            }
        }
        hits
    });
}

#[divan::bench]
fn subentity_component_lookup_miss(b: Bencher) {
    let (w, lh, sh, actors) = build_world(1024);
    b.bench_local(|| {
        let mut hits = 0u64;
        for &ah in &actors {
            let actor = &w.levels[lh].stages[sh].actors[ah];
            for sub in actor.sub_entities.iter().flatten() {
                if sub.component(black_box(ComponentType::Audio)).is_some() { hits += 1; }
            }
        }
        hits
    });
}

#[divan::bench]
fn subentity_has_component(b: Bencher) {
    let (mut w, lh, sh, actors) = build_world(1024);
    for &ah in &actors {
        w.add_component(lh, sh, ah, 0, PhysicsComponent {
            mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
        });
    }
    b.bench_local(|| {
        let mut hits = 0u64;
        for &ah in &actors {
            let actor = &w.levels[lh].stages[sh].actors[ah];
            for sub in actor.sub_entities.iter().flatten() {
                if sub.has_component(black_box(ComponentType::Physics)) { hits += 1; }
            }
        }
        hits
    });
}

// ── Stage cache: add_component (idempotent contains check) ─────────────────

#[divan::bench]
fn stage_add_component_first_time(b: Bencher) {
    // Each iteration: fresh world, add Physics to all 256 actors → cache pushes.
    b.with_inputs(|| build_world(256))
        .bench_local_values(|(mut w, lh, sh, actors)| {
            for &ah in &actors {
                w.add_component(lh, sh, ah, 0, PhysicsComponent {
                    mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
                });
            }
            (w, actors)
        });
}

#[divan::bench]
fn stage_add_component_idempotent(b: Bencher) {
    // Component already present → cache.contains returns true → no push.
    let (mut w, lh, sh, actors) = build_world(256);
    for &ah in &actors {
        w.add_component(lh, sh, ah, 0, PhysicsComponent {
            mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
        });
    }
    b.bench_local(|| {
        for &ah in &actors {
            // Re-add same component — cache.contains skips the push.
            w.add_component(lh, sh, ah, 0, PhysicsComponent {
                mass: 2.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
            });
        }
    });
}

// ── Stage::cache slice scan — "all actors with Physics" ────────────────────

#[divan::bench(args = SIZES)]
fn stage_cache_scan(b: Bencher, n: usize) {
    let (mut w, lh, sh, actors) = build_world(n);
    for &ah in &actors {
        w.add_component(lh, sh, ah, 0, PhysicsComponent {
            mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
        });
    }
    let stage = &w.levels[lh].stages[sh];
    b.bench_local(|| {
        let mut sum = 0u64;
        for &ah in stage.cache[ComponentType::Physics.index()].iter() {
            sum = sum.wrapping_add(ah.idx as u64);
        }
        sum
    });
}

// ── despawn cascades (cache eviction cost) ─────────────────────────────────

#[divan::bench(args = SIZES)]
fn level_despawn_actor_with_cache(b: Bencher, n: usize) {
    // Each iteration: build a stage of n actors with Physics, then despawn all.
    // Measures the level + stage cache.retain() cascade cost.
    b.with_inputs(|| {
        let (mut w, lh, sh, actors) = build_world(n);
        for &ah in &actors {
            w.add_component(lh, sh, ah, 0, PhysicsComponent {
                mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
            });
        }
        (w, lh, sh, actors)
    }).bench_local_values(|(mut w, lh, sh, actors)| {
        for ah in actors {
            w.despawn_actor(lh, sh, ah);
        }
        w
    });
}

#[divan::bench]
fn stage_despawn_subentity_partial_evict(b: Bencher) {
    // Spawn an actor with TWO sub-entities both carrying Physics.
    // Despawn one → cache eviction must NOT happen because the other still has Physics.
    // Then despawn the other → cache eviction triggers.
    b.with_inputs(|| {
        let mut w = World::new(WorldId::new(1));
        let lh = w.spawn_level(LevelId::new(1), "L");
        let sh = w.spawn_stage(lh, StageId::new(1), "S").unwrap();
        let aid = ActorId::new(1);
        let ah = w.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();
        let cvi = w.spawn_sub_entity(
            lh, sh, ah,
            ActorType::Character(Character {
                id: CharacterId::new(1), name: "c".into(),
                visible: true, physical: true, playable: false,
            }),
            Affine3A::IDENTITY,
        ).unwrap();
        let ivi = w.spawn_sub_entity(
            lh, sh, ah,
            ActorType::Item(Item {
                id: ItemId::new(1), name: "i".into(),
                visible: true, physical: false,
            }),
            Affine3A::IDENTITY,
        ).unwrap();
        w.add_component(lh, sh, ah, cvi, PhysicsComponent {
            mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
        });
        w.add_component(lh, sh, ah, ivi, PhysicsComponent {
            mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
        });
        (w, lh, sh, ah, cvi, ivi)
    }).bench_local_values(|(mut w, lh, sh, ah, cvi, ivi)| {
        w.despawn_sub_entity(lh, sh, ah, cvi); // partial: other still has Physics
        w.despawn_sub_entity(lh, sh, ah, ivi); // full: cache eviction triggers
        w
    });
}
