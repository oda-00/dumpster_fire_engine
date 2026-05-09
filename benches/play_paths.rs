// Benchmarks for `Play` hot/cold paths (play.rs).
// - instantiate (cold)
// - handle_for (hot, range-compressed direct lookup vs HashMap baseline)
// - collect_effects (hot, per-tick chain build + dedup)
// - apply_transition via tick (cold but per-event)
// - compute_static_troupes (cold, called in instantiate)
//
//   cargo bench --bench play_paths

use std::collections::HashMap;
use std::sync::Arc;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use glam::Affine3A;
use thin_vec::thin_vec;
use dumpster_fire_engine::resource_manager::*;

fn world_with_one_actor() -> (World, LevelHandle, StageHandle, ActorHandle, ActorId) {
    let mut w = World::new(WorldId::new(1));
    let lh = w.spawn_level(LevelId::new(1), "L");
    let sh = w.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let aid = ActorId::new(1);
    let ah = w.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();
    w.propagate_transforms();
    (w, lh, sh, ah, aid)
}

fn build_flat_script(n: usize, lh: LevelHandle, sh: StageHandle, ah: ActorHandle, aid: ActorId) -> Script {
    let s_root = SceneId::new(1);
    let mut script = Script::new(ScriptId::new(1), "flat", s_root);
    let actives = vec![ActiveActor::new(lh, sh, ah, aid)];

    let mut children = thin_vec::ThinVec::new();
    for i in 0..n { children.push(SceneId::new((i + 2) as i64)); }
    let initial_child = children[0];

    script.add_scene(SceneDef {
        id: s_root, stage: StageId::new(1), parent: None,
        kind: SceneKind::Compound { children: children.clone(), initial: initial_child, history: None },
        troupes: thin_vec![], initial_actors: thin_vec![],
        root: BtNode::empty(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    });
    for i in 0..n {
        script.add_scene(SceneDef {
            id: SceneId::new((i + 2) as i64), stage: StageId::new(1), parent: Some(s_root),
            kind: SceneKind::Atomic,
            troupes: thin_vec![],
            initial_actors: thin_vec![actives.iter().cloned().collect()],
            root: BtNode::empty(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![], transitions: thin_vec![],
        });
    }
    script
}

fn build_deep_script(depth: usize, lh: LevelHandle, sh: StageHandle, ah: ActorHandle, aid: ActorId) -> Script {
    assert!(depth >= 2);
    let mut script = Script::new(ScriptId::new(2), "deep", SceneId::new(1));
    let actives = vec![ActiveActor::new(lh, sh, ah, aid)];

    for d in 1..depth {
        let id = SceneId::new(d as i64);
        let child = SceneId::new((d + 1) as i64);
        let parent = if d == 1 { None } else { Some(SceneId::new((d - 1) as i64)) };
        script.add_scene(SceneDef {
            id, stage: StageId::new(1), parent,
            kind: SceneKind::Compound {
                children: thin_vec![child], initial: child, history: None,
            },
            troupes: thin_vec![], initial_actors: thin_vec![],
            root: BtNode::empty(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![], transitions: thin_vec![],
        });
    }
    script.add_scene(SceneDef {
        id: SceneId::new(depth as i64), stage: StageId::new(1),
        parent: Some(SceneId::new((depth - 1) as i64)),
        kind: SceneKind::Atomic,
        troupes: thin_vec![],
        initial_actors: thin_vec![actives.iter().cloned().collect()],
        root: BtNode::empty(),
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    });
    script
}

fn bench_instantiate(c: &mut Criterion) {
    let mut g = c.benchmark_group("instantiate");
    let (_w, lh, sh, ah, aid) = world_with_one_actor();

    for &n in &[5usize, 64, 256] {
        let script = build_flat_script(n, lh, sh, ah, aid);
        g.throughput(Throughput::Elements(n as u64 + 1));
        g.bench_with_input(BenchmarkId::new("flat", n), &script, |b, script| {
            b.iter(|| {
                let p = Play::instantiate(PlayId::new(1), "p", script, StageId::new(1), lh, sh);
                black_box(p);
            });
        });
    }

    for &depth in &[16usize, 64, 256] {
        let script = build_deep_script(depth, lh, sh, ah, aid);
        g.throughput(Throughput::Elements(depth as u64));
        g.bench_with_input(BenchmarkId::new("deep", depth), &script, |b, script| {
            b.iter(|| {
                let p = Play::instantiate(PlayId::new(1), "p", script, StageId::new(1), lh, sh);
                black_box(p);
            });
        });
    }
    g.finish();
}

fn bench_handle_for(c: &mut Criterion) {
    let mut g = c.benchmark_group("handle_for");
    let (_w, lh, sh, ah, aid) = world_with_one_actor();

    for &n in &[16usize, 256, 4096] {
        let script = build_flat_script(n, lh, sh, ah, aid);
        let play = Play::instantiate(PlayId::new(1), "p", &script, StageId::new(1), lh, sh);
        let queries: Vec<SceneId> = (1..=(n as i64 + 1)).map(SceneId::new).collect();

        g.throughput(Throughput::Elements(queries.len() as u64));

        g.bench_with_input(BenchmarkId::new("range_compressed", n), &queries, |b, queries| {
            b.iter(|| {
                let mut hits = 0u64;
                for &q in queries {
                    if play.handle_for(black_box(q)).is_some() { hits += 1; }
                }
                hits
            });
        });

        let mut map: HashMap<i64, SceneHandle> = HashMap::with_capacity(n + 1);
        for &q in &queries {
            if let Some(h) = play.handle_for(q) {
                map.insert(q.raw(), h);
            }
        }
        g.bench_with_input(BenchmarkId::new("hashmap_control", n), &queries, |b, queries| {
            b.iter(|| {
                let mut hits = 0u64;
                for &q in queries {
                    if map.contains_key(&black_box(q.raw())) { hits += 1; }
                }
                hits
            });
        });
    }
    g.finish();
}

fn bench_collect_effects(c: &mut Criterion) {
    let mut g = c.benchmark_group("collect_effects");

    for &depth in &[8usize, 64, 256] {
        let mut w = World::new(WorldId::new(1));
        let lh = w.spawn_level(LevelId::new(1), "L");
        let sh = w.spawn_stage(lh, StageId::new(1), "S").unwrap();
        let aid = ActorId::new(1);
        let ah = w.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();

        let script = build_deep_script(depth, lh, sh, ah, aid);
        let play = Play::instantiate(PlayId::new(1), "p", &script, StageId::new(1), lh, sh);
        w.levels[lh].stages[sh].set_play(play);
        w.tick(1.0 / 60.0);

        g.throughput(Throughput::Elements(depth as u64));
        g.bench_with_input(BenchmarkId::from_parameter(format!("depth_{depth}")), &depth, |b, _| {
            let mut sink: Vec<Effect> = Vec::with_capacity(64);
            let mut chain: Vec<SceneHandle> = Vec::with_capacity(depth);
            b.iter(|| {
                sink.clear();
                chain.clear();
                let world_view: &World = &w;
                let stage = &w.levels[lh].stages[sh];
                stage.collect_effects(black_box(1.0 / 60.0), world_view, &mut sink, &mut chain);
                black_box(&sink);
            });
        });
    }
    g.finish();
}

fn bench_apply_transition(c: &mut Criterion) {
    let mut g = c.benchmark_group("apply_transition_via_tick");

    g.bench_function("sibling_swap_every_tick", |b| {
        let mut w = World::new(WorldId::new(1));
        let lh = w.spawn_level(LevelId::new(1), "L");
        let sh = w.spawn_stage(lh, StageId::new(1), "S").unwrap();
        let aid = ActorId::new(1);
        let ah = w.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();

        let s_root = SceneId::new(1);
        let s_a = SceneId::new(2);
        let s_b = SceneId::new(3);
        let mut script = Script::new(ScriptId::new(1), "swap", s_root);
        let actives = vec![ActiveActor::new(lh, sh, ah, aid)];
        script.add_scene(SceneDef {
            id: s_root, stage: StageId::new(1), parent: None,
            kind: SceneKind::Compound { children: thin_vec![s_a, s_b], initial: s_a, history: None },
            troupes: thin_vec![], initial_actors: thin_vec![],
            root: BtNode::empty(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![], transitions: thin_vec![],
        });
        script.add_scene(SceneDef {
            id: s_a, stage: StageId::new(1), parent: Some(s_root),
            kind: SceneKind::Atomic,
            troupes: thin_vec![], initial_actors: thin_vec![actives.iter().cloned().collect()],
            root: BtNode::empty(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![],
            transitions: thin_vec![Transition {
                condition: Condition::Always, target: s_b, effects: Arc::default(),
            }],
        });
        script.add_scene(SceneDef {
            id: s_b, stage: StageId::new(1), parent: Some(s_root),
            kind: SceneKind::Atomic,
            troupes: thin_vec![], initial_actors: thin_vec![actives.iter().cloned().collect()],
            root: BtNode::empty(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![],
            transitions: thin_vec![Transition {
                condition: Condition::Always, target: s_a, effects: Arc::default(),
            }],
        });
        let play = Play::instantiate(PlayId::new(1), "p", &script, StageId::new(1), lh, sh);
        w.levels[lh].stages[sh].set_play(play);
        for _ in 0..2 { w.tick(1.0 / 60.0); }
        b.iter(|| w.tick(black_box(1.0 / 60.0)));
    });

    g.finish();
}

fn bench_compute_static_troupes(c: &mut Criterion) {
    let mut g = c.benchmark_group("compute_static_troupes_via_instantiate");
    let (_w, lh, sh, ah, aid) = world_with_one_actor();

    for &n in &[8usize, 64, 256] {
        let mut script = Script::new(ScriptId::new(3), "troupes", SceneId::new(1));
        let actives = vec![ActiveActor::new(lh, sh, ah, aid)];
        let mut children = thin_vec::ThinVec::new();
        for i in 0..n { children.push(SceneId::new((i + 2) as i64)); }
        let init = children[0];
        script.add_scene(SceneDef {
            id: SceneId::new(1), stage: StageId::new(1), parent: None,
            kind: SceneKind::Compound { children, initial: init, history: None },
            troupes: thin_vec![], initial_actors: thin_vec![],
            root: BtNode::empty(),
            on_enter: thin_vec![], on_exit: thin_vec![],
            handlers: thin_vec![], transitions: thin_vec![],
        });
        for i in 0..n {
            let troupe = TroupeId::new(i as i64 + 1);
            script.add_scene(SceneDef {
                id: SceneId::new((i + 2) as i64), stage: StageId::new(1), parent: Some(SceneId::new(1)),
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
            });
        }

        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &script, |b, script| {
            b.iter(|| {
                let p = Play::instantiate(PlayId::new(99), "p", script, StageId::new(1), lh, sh);
                black_box(p);
            });
        });
    }
    g.finish();
}

criterion_group!(
    benches,
    bench_instantiate,
    bench_handle_for,
    bench_collect_effects,
    bench_apply_transition,
    bench_compute_static_troupes,
);
criterion_main!(benches);
