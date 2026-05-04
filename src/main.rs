use dumpster_fire_engine::resource_manager::*;
use glam::{Affine3A, Vec3};

fn main() {
    println!("=== dumpster_fire_engine scene-graph smoke test ===\n");

    let mut world = World::new();

    // Ownership chain: World → Level → Stage → Actor → SubEntity → Component
    let lh = world.spawn_level("default_level");
    let sh = world.spawn_stage(lh, "default_stage").unwrap();

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
            id:       ItemId::new(7),
            name:     "iron sword".into(),
            visible:  true,
            physical: true,
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
    let actor = &world.levels[lh].stages[sh].actors[ah];
    println!("Actor ActorId:        {:?}", actor.id);
    println!("Actor world position: {:?}", actor.world.translation);
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
}
