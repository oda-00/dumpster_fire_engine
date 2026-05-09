// Microbenchmarks for the SoA transform kernels in stage.rs:
// - `Stage::propagate_transforms` (4× unrolled inner loop, dirty-only)
// - `Stage::set_actor_local` / `set_sub_entity_local` (dirty-flag write paths)
// - `Stage::cue_troupe_direct` via integrated Play setup
//
//   cargo bench --bench transform_kernels

use divan::{black_box, Bencher};
use glam::{Affine3A, Vec3};
use thin_vec::thin_vec;
use dumpster_fire_engine::resource_manager::*;

fn main() { divan::main(); }

const SIZES: &[usize] = &[1, 4, 16, 64, 256, 1024, 10_000];

fn build_world_with_actors(n: usize) -> (World, LevelHandle, StageHandle, Vec<ActorHandle>) {
    let mut world = World::new(WorldId::new(1));
    let lh = world.spawn_level(LevelId::new(1), "L");
    let sh = world.spawn_stage(lh, StageId::new(1), "S").unwrap();
    let actors: Vec<_> = (0..n).map(|i| {
        world.spawn_actor(
            lh, sh, ActorId::new(i as i64 + 1),
            Affine3A::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
        ).unwrap()
    }).collect();
    world.propagate_transforms();
    (world, lh, sh, actors)
}

fn dirty_first_n(world: &mut World, lh: LevelHandle, sh: StageHandle, actors: &[ActorHandle], n: usize) {
    for &h in actors.iter().take(n) {
        world.set_actor_local(lh, sh, h, Affine3A::from_translation(Vec3::new(1.0, 0.0, 0.0)));
    }
}

#[divan::bench(args = SIZES)]
fn propagate_dirty_dense(b: Bencher, n: usize) {
    b.with_inputs(|| {
        let (mut w, lh, sh, actors) = build_world_with_actors(n);
        dirty_first_n(&mut w, lh, sh, &actors, n);
        w
    }).bench_local_values(|mut w| {
        w.propagate_transforms();
        w
    });
}

#[divan::bench(args = SIZES)]
fn propagate_dirty_sparse_10pct(b: Bencher, n: usize) {
    let total = n.saturating_mul(10).max(n);
    b.with_inputs(|| {
        let (mut w, lh, sh, actors) = build_world_with_actors(total);
        dirty_first_n(&mut w, lh, sh, &actors, n);
        w
    }).bench_local_values(|mut w| {
        w.propagate_transforms();
        w
    });
}

#[divan::bench]
fn set_actor_local_first_dirty(b: Bencher) {
    let (mut w, lh, sh, actors) = build_world_with_actors(1024);
    let t = Affine3A::from_translation(Vec3::new(2.0, 0.0, 0.0));
    b.bench_local(|| {
        for &h in &actors {
            w.set_actor_local(lh, sh, h, t);
        }
        w.propagate_transforms();
    });
}

#[divan::bench]
fn set_actor_local_already_dirty(b: Bencher) {
    let (mut w, lh, sh, actors) = build_world_with_actors(1024);
    for &h in &actors {
        w.set_actor_local(lh, sh, h, Affine3A::IDENTITY);
    }
    let t = Affine3A::from_translation(Vec3::new(2.0, 0.0, 0.0));
    b.bench_local(|| {
        for &h in &actors {
            w.set_actor_local(lh, sh, h, t);
        }
    });
}

#[divan::bench]
fn set_sub_entity_local_first_dirty(b: Bencher) {
    let (mut w, lh, sh, actors) = build_world_with_actors(1024);
    for (i, &ah) in actors.iter().enumerate() {
        w.spawn_sub_entity(
            lh, sh, ah,
            ActorType::Character(Character {
                id: CharacterId::new(i as i64 + 1),
                name: "n".into(),
                visible: true, physical: true, playable: false,
            }),
            Affine3A::IDENTITY,
        );
    }
    w.propagate_transforms();

    let t = Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0));
    let vi = 0;
    b.bench_local(|| {
        for &h in &actors {
            w.set_sub_entity_local(lh, sh, h, vi, t);
        }
        w.propagate_transforms();
    });
}

// ── cue_troupe_direct: three branches ──────────────────────────────────────

fn build_play_with_troupe(n: usize, identity_only: bool)
    -> (World, LevelHandle, StageHandle, TroupeId)
{
    let (mut w, lh, sh, actors) = build_world_with_actors(n);
    let troupe = TroupeId::new(1);

    let actives: Vec<ActiveActor> = actors.iter().enumerate().map(|(i, h)|
        ActiveActor::new(lh, sh, *h, ActorId::new(i as i64 + 1))).collect();

    let bt = if identity_only {
        BtNode::leaf(
            Condition::Always,
            Effect::CueTroupe { level_h: lh, stage_h: sh, troupe, delta: Affine3A::IDENTITY },
            false,
        )
    } else {
        BtNode::leaf(
            Condition::Always,
            Effect::CueTroupe {
                level_h: lh, stage_h: sh, troupe,
                delta: Affine3A::from_translation(Vec3::new(0.001, 0.0, 0.0)),
            },
            false,
        )
    };

    let scene = SceneDef {
        id: SceneId::new(1), stage: StageId::new(1), parent: None,
        kind: SceneKind::Atomic,
        troupes: thin_vec![troupe],
        initial_actors: thin_vec![actives.iter().cloned().collect()],
        root: bt,
        on_enter: thin_vec![], on_exit: thin_vec![],
        handlers: thin_vec![], transitions: thin_vec![],
    };
    let mut script = Script::new(ScriptId::new(1), "s", SceneId::new(1));
    script.add_scene(scene);
    let play = Play::instantiate(PlayId::new(1), "p", &script, StageId::new(1), lh, sh);
    w.levels[lh].stages[sh].set_play(play);
    w.tick(1.0 / 60.0);
    (w, lh, sh, troupe)
}

#[divan::bench(args = &[64usize, 1024, 10_000])]
fn cue_troupe_static_skip(b: Bencher, n: usize) {
    let (mut w, lh, sh, troupe) = build_play_with_troupe(n, true);
    b.bench_local(|| {
        let stage = &mut w.levels[lh].stages[sh];
        stage.cue_troupe_direct(black_box(troupe), Affine3A::IDENTITY);
    });
}

#[divan::bench(args = &[64usize, 1024, 10_000])]
fn cue_troupe_identity_block(b: Bencher, n: usize) {
    let (mut w, lh, sh, troupe) = build_play_with_troupe(n, false);
    b.bench_local(|| {
        let stage = &mut w.levels[lh].stages[sh];
        stage.cue_troupe_direct(black_box(troupe), Affine3A::IDENTITY);
    });
}

#[divan::bench(args = &[64usize, 1024, 10_000])]
fn cue_troupe_delta_block(b: Bencher, n: usize) {
    let (mut w, lh, sh, troupe) = build_play_with_troupe(n, false);
    let delta = Affine3A::from_translation(Vec3::new(0.5, 0.0, 0.0));
    b.bench_local(|| {
        let stage = &mut w.levels[lh].stages[sh];
        stage.cue_troupe_direct(black_box(troupe), black_box(delta));
    });
}
