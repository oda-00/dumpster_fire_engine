// Realistic-game-shaped tick benchmark using criterion.
//
// Builds a populated world (multi-level, multi-stage, many actors per stage,
// each with sub-entities and components) and binds a non-trivial HSM Script
// per Stage that ticks a BT every frame, cues troupes, transitions between
// scenes, and walks AND-parallel regions. No assets or rendering — measures
// the engine's tick cost across all currently-implemented features.
//
//   cargo bench --bench game_tick

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use glam::{Affine3A, Vec3};
use std::sync::Arc;
use thin_vec::{ThinVec, thin_vec};

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
    const fn small()  -> Self { Scale { levels: 1, stages_per_level: 1, actors_per_stage: 50,  bt_leaves: 4 } }
    const fn medium() -> Self { Scale { levels: 2, stages_per_level: 2, actors_per_stage: 500,  bt_leaves: 6 } }
    const fn large()  -> Self { Scale { levels: 2, stages_per_level: 4, actors_per_stage: 1000,  bt_leaves: 8 } }
    const fn xlarge() -> Self { Scale { levels: 4, stages_per_level: 4, actors_per_stage: 10000, bt_leaves: 8 } }

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
    actors: ThinVec<(ActorId, ActorHandle)>,
}

fn build_world(scale: Scale) -> (World, ThinVec<StageHandles>) {
    let mut world = World::new(WorldId::new(1));
    let mut handles = ThinVec::with_capacity(scale.levels * scale.stages_per_level);

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

            let mut actors = ThinVec::with_capacity(scale.actors_per_stage);
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
                        id:          ItemId::new(actor_id_counter),
                        name:        "trinket".into(),
                        quantity:    (1, 1, 1),
                        description: Arc::from(""),
                        stackable:   false,
                        visible:     true,
                        physical:    false,
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
    let lhs_actors: ThinVec<ActiveActor> = lhs.iter()
        .map(|(id, h)| ActiveActor::new(stage.lh, stage.sh, *h, *id))
        .collect();
    let rhs_actors: ThinVec<ActiveActor> = rhs.iter()
        .map(|(id, h)| ActiveActor::new(stage.lh, stage.sh, *h, *id))
        .collect();

    let bt_actor = stage.actors[0].1;

    let make_per_tick_bt = || -> BtNode {
<<<<<<< HEAD
        let mut nodes = ThinVec::with_capacity(scale.bt_leaves);
=======
        let mut nodes: ThinVec<BtNode> = ThinVec::with_capacity(scale.bt_leaves);
>>>>>>> 76b59223ea8f67890b57fab9a68963c98741f1ec
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
            children: thin_vec![
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
            target: s_action, effects: Arc::default(),
        }],
    };

    let action = SceneDef {
        id: s_action, stage: stage_id, parent: Some(s_root),
        kind: SceneKind::Atomic,
        troupes: thin_ThinThinVec![troupe_lhs, troupe_rhs],
        initial_actors: thin_ThinVec![
            lhs_actors.iter().cloned().collect(),
            rhs_actors.iter().cloned().collect(),
        ],
<<<<<<< HEAD
        root: BtNode::Sequence(ThinVec![
=======
        root: BtNode::Sequence(thin_vec![
>>>>>>> 76b59223ea8f67890b57fab9a68963c98741f1ec
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
            target: s_climax, effects: Arc::default(),
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
            target: s_walk, effects: Arc::default(),
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
            let mut sink: ThinVec<Effect> = ThinVec::new();
            let mut chain: ThinVec<SceneHandle> = ThinVec::with_capacity(8);
            for level in world.levels.values() {
                level.collect_effects(black_box(DT), &world, &mut sink, &mut chain);
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

// ── Transition-storm world ─────────────────────────────────────────────────
//
// Every scene has a Condition::Always transition back to its sibling so
// apply_transition fires on every tick. This isolates the cost of the HSM
// transition path (exit/enter chains, leaf-drop, history update, Mealy stash).

fn build_script_storm(scale: Scale, stage: &StageHandles, stage_id: StageId) -> Script {
    let s_root = SceneId::new(1);
    let s_a    = SceneId::new(2);
    let s_b    = SceneId::new(3);

    let troupe_all = TroupeId::new(1);
    let all_actors: Vec<ActiveActor> = stage.actors.iter()
        .map(|(id, h)| ActiveActor::new(stage.lh, stage.sh, *h, *id))
        .collect();

    let bt_actor = stage.actors[0].1;
    let make_bt = || -> BtNode {
        let mut nodes: ThinVec<BtNode> = ThinVec::with_capacity(scale.bt_leaves);
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

    let scene_a = SceneDef {
        id: s_a, stage: stage_id, parent: Some(s_root),
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe_all],
        initial_actors: thin_vec![all_actors.iter().cloned().collect()],
        root: make_bt(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![],
        transitions: thin_vec![Transition {
            condition: Condition::Always,
            target: s_b, effects: Arc::default(),
        }],
    };

    let scene_b = SceneDef {
        id: s_b, stage: stage_id, parent: Some(s_root),
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe_all],
        initial_actors: thin_vec![all_actors.iter().cloned().collect()],
        root: make_bt(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![],
        transitions: thin_vec![Transition {
            condition: Condition::Always,
            target: s_a, effects: Arc::default(),
        }],
    };

    let root = SceneDef {
        id: s_root, stage: stage_id, parent: None,
        kind: SceneKind::Compound {
            children: thin_vec![s_a, s_b],
            initial: s_a, history: None,
        },
        troupes: thin_vec![], initial_actors: thin_vec![],
        root: BtNode::empty(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    };

    let mut script = Script::new(ScriptId::new(2), "storm_script", s_root);
    script.add_scene(root);
    script.add_scene(scene_a);
    script.add_scene(scene_b);
    script
}

fn build_world_storm(scale: Scale) -> (World, Vec<StageHandles>) {
    let mut world = World::new(WorldId::new(2));
    let mut handles = Vec::with_capacity(scale.levels * scale.stages_per_level);

    let mut sid_counter: i64      = 1;
    let mut actor_id_counter: i64 = 1;
    let mut char_id_counter:  i64 = 1;

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
                let ah = world.spawn_actor(
                    lh, sh, aid,
                    Affine3A::from_translation(Vec3::new(ai as f32, 0.0, 0.0)),
                ).unwrap();
                world.spawn_sub_entity(
                    lh, sh, ah,
                    ActorType::Character(Character {
                        id: CharacterId::new(char_id_counter),
                        name: format!("c{char_id_counter}").into(),
                        visible: true, physical: true, playable: false,
                    }),
                    Affine3A::IDENTITY,
                ).unwrap();
                char_id_counter += 1;
                actors.push((aid, ah));
            }
            handles.push(StageHandles { lh, sh, actors });
        }
    }

    for (idx, stage) in handles.iter().enumerate() {
        let stage_id = world.levels[stage.lh].stages[stage.sh].id;
        let script = build_script_storm(scale, stage, stage_id);
        let play = Play::instantiate(
            PlayId::new(idx as i64 + 1),
            format!("storm_play_{idx}"),
            &script,
            stage_id,
            stage.lh,
            stage.sh,
        );
        world.levels[stage.lh].stages[stage.sh].set_play(play);
    }

    (world, handles)
}

/// Measures the transition hot path: every tick fires `apply_transition` because
/// every scene has a `Condition::Always` edge to its sibling. Shows the effect
/// of the scratch-buffer / Arc-mealy optimisations in isolation from steady-state
/// BT + troupe work.
fn bench_transition_storm(c: &mut Criterion) {
    let scale = Scale::medium();
    let (mut world, _h) = build_world_storm(scale);
    // Warm up the allocator; transitions start firing immediately (Condition::Always).
    for _ in 0..60 { world.tick(DT); }

    let mut g = c.benchmark_group("transition_storm");
    g.throughput(Throughput::Elements(scale.total_actors()));
    g.bench_function(scale.label(), |b| {
        b.iter(|| world.tick(black_box(DT)));
    });
    g.finish();
}

// ── Deep-HSM bench ─────────────────────────────────────────────────────────
//
// Linear Compound chain root → c1 → c2 → ... → leaf, depth = `depth`. Stresses
// the chain-build inner loop in collect_effects and the dedup walk: every tick
// walks `depth` parents per leaf. Designed to expose cache-density wins on the
// per-tick ancestor traversal where the per-stage scene count is large.

fn build_script_deep(depth: usize, stage: &StageHandles, stage_id: StageId) -> Script {
    assert!(depth >= 2, "deep HSM needs at least root + leaf");

    let troupe_all = TroupeId::new(1);
    let all_actors: Vec<ActiveActor> = stage.actors.iter()
        .map(|(id, h)| ActiveActor::new(stage.lh, stage.sh, *h, *id))
        .collect();
    let bt_actor = stage.actors[0].1;

    let mut script = Script::new(ScriptId::new(3), "deep_script", SceneId::new(1));

    // Root + intermediate compounds: 1..=depth-1, each with single child = next id.
    for d in 1..depth {
        let id     = SceneId::new(d as i64);
        let child  = SceneId::new((d + 1) as i64);
        let parent = if d == 1 { None } else { Some(SceneId::new((d - 1) as i64)) };
        script.add_scene(SceneDef {
            id, stage: stage_id, parent,
            kind: SceneKind::Compound {
                children: thin_vec![child], initial: child, history: None,
            },
            troupes: thin_vec![], initial_actors: thin_vec![],
            root: BtNode::empty(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![], transitions: thin_vec![],
        });
    }

    // Leaf at id=depth: Atomic with one trivial BT leaf.
    let leaf_id   = SceneId::new(depth as i64);
    let leaf_par  = SceneId::new((depth - 1) as i64);
    script.add_scene(SceneDef {
        id: leaf_id, stage: stage_id, parent: Some(leaf_par),
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe_all],
        initial_actors: thin_vec![all_actors.iter().cloned().collect()],
        root: BtNode::leaf(
            Condition::Always,
            Effect::SetActorLocal {
                level_h: stage.lh, stage_h: stage.sh, actor_h: bt_actor,
                local: Affine3A::from_translation(Vec3::new(0.001, 0.0, 0.0)),
            },
            false,
        ),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    });

    script
}

fn build_world_deep(scale: Scale, depth: usize) -> (World, Vec<StageHandles>) {
    let (mut world, handles) = build_world(scale);
    // Replace each Stage's Play with a deep-HSM one.
    for (idx, stage) in handles.iter().enumerate() {
        let stage_id = world.levels[stage.lh].stages[stage.sh].id;
        let script = build_script_deep(depth, stage, stage_id);
        let play = Play::instantiate(
            PlayId::new(1000 + idx as i64),
            format!("deep_play_{idx}"),
            &script,
            stage_id, stage.lh, stage.sh,
        );
        world.levels[stage.lh].stages[stage.sh].set_play(play);
    }
    (world, handles)
}

fn bench_deep_hsm(c: &mut Criterion) {
    let scale = Scale::small();
    let mut g = c.benchmark_group("deep_hsm");
    for &depth in &[16usize, 64, 256] {
        let (mut world, _h) = build_world_deep(scale, depth);
        for _ in 0..60 { world.tick(DT); }

        g.throughput(Throughput::Elements(depth as u64));
        g.bench_with_input(BenchmarkId::from_parameter(format!("depth={depth}")), &depth, |b, _| {
            b.iter(|| world.tick(black_box(DT)));
        });
    }
    g.finish();
}

criterion_group!(benches,
    bench_world_build,
    bench_tick_steady,
    bench_tick_scaling,
    bench_tick_phases,
    bench_transition_storm,
    bench_deep_hsm,
);
criterion_main!(benches);
