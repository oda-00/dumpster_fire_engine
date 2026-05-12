use std::sync::Arc;
use dumpster_fire_engine::{ThinVec, thin_vec};
use dumpster_fire_engine::resource_manager::*;
use glam::{Affine3A, Vec3};

fn main() {
    println!("=== dumpster_fire_engine scene-graph smoke test ===\n");

    let mut world = World::new(WorldId::new(1));

    // Ownership chain: World → Level → Stage → Actor → SubEntity → Component
    let lh = world.spawn_level(LevelId::new(1), "default_level");
    let sh = world.spawn_stage(lh, StageId::new(1), "default_stage").unwrap();

    let ah = world.spawn_actor(
        lh, sh,
        ActorId::new(1),
        Affine3A::from_translation(Vec3::ZERO),
    ).unwrap();

    // Spawn sub-entities; returns variant index (0–3) for later access.
    let bobby_vi = world.spawn_sub_entity(
        lh, sh, ah,
        ActorType::Character(Character {
            id:       CharacterId::new(42),
            name:     "bobby".into(),
            visible:  true,
            physical: true,
            playable: true,
        }),
        Affine3A::from_translation(Vec3::new(10.0, 0.0, 0.0)),
    ).unwrap();

    // Route all component writes through World so Level/Stage caches stay consistent.
    world.add_component(lh, sh, ah, bobby_vi, PhysicsComponent {
        mass:         80.0,
        velocity:     (0.0, 0.0, 0.0),
        acceleration: (0.0, -9.8, 0.0),
    });
    world.add_component(lh, sh, ah, bobby_vi, TransformComponent {
        position:   (10.0, 0.0, 0.0),
        rotation:   (0.0, 0.0, 0.0),
        scale:      (1.0, 1.0, 1.0),
        _transform: true,
    });

    let sword_vi = world.spawn_sub_entity(
        lh, sh, ah,
       ActorType::Item(Item {
            id:          ItemId::new(7),
            name:        "iron sword".into(),
            quantity:    (1, 1, 1),
            description: Arc::from(""),
            stackable:   false,
            visible:     true,
            physical:    true,
}),
        Affine3A::from_scale_rotation_translation(
            glam::Vec3::new(0.2, 1.0, 0.2),
            glam::Quat::IDENTITY,
            glam::Vec3::new(5.0, 0.5, 0.0),
        ),
    ).unwrap();

    world.add_component(lh, sh, ah, sword_vi, CollisionComponent {
        shape:     CollisionShape::Box,
        position:  (5.0, 0.5, 0.0),
        rotation:  (0.0, 0.0, 0.0),
        scale:     (0.2, 1.0, 0.2),
        collision: true,
    });

    world.propagate_transforms();

    // Access actor through the ownership chain.
    let stage_ref = &world.levels[lh].stages[sh];
    let actor = &stage_ref.actors[ah];
    println!("Actor ActorId:        {:?}", actor.id);
    println!("Actor world position: {:?}", stage_ref.worlds[ah.idx as usize].translation);
    println!();

    // Iterate all sub-entities in the stage.
    for actor in world.levels[lh].stages[sh].actors.values() {
        for sub in actor.sub_entities.iter().flatten() {
            let addr = ActorAddress { actor: actor.id, subtype: sub.sub_entity_id() };
            println!("SubEntity Address:    {}", addr);
            println!("SubEntity Name:       {}", sub.name());
            println!("SubEntity local pos:  {:?}", sub.local.translation);
            println!("SubEntity world pos:  {:?}", sub.world.translation);
            println!("SubEntity Subtype ID: {:?}", sub.sub_entity_id());
            println!();
        }
    }

    // Look up Bobby by SubtypeId variant index.
    let bobby_id = SubtypeId::Character(CharacterId::new(42));
    let actor = &world.levels[lh].stages[sh].actors[ah];
    if let Some(bobby) = &actor.sub_entities[bobby_id.variant_idx()] {
        println!("Found Bobby!");
        println!("Bobby name:           {}", bobby.name());
        if let Some(p) = bobby.component(ComponentType::Physics) {
            println!("Bobby physics:        {:?}", p);
        }
        if let Some(t) = bobby.component(ComponentType::Transform) {
            println!("Bobby transform:      {:?}", t);
        }
        println!("Bobby has Physics?    {}", bobby.has_component(ComponentType::Physics));
        println!("Bobby has Audio?      {}", bobby.has_component(ComponentType::Audio));
        println!();
    }

    let sword_id = SubtypeId::Item(ItemId::new(7));
    let actor = &world.levels[lh].stages[sh].actors[ah];
    if let Some(sword) = &actor.sub_entities[sword_id.variant_idx()] {
        println!("Found Sword!");
        println!("Sword name:           {}", sword.name());
        if let Some(c) = sword.component(ComponentType::Collision) {
            println!("Sword collision:      {:?}", c);
        }
        println!();
    }

    // Mutate physics through direct access (caller already holds the reference chain).
    {
        let actor = &mut world.levels[lh].stages[sh].actors[ah];
        if let Some(bobby) = actor.sub_entities[bobby_vi].as_mut()
            && let Some(Component::Physics(p)) = bobby.component_mut(ComponentType::Physics)
        {
            p.velocity = (1.5, 0.0, 0.0);
            println!("Bobby physics mutated: {:?}", p);
        }
    }
    println!();

    // Move Bobby, re-propagate.
    world.set_sub_entity_local(
        lh, sh, ah, bobby_vi,
        Affine3A::from_translation(Vec3::new(20.0, 5.0, 0.0)),
    );
    world.propagate_transforms();
    let actor = &world.levels[lh].stages[sh].actors[ah];
    println!("After moving Bobby:   world pos {:?}", actor.sub_entities[bobby_vi].as_ref().unwrap().world.translation);
    println!();

    // Move parent Actor, re-propagate — both children should shift.
    world.set_actor_local(
        lh, sh, ah,
        Affine3A::from_translation(Vec3::new(100.0, 0.0, 0.0)),
    );
    world.propagate_transforms();
    let actor = &world.levels[lh].stages[sh].actors[ah];
    println!("After moving Actor (parent):");
    println!("  Bobby world pos:    {:?}", actor.sub_entities[bobby_vi].as_ref().unwrap().world.translation);
    println!("  Sword world pos:    {:?}", actor.sub_entities[sword_vi].as_ref().unwrap().world.translation);
    println!();

    // Remove a component via World (keeps caches consistent).
    let removed = world.remove_component::<TransformComponent>(lh, sh, ah, bobby_vi);
    println!("Removed from Bobby:   {:?}", removed.map(|_| ComponentType::Transform));
    let actor = &world.levels[lh].stages[sh].actors[ah];
    println!("Bobby has Transform?  {}", actor.sub_entities[bobby_vi].as_ref().unwrap().has_component(ComponentType::Transform));
    println!();

    println!("(typed-ID separation enforced at compile time)");
    println!();

    // ─────────────────────────────────────────────────────────────────────
    // Event-manager smoke test: HSM + BT + Mealy + Troupes + tick cascade.
    // ─────────────────────────────────────────────────────────────────────
    println!("=== event_manager smoke test ===");

    use thin_vec::thin_vec;

    // Add a second actor (Carol) so AndParallel regions have something to do.
    let ah_carol = world.spawn_actor(
        lh, sh,
        ActorId::new(2),
        Affine3A::from_translation(Vec3::ZERO),
    ).unwrap();
    world.spawn_sub_entity(
        lh, sh, ah_carol,
        ActorType::Character(Character {
            id: CharacterId::new(2),
            name: "carol".into(),
            visible: true, physical: true, playable: false,
        }),
        Affine3A::IDENTITY,
    );

    // SceneIds for the HSM tree.
    let s_root        = SceneId::new(100);
    let s_act1        = SceneId::new(101);
    let s_act2        = SceneId::new(102);
    let s_bobby_enter = SceneId::new(103);
    let s_carol_enter = SceneId::new(104);

    let troupe_chorus = TroupeId::new(1);

    // ── Act1: Atomic with a 2-leaf BT Sequence ────────────────────────────

    let act1_bt = BtNode::Sequence(vec![
        BtNode::leaf(
            Condition::OnTick(0),
            Effect::SetActorLocal {
                level_h: lh, stage_h: sh, actor_h: ah,
                local: Affine3A::from_translation(Vec3::new(50.0, 0.0, 0.0)),
            },
            true,
        ),
        BtNode::leaf(
            Condition::AfterSeconds(0.5),
            Effect::CueTroupe {
                level_h: lh, stage_h: sh,
                troupe: troupe_chorus,
                delta: Affine3A::from_translation(Vec3::new(10.0, 0.0, 0.0)),
            },
            true,
        ),
    ]);

    let act1 = SceneDef {
        id:             s_act1,
        stage:          StageId::new(1),
        parent:         Some(s_root),
        kind:           SceneKind::Atomic,
        troupes:        thin_vec![troupe_chorus],
        initial_actors: thin_vec![thin_vec![
            ActiveActor::new(lh, sh, ah, ActorId::new(1)),
        ]],
        root:           act1_bt,
        on_enter:       thin_vec![],
        on_exit:        thin_vec![],
        handlers:       thin_vec![],
        transitions:    thin_vec![Transition {
            condition: Condition::AfterSeconds(2.0),
            target:    s_act2,
            effects:   Arc::default(),
        }],
    };

    // ── Act2 children: BobbyEnters and CarolEnters (Atomic) ───────────────

    let bobby_enters = SceneDef {
        id:             s_bobby_enter,
        stage:          StageId::new(1),
        parent:         Some(s_act2),
        kind:           SceneKind::Atomic,
        troupes:        thin_vec![],
        initial_actors: thin_vec![],
        root:           BtNode::leaf(
            Condition::OnEnter,
            Effect::SetActorLocal {
                level_h: lh, stage_h: sh, actor_h: ah,
                local: Affine3A::from_translation(Vec3::new(200.0, 0.0, 0.0)),
            },
            true,
        ),
        on_enter:       thin_vec![],
        on_exit:        thin_vec![],
        handlers:       thin_vec![],
        transitions:    thin_vec![],
    };

    let carol_enters = SceneDef {
        id:             s_carol_enter,
        stage:          StageId::new(1),
        parent:         Some(s_act2),
        kind:           SceneKind::Atomic,
        troupes:        thin_vec![],
        initial_actors: thin_vec![],
        root:           BtNode::leaf(
            Condition::OnEnter,
            Effect::SetActorLocal {
                level_h: lh, stage_h: sh, actor_h: ah_carol,
                local: Affine3A::from_translation(Vec3::new(-200.0, 0.0, 0.0)),
            },
            true,
        ),
        on_enter:       thin_vec![],
        on_exit:        thin_vec![],
        handlers:       thin_vec![],
        transitions:    thin_vec![],
    };

    // ── Act2: AndParallel with two regions (one for each actor) ───────────

    let act2 = SceneDef {
        id:             s_act2,
        stage:          StageId::new(1),
        parent:         Some(s_root),
        kind:           SceneKind::AndParallel {
            regions: thin_vec![
                Region {
                    children: thin_vec![s_bobby_enter],
                    initial:  s_bobby_enter,
                    history:  None,
                },
                Region {
                    children: thin_vec![s_carol_enter],
                    initial:  s_carol_enter,
                    history:  None,
                },
            ],
        },
        troupes:        thin_vec![],
        initial_actors: thin_vec![],
        root:           BtNode::empty(),
        on_enter:       thin_vec![],
        on_exit:        thin_vec![],
        handlers:       thin_vec![],
        transitions:    thin_vec![],
    };

    // ── Root: Compound, initial = Act1 ────────────────────────────────────

    let root_def = SceneDef {
        id:             s_root,
        stage:          StageId::new(1),
        parent:         None,
        kind:           SceneKind::Compound {
            children: thin_vec![s_act1, s_act2],
            initial:  s_act1,
            history:  None,
        },
        troupes:        thin_vec![],
        initial_actors: thin_vec![],
        root:           BtNode::empty(),
        on_enter:       thin_vec![],
        on_exit:        thin_vec![],
        handlers:       thin_vec![],
        transitions:    thin_vec![],
    };

    // ── Build script and play ─────────────────────────────────────────────

    let mut script = Script::new(ScriptId::new(1), "thespian_demo", s_root);
    script.add_scene(root_def);
    script.add_scene(act1);
    script.add_scene(act2);
    script.add_scene(bobby_enters);
    script.add_scene(carol_enters);

    let play = Play::instantiate(
        PlayId::new(1),
        "demo_play",
        &script,
        StageId::new(1),
        lh, sh,
    );

    world.levels[lh].stages[sh].set_play(play);

    let bobby_world = |w: &World| w.levels[lh].stages[sh].worlds[ah.idx as usize].translation;
    let carol_world = |w: &World| w.levels[lh].stages[sh].worlds[ah_carol.idx as usize].translation;

    println!("Initial Bobby world pos: {:?}", bobby_world(&world));
    println!("Initial Carol world pos: {:?}", carol_world(&world));
    println!();

    // Drive the cascade. 60 Hz × 2.5 s = 150 ticks.
    for tick_n in 0..150 {
        world.tick(1.0 / 60.0);

        let leaves: ThinVec<i64> = world.levels[lh].stages[sh].play.as_ref()
            .map(|p| p.active_leaves.iter()
                 .map(|&h| p.scenes[h].id.raw())
                 .collect())
            .unwrap_or_default();

        match tick_n {
            0 => println!("t=  0  leaves={:?}  bobby={:?}", leaves, bobby_world(&world)),
            1 => println!("t=  1  leaves={:?}  bobby={:?}", leaves, bobby_world(&world)),
            30 => println!("t= 30  leaves={:?}  bobby={:?} (chorus cued)", leaves, bobby_world(&world)),
            120 => println!("t=120  leaves={:?}", leaves),
            121 => println!("t=121  leaves={:?}", leaves),
            122 => println!("t=122  leaves={:?}  bobby={:?}  carol={:?}",
                leaves, bobby_world(&world), carol_world(&world)),
            _ => {}
        }
    }

    println!();
    println!("Final Bobby world pos: {:?}", bobby_world(&world));
    println!("Final Carol world pos: {:?}", carol_world(&world));
    println!("{}", std::mem::size_of::<Payload>());
}
