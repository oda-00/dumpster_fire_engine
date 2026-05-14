// Deterministic instruction-count + cache-metric benchmarks via valgrind callgrind.
// Targets the absolute hottest kernels — the ones where wall-clock noise would
// hide regressions of <1%.
//
// Run requires `valgrind` on PATH:
//   cargo bench --bench iai_critical
//
// Output (per benchmark):
//   Instructions
//   L1 Hits / L2 Hits / RAM Hits
//   Total read/write
//   Cycles (estimated)

use iai_callgrind::{
    library_benchmark, library_benchmark_group, main,
};
use std::hint::black_box;
use glam::Affine3A;
use thin_vec::{ThinVec, thin_vec};
use dumpster_fire_engine::resource_manager::*;

// ── Setup helpers (NOT timed — iai measures only the #[library_benchmark] body) ──

fn build_world_with_n(n: usize) -> (World, LevelHandle, StageHandle, Vec<ActorHandle>) {
    let mut w = World::new(WorldId::new(1));
    let lh = w.spawn_level(LevelId::new(1), "L");
    let sh = w.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let actors: Vec<_> = (0..n).map(|i| {
        w.spawn_actor(
            lh, sh, ActorId::new(i as i64 + 1),
            Affine3A::IDENTITY,
        ).unwrap()
    }).collect();
    w.propagate_transforms();
    (w, lh, sh, actors)
}

fn build_arena_filled() -> (Arena<ActorTag, u64>, Vec<Handle<ActorTag>>) {
    let mut a: Arena<ActorTag, u64> = Arena::with_capacity(10_000);
    let h: Vec<_> = (0..10_000).map(|i| a.insert(i as u64)).collect();
    (a, h)
}

// ── Critical kernels ───────────────────────────────────────────────────────

#[library_benchmark]
fn propagate_dirty_1024() {
    let (mut w, lh, sh, actors) = build_world_with_n(1024);
    for &h in &actors {
        w.set_actor_local(lh, sh, h, Affine3A::from_translation(glam::Vec3::new(1.0, 0.0, 0.0)));
    }
    black_box(w.propagate_transforms());
}

#[library_benchmark]
fn cue_troupe_delta_1024() {
    let mut w = World::new(WorldId::new(1));
    let lh = w.spawn_level(LevelId::new(1), "L");
    let sh = w.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let actors: Vec<_> = (0..1024).map(|i| {
        w.spawn_actor(lh, sh, ActorId::new(i as i64 + 1), Affine3A::IDENTITY).unwrap()
    }).collect();
    let troupe = TroupeId::new(1);
    let actives: Vec<ActiveActor> = actors.iter().enumerate().map(|(i, h)|
        ActiveActor::new(lh, sh, *h, ActorId::new(i as i64 + 1))).collect();
    let scene = SceneDef {
        id: SceneId::new(1), stage: StageId::new(1), parent: None,
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe],
        initial_actors: thin_vec![actives.iter().cloned().collect()],
        root: BtNode::leaf(
            Condition::Always,
            Effect::CueTroupe {
                level_h: lh, stage_h: sh, troupe,
                delta: Affine3A::from_translation(glam::Vec3::new(0.001, 0.0, 0.0)),
            },
            false,
        ),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    };
    let mut script = Script::new(ScriptId::new(1), "s", SceneId::new(1));
    script.add_scene(scene);
    let play = Play::instantiate(PlayId::new(1), "p", &script, StageId::new(1), lh, sh);
    w.levels[lh].stages[sh].set_play(play);
    w.tick(1.0 / 60.0);

    let stage = &mut w.levels[lh].stages[sh];
    let delta = Affine3A::from_translation(glam::Vec3::new(0.5, 0.0, 0.0));
    black_box(stage.cue_troupe_direct(troupe, delta));
}

#[library_benchmark]
fn condition_always() {
    let (mut w, lh, sh, _) = build_world_with_n(8);
    let actives = (0..8).map(|i| ActiveActor::new(lh, sh,
        w.spawn_actor(lh, sh, ActorId::new(1000 + i), Affine3A::IDENTITY).unwrap(),
        ActorId::new(1000 + i)
    )).collect::<Vec<_>>();
    let _ = actives;

    let actors = Troupe(ThinVec::new());
    let troupes: Vec<TroupeId> = vec![];
    let events: Vec<Event> = vec![];
    let ctx = EvalCtx {
        world: &w, level_h: lh, stage_h: sh,
        scene_id: SceneId::new(1),
        elapsed: 0.0, tick_count: 0,
        events_seen: &events, actors: &actors, troupes: &troupes,
    };
    let c = Condition::Always;
    for _ in 0..1000 { black_box(c.eval(&ctx)); }
}

