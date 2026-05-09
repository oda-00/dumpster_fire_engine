// Microbenchmarks for `BtNode::tick` per variant (scene.rs).
//
//   cargo bench --bench bt_nodes

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use divan::{black_box, Bencher};
use glam::Affine3A;
use dumpster_fire_engine::resource_manager::*;

fn main() { divan::main(); }

// ── Fixture (no sink — keep it local in each bench to dodge borrow conflicts) ──

struct Fixture {
    world: World,
    lh: LevelHandle,
    sh: StageHandle,
    actors: Troupe,
    troupes: Vec<TroupeId>,
    events: Vec<Event>,
}

fn build_fixture() -> Fixture {
    let mut world = World::new(WorldId::new(1));
    let lh = world.spawn_level(LevelId::new(1), "L");
    let sh = world.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let aid = ActorId::new(1);
    let ah = world.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();
    world.propagate_transforms();

    let actives = vec![ActiveActor::new(lh, sh, ah, aid)];
    Fixture {
        world,
        lh, sh,
        actors: Troupe(vec![actives]),
        troupes: vec![TroupeId::new(1)],
        events: Vec::new(),
    }
}

fn ctx<'a>(f: &'a Fixture, elapsed: f32) -> EvalCtx<'a> {
    EvalCtx {
        world:       &f.world,
        level_h:     f.lh,
        stage_h:     f.sh,
        scene_id:    SceneId::new(1),
        elapsed,
        tick_count:  0,
        events_seen: &f.events,
        actors:      &f.actors,
        troupes:     &f.troupes,
    }
}

fn dummy_effect() -> Effect {
    Effect::SpawnActor {
        level_h: LevelHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData },
        stage_h: StageHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData },
        id: ActorId::new(0), local: Affine3A::IDENTITY,
    }
}

fn pass_leaf() -> BtNode { BtNode::leaf(Condition::Always, dummy_effect(), false) }
fn fail_leaf() -> BtNode { BtNode::leaf(Condition::Never,  dummy_effect(), false) }

// ── Leaf ───────────────────────────────────────────────────────────────────

