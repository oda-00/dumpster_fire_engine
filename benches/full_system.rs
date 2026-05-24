// Full-system benchmark: exercises ALL major subsystems together in a single
// file. Groups:
//   - world_full_tick   : World::tick + ui_tick at 4 scales
//   - ui_pipeline       : UiManager cascade (build, tick) at various panel/widget counts
//   - memory_churn      : long-running spawn/despawn stability
//   - despawn_storm     : mass actor despawn with cache eviction
//   - integrated_script : scripts driving world mutations (troupe cues + transforms)
//
//   cargo bench --bench full_system

use divan::{Bencher, black_box};
use glam::{Affine3A, Vec3};
use std::sync::Arc;
use thin_vec::{ThinVec, thin_vec};

use dumpster_fire_engine::resource_manager::{
    ui_manager::{LabelData, Panel, Rect, UiInputState, Widget},
    *,
};

fn main() {
    divan::main();
}

const DT: f32 = 1.0 / 60.0;

// ── Shared world-construction helpers ─────────────────────────────────────────

struct FullSetup {
    world: World,
    stages: Vec<(LevelHandle, StageHandle, Vec<ActorHandle>)>,
}

fn build_world_with_play(actor_count: usize) -> FullSetup {
    let n_levels = if actor_count >= 200 { 2 } else { 1 };
    let n_stages = if actor_count >= 100 { 2 } else { 1 };
    let total_slots = n_levels * n_stages;
    let per_stage = actor_count.div_ceil(total_slots);

    let mut world = World::new(WorldId::new(1));
    let mut stages: Vec<(LevelHandle, StageHandle, Vec<ActorHandle>)> =
        Vec::with_capacity(total_slots);
    let mut actor_id_ctr: i64 = 1;
    let mut char_id_ctr: i64 = 1;
    let mut stage_id_ctr: i64 = 1;

    let primary_lh = world.spawn_level(LevelId::new(1), "level_0");

    for li in 0..n_levels {
        let lh = if li == 0 {
            primary_lh
        } else {
            world.spawn_level(LevelId::new(li as i64 + 1), format!("level_{li}"))
        };
        for si in 0..n_stages {
            let sh = world
                .spawn_stage(lh, StageId::new(stage_id_ctr), format!("stage_{li}_{si}"))
                .unwrap();
            stage_id_ctr += 1;

            let mut handles = Vec::with_capacity(per_stage);
            for ai in 0..per_stage {
                let aid = ActorId::new(actor_id_ctr);
                actor_id_ctr += 1;
                let ah = world
                    .spawn_actor(
                        lh,
                        sh,
                        aid,
                        Affine3A::from_translation(Vec3::new(ai as f32, 0.0, 0.0)),
                    )
                    .unwrap();

                // Character sub-entity: Physics + Transform components
                let cvi = world
                    .spawn_sub_entity(
                        lh,
                        sh,
                        ah,
                        ActorType::Character(Character {
                            id: CharacterId::new(char_id_ctr),
                            name: format!("c{char_id_ctr}").into(),
                            visible: true,
                            physical: true,
                            playable: false,
                            mesh: None,
                        }),
                        Affine3A::IDENTITY,
                    )
                    .unwrap();
                char_id_ctr += 1;
                world.add_component(
                    lh,
                    sh,
                    ah,
                    cvi,
                    PhysicsComponent {
                        mass: 70.0,
                        velocity: (0.0, 0.0, 0.0),
                        acceleration: (0.0, -9.8, 0.0),
                    },
                );
                world.add_component(
                    lh,
                    sh,
                    ah,
                    cvi,
                    TransformComponent {
                        position: (ai as f32, 0.0, 0.0),
                        rotation: (0.0, 0.0, 0.0),
                        scale: (1.0, 1.0, 1.0),
                        _transform: true,
                    },
                );

                // Item sub-entity: Collision component
                let ivi = world
                    .spawn_sub_entity(
                        lh,
                        sh,
                        ah,
                        ActorType::Item(Item {
                            id: ItemId::new(actor_id_ctr),
                            name: "trinket".into(),
                            quantity: (1, 1, 1),
                            description: Arc::from(""),
                            stackable: false,
                            visible: true,
                            physical: false,
                            mesh: None,
                        }),
                        Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                    )
                    .unwrap();
                world.add_component(
                    lh,
                    sh,
                    ah,
                    ivi,
                    CollisionComponent {
                        shape: CollisionShape::Sphere,
                        position: (0.0, 1.0, 0.0),
                        rotation: (0.0, 0.0, 0.0),
                        scale: (0.3, 0.3, 0.3),
                        collision: true,
                    },
                );

                handles.push(ah);
            }
            stages.push((lh, sh, handles));
        }
    }

    // Bind a Play with HSM + BT to every Stage.
    for (play_idx, &(lh, sh, ref actors)) in stages.iter().enumerate() {
        let stage_id = world.levels[lh].stages[sh].id;
        let script = build_full_script(lh, sh, stage_id, actors);
        let play = Play::instantiate(
            PlayId::new(play_idx as i64 + 1),
            format!("play_{play_idx}"),
            &script,
            stage_id,
            lh,
            sh,
        );
        world.levels[lh].stages[sh].set_play(play);
    }

    FullSetup { world, stages }
}

