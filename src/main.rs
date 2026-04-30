use dumpster_fire_engine::resource_manager::*;

fn main() {
    println!("=== dumpster_fire_engine scene-graph smoke test ===\n");

    let mut world = World::new();
    let level = world.spawn_level("default_level");
    let stage = world.spawn_stage(level, "default_stage").unwrap();

    let actor = world.spawn_actor(
        stage,
        ActorId::new(1),
        Transform::new((0.0, 0.0, 0.0), (0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
    ).unwrap();

    let bobby_key = world.spawn_sub_entity(
        actor,
        ActorType::Character(Character {
            id: CharacterId::new(42),
            name: "bobby".into(),
            visible: true,
            physical: true,
            playable: true,
        }),
        Transform::new((10.0, 0.0, 0.0), (0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
    ).unwrap();

    world.sub_entities[bobby_key].add_component(Component::Physics(PhysicsComponent {
        mass: 80.0,
        velocity: (0.0, 0.0, 0.0),
        acceleration: (0.0, -9.8, 0.0),
    }));
    world.sub_entities[bobby_key].add_component(Component::Transform(TransformComponent {
        position: (10.0, 0.0, 0.0),
        rotation: (0.0, 0.0, 0.0),
        scale: (1.0, 1.0, 1.0),
        _transform: true,
    }));

    let sword_key = world.spawn_sub_entity(
        actor,
        ActorType::Item(Item {
            id: ItemId::new(7),
            name: "iron sword".into(),
            visible: true,
            physical: true,
        }),
        Transform::new((5.0, 0.5, 0.0), (0.0, 0.0, 0.0), (0.2, 1.0, 0.2)),
    ).unwrap();

    world.sub_entities[sword_key].add_component(Component::Collision(CollisionComponent {
        shape: CollisionShape::Box,
        position: (5.0, 0.5, 0.0),
        rotation: (0.0, 0.0, 0.0),
        scale: (0.2, 1.0, 0.2),
        collision: true,
    }));

    world.propagate_transforms();

    println!("Actor ActorId:        {:?}", world.actors[actor].id);
    println!("Actor world position: {:?}", world.actors[actor].world.position);
    println!();

    for (sub_key, sub) in &world.sub_entities {
        let addr = ActorAddress {
            actor: world.actors[sub.parent].id,
            subtype: sub.sub_entity_id(),
        };
        println!("SubEntity Address:    {}", addr);
        println!("SubEntity Name:       {}", sub.name());
        println!("SubEntity local pos:  {:?}", sub.local.position);
        println!("SubEntity world pos:  {:?}", sub.world.position);
        println!("SubEntity Subtype ID: {:?}", sub.sub_entity_id());
        println!("(handle: {:?})", sub_key);
        println!();
    }

    let bobby_id = SubtypeId::Character(CharacterId::new(42));
    if let Some(&bobby_handle) = world.actors[actor].children.get(&bobby_id) {
        let bobby = &world.sub_entities[bobby_handle];
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
    if let Some(&sword_handle) = world.actors[actor].children.get(&sword_id) {
        let sword = &world.sub_entities[sword_handle];
        println!("Found Sword!");
        println!("Sword name:           {}", sword.name());
        if let Some(c) = sword.component(ComponentType::Collision) {
            println!("Sword collision:      {:?}", c);
        }
        println!();
    }

    if let Some(Component::Physics(p)) =
        world.sub_entities[bobby_key].component_mut(ComponentType::Physics)
    {
        p.velocity = (1.5, 0.0, 0.0);
        println!("Bobby physics mutated: {:?}", p);
    }
    println!();

    world.set_sub_entity_local(
        bobby_key,
        Transform::new((20.0, 5.0, 0.0), (0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
    );
    world.propagate_transforms();
    println!("After moving Bobby:   world pos {:?}", world.sub_entities[bobby_key].world.position);
    println!();

    world.set_actor_local(
        actor,
        Transform::new((100.0, 0.0, 0.0), (0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
    );
    world.propagate_transforms();
    println!("After moving Actor (parent):");
    println!("  Bobby world pos:    {:?}", world.sub_entities[bobby_key].world.position);
    println!("  Sword world pos:    {:?}", world.sub_entities[sword_key].world.position);
    println!();

    let removed = world.sub_entities[bobby_key].remove_component(ComponentType::Transform);
    println!("Removed from Bobby:   {:?}", removed.map(|c| c.component_type()));
    println!("Bobby has Transform?  {}", world.sub_entities[bobby_key].has_component(ComponentType::Transform));
    println!();

    println!("(typed-ID separation enforced at compile time)");
}