#[divan::bench]
fn leaf_pass(b: Bencher) {
    let f = build_fixture();
    let n = pass_leaf();
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        black_box(n.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench]
fn leaf_fail(b: Bencher) {
    let f = build_fixture();
    let n = fail_leaf();
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        black_box(n.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench]
fn leaf_once_fired_skip(b: Bencher) {
    let f = build_fixture();
    let n = BtNode::leaf(Condition::Always, dummy_effect(), true);
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    n.tick(&ctx(&f, 0.0), &mut sink); // prime: sets the AtomicBool
    b.bench_local(|| {
        sink.clear();
        black_box(n.tick(&ctx(&f, 0.0), &mut sink))
    });
}

// ── Sequence / Selector ────────────────────────────────────────────────────

#[divan::bench(args = &[1usize, 4, 16, 64])]
fn sequence_n_success(b: Bencher, n: usize) {
    let f = build_fixture();
    let nodes = (0..n).map(|_| pass_leaf()).collect();
    let seq = BtNode::Sequence(nodes);
    let mut sink: Vec<Effect> = Vec::with_capacity(64);
    b.bench_local(|| {
        sink.clear();
        black_box(seq.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench(args = &[1usize, 4, 16, 64])]
fn selector_all_fail(b: Bencher, n: usize) {
    let f = build_fixture();
    let nodes = (0..n).map(|_| fail_leaf()).collect();
    let sel = BtNode::Selector(nodes);
    let mut sink: Vec<Effect> = Vec::with_capacity(64);
    b.bench_local(|| {
        sink.clear();
        black_box(sel.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench(args = &[1usize, 4, 16, 64])]
fn selector_first_succeeds(b: Bencher, n: usize) {
    let f = build_fixture();
    let mut nodes = vec![pass_leaf()];
    for _ in 1..n { nodes.push(fail_leaf()); }
    let sel = BtNode::Selector(nodes);
    let mut sink: Vec<Effect> = Vec::with_capacity(64);
    b.bench_local(|| {
        sink.clear();
        black_box(sel.tick(&ctx(&f, 0.0), &mut sink))
    });
}

// ── Parallel — one bench per policy (ParallelPolicy doesn't impl Display) ──

#[divan::bench]
fn parallel_all_succeed(b: Bencher) {
    let f = build_fixture();
    let children = (0..8).map(|i| if i % 2 == 0 { pass_leaf() } else { fail_leaf() }).collect();
    let par = BtNode::Parallel { children, policy: ParallelPolicy::AllSucceed };
    let mut sink: Vec<Effect> = Vec::with_capacity(16);
    b.bench_local(|| {
        sink.clear();
        black_box(par.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench]
fn parallel_any_succeed(b: Bencher) {
    let f = build_fixture();
    let children = (0..8).map(|i| if i % 2 == 0 { pass_leaf() } else { fail_leaf() }).collect();
    let par = BtNode::Parallel { children, policy: ParallelPolicy::AnySucceed };
    let mut sink: Vec<Effect> = Vec::with_capacity(16);
    b.bench_local(|| {
        sink.clear();
        black_box(par.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench]
fn parallel_all_complete(b: Bencher) {
    let f = build_fixture();
    let children = (0..8).map(|i| if i % 2 == 0 { pass_leaf() } else { fail_leaf() }).collect();
    let par = BtNode::Parallel { children, policy: ParallelPolicy::AllComplete };
    let mut sink: Vec<Effect> = Vec::with_capacity(16);
    b.bench_local(|| {
        sink.clear();
        black_box(par.tick(&ctx(&f, 0.0), &mut sink))
    });
}

// ── Repeat ─────────────────────────────────────────────────────────────────

#[divan::bench]
fn repeat_unbounded(b: Bencher) {
    let f = build_fixture();
    let r = BtNode::Repeat {
        child: Arc::new(pass_leaf()),
        count: 0,
        current: AtomicU32::new(0),
    };
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        black_box(r.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench]
fn repeat_finite(b: Bencher) {
    let f = build_fixture();
    let r = BtNode::Repeat {
        child: Arc::new(pass_leaf()),
        count: 1_000_000,
        current: AtomicU32::new(0),
    };
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        black_box(r.tick(&ctx(&f, 0.0), &mut sink))
    });
}

// ── Decorators ─────────────────────────────────────────────────────────────

#[divan::bench]
fn decorator_inverter(b: Bencher) {
    let f = build_fixture();
    let d = BtNode::Decorator { decorator: Decorator::Inverter, child: Arc::new(fail_leaf()) };
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        black_box(d.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench]
fn decorator_guard_pass(b: Bencher) {
    let f = build_fixture();
    let d = BtNode::Decorator {
        decorator: Decorator::Guard(Condition::Always),
        child: Arc::new(pass_leaf()),
    };
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        black_box(d.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench]
fn decorator_guard_fail(b: Bencher) {
    let f = build_fixture();
    let d = BtNode::Decorator {
        decorator: Decorator::Guard(Condition::Never),
        child: Arc::new(pass_leaf()),
    };
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        black_box(d.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench]
fn decorator_until_success(b: Bencher) {
    let f = build_fixture();
    let d = BtNode::Decorator {
        decorator: Decorator::UntilSuccess,
        child: Arc::new(fail_leaf()),
    };
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        black_box(d.tick(&ctx(&f, 0.0), &mut sink))
    });
}

#[divan::bench]
fn decorator_cooldown_active(b: Bencher) {
    let f = build_fixture();
    let d = BtNode::Decorator {
        decorator: Decorator::Cooldown {
            duration: 10.0,
            last_success_at: AtomicU32::new(1.0_f32.to_bits()),
        },
        child: Arc::new(pass_leaf()),
    };
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        black_box(d.tick(&ctx(&f, 1.5), &mut sink))
    });
}

#[divan::bench]
fn decorator_cooldown_ready(b: Bencher) {
    let f = build_fixture();
    let d = BtNode::Decorator {
        decorator: Decorator::cooldown(0.5),
        child: Arc::new(pass_leaf()),
    };
    let mut sink: Vec<Effect> = Vec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        if let BtNode::Decorator { decorator: Decorator::Cooldown { last_success_at, .. }, .. } = &d {
            last_success_at.store(f32::NEG_INFINITY.to_bits(), Ordering::Relaxed);
        }
        black_box(d.tick(&ctx(&f, 5.0), &mut sink))
    });
}

// ── reset() ────────────────────────────────────────────────────────────────

#[divan::bench]
fn reset_full_tree(b: Bencher) {
    let tree = BtNode::Sequence(vec![
        BtNode::Selector(vec![pass_leaf(), fail_leaf()]),
        BtNode::Repeat {
            child: Arc::new(pass_leaf()),
            count: 5,
            current: AtomicU32::new(3),
        },
        BtNode::Decorator {
            decorator: Decorator::cooldown(1.0),
            child: Arc::new(pass_leaf()),
        },
        BtNode::Parallel {
            children: vec![pass_leaf(), pass_leaf(), fail_leaf()],
            policy: ParallelPolicy::AllComplete,
        },
    ]);
    b.bench_local(|| black_box(tree.reset()));
}
