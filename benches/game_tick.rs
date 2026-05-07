// Realistic-game-shaped tick benchmark using criterion.
//
// Builds a populated world (multi-level, multi-stage, many actors per stage,
// each with sub-entities and components) and binds a non-trivial HSM Script
// per Stage that ticks a BT every frame, cues troupes, transitions between
// scenes, and walks AND-parallel regions. No assets or rendering — measures
// the engine's tick cost across all currently-implemented features.
//
//   cargo bench --bench game_tick

use std::time::{Duration, Instant};

use criterion::{black_box, BenchmarkId, Criterion, Throughput};
use glam::{Affine3A, Vec3};
use thin_vec::thin_vec;

use dumpster_fire_engine::resource_manager::*;

// ── Configuration ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Scale {
    levels:           usize,
    stages_per_level: usize,
    actors_per_stage: usize,
    bt_leaves:        usize,
}

impl Scale {
    const fn small()  -> Self { Scale { levels: 1, stages_per_level: 1, actors_per_stage: 16,  bt_leaves: 4 } }
    const fn medium() -> Self { Scale { levels: 2, stages_per_level: 2, actors_per_stage: 32,  bt_leaves: 6 } }
    const fn large()  -> Self { Scale { levels: 2, stages_per_level: 4, actors_per_stage: 64,  bt_leaves: 8 } }
    const fn xlarge() -> Self { Scale { levels: 4, stages_per_level: 4, actors_per_stage: 128, bt_leaves: 8 } }

    fn label(&self) -> String {
        let total = self.levels * self.stages_per_level * self.actors_per_stage;
        format!("{}L×{}S×{}A({}t)", self.levels, self.stages_per_level, self.actors_per_stage, total)
    }

    fn total_actors(&self) -> u64 {
        (self.levels * self.stages_per_level * self.actors_per_stage) as u64
    }
}

// ── World construction ─────────────────────────────────────────────────────

struct StageHandles {
    lh: LevelHandle,
    sh: StageHandle,
    actors: Vec<(ActorId, ActorHandle)>,
}

fn build_world(scale: Scale) -> (World, Vec<StageHandles>) {
    let mut world = World::new(WorldId::new(1));
    let mut handles = Vec::with_capacity(scale.levels * scale.stages_per_level);

    let mut sid_counter: i64 = 1;
    let mut actor_id_counter: i64 = 1;
    let mut character_id_counter: i64 = 1;

    for li in 0..scale.levels {
        let lh = world.spawn_level(LevelId::new(li as i64 + 1), format!("level_{li}"));

        for si in 0..scale.stages_per_level {
            let sh = world
                .spawn_stage(lh, StageId::new(sid_counter), format!("stage_{li}_{si}"))
                .unwrap();
            sid_counter += 1;

            let mut actors = Vec::with_capacity(scale.actors_per_stage);
            for ai in 0..scale.actors_per_stage {
                let aid = ActorId::new(actor_id_counter);
                actor_id_counter += 1;

                let ah = world
                    .spawn_actor(
                        lh, sh, aid,
                        Affine3A::from_translation(Vec3::new(ai as f32, 0.0, 0.0)),
                    )
                    .unwrap();

                let cvi = world.spawn_sub_entity(
                    lh, sh, ah,
                    ActorType::Character(Character {
                        id: CharacterId::new(character_id_counter),
                        name: format!("c{character_id_counter}").into(),
                        visible: true, physical: true, playable: false,
                    }),
                    Affine3A::IDENTITY,
                ).unwrap();
                character_id_counter += 1;

                world.add_component(lh, sh, ah, cvi, PhysicsComponent {
                    mass: 70.0,
                    velocity:     (0.0, 0.0, 0.0),
                    acceleration: (0.0, -9.8, 0.0),
                });
                world.add_component(lh, sh, ah, cvi, TransformComponent {
                    position: (ai as f32, 0.0, 0.0),
                    rotation: (0.0, 0.0, 0.0),
                    scale:    (1.0, 1.0, 1.0),
                    _transform: true,
                });

                let ivi = world.spawn_sub_entity(
                    lh, sh, ah,
                    ActorType::Item(Item {
                        id: ItemId::new(actor_id_counter),
                        name: "trinket".into(),
                        visible: true, physical: false,
                    }),
                    Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                ).unwrap();
                world.add_component(lh, sh, ah, ivi, CollisionComponent {
                    shape: CollisionShape::Sphere,
                    position: (0.0, 1.0, 0.0),
                    rotation: (0.0, 0.0, 0.0),
                    scale:    (0.3, 0.3, 0.3),
                    collision: true,
                });

                actors.push((aid, ah));
            }

            handles.push(StageHandles { lh, sh, actors });
        }
    }

    // Bind a Play to every Stage after the world is fully populated.
    for (idx, stage) in handles.iter().enumerate() {
        let stage_id = world.levels[stage.lh].stages[stage.sh].id;
        let script = build_script(scale, stage, stage_id);
        let play = Play::instantiate(
            PlayId::new(idx as i64 + 1),
            format!("play_{idx}"),
            &script,
            stage_id,
            stage.lh,
            stage.sh,
        );
        world.levels[stage.lh].stages[stage.sh].set_play(play);
    }

    (world, handles)
}

