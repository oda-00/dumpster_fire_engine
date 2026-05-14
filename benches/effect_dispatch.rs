// Per-variant benchmarks for `World::apply_effect` (world.rs:283).
// Each Effect variant has its own bench so individual hot paths are isolated.
// Also benches `Effect::clone` per variant (Arc-bump cost).
//
//   cargo bench --bench effect_dispatch

use std::sync::Arc;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use glam::{Affine3A, Vec3};
use thin_vec::thin_vec;
use dumpster_fire_engine::resource_manager::*;

// ── Helpers ────────────────────────────────────────────────────────────────

fn build_world_with_play() -> (World, LevelHandle, StageHandle, ActorHandle, ActorId) {
    let mut world = World::new(WorldId::new(1));
    let lh = world.spawn_level(LevelId::new(1), "L");
    let sh = world.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let aid = ActorId::new(1);
    let ah = world.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();
    world.spawn_sub_entity(
        lh, sh, ah,
        ActorType::Character(Character {
            id: CharacterId::new(1), name: "n".into(),
            visible: true, physical: true, playable: false,
        }),
        Affine3A::IDENTITY,
    );
    world.add_component(lh, sh, ah, 0, PhysicsComponent {
        mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
    });

    // Play with one Atomic scene + one troupe (so Emit / Cue have a target).
    let troupe = TroupeId::new(1);
    let scene = SceneDef {
        id: SceneId::new(1), stage: StageId::new(1), parent: None,
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe],
        initial_actors: thin_vec![vec![ActiveActor::new(lh, sh, ah, aid)].into_iter().collect()],
        root: BtNode::leaf(
            Condition::Never,
            Effect::CueTroupe { level_h: lh, stage_h: sh, troupe, delta: Affine3A::IDENTITY },
            false,
        ),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    };
    let mut script = Script::new(ScriptId::new(1), "s", SceneId::new(1));
    script.add_scene(scene);
    let play = Play::instantiate(PlayId::new(1), "p", &script, StageId::new(1), lh, sh);
    world.levels[lh].stages[sh].set_play(play);
    world.tick(1.0 / 60.0);

    (world, lh, sh, ah, aid)
}

// ── Per-variant apply_effect benches ───────────────────────────────────────