// Two-scene compound with Always transitions so the HSM fires a transition
// every tick. Each scene runs an N-leaf Sequence BT + one CueTroupe.
fn build_full_script(
    lh: LevelHandle,
    sh: StageHandle,
    stage_id: StageId,
    actors: &[ActorHandle],
) -> Script {
    let s_root = SceneId::new(1);
    let s_a = SceneId::new(2);
    let s_b = SceneId::new(3);
    let troupe = TroupeId::new(1);

    let actives: ThinVec<ActiveActor> = actors
        .iter()
        .enumerate()
        .map(|(i, &h)| ActiveActor::new(lh, sh, h, ActorId::new(i as i64 + 1)))
        .collect();

    let bt_actor = actors[0];
    let bt_nodes: ThinVec<BtNode> = (0..8u32)
        .map(|k| {
            BtNode::leaf(
                Condition::Always,
                Effect::SetActorLocal {
                    level_h: lh,
                    stage_h: sh,
                    actor_h: bt_actor,
                    local: Affine3A::from_translation(Vec3::new(k as f32 * 0.01, 0.0, 0.0)),
                },
                false,
            )
        })
        .collect();

    let make_scene = |id: SceneId, target: SceneId| -> SceneDef {
        SceneDef {
            id,
            stage: stage_id,
            parent: Some(s_root),
            kind: SceneKind::Atomic,
            troupes: thin_vec![troupe],
            initial_actors: thin_vec![actives.iter().cloned().collect()],
            root: BtNode::Parallel {
                children: thin_vec![
                    BtNode::Sequence(bt_nodes.clone()),
                    BtNode::leaf(
                        Condition::Always,
                        Effect::CueTroupe {
                            level_h: lh,
                            stage_h: sh,
                            troupe,
                            delta: Affine3A::from_translation(Vec3::new(0.001, 0.0, 0.0)),
                        },
                        false,
                    ),
                ],
                policy: ParallelPolicy::AllComplete,
            },
            on_enter: thin_vec![],
            on_exit: thin_vec![],
            handlers: thin_vec![],
            transitions: thin_vec![Transition {
                condition: Condition::Always,
                target,
                effects: Arc::default(),
            }],
        }
    };

    let root = SceneDef {
        id: s_root,
        stage: stage_id,
        parent: None,
        kind: SceneKind::Compound {
            children: thin_vec![s_a, s_b],
            initial: s_a,
            history: None,
        },
        troupes: thin_vec![],
        initial_actors: thin_vec![],
        root: BtNode::empty(),
        on_enter: thin_vec![],
        on_exit: thin_vec![],
        handlers: thin_vec![],
        transitions: thin_vec![],
    };

    let mut script = Script::new(ScriptId::new(1), "full_script", s_root);
    script.add_scene(root);
    script.add_scene(make_scene(s_a, s_b));
    script.add_scene(make_scene(s_b, s_a));
    script
}

// ── Group: world_full_tick ─────────────────────────────────────────────────
// Full World::tick + World::ui_tick at four scales. Warms up 120 ticks (past
// the first HSM transition) before measurement begins.

const TICK_SCALES: &[usize] = &[50, 500, 2_000, 10_000];

#[divan::bench(args = TICK_SCALES)]
fn world_full_tick(b: Bencher, n: usize) {
    let mut setup = build_world_with_play(n);
    let input = UiInputState::default();
    for _ in 0..120 {
        setup.world.tick(DT);
    }

    let total: u64 = setup.stages.iter().map(|(.., a)| a.len() as u64).sum();
    b.counter(divan::counter::ItemsCount::new(total))
        .bench_local(|| {
            setup.world.tick(black_box(DT));
            setup.world.ui_tick(black_box(&input), DT);
        });
}