// ── Script construction ────────────────────────────────────────────────────
//
// Compound root cycling Walk → Action → Climax (AndParallel) → Walk.
// Every Atomic scene's body is per-tick work (Condition::Always, once=false)
// so each frame exercises BT walking + apply_effect.

fn build_script(scale: Scale, stage: &StageHandles, stage_id: StageId) -> Script {
    let s_root      = SceneId::new(1);
    let s_walk      = SceneId::new(2);
    let s_action    = SceneId::new(3);
    let s_climax    = SceneId::new(4);
    let s_climax_a  = SceneId::new(5);
    let s_climax_b  = SceneId::new(6);

    let troupe_lhs = TroupeId::new(1);
    let troupe_rhs = TroupeId::new(2);

    let (lhs, rhs) = stage.actors.split_at(stage.actors.len() / 2);
    let lhs_actors: Vec<ActiveActor> = lhs.iter()
        .map(|(id, h)| ActiveActor::new(stage.lh, stage.sh, *h, *id))
        .collect();
    let rhs_actors: Vec<ActiveActor> = rhs.iter()
        .map(|(id, h)| ActiveActor::new(stage.lh, stage.sh, *h, *id))
        .collect();

    let bt_actor = stage.actors[0].1;

    let make_per_tick_bt = || -> BtNode {
        let mut nodes = Vec::with_capacity(scale.bt_leaves);
        for k in 0..scale.bt_leaves {
            let dx = (k as f32) * 0.01;
            nodes.push(BtNode::leaf(
                Condition::Always,
                Effect::SetActorLocal {
                    level_h: stage.lh, stage_h: stage.sh, actor_h: bt_actor,
                    local: Affine3A::from_translation(Vec3::new(dx, 0.0, 0.0)),
                },
                false,
            ));
        }
        BtNode::Sequence(nodes)
    };

    let walk = SceneDef {
        id: s_walk, stage: stage_id, parent: Some(s_root),
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe_lhs, troupe_rhs],
        initial_actors: thin_vec![
            lhs_actors.iter().cloned().collect(),
            rhs_actors.iter().cloned().collect(),
        ],
        root: BtNode::Parallel {
            children: vec![
                make_per_tick_bt(),
                BtNode::leaf(
                    Condition::Always,
                    Effect::CueTroupe {
                        level_h: stage.lh, stage_h: stage.sh,
                        troupe: troupe_lhs,
                        delta: Affine3A::from_translation(Vec3::new(0.001, 0.0, 0.0)),
                    },
                    false,
                ),
            ],
            policy: ParallelPolicy::AllComplete,
        },
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![],
        transitions: thin_vec![Transition {
            condition: Condition::AfterSeconds(0.5),
            target: s_action, effects: thin_vec![],
        }],
    };

    let action = SceneDef {
        id: s_action, stage: stage_id, parent: Some(s_root),
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe_lhs, troupe_rhs],
        initial_actors: thin_vec![
            lhs_actors.iter().cloned().collect(),
            rhs_actors.iter().cloned().collect(),
        ],
        root: BtNode::Sequence(vec![
            BtNode::leaf(
                Condition::Always,
                Effect::CueTroupe {
                    level_h: stage.lh, stage_h: stage.sh,
                    troupe: troupe_rhs,
                    delta: Affine3A::from_translation(Vec3::new(-0.001, 0.0, 0.0)),
                },
                false,
            ),
            make_per_tick_bt(),
        ]),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![],
        transitions: thin_vec![Transition {
            condition: Condition::AfterSeconds(1.0),
            target: s_climax, effects: thin_vec![],
        }],
    };

    let climax_a = SceneDef {
        id: s_climax_a, stage: stage_id, parent: Some(s_climax),
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe_lhs],
        initial_actors: thin_vec![lhs_actors.iter().cloned().collect()],
        root: BtNode::leaf(
            Condition::Always,
            Effect::CueTroupe {
                level_h: stage.lh, stage_h: stage.sh,
                troupe: troupe_lhs,
                delta: Affine3A::from_translation(Vec3::new(0.0, 0.001, 0.0)),
            },
            false,
        ),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    };

    let climax_b = SceneDef {
        id: s_climax_b, stage: stage_id, parent: Some(s_climax),
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe_rhs],
        initial_actors: thin_vec![rhs_actors.iter().cloned().collect()],
        root: BtNode::leaf(
            Condition::Always,
            Effect::CueTroupe {
                level_h: stage.lh, stage_h: stage.sh,
                troupe: troupe_rhs,
                delta: Affine3A::from_translation(Vec3::new(0.0, -0.001, 0.0)),
            },
            false,
        ),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    };

    let climax = SceneDef {
        id: s_climax, stage: stage_id, parent: Some(s_root),
        kind: SceneKind::AndParallel {
            regions: thin_vec![
                Region { children: thin_vec![s_climax_a], initial: s_climax_a, history: None },
                Region { children: thin_vec![s_climax_b], initial: s_climax_b, history: None },
            ],
        },
        troupes: thin_vec![], initial_actors: thin_vec![],
        root: BtNode::empty(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![],
        transitions: thin_vec![Transition {
            condition: Condition::AfterSeconds(0.5),
            target: s_walk, effects: thin_vec![],
        }],
    };

    let root = SceneDef {
        id: s_root, stage: stage_id, parent: None,
        kind: SceneKind::Compound {
            children: thin_vec![s_walk, s_action, s_climax],
            initial: s_walk, history: None,
        },
        troupes: thin_vec![], initial_actors: thin_vec![],
        root: BtNode::empty(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    };

    let mut script = Script::new(ScriptId::new(1), "stress_script", s_root);
    script.add_scene(root);
    script.add_scene(walk);
    script.add_scene(action);
    script.add_scene(climax);
    script.add_scene(climax_a);
    script.add_scene(climax_b);
    script
}

// ── Benchmarks ──────────────────────────────────────────────────────────────

const DT: f32 = 1.0 / 60.0;

/// Times the up-front cost of building the populated world + binding a Play
/// to every stage. Useful for level-load budgets.
fn bench_world_build(c: &mut Criterion) {
    let mut g = c.benchmark_group("world_build");
    for &scale in &[Scale::small(), Scale::medium(), Scale::large()] {
        g.throughput(Throughput::Elements(scale.total_actors()));
        g.bench_with_input(BenchmarkId::from_parameter(scale.label()), &scale, |b, &s| {
            b.iter(|| {
                let (w, h) = build_world(s);
                black_box((w, h));
            });
        });
    }
    g.finish();
}

/// Steady-state tick cost at the medium scale. The world is built once and
/// warmed up; criterion then times one tick per iteration. Throughput is
/// reported in actors per second so cross-scale comparisons normalise.
fn bench_tick_steady(c: &mut Criterion) {
    let scale = Scale::medium();
    let (mut world, _h) = build_world(scale);
    for _ in 0..120 { world.tick(DT); } // warmup past the first scene transition

    let mut g = c.benchmark_group("tick_steady_state");
    g.throughput(Throughput::Elements(scale.total_actors()));
    g.bench_function(scale.label(), |b| {
        b.iter(|| {
            world.tick(black_box(DT));
        });
    });
    g.finish();
}

/// One-tick cost across world sizes. Each input gets its own warmed-up world.
fn bench_tick_scaling(c: &mut Criterion) {
    let mut g = c.benchmark_group("tick_scaling");
    for &scale in &[Scale::small(), Scale::medium(), Scale::large(), Scale::xlarge()] {
        let (mut world, _h) = build_world(scale);
        for _ in 0..120 { world.tick(DT); }

        g.throughput(Throughput::Elements(scale.total_actors()));
        g.bench_with_input(BenchmarkId::from_parameter(scale.label()), &scale, |b, _| {
            b.iter(|| {
                world.tick(black_box(DT));
            });
        });
    }
    g.finish();
}

/// Per-pass breakdown — measures collect_effects (read-only walk) vs
/// post_tick (mut bookkeeping) vs propagate_transforms in isolation, so
/// callers can see which phase dominates.
fn bench_tick_phases(c: &mut Criterion) {
    let scale = Scale::medium();
    let (mut world, _h) = build_world(scale);
    for _ in 0..120 { world.tick(DT); }

    let mut g = c.benchmark_group("tick_phases");
    g.throughput(Throughput::Elements(scale.total_actors()));

    g.bench_function("collect_effects", |b| {
        b.iter(|| {
            let mut sink: Vec<Effect> = Vec::new();
            for level in world.levels.values() {
                level.collect_effects(black_box(DT), &world, &mut sink);
            }
            black_box(sink);
        });
    });

    g.bench_function("post_tick", |b| {
        b.iter(|| {
            for level in world.levels.values_mut() {
                level.post_tick(black_box(DT));
            }
        });
    });

    g.bench_function("propagate_transforms", |b| {
        b.iter(|| {
            world.propagate_transforms();
        });
    });

    g.bench_function("full_tick", |b| {
        b.iter(|| {
            world.tick(black_box(DT));
        });
    });

    g.finish();
}

// ── Custom realistic-game report ───────────────────────────────────────────
//
// Criterion gives you statistically-rigorous timings of one tick in isolation.
// This second pass measures a long sustained run the way a game would actually
// run: many ticks back-to-back with no resampling between them. We report
// p50/p95/p99 latency, throughput in ticks/sec, and the 60-Hz real-time budget.

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64) * p / 100.0).floor() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn fmt_dur(d: Duration) -> String {
    let ns = d.as_nanos();
    if ns >= 1_000_000      { format!("{:>8.3} ms", ns as f64 / 1e6) }
    else if ns >= 1_000     { format!("{:>8.3} µs", ns as f64 / 1e3) }
    else                    { format!("{:>8} ns", ns) }
}