fn bench_apply_effect(c: &mut Criterion) {
    let mut g = c.benchmark_group("apply_effect");

    // SetActorLocal — fast path through SoA write.
    g.bench_function("set_actor_local", |b| {
        let (mut w, lh, sh, ah, _) = build_world_with_play();
        let t = Affine3A::from_translation(Vec3::new(1.0, 0.0, 0.0));
        b.iter(|| {
            w.apply_effect(Effect::SetActorLocal {
                level_h: lh, stage_h: sh, actor_h: ah, local: black_box(t),
            });
        });
    });

    g.bench_function("set_sub_entity_local", |b| {
        let (mut w, lh, sh, ah, _) = build_world_with_play();
        let t = Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0));
        b.iter(|| {
            w.apply_effect(Effect::SetSubEntityLocal {
                level_h: lh, stage_h: sh, actor_h: ah, variant_idx: 0, local: black_box(t),
            });
        });
    });

    // AddComponent — Arc::try_unwrap fast path (refcount = 1).
    g.bench_function("add_component_unique_owner", |b| {
        let (mut w, lh, sh, ah, _) = build_world_with_play();
        b.iter(|| {
            let arc = Arc::new(AddComponentEffect {
                level_h: lh, stage_h: sh, actor_h: ah, variant_idx: 0,
                component: Component::Audio(AudioComponent {
                    volume: 1.0, pitch: 1.0, _loop: false, _playing: false,
                }),
            });
            w.apply_effect(Effect::AddComponent(arc));
        });
    });

    // AddComponent — shared (refcount > 1) → forces deep clone via clone_component_pub.
    g.bench_function("add_component_shared_clone", |b| {
        let (mut w, lh, sh, ah, _) = build_world_with_play();
        b.iter(|| {
            let arc = Arc::new(AddComponentEffect {
                level_h: lh, stage_h: sh, actor_h: ah, variant_idx: 0,
                component: Component::Audio(AudioComponent {
                    volume: 1.0, pitch: 1.0, _loop: false, _playing: false,
                }),
            });
            let _holder = Arc::clone(&arc); // bump refcount to 2 → forces deep-clone fallback
            w.apply_effect(Effect::AddComponent(arc));
        });
    });

    g.bench_function("remove_component", |b| {
        let (mut w, lh, sh, ah, _) = build_world_with_play();
        b.iter(|| {
            // Re-add the component each iteration so remove always has work to do.
            w.add_component(lh, sh, ah, 0, AudioComponent {
                volume: 1.0, pitch: 1.0, _loop: false, _playing: false,
            });
            w.apply_effect(Effect::RemoveComponent {
                level_h: lh, stage_h: sh, actor_h: ah, variant_idx: 0,
                component_type: ComponentType::Audio,
            });
        });
    });

    g.bench_function("spawn_actor", |b| {
        let (mut w, lh, sh, _, _) = build_world_with_play();
        let mut next_id = 1000i64;
        b.iter(|| {
            next_id += 1;
            w.apply_effect(Effect::SpawnActor {
                level_h: lh, stage_h: sh,
                id: ActorId::new(next_id), local: Affine3A::IDENTITY,
            });
        });
    });

    g.bench_function("despawn_actor_spawn_pair", |b| {
        // Measures spawn+despawn together (can't separate because both need &mut w
        // and the borrow checker rejects shared captures across criterion's two-closure API).
        let (mut w, lh, sh, _, _) = build_world_with_play();
        let mut next_id = 99_000i64;
        b.iter(|| {
            next_id += 1;
            let ah = w.spawn_actor(lh, sh, ActorId::new(next_id), Affine3A::IDENTITY).unwrap();
            w.apply_effect(Effect::DespawnActor { level_h: lh, stage_h: sh, actor_h: ah });
        });
    });

    g.bench_function("despawn_sub_entity_spawn_pair", |b| {
        let (mut w, lh, sh, ah, _) = build_world_with_play();
        let mut next_id = 0i64;
        b.iter(|| {
            next_id += 1;
            w.spawn_sub_entity(
                lh, sh, ah,
                ActorType::Item(Item {
                    id:          ItemId::new(next_id),
                    name:        "i".into(),
                    quantity:    (1, 1, 1),
                    description: Arc::from(""),
                    stackable:   false,
                    visible:     true,
                    physical:    false,
        }),
                Affine3A::IDENTITY,
            );
            w.apply_effect(Effect::DespawnSubEntity {
                level_h: lh, stage_h: sh, actor_h: ah, variant_idx: 2,
            });
        });
    });

    g.bench_function("cue_troupe", |b| {
        let (mut w, lh, sh, _, _) = build_world_with_play();
        let troupe = TroupeId::new(1);
        let delta = Affine3A::from_translation(Vec3::new(0.001, 0.0, 0.0));
        b.iter(|| {
            w.apply_effect(Effect::CueTroupe {
                level_h: lh, stage_h: sh, troupe, delta: black_box(delta),
            });
        });
    });

    g.bench_function("emit_play", |b| {
        let (mut w, lh, sh, _, _) = build_world_with_play();
        b.iter(|| {
            w.apply_effect(Effect::Emit {
                level_h: lh, stage_h: sh,
                target: EventTarget::Play,
                event: Event::Tick { dt: 1.0 / 60.0 },
            });
        });
    });

    g.bench_function("emit_current_scene", |b| {
        let (mut w, lh, sh, _, _) = build_world_with_play();
        b.iter(|| {
            w.apply_effect(Effect::Emit {
                level_h: lh, stage_h: sh,
                target: EventTarget::CurrentScene,
                event: Event::Tick { dt: 1.0 / 60.0 },
            });
        });
    });

    g.bench_function("emit_specific_scene", |b| {
        let (mut w, lh, sh, _, _) = build_world_with_play();
        b.iter(|| {
            w.apply_effect(Effect::Emit {
                level_h: lh, stage_h: sh,
                target: EventTarget::Scene(SceneId::new(1)),
                event: Event::Tick { dt: 1.0 / 60.0 },
            });
        });
    });

    g.bench_function("schedule_transition", |b| {
        let (mut w, lh, sh, _, _) = build_world_with_play();
        let mealy: Arc<[Effect]> = Arc::from(ThinVec::<Effect>::new());
        b.iter(|| {
            w.apply_effect(Effect::ScheduleTransition {
                level_h: lh, stage_h: sh,
                source: SceneId::new(1), target: SceneId::new(1),
                mealy: Arc::clone(&mealy),
            });
        });
    });

    g.finish();
}

