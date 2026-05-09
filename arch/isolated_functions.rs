// Isolated function-level benchmarks.
//
// Each benchmark targets a single public function so regressions can be
// pinpointed to the exact call site, unlike game_tick.rs which measures
// composite passes. Also surfaces PlayStats counters per bench for workload
// visibility.
//
//   cargo bench --bench isolated_functions

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use glam::{Affine3A, Vec3};
use std::sync::Arc;
use thin_vec::thin_vec;

use dumpster_fire_engine::resource_manager::*;

const DT: f32 = 1.0 / 60.0;

// ── Shared world builders ──────────────────────────────────────────────────

/// Minimal single-stage world with `n` actors, each carrying a Character
/// sub-entity (Physics + Transform) and an Item sub-entity (Collision).
/// Returns (world, level_handle, stage_handle, actor_handles).
fn build_flat_world(n: usize) -> (World, LevelHandle, StageHandle, Vec<ActorHandle>) {
    let mut world = World::new(WorldId::new(1));
    let lh = world.spawn_level(LevelId::new(1), "bench_level");
    let sh = world.spawn_stage(lh, StageId::new(1), "bench_stage").unwrap();

    let mut actors = Vec::with_capacity(n);
    for i in 0..n {
        let ah = world.spawn_actor(
            lh, sh, ActorId::new(i as i64 + 1),
            Affine3A::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
        ).unwrap();

        let cvi = world.spawn_sub_entity(
            lh, sh, ah,
            ActorType::Character(Character {
                id: CharacterId::new(i as i64 + 1),
                name: format!("c{i}").into(),
                visible: true, physical: true, playable: false,
            }),
            Affine3A::IDENTITY,
        ).unwrap();

        world.add_component(lh, sh, ah, cvi, PhysicsComponent {
            mass: 70.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, -9.8, 0.0),
        });
        world.add_component(lh, sh, ah, cvi, TransformComponent {
            position: (i as f32, 0.0, 0.0), rotation: (0.0, 0.0, 0.0),
            scale: (1.0, 1.0, 1.0), _transform: true,
        });

        let ivi = world.spawn_sub_entity(
            lh, sh, ah,
            ActorType::Item(Item {
                id: ItemId::new(i as i64 + 1000),
                name: "trinket".into(),
                visible: true, physical: false,
            }),
            Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        ).unwrap();
        world.add_component(lh, sh, ah, ivi, CollisionComponent {
            shape: CollisionShape::Sphere,
            position: (0.0, 1.0, 0.0), rotation: (0.0, 0.0, 0.0),
            scale: (0.3, 0.3, 0.3), collision: true,
        });

        actors.push(ah);
    }
    (world, lh, sh, actors)
}

