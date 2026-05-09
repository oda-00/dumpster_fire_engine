// Microbenchmarks for `Condition::eval` (scene.rs) — one bench per variant.
//
//   cargo bench --bench condition_eval

use divan::{black_box, Bencher};
use glam::{Affine3A, Vec3};
use std::sync::Arc;
use thin_vec::ThinVec;
use dumpster_fire_engine::resource_manager::*;

fn main() { divan::main(); }

// ── Shared fixture: a populated EvalCtx the conditions can read against ────

struct Fixture {
    world: World,
    lh: LevelHandle,
    sh: StageHandle,
    troupe_a: TroupeId,
    troupes: Vec<TroupeId>,
    actors: Troupe,
    events: Vec<Event>,
    actor_ids: Vec<ActorId>,
}

fn build_fixture(troupe_size: usize, n_events: usize) -> Fixture {
    let mut world = World::new(WorldId::new(1));
    let lh = world.spawn_level(LevelId::new(1), "L");
    let sh = world.spawn_stage(lh, StageId::new(1), "S").unwrap();

    let troupe_a = TroupeId::new(1);
    let mut actor_ids = Vec::with_capacity(troupe_size);
    let mut active = Vec::with_capacity(troupe_size);
    for i in 0..troupe_size {
        let aid = ActorId::new(i as i64 + 1);
        let ah = world.spawn_actor(
            lh, sh, aid,
            Affine3A::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
        ).unwrap();
        // Give every actor a Character sub-entity with one Component for
        // ActorHasComponent to find.
        world.spawn_sub_entity(
            lh, sh, ah,
            ActorType::Character(Character {
                id: CharacterId::new(i as i64 + 1),
                name: "n".into(),
                visible: true, physical: true, playable: false,
            }),
            Affine3A::IDENTITY,
        );
        world.add_component(lh, sh, ah, 0, PhysicsComponent {
            mass: 1.0, velocity: (0.0, 0.0, 0.0), acceleration: (0.0, 0.0, 0.0),
        });
        active.push(ActiveActor::new(lh, sh, ah, aid));
        actor_ids.push(aid);
    }
    world.propagate_transforms();

    let actors = Troupe(vec![active]);
    let troupes = vec![troupe_a];

    let events: Vec<Event> = (0..n_events)
        .map(|i| Event::Custom(EventId::new(i as i64), Arc::new(Payload::None)))
        .collect();

    Fixture { world, lh, sh, troupe_a, troupes, actors, events, actor_ids }
}

fn ctx<'a>(f: &'a Fixture) -> EvalCtx<'a> {
    EvalCtx {
        world:       &f.world,
        level_h:     f.lh,
        stage_h:     f.sh,
        scene_id:    SceneId::new(1),
        elapsed:     1.5,
        tick_count:  10,
        events_seen: &f.events,
        actors:      &f.actors,
        troupes:     &f.troupes,
    }
}

// ── Trivial scalar variants ────────────────────────────────────────────────

#[divan::bench]
fn always(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::Always;
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench]
fn never(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::Never;
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench]
fn on_enter(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::OnEnter;
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench]
fn after_seconds(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::AfterSeconds(1.0);
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench]
fn on_tick(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::OnTick(10);
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

// ── Actor-targeted variants (resolve into world arrays) ────────────────────

#[divan::bench]
fn actor_near(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::ActorNear {
        actor: f.actor_ids[3],
        target: Vec3::new(3.5, 0.0, 0.0),
        radius: 1.0,
    };
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench]
fn actor_moved_this_tick(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::ActorMovedThisTick(f.actor_ids[3]);
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench]
fn actor_has_component_hit(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::ActorHasComponent {
        actor: f.actor_ids[3],
        component_type: ComponentType::Physics,
    };
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench]
fn actor_has_component_miss(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::ActorHasComponent {
        actor: f.actor_ids[3],
        component_type: ComponentType::Audio,
    };
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

// ── Troupe quantifiers ─────────────────────────────────────────────────────

#[divan::bench(args = &[8usize, 64, 256])]
fn troupe_all(b: Bencher, n: usize) {
    let f = build_fixture(n, 4);
    let inner = Arc::new(Condition::ActorHasComponent {
        actor: f.actor_ids[0], // ignored — TroupeAll re-targets per member
        component_type: ComponentType::Physics,
    });
    let c = Condition::TroupeAll { troupe: f.troupe_a, predicate: inner };
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench(args = &[8usize, 64, 256])]
fn troupe_any(b: Bencher, n: usize) {
    let f = build_fixture(n, 4);
    let inner = Arc::new(Condition::ActorHasComponent {
        actor: f.actor_ids[0],
        component_type: ComponentType::Physics,
    });
    let c = Condition::TroupeAny { troupe: f.troupe_a, predicate: inner };
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

// ── Event-driven ───────────────────────────────────────────────────────────

#[divan::bench(args = &[1usize, 16, 256])]
fn event_fired_hit_at_end(b: Bencher, n: usize) {
    // Worst case: target id is the last event in the list — full linear scan.
    let f = build_fixture(8, n);
    let target = EventId::new((n.saturating_sub(1)) as i64);
    let c = Condition::EventFired(target);
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench(args = &[1usize, 16, 256])]
fn event_fired_miss(b: Bencher, n: usize) {
    let f = build_fixture(8, n);
    let c = Condition::EventFired(EventId::new(99_999));
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

// ── Combinators ────────────────────────────────────────────────────────────

#[divan::bench(args = &[2usize, 8, 32])]
fn all_n_short_circuit(b: Bencher, n: usize) {
    let f = build_fixture(8, 4);
    // First condition false → short-circuit immediately.
    let mut v: ThinVec<Condition> = ThinVec::new();
    v.push(Condition::Never);
    for _ in 1..n { v.push(Condition::Always); }
    let c = Condition::All(v);
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench(args = &[2usize, 8, 32])]
fn all_n_full_walk(b: Bencher, n: usize) {
    let f = build_fixture(8, 4);
    let mut v: ThinVec<Condition> = ThinVec::new();
    for _ in 0..n { v.push(Condition::Always); }
    let c = Condition::All(v);
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench(args = &[2usize, 8, 32])]
fn any_n_full_walk(b: Bencher, n: usize) {
    let f = build_fixture(8, 4);
    let mut v: ThinVec<Condition> = ThinVec::new();
    for _ in 0..n { v.push(Condition::Never); }
    let c = Condition::Any(v);
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench]
fn not_inv(b: Bencher) {
    let f = build_fixture(8, 4);
    let c = Condition::Not(Arc::new(Condition::Always));
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}

#[divan::bench]
fn custom_fnptr(b: Bencher) {
    fn always_true(_: &EvalCtx<'_>) -> bool { true }
    let f = build_fixture(8, 4);
    let c = Condition::Custom(always_true);
    b.bench_local(|| black_box(c.eval(&ctx(&f))));
}
