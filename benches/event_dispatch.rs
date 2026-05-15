// Benchmarks for event matching + handler dispatch + Troupe iteration.
//
//   cargo bench --bench event_dispatch

use std::sync::Arc;
use divan::{black_box, Bencher};
use glam::{Affine3A, Vec3};
use thin_vec::{ThinVec, thin_vec};
use dumpster_fire_engine::resource_manager::*;

fn main() { divan::main(); }

// ── EventMatcher::matches per variant ──────────────────────────────────────

#[divan::bench]
fn matcher_any_against_tick(b: Bencher) {
    let m = EventMatcher::Any;
    let e = Event::Tick { dt: 1.0 / 60.0 };
    b.bench_local(|| black_box(m.matches(&e)));
}

#[divan::bench]
fn matcher_tick_against_tick(b: Bencher) {
    let m = EventMatcher::Tick;
    let e = Event::Tick { dt: 1.0 / 60.0 };
    b.bench_local(|| black_box(m.matches(&e)));
}

#[divan::bench]
fn matcher_tick_against_scene_entered(b: Bencher) {
    let m = EventMatcher::Tick;
    let e = Event::SceneEntered(SceneId::new(1));
    b.bench_local(|| black_box(m.matches(&e)));
}

#[divan::bench]
fn matcher_custom_id_match(b: Bencher) {
    let m = EventMatcher::Custom(EventId::new(42));
    let e = Event::Custom(EventId::new(42), Arc::new(Payload::None));
    b.bench_local(|| black_box(m.matches(&e)));
}

#[divan::bench]
fn matcher_custom_id_miss(b: Bencher) {
    let m = EventMatcher::Custom(EventId::new(42));
    let e = Event::Custom(EventId::new(99), Arc::new(Payload::None));
    b.bench_local(|| black_box(m.matches(&e)));
}

#[divan::bench]
fn matcher_actor_moved(b: Bencher) {
    let m = EventMatcher::ActorMoved;
    let e = Event::ActorMoved {
        actor: ActorId::new(1),
        from: Vec3::ZERO, to: Vec3::new(1.0, 0.0, 0.0),
    };
    b.bench_local(|| black_box(m.matches(&e)));
}

// ── Handler dispatch — N events × M handlers (the inner loop in Play::collect_effects) ─

fn make_events(n: usize) -> ThinVec<Event> {
    (0..n).map(|i| match i % 4 {
        0 => Event::Tick { dt: 1.0 / 60.0 },
        1 => Event::SceneEntered(SceneId::new(i as i64)),
        2 => Event::Custom(EventId::new(i as i64), Arc::new(Payload::None)),
        _ => Event::ActorEntered(ActorId::new(i as i64)),
    }).collect()
}

fn make_handlers(m: usize) -> ThinVec<Handler> {
    fn no_op(_: &Event, _: &EvalCtx<'_>, _: &mut thin_vec::ThinVec<Effect>) {}
    (0..m).map(|i| Handler {
        matcher: match i % 5 {
            0 => EventMatcher::Tick,
            1 => EventMatcher::SceneEntered,
            2 => EventMatcher::ActorEntered,
            3 => EventMatcher::Any,
            _ => EventMatcher::Custom(EventId::new(i as i64)),
        },
        action: no_op,
    }).collect()
}

#[divan::bench(args = &[(1usize, 1usize), (4, 4), (16, 16)])]
fn handler_dispatch(b: Bencher, args: (usize, usize)) {
    let (n_events, m_handlers) = args;
    let events = make_events(n_events);
    let handlers = make_handlers(m_handlers);

    // Build a minimal EvalCtx the handler invocation can pass through.
    let mut world = World::new(WorldId::new(1));
    let lh = world.spawn_level(LevelId::new(1), "L");
    let sh = world.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let aid = ActorId::new(1);
    let ah = world.spawn_actor(lh, sh, aid, Affine3A::IDENTITY).unwrap();
    let _ = ah;
    world.propagate_transforms();

    let troupe_ids: ThinVec<TroupeId> = thin_vec![];
    let actors = Troupe(thin_vec![]);
    let ctx = EvalCtx {
        world: &world, level_h: lh, stage_h: sh,
        scene_id: SceneId::new(1),
        elapsed: 0.0, tick_count: 0,
        events_seen: &events, actors: &actors, troupes: &troupe_ids,
    };

    let mut sink: thin_vec::ThinVec<Effect> = thin_vec::ThinVec::with_capacity(8);
    b.bench_local(|| {
        sink.clear();
        for ev in &events {
            for h in &handlers {
                if h.matcher.matches(ev) {
                    (h.action)(ev, &ctx, &mut sink);
                }
            }
        }
        black_box(&sink);
    });
}

// ── Troupe iteration — iter_all() flattens nested ThinVec<ThinVec<ActiveActor>> ────

fn build_troupe(n_groups: usize, group_size: usize, lh: LevelHandle, sh: StageHandle) -> Troupe {
    let groups: thin_vec::ThinVec<thin_vec::ThinVec<ActiveActor>> = (0..n_groups).map(|g| {
        (0..group_size).map(|a| ActiveActor::new(
            lh, sh,
            ActorHandle {
                idx: (g * group_size + a) as u32,
                generation: std::num::NonZeroU32::new(1).unwrap(),
                _tag: std::marker::PhantomData,
            },
            ActorId::new((g * group_size + a) as i64 + 1),
        )).collect()
    }).collect();
    Troupe(groups)
}

#[divan::bench(args = &[8usize, 64, 256])]
fn troupe_iter_all(b: Bencher, n: usize) {
    let lh = LevelHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };
    let sh = StageHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };
    let t = build_troupe(2, n / 2, lh, sh);
    b.bench_local(|| {
        let mut sum = 0i64;
        for a in t.iter_all() { sum = sum.wrapping_add(a.actor_id.raw()); }
        sum
    });
}

#[divan::bench(args = &[8usize, 64, 256])]
fn troupe_group_lookup(b: Bencher, n: usize) {
    let lh = LevelHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };
    let sh = StageHandle { idx: 0, generation: std::num::NonZeroU32::new(1).unwrap(), _tag: std::marker::PhantomData };
    let t = build_troupe(4, n / 4, lh, sh);
    b.bench_local(|| {
        let g = t.group(black_box(2));
        black_box(g.map(|s| s.len()).unwrap_or(0))
    });
}