/// Build a world with a Play bound (for event_manager benches).
fn build_world_with_play(n: usize) -> (World, LevelHandle, StageHandle, Vec<ActorHandle>) {
    let (mut world, lh, sh, actors) = build_flat_world(n);

    let stage_id = world.levels[lh].stages[sh].id;
    let troupe_all = TroupeId::new(1);
    let all_actors: Vec<ActiveActor> = actors.iter().enumerate()
        .map(|(i, &ah)| ActiveActor::new(lh, sh, ah, ActorId::new(i as i64 + 1)))
        .collect();

    let bt_actor = actors[0];

    // Simple script: Compound root → two Atomic children (walk ↔ action)
    let s_root   = SceneId::new(1);
    let s_walk   = SceneId::new(2);
    let s_action = SceneId::new(3);

    let make_bt = || BtNode::Sequence(vec![
        BtNode::leaf(
            Condition::Always,
            Effect::SetActorLocal {
                level_h: lh, stage_h: sh, actor_h: bt_actor,
                local: Affine3A::from_translation(Vec3::new(0.01, 0.0, 0.0)),
            },
            false,
        ),
        BtNode::leaf(
            Condition::Always,
            Effect::CueTroupe {
                level_h: lh, stage_h: sh, troupe: troupe_all,
                delta: Affine3A::from_translation(Vec3::new(0.001, 0.0, 0.0)),
            },
            false,
        ),
    ]);

    let walk = SceneDef {
        id: s_walk, stage: stage_id, parent: Some(s_root),
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe_all],
        initial_actors: thin_vec![all_actors.iter().cloned().collect()],
        root: make_bt(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![],
        transitions: thin_vec![Transition {
            condition: Condition::AfterSeconds(1.0),
            target: s_action, effects: Arc::default(),
        }],
    };

    let action = SceneDef {
        id: s_action, stage: stage_id, parent: Some(s_root),
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe_all],
        initial_actors: thin_vec![all_actors.iter().cloned().collect()],
        root: make_bt(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![],
        transitions: thin_vec![Transition {
            condition: Condition::AfterSeconds(1.0),
            target: s_walk, effects: Arc::default(),
        }],
    };

    let root = SceneDef {
        id: s_root, stage: stage_id, parent: None,
        kind: SceneKind::Compound {
            children: thin_vec![s_walk, s_action],
            initial: s_walk, history: None,
        },
        troupes: thin_vec![], initial_actors: thin_vec![],
        root: BtNode::empty(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    };

    let mut script = Script::new(ScriptId::new(1), "bench_script", s_root);
    script.add_scene(root);
    script.add_scene(walk);
    script.add_scene(action);

    let play = Play::instantiate(
        PlayId::new(1), "bench_play", &script, stage_id, lh, sh,
    );
    world.levels[lh].stages[sh].set_play(play);

    // Warmup past first transition
    for _ in 0..120 { world.tick(DT); }

    (world, lh, sh, actors)
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Transform operations (Stage-level SoA)
// ═══════════════════════════════════════════════════════════════════════════

/// set_actor_local: O(1) SoA write + conditional dirty-list push.
fn bench_set_actor_local(c: &mut Criterion) {
    let mut g = c.benchmark_group("set_actor_local");
    for &n in &[100, 1000, 10_000] {
        let (mut world, lh, sh, actors) = build_flat_world(n);
        // Clear dirty state from spawns
        world.propagate_transforms();

        let delta = Affine3A::from_translation(Vec3::new(0.01, 0.0, 0.0));
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                for &ah in actors.iter() {
                    world.set_actor_local(lh, sh, ah, black_box(delta));
                }
                // Re-propagate so dirty list is drained for next iteration
                world.propagate_transforms();
            });
        });
    }
    g.finish();
}

/// propagate_transforms at Stage level with varying dirty fractions.
fn bench_propagate_dirty_fraction(c: &mut Criterion) {
    let mut g = c.benchmark_group("propagate_transforms");
    let n = 10_000;

    for &pct in &[1, 10, 50, 100] {
        let (mut world, lh, sh, actors) = build_flat_world(n);
        world.propagate_transforms(); // clear

        let dirty_count = n * pct / 100;
        let label = format!("{n}actors_{pct}pct_dirty");
        let delta = Affine3A::from_translation(Vec3::new(0.01, 0.0, 0.0));

        g.throughput(Throughput::Elements(dirty_count as u64));
        g.bench_function(&label, |b| {
            b.iter(|| {
                // Mark a fraction dirty
                for &ah in actors[..dirty_count].iter() {
                    world.set_actor_local(lh, sh, ah, delta);
                }
                world.propagate_transforms();
            });
        });
    }
    g.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Spawn / despawn cycle
// ═══════════════════════════════════════════════════════════════════════════

/// spawn_actor + despawn_actor round-trip (Arena insert/remove + SoA bookkeeping).
fn bench_spawn_despawn_actor(c: &mut Criterion) {
    let mut g = c.benchmark_group("spawn_despawn_actor");
    for &n in &[100, 1000] {
        let (mut world, lh, sh, _) = build_flat_world(0); // empty stage

        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut handles = Vec::with_capacity(n as usize);
                for i in 0..n as i64 {
                    let ah = world.spawn_actor(
                        lh, sh, ActorId::new(i + 1),
                        Affine3A::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
                    ).unwrap();
                    handles.push(ah);
                }
                for ah in handles {
                    world.despawn_actor(lh, sh, ah);
                }
            });
        });
    }
    g.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Component cache operations