// ── Group: ui_pipeline ────────────────────────────────────────────────────
// Isolated UiManager cascade: build + tick. Named separately because divan
// 0.1 does not support tuple args.

fn build_ui_world(panels: usize, widgets_per: usize) -> World {
    let mut world = World::new(WorldId::new(99));
    for p in 0..panels {
        let ph = world.ui.spawn_panel(Panel::new(Rect {
            x: p as f32 * 210.0,
            y: 0.0,
            w: 200.0,
            h: 400.0,
        }));
        for w in 0..widgets_per {
            let wh = world
                .ui
                .widgets
                .insert(Widget::Label(LabelData::new(format!("l{p}_{w}"))));
            world.ui.panels.get_mut(ph).unwrap().children.push(wh);
        }
    }
    world
}

// 1 panel × 10 widgets — minimal baseline
#[divan::bench]
fn ui_tick_1p_10w(b: Bencher) {
    let mut world = build_ui_world(1, 10);
    let input = UiInputState::default();
    b.bench_local(|| {
        world.ui_tick(black_box(&input), DT);
    });
}

// 5 panels × 20 widgets — small scene inspector
#[divan::bench]
fn ui_tick_5p_20w(b: Bencher) {
    let mut world = build_ui_world(5, 20);
    let input = UiInputState::default();
    b.bench_local(|| {
        world.ui_tick(black_box(&input), DT);
    });
}

// 10 panels × 50 widgets — medium editor layout
#[divan::bench]
fn ui_tick_10p_50w(b: Bencher) {
    let mut world = build_ui_world(10, 50);
    let input = UiInputState::default();
    b.bench_local(|| {
        world.ui_tick(black_box(&input), DT);
    });
}

// 20 panels × 100 widgets — heavy dashboard
#[divan::bench]
fn ui_tick_20p_100w(b: Bencher) {
    let mut world = build_ui_world(20, 100);
    let input = UiInputState::default();
    b.bench_local(|| {
        world.ui_tick(black_box(&input), DT);
    });
}

// ── Group: memory_churn ───────────────────────────────────────────────────
// Long-running spawn/despawn stability — not covered by any existing bench.

// 100 iterations: spawn N actors with 3 components → tick → despawn all.
// Tests steady-state allocator pressure and arena free-list hygiene.
const CHURN_SIZES: &[usize] = &[50, 200, 1_000];

#[divan::bench(args = CHURN_SIZES)]
fn spawn_tick_despawn_cycle(b: Bencher, n: usize) {
    b.bench_local(|| {
        let mut world = World::new(WorldId::new(3));
        let lh = world.spawn_level(LevelId::new(1), "L");
        let sh = world.spawn_stage(lh, StageId::new(1), "S").unwrap();
        for _cycle in 0..100 {
            let handles: Vec<ActorHandle> = (0..n)
                .map(|i| {
                    let aid = ActorId::new(i as i64 + 1);
                    let ah = world.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();
                    let cvi = world
                        .spawn_sub_entity(
                            lh,
                            sh,
                            ah,
                            ActorType::Character(Character {
                                id: CharacterId::new(i as i64 + 1),
                                name: "c".into(),
                                visible: true,
                                physical: true,
                                playable: false,
                                mesh: None,
                            }),
                            Affine3A::IDENTITY,
                        )
                        .unwrap();
                    world.add_component(
                        lh,
                        sh,
                        ah,
                        cvi,
                        PhysicsComponent {
                            mass: 1.0,
                            velocity: (0.0, 0.0, 0.0),
                            acceleration: (0.0, 0.0, 0.0),
                        },
                    );
                    world.add_component(
                        lh,
                        sh,
                        ah,
                        cvi,
                        TransformComponent {
                            position: (0.0, 0.0, 0.0),
                            rotation: (0.0, 0.0, 0.0),
                            scale: (1.0, 1.0, 1.0),
                            _transform: true,
                        },
                    );
                    world.add_component(
                        lh,
                        sh,
                        ah,
                        cvi,
                        AudioComponent {
                            volume: 1.0,
                            pitch: 1.0,
                            _loop: false,
                            _playing: false,
                        },
                    );
                    ah
                })
                .collect();
            world.tick(DT);
            for ah in handles {
                world.despawn_actor(lh, sh, ah);
            }
        }
        black_box(world.levels[lh].stages[sh].actors.len())
    });
}