#[library_benchmark]
fn condition_actor_near() {
    let (mut w, lh, sh, _) = build_world_with_n(8);
    let aid = ActorId::new(99);
    let _ah = w.spawn_actor(lh, sh, aid, Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0))).unwrap();
    w.propagate_transforms();

    let actives: ThinVec<ActiveActor> = thin_vec![ActiveActor::new(lh, sh, _ah, aid)];
    let actors = Troupe(thin_vec![actives]);
    let troupes: Vec<TroupeId> = vec![TroupeId::new(1)];
    let events: Vec<Event> = vec![];
    let ctx = EvalCtx {
        world: &w, level_h: lh, stage_h: sh,
        scene_id: SceneId::new(1),
        elapsed: 0.0, tick_count: 0,
        events_seen: &events, actors: &actors, troupes: &troupes,
    };
    let c = Condition::ActorNear {
        actor: aid, target: glam::Vec3::new(3.5, 0.0, 0.0), radius: 1.0,
    };
    for _ in 0..1000 { black_box(c.eval(&ctx)); }
}

#[library_benchmark]
fn bt_leaf_pass() {
    let (w, lh, sh, _) = build_world_with_n(1);
    let aid = ActorId::new(1);
    let placeholder_ah = ActorHandle {
        idx: 0,
        generation: std::num::NonZeroU32::new(1).unwrap(),
        _tag: std::marker::PhantomData,
    };
    let actives: ThinVec<ActiveActor> = thin_vec![ActiveActor::new(lh, sh, placeholder_ah, aid)];
    let actors = Troupe(thin_vec![actives]);
    let troupes: Vec<TroupeId> = vec![];
    let events: Vec<Event> = vec![];
    let ctx = EvalCtx {
        world: &w, level_h: lh, stage_h: sh,
        scene_id: SceneId::new(1),
        elapsed: 0.0, tick_count: 0,
        events_seen: &events, actors: &actors, troupes: &troupes,
    };

    let n = BtNode::leaf(
        Condition::Always,
        Effect::SetActorLocal {
            level_h: lh, stage_h: sh,
            actor_h: ActorHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData },
            local: Affine3A::IDENTITY,
        },
        false,
    );
    let mut sink: ThinVec<Effect> = ThinVec::with_capacity(8);
    for _ in 0..1000 {
        sink.clear();
        black_box(n.tick(&ctx, &mut sink));
    }
}

#[library_benchmark]
fn arena_get_hit_10k() {
    let (a, handles) = build_arena_filled();
    let mut sum = 0u64;
    for h in &handles { sum = sum.wrapping_add(*a.get(*h).unwrap()); }
    black_box(sum);
}

#[library_benchmark]
fn play_handle_for_lookup() {
    let mut w = World::new(WorldId::new(1));
    let lh = w.spawn_level(LevelId::new(1), "L");
    let sh = w.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let aid = ActorId::new(1);
    let _ah = w.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();

    // 256-scene flat script.
    let s_root = SceneId::new(1);
    let mut script = Script::new(ScriptId::new(1), "s", s_root);
    let mut children: thin_vec::ThinVec<SceneId> = thin_vec::ThinVec::new();
    for i in 0..256 { children.push(SceneId::new((i + 2) as i64)); }
    let init = children[0];
    script.add_scene(SceneDef {
        id: s_root, stage: StageId::new(1), parent: None,
        kind: SceneKind::Compound { children, initial: init, history: None },
        troupes: thin_vec![], initial_actors: thin_vec![],
        root: BtNode::empty(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    });
    for i in 0..256 {
        script.add_scene(SceneDef {
            id: SceneId::new((i + 2) as i64), stage: StageId::new(1), parent: Some(s_root),
            kind: SceneKind::Atomic,
            troupes: thin_vec![], initial_actors: thin_vec![],
            root: BtNode::empty(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![], transitions: thin_vec![],
        });
    }
    let play = Play::instantiate(PlayId::new(1), "p", &script, StageId::new(1), lh, sh);

    let mut hits = 0u64;
    for i in 1..=257i64 {
        if play.handle_for(SceneId::new(i)).is_some() { hits += 1; }
    }
    black_box(hits);
}

// ── Group ──────────────────────────────────────────────────────────────────

library_benchmark_group!(
    name = critical;
    benchmarks =
        propagate_dirty_1024,
        cue_troupe_delta_1024,
        condition_always,
        condition_actor_near,
        bt_leaf_pass,
        arena_get_hit_10k,
        play_handle_for_lookup,
);

main!(library_benchmark_groups = critical);