// ── Effect::clone per variant — Arc-bump vs shallow copy ───────────────────

fn bench_effect_clone(c: &mut Criterion) {
    let mut g = c.benchmark_group("effect_clone");

    let lh = LevelHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };
    let sh = StageHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };
    let ah = ActorHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };

    g.bench_function("set_actor_local", |b| {
        let e = Effect::SetActorLocal { level_h: lh, stage_h: sh, actor_h: ah, local: Affine3A::IDENTITY };
        b.iter(|| black_box(e.clone()));
    });

    g.bench_function("add_component_arc_bump", |b| {
        let e = Effect::AddComponent(Arc::new(AddComponentEffect {
            level_h: lh, stage_h: sh, actor_h: ah, variant_idx: 0,
            component: Component::Audio(AudioComponent {
                volume: 1.0, pitch: 1.0, _loop: false, _playing: false,
            }),
        }));
        b.iter(|| black_box(e.clone()));
    });

    g.bench_function("schedule_transition_arc_slice_bump", |b| {
        let mealy: Arc<[Effect]> = Arc::from(ThinVec::<Effect>::new());
        let e = Effect::ScheduleTransition {
            level_h: lh, stage_h: sh,
            source: SceneId::new(1), target: SceneId::new(2),
            mealy,
        };
        b.iter(|| black_box(e.clone()));
    });

    g.bench_function("emit", |b| {
        let e = Effect::Emit {
            level_h: lh, stage_h: sh,
            target: EventTarget::Play,
            event: Event::Tick { dt: 1.0 / 60.0 },
        };
        b.iter(|| black_box(e.clone()));
    });

    g.finish();
}

// ── Cross-variant batch clone — measures inline-Effect-size cache effect ───

fn bench_effect_clone_batch(c: &mut Criterion) {
    let mut g = c.benchmark_group("effect_clone_batch");
    let lh = LevelHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };
    let sh = StageHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };
    let ah = ActorHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };

    for &n in &[16usize, 256, 4096] {
        let effs: ThinVec<Effect> = (0..n).map(|i| {
            if i % 3 == 0 {
                Effect::SetActorLocal { level_h: lh, stage_h: sh, actor_h: ah, local: Affine3A::IDENTITY }
            } else if i % 3 == 1 {
                Effect::Emit { level_h: lh, stage_h: sh, target: EventTarget::Play, event: Event::Tick { dt: 0.0 } }
            } else {
                Effect::CueTroupe { level_h: lh, stage_h: sh, troupe: TroupeId::new(1), delta: Affine3A::IDENTITY }
            }
        }).collect();

        g.bench_with_input(BenchmarkId::from_parameter(n), &effs, |b, effs| {
            b.iter(|| {
                let cloned: ThinVec<Effect> = effs.iter().cloned().collect();
                black_box(cloned)
            });
        });
    }
    g.finish();
}

criterion_group!(benches, bench_apply_effect, bench_effect_clone, bench_effect_clone_batch);
criterion_main!(benches);