// Spawn N, despawn every other actor (slot gaps), spawn N/2 more (free-list reuse).
// Exercises Arena::insert's free-list path under a fragmented layout.
#[divan::bench(args = [500usize, 2_000])]
fn fragmentation_recovery(b: Bencher, n: usize) {
    b.with_inputs(|| {
        let mut world = World::new(WorldId::new(4));
        let lh = world.spawn_level(LevelId::new(1), "L");
        let sh = world.spawn_stage(lh, StageId::new(1), "S").unwrap();
        let handles: Vec<ActorHandle> = (0..n)
            .map(|i| {
                world
                    .spawn_actor(lh, sh, ActorId::new(i as i64 + 1), Affine3A::IDENTITY)
                    .unwrap()
            })
            .collect();
        (world, lh, sh, handles)
    })
    .bench_local_values(|(mut world, lh, sh, handles)| {
        for &ah in handles.iter().step_by(2) {
            world.despawn_actor(lh, sh, ah);
        }
        for j in 0..(n / 2) as i64 {
            world
                .spawn_actor(lh, sh, ActorId::new(100_000 + j), Affine3A::IDENTITY)
                .unwrap();
        }
        black_box(world)
    });
}

// ── Group: despawn_storm ──────────────────────────────────────────────────
// Mass actor despawn with all 4 simple component types so every cache slot
// gets evicted. Tests worst-case cache_remove_actor fan-out.

const STORM_SIZES: &[usize] = &[100, 500, 1_000, 5_000];

fn build_stage_all_components(n: usize) -> (World, LevelHandle, StageHandle, Vec<ActorHandle>) {
    let mut world = World::new(WorldId::new(5));
    let lh = world.spawn_level(LevelId::new(1), "L");
    let sh = world.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let handles: Vec<ActorHandle> = (0..n)
        .map(|i| {
            let ah = world
                .spawn_actor(lh, sh, ActorId::new(i as i64 + 1), Affine3A::IDENTITY)
                .unwrap();
            let cvi = world
                .spawn_sub_entity(
                    lh,
                    sh,
                    ah,
                    ActorType::Character(Character {
                        id: CharacterId::new(i as i64 + 1),
                        name: "c".into(),
                        visible: true,
                        physical: true,
                        playable: false,
                        mesh: None,
                    }),
                    Affine3A::IDENTITY,
                )
                .unwrap();
            world.add_component(
                lh,
                sh,
                ah,
                cvi,
                PhysicsComponent {
                    mass: 1.0,
                    velocity: (0.0, 0.0, 0.0),
                    acceleration: (0.0, 0.0, 0.0),
                },
            );
            world.add_component(
                lh,
                sh,
                ah,
                cvi,
                TransformComponent {
                    position: (0.0, 0.0, 0.0),
                    rotation: (0.0, 0.0, 0.0),
                    scale: (1.0, 1.0, 1.0),
                    _transform: true,
                },
            );
            world.add_component(
                lh,
                sh,
                ah,
                cvi,
                AudioComponent {
                    volume: 1.0,
                    pitch: 1.0,
                    _loop: false,
                    _playing: false,
                },
            );
            world.add_component(
                lh,
                sh,
                ah,
                cvi,
                CollisionComponent {
                    shape: CollisionShape::Box,
                    position: (0.0, 0.0, 0.0),
                    rotation: (0.0, 0.0, 0.0),
                    scale: (1.0, 1.0, 1.0),
                    collision: true,
                },
            );
            ah
        })
        .collect();
    world.propagate_transforms();
    (world, lh, sh, handles)
}

// Despawn all N actors — worst-case cache eviction × ComponentType::COUNT.
#[divan::bench(args = STORM_SIZES)]
fn despawn_all_with_components(b: Bencher, n: usize) {
    b.with_inputs(|| build_stage_all_components(n))
        .bench_local_values(|(mut world, lh, sh, handles)| {
            for ah in handles {
                world.despawn_actor(lh, sh, ah);
            }
            black_box(world)
        });
}