fn realistic_report(scale: Scale, warmup_ticks: usize, measure_ticks: usize) {
    println!();
    println!("=== realistic-run report — scale {} ===", scale.label());

    let build_t0 = Instant::now();
    let (mut world, handles) = build_world(scale);
    let build_d = build_t0.elapsed();
    println!("build:    {} ({} stages with Plays)", fmt_dur(build_d), handles.len());

    let warmup_t0 = Instant::now();
    for _ in 0..warmup_ticks { world.tick(DT); }
    let warmup_d = warmup_t0.elapsed();
    println!("warmup:   {} ({} ticks → {}/tick)",
        fmt_dur(warmup_d), warmup_ticks,
        fmt_dur(warmup_d / warmup_ticks.max(1) as u32));

    let mut samples: Vec<Duration> = Vec::with_capacity(measure_ticks);
    let measure_t0 = Instant::now();
    for _ in 0..measure_ticks {
        let t = Instant::now();
        world.tick(DT);
        samples.push(t.elapsed());
    }
    let measure_d = measure_t0.elapsed();

    let total: Duration = samples.iter().sum();
    let mean = total / samples.len() as u32;
    let mut sorted = samples.clone();
    sorted.sort();
    let p50 = percentile(&sorted, 50.0);
    let p95 = percentile(&sorted, 95.0);
    let p99 = percentile(&sorted, 99.0);
    let min = *sorted.first().unwrap();
    let max = *sorted.last().unwrap();
    let throughput = measure_ticks as f64 / measure_d.as_secs_f64();
    let realtime_mult = throughput * DT as f64;

    println!();
    println!("ticks:    {}", measure_ticks);
    println!("wall:     {}", fmt_dur(measure_d));
    println!("rate:     {:>8.1} ticks/sec", throughput);
    println!("budget:   {:>8.2}× real-time at 60 Hz   ({:.1}% of 16.667 ms frame)",
        realtime_mult, mean.as_secs_f64() * 1000.0 / 16.667 * 100.0);
    println!();
    println!("per-tick latency");
    println!("  min:    {}", fmt_dur(min));
    println!("  mean:   {}", fmt_dur(mean));
    println!("  p50:    {}", fmt_dur(p50));
    println!("  p95:    {}", fmt_dur(p95));
    println!("  p99:    {}", fmt_dur(p99));
    println!("  max:    {}", fmt_dur(max));

    // Sentinel access to defeat dead-code elimination of the tick loop and
    // give the user evidence the bench actually ran.
    let sample_actor = handles[0].actors[0].1;
    let pos = world.levels[handles[0].lh].stages[handles[0].sh]
        .actors[sample_actor].world.translation;
    println!();
    println!("end-of-run sentinel actor world pos: {:?}", pos);
}

// ── Custom main: criterion suite, then realistic report ────────────────────

fn main() {
    let mut crit = Criterion::default().configure_from_args();
    bench_world_build(&mut crit);
    bench_tick_steady(&mut crit);
    bench_tick_scaling(&mut crit);
    bench_tick_phases(&mut crit);
    crit.final_summary();

    // Skip the realistic report when criterion is in --test mode (e.g. CI
    // dry runs `cargo test --benches`); otherwise the long sustained run
    // would dominate the test budget.
    let in_test_mode = std::env::args().any(|a| a == "--test");
    if !in_test_mode {
        realistic_report(Scale::medium(), 500, 5_000);
        realistic_report(Scale::large(),  500, 5_000);
    }
}