// ═══════════════════════════════════════════════════════════════════════════

/// add_component: write component + cache.contains() + conditional push.
fn bench_add_remove_component(c: &mut Criterion) {
    let mut g = c.benchmark_group("component_cache");
    for &n in &[100, 1000] {
        // Build world with actors but no components initially
        let mut world = World::new(WorldId::new(1));
        let lh = world.spawn_level(LevelId::new(1), "bench_level");
        let sh = world.spawn_stage(lh, StageId::new(1), "bench_stage").unwrap();

        let mut actor_data: Vec<(ActorHandle, usize)> = Vec::with_capacity(n as usize);
        for i in 0..n as i64 {
            let ah = world.spawn_actor(
                lh, sh, ActorId::new(i + 1),
                Affine3A::IDENTITY,
            ).unwrap();
            let vi = world.spawn_sub_entity(
                lh, sh, ah,
                ActorType::Character(Character {
                    id: CharacterId::new(i + 1),
                    name: format!("c{i}").into(),
                    visible: true, physical: true, playable: false,
                }),
                Affine3A::IDENTITY,
            ).unwrap();
            actor_data.push((ah, vi));
        }

        g.throughput(Throughput::Elements(n));
        g.bench_with_input(BenchmarkId::new("add_then_remove", n), &n, |b, _| {
            b.iter(|| {
                // Add Physics to every actor
                for &(ah, vi) in actor_data.iter() {
                    world.add_component(lh, sh, ah, vi, PhysicsComponent {
                        mass: 70.0, velocity: (0.0, 0.0, 0.0),
                        acceleration: (0.0, -9.8, 0.0),
                    });
                }
                // Remove Physics from every actor
                for &(ah, vi) in actor_data.iter() {
                    world.remove_component::<PhysicsComponent>(lh, sh, ah, vi);
                }
            });
        });
    }
    g.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. apply_effect dispatch — individual effect variants
// ═══════════════════════════════════════════════════════════════════════════

fn bench_apply_effect(c: &mut Criterion) {
    let mut g = c.benchmark_group("apply_effect");
    let n = 1000;
    let (mut world, lh, sh, actors) = build_flat_world(n);
    world.propagate_transforms();

    let delta = Affine3A::from_translation(Vec3::new(0.01, 0.0, 0.0));

    // SetActorLocal — the most common effect variant.
    g.throughput(Throughput::Elements(n as u64));
    g.bench_function("SetActorLocal", |b| {
        b.iter(|| {
            for &ah in actors.iter() {
                world.apply_effect(Effect::SetActorLocal {
                    level_h: lh, stage_h: sh, actor_h: ah, local: delta,
                });
            }
            world.propagate_transforms();
        });
    });

    // CueTroupe — requires a Play with active actors.
    let (mut world2, lh2, sh2, _actors2) = build_world_with_play(500);
    g.throughput(Throughput::Elements(500));
    g.bench_function("CueTroupe", |b| {
        b.iter(|| {
            world2.apply_effect(Effect::CueTroupe {
                level_h: lh2, stage_h: sh2,
                troupe: TroupeId::new(1),
                delta: Affine3A::from_translation(Vec3::new(0.001, 0.0, 0.0)),
            });
            world2.propagate_transforms();
        });
    });

    g.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. cue_troupe_direct — identity vs delta, varying roster size
// ═══════════════════════════════════════════════════════════════════════════

fn bench_cue_troupe_direct(c: &mut Criterion) {
    let mut g = c.benchmark_group("cue_troupe_direct");

    for &n in &[100, 500, 2000] {
        let (mut world, lh, sh, _) = build_world_with_play(n);
        let delta = Affine3A::from_translation(Vec3::new(0.001, 0.0, 0.0));
        let troupe = TroupeId::new(1);

        g.throughput(Throughput::Elements(n as u64));

        g.bench_with_input(BenchmarkId::new("delta", n), &n, |b, _| {
            b.iter(|| {
                world.levels[lh].stages[sh].cue_troupe_direct(troupe, black_box(delta));
                world.propagate_transforms();
            });
        });

        // Re-build for identity test (clean state)
        let (mut world2, lh2, sh2, _) = build_world_with_play(n);
        g.bench_with_input(BenchmarkId::new("identity", n), &n, |b, _| {
            b.iter(|| {
                world2.levels[lh2].stages[sh2].cue_troupe_direct(
                    troupe, black_box(Affine3A::IDENTITY),
                );
                world2.propagate_transforms();
            });
        });
    }
    g.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. collect_effects — isolated read-only BT walk
// ═══════════════════════════════════════════════════════════════════════════

fn bench_collect_effects_isolated(c: &mut Criterion) {
    let mut g = c.benchmark_group("collect_effects_isolated");

    for &n in &[100, 500, 2000] {
        let (world, lh, _sh, _) = build_world_with_play(n);

        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let mut sink: Vec<Effect> = Vec::new();
                let mut chain: Vec<SceneHandle> = Vec::with_capacity(8);
                for level in world.levels.values() {
                    level.collect_effects(black_box(DT), &world, &mut sink, &mut chain);
                }
                black_box(sink.len());
            });
        });
    }
    g.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. post_tick_bookkeeping — isolated mut pass