// Despawn half, then respawn the same count — combines eviction + free-list reuse.
#[divan::bench(args = STORM_SIZES)]
fn despawn_half_replace(b: Bencher, n: usize) {
    b.with_inputs(|| build_stage_all_components(n))
        .bench_local_values(|(mut world, lh, sh, handles)| {
            let half = handles.len() / 2;
            for &ah in &handles[..half] {
                world.despawn_actor(lh, sh, ah);
            }
            for j in 0..half as i64 {
                world
                    .spawn_actor(lh, sh, ActorId::new(100_000 + j), Affine3A::IDENTITY)
                    .unwrap();
            }
            black_box(world)
        });
}

// ── Group: integrated_script_world ───────────────────────────────────────
// Scripts driving world mutations: each "script stage" fires 2 CueTroupe +
// 1 SetActorLocal per tick. Tests the full BT→Effect→World apply cascade
// across N independent stages.

const SCRIPT_COUNTS: &[usize] = &[50, 500];

fn build_script_world(n_scripts: usize) -> World {
    let mut world = World::new(WorldId::new(7));
    let lh = world.spawn_level(LevelId::new(1), "L");
    for i in 0..n_scripts {
        let sh = world
            .spawn_stage(lh, StageId::new(i as i64 + 1), format!("s{i}"))
            .unwrap();
        let aid = ActorId::new(i as i64 + 1);
        let ah = world.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();
        let cvi = world
            .spawn_sub_entity(
                lh,
                sh,
                ah,
                ActorType::Character(Character {
                    id: CharacterId::new(i as i64 + 1),
                    name: "c".into(),
                    visible: true,
                    physical: true,
                    playable: false,
                    mesh: None,
                }),
                Affine3A::IDENTITY,
            )
            .unwrap();
        world.add_component(
            lh,
            sh,
            ah,
            cvi,
            PhysicsComponent {
                mass: 1.0,
                velocity: (0.0, 0.0, 0.0),
                acceleration: (0.0, 0.0, 0.0),
            },
        );
        let stage_id = world.levels[lh].stages[sh].id;
        let script = build_integrated_script(lh, sh, stage_id, ah, aid);
        let play = Play::instantiate(
            PlayId::new(i as i64 + 1),
            format!("p{i}"),
            &script,
            stage_id,
            lh,
            sh,
        );
        world.levels[lh].stages[sh].set_play(play);
    }
    world
}

fn build_integrated_script(
    lh: LevelHandle,
    sh: StageHandle,
    stage_id: StageId,
    ah: ActorHandle,
    aid: ActorId,
) -> Script {
    let troupe1 = TroupeId::new(1);
    let troupe2 = TroupeId::new(2);
    let actives: ThinVec<ActiveActor> = thin_vec![ActiveActor::new(lh, sh, ah, aid)];
    let s_root = SceneId::new(1);

    let scene = SceneDef {
        id: s_root,
        stage: stage_id,
        parent: None,
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe1, troupe2],
        initial_actors: thin_vec![actives.clone(), actives],
        root: BtNode::Sequence(thin_vec![
            BtNode::leaf(
                Condition::Always,
                Effect::CueTroupe {
                    level_h: lh,
                    stage_h: sh,
                    troupe: troupe1,
                    delta: Affine3A::from_translation(Vec3::new(0.001, 0.0, 0.0)),
                },
                false,
            ),
            BtNode::leaf(
                Condition::Always,
                Effect::CueTroupe {
                    level_h: lh,
                    stage_h: sh,
                    troupe: troupe2,
                    delta: Affine3A::from_translation(Vec3::new(0.0, 0.001, 0.0)),
                },
                false,
            ),
            BtNode::leaf(
                Condition::Always,
                Effect::SetActorLocal {
                    level_h: lh,
                    stage_h: sh,
                    actor_h: ah,
                    local: Affine3A::IDENTITY
                },
                false,
            ),
        ]),
        on_enter: thin_vec![],
        on_exit: thin_vec![],
        handlers: thin_vec![],
        transitions: thin_vec![],
    };

    let mut script = Script::new(ScriptId::new(1), "integrated", s_root);
    script.add_scene(scene);
    script
}

#[divan::bench(args = SCRIPT_COUNTS)]
fn scripts_drive_world(b: Bencher, n: usize) {
    let mut world = build_script_world(n);
    for _ in 0..60 {
        world.tick(DT);
    }
    b.bench_local(|| {
        world.tick(black_box(DT));
    });
}