// ═══════════════════════════════════════════════════════════════════════════

fn bench_post_tick_isolated(c: &mut Criterion) {
    let mut g = c.benchmark_group("post_tick_isolated");

    for &n in &[100, 500, 2000] {
        let (mut world, _lh, _sh, _) = build_world_with_play(n);

        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                for level in world.levels.values_mut() {
                    level.post_tick(black_box(DT));
                }
            });
        });
    }
    g.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. BtNode::tick — per-node-type cost in isolation
// ═══════════════════════════════════════════════════════════════════════════

fn bench_bt_node_tick(c: &mut Criterion) {
    let mut g = c.benchmark_group("bt_node_tick");

    // Build a minimal world for EvalCtx
    let (world, lh, sh, actors) = build_flat_world(10);
    let all_actors: Vec<ActiveActor> = actors.iter().enumerate()
        .map(|(i, &ah)| ActiveActor::new(lh, sh, ah, ActorId::new(i as i64 + 1)))
        .collect();

    let troupe = Troupe(vec![all_actors]);
    let troupes = vec![TroupeId::new(1)];
    let events: thin_vec::ThinVec<Event> = thin_vec![];

    let ctx = EvalCtx {
        world: &world,
        level_h: lh, stage_h: sh,
        scene_id: SceneId::new(1),
        elapsed: 1.0, tick_count: 60,
        events_seen: &events,
        actors: &troupe,
        troupes: &troupes,
    };

    let bt_actor = actors[0];
    let make_leaf = || BtNode::leaf(
        Condition::Always,
        Effect::SetActorLocal {
            level_h: lh, stage_h: sh, actor_h: bt_actor,
            local: Affine3A::from_translation(Vec3::new(0.01, 0.0, 0.0)),
        },
        false,
    );

    // Single leaf
    let leaf = make_leaf();
    g.bench_function("leaf_always", |b| {
        b.iter(|| {
            let mut out = Vec::new();
            leaf.tick(&ctx, &mut out);
            black_box(out);
        });
    });

    // Sequence(8 leaves)
    let seq = BtNode::Sequence((0..8).map(|_| make_leaf()).collect());
    g.throughput(Throughput::Elements(8));
    g.bench_function("sequence_8", |b| {
        b.iter(|| {
            let mut out = Vec::new();
            seq.tick(&ctx, &mut out);
            black_box(out);
        });
    });

    // Parallel(8 leaves)
    let par = BtNode::Parallel {
        children: (0..8).map(|_| make_leaf()).collect(),
        policy: ParallelPolicy::AllComplete,
    };
    g.bench_function("parallel_8", |b| {
        b.iter(|| {
            let mut out = Vec::new();
            par.tick(&ctx, &mut out);
            black_box(out);
        });
    });

    // Decorator(Guard(Always), leaf)
    let guarded = BtNode::Decorator {
        decorator: Decorator::Guard(Condition::Always),
        child: Arc::new(make_leaf()),
    };
    g.bench_function("decorator_guard", |b| {
        b.iter(|| {
            let mut out = Vec::new();
            guarded.tick(&ctx, &mut out);
            black_box(out);
        });
    });

    // Condition::AfterSeconds — the timer branch
    let timed_leaf = BtNode::leaf(
        Condition::AfterSeconds(0.5),
        Effect::SetActorLocal {
            level_h: lh, stage_h: sh, actor_h: bt_actor,
            local: Affine3A::IDENTITY,
        },
        false,
    );
    g.bench_function("leaf_after_seconds", |b| {
        b.iter(|| {
            let mut out = Vec::new();
            timed_leaf.tick(&ctx, &mut out);
            black_box(out);
        });
    });

    g.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// 9. Condition::eval — individual condition variant costs
// ═══════════════════════════════════════════════════════════════════════════

fn bench_condition_eval(c: &mut Criterion) {
    let mut g = c.benchmark_group("condition_eval");

    let (world, lh, sh, actors) = build_flat_world(100);
    let all_actors: Vec<ActiveActor> = actors.iter().enumerate()
        .map(|(i, &ah)| ActiveActor::new(lh, sh, ah, ActorId::new(i as i64 + 1)))
        .collect();
    let troupe = Troupe(vec![all_actors]);
    let troupes = vec![TroupeId::new(1)];
    let events: thin_vec::ThinVec<Event> = thin_vec![];

    let ctx = EvalCtx {
        world: &world,
        level_h: lh, stage_h: sh,
        scene_id: SceneId::new(1),
        elapsed: 1.0, tick_count: 60,
        events_seen: &events,
        actors: &troupe,
        troupes: &troupes,
    };

    g.bench_function("Always", |b| {
        let cond = Condition::Always;
        b.iter(|| black_box(cond.eval(&ctx)));
    });

    g.bench_function("AfterSeconds_true", |b| {
        let cond = Condition::AfterSeconds(0.5);
        b.iter(|| black_box(cond.eval(&ctx)));
    });

    g.bench_function("AfterSeconds_false", |b| {
        let cond = Condition::AfterSeconds(999.0);
        b.iter(|| black_box(cond.eval(&ctx)));
    });

    g.bench_function("ActorNear_hit", |b| {
        let cond = Condition::ActorNear {
            actor: ActorId::new(1),
            target: Vec3::new(0.0, 0.0, 0.0),
            radius: 100.0,
        };
        b.iter(|| black_box(cond.eval(&ctx)));
    });

    g.bench_function("ActorNear_miss", |b| {
        let cond = Condition::ActorNear {
            actor: ActorId::new(1),
            target: Vec3::new(99999.0, 0.0, 0.0),
            radius: 1.0,
        };
        b.iter(|| black_box(cond.eval(&ctx)));
    });

    g.bench_function("ActorHasComponent", |b| {
        let cond = Condition::ActorHasComponent {
            actor: ActorId::new(1),
            component_type: ComponentType::Physics,
        };
        b.iter(|| black_box(cond.eval(&ctx)));
    });

    g.bench_function("TroupeAll_100actors", |b| {
        let cond = Condition::TroupeAll {
            troupe: TroupeId::new(1),
            predicate: Arc::new(Condition::ActorHasComponent {
                actor: ActorId::new(1), // re-targeted per member
                component_type: ComponentType::Physics,
            }),
        };
        b.iter(|| black_box(cond.eval(&ctx)));
    });

    g.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// 10. PlayStats — verify counters are reporting after a tick
// ═══════════════════════════════════════════════════════════════════════════

fn bench_tick_with_stats(c: &mut Criterion) {
    let mut g = c.benchmark_group("tick_with_stats");
    let n = 500;
    let (mut world, lh, sh, _) = build_world_with_play(n);

    g.throughput(Throughput::Elements(n as u64));
    g.bench_function("full_tick_500", |b| {
        b.iter(|| {
            world.tick(black_box(DT));
        });
    });
    g.finish();

    // Print stats after the bench run for visibility
    let play = world.levels[lh].stages[sh].play.as_ref().unwrap();
    let s = play.stats.snapshot();
    eprintln!("\n── PlayStats after bench ──");
    eprintln!("  chain_steps:       {}", s.chain_steps);
    eprintln!("  scenes_processed:  {}", s.scenes_processed);
    eprintln!("  dedup_skips:       {}", s.dedup_skips);
    eprintln!("  transitions_fired: {}", s.transitions_fired);
    eprintln!("  bt_ticks:          {}", s.bt_ticks);
}

// ═══════════════════════════════════════════════════════════════════════════
// 11. Play::instantiate — one-time script materialization cost
// ═══════════════════════════════════════════════════════════════════════════

fn bench_play_instantiate(c: &mut Criterion) {
    let mut g = c.benchmark_group("play_instantiate");

    for &n in &[50, 500, 2000] {
        let (world, lh, sh, actors) = build_flat_world(n);
        let stage_id = world.levels[lh].stages[sh].id;

        let troupe_all = TroupeId::new(1);
        let all_actors: Vec<ActiveActor> = actors.iter().enumerate()
            .map(|(i, &ah)| ActiveActor::new(lh, sh, ah, ActorId::new(i as i64 + 1)))
            .collect();

        let bt_actor = actors[0];
        let s_root = SceneId::new(1);
        let s_walk = SceneId::new(2);
        let s_action = SceneId::new(3);

        let make_bt = || BtNode::Sequence(vec![
            BtNode::leaf(
                Condition::Always,
                Effect::SetActorLocal {
                    level_h: lh, stage_h: sh, actor_h: bt_actor,
                    local: Affine3A::from_translation(Vec3::new(0.01, 0.0, 0.0)),
                },
                false,
            ),
        ]);

        let mut script = Script::new(ScriptId::new(1), "bench", s_root);
        script.add_scene(SceneDef {
            id: s_root, stage: stage_id, parent: None,
            kind: SceneKind::Compound {
                children: thin_vec![s_walk, s_action],
                initial: s_walk, history: None,
            },
            troupes: thin_vec![], initial_actors: thin_vec![],
            root: BtNode::empty(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![], transitions: thin_vec![],
        });
        script.add_scene(SceneDef {
            id: s_walk, stage: stage_id, parent: Some(s_root),
            kind: SceneKind::Atomic,
            troupes: thin_vec![troupe_all],
            initial_actors: thin_vec![all_actors.iter().cloned().collect()],
            root: make_bt(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![],
            transitions: thin_vec![Transition {
                condition: Condition::AfterSeconds(1.0),
                target: s_action, effects: Arc::default(),
            }],
        });
        script.add_scene(SceneDef {
            id: s_action, stage: stage_id, parent: Some(s_root),
            kind: SceneKind::Atomic,
            troupes: thin_vec![troupe_all],
            initial_actors: thin_vec![all_actors.iter().cloned().collect()],
            root: make_bt(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![],
            transitions: thin_vec![Transition {
                condition: Condition::AfterSeconds(1.0),
                target: s_walk, effects: Arc::default(),
            }],
        });

        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let play = Play::instantiate(
                    PlayId::new(1), "bench_play", &script, stage_id, lh, sh,
                );
                black_box(play);
            });
        });
    }
    g.finish();
}

criterion_group!(benches,
    bench_set_actor_local,
    bench_propagate_dirty_fraction,
    bench_spawn_despawn_actor,
    bench_add_remove_component,
    bench_apply_effect,
    bench_cue_troupe_direct,
    bench_collect_effects_isolated,
    bench_post_tick_isolated,
    bench_bt_node_tick,
    bench_condition_eval,
    bench_tick_with_stats,
    bench_play_instantiate,
);
criterion_main!(benches);
