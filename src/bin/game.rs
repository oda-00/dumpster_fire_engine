// Ember Delve — a tiny console dungeon crawler built ENTIRELY out of
// dumpster_fire_engine primitives. Every persistent piece of game state
// (walls, player, enemies, gold, the exit, even score & turn counter) is
// an Actor + SubEntity + Components inside the engine's World. There is
// no parallel std::collections shadow state.
//
// Run with:  cargo run --bin game

use dumpster_fire_engine::resource_manager::*;
use glam::{Affine3A, Vec3};
use std::{io::{self, Write}, sync::Arc};



const MAP: [&str; 10] = [
    "++++++++++++",
    "+1.........+",
    "+.++.......+",
    "+..........+",
    "+.....++...+",
    "+..$...!...+",
    "+...++.....+",
    "+..!.....$>+",
    "+..........+",
    "++++++++++++",
];
const W: usize = 12;
const H: usize = 10;

// ── tiny helpers ────────────────────────────────────────────────────────────

fn at(x: i32, y: i32) -> Affine3A {
    Affine3A::from_translation(Vec3::new(x as f32, y as f32, 0.0))
}

fn cell(world: &World, lh: LevelHandle, sh: StageHandle, ah: ActorHandle) -> (i32, i32) {
    let t = world.levels[lh].stages[sh].worlds[ah.idx as usize].translation;
    (t.x.round() as i32, t.y.round() as i32)
}

// `UtilityComponent.name` is the actor's type tag in this game.
fn tag<'a>(world: &'a World, lh: LevelHandle, sh: StageHandle, ah: ActorHandle) -> &'a str {
    let actor = &world.levels[lh].stages[sh].actors[ah];
    for sub in actor.sub_entities.iter().flatten() {
        if let Some(Component::Utility(u)) = sub.component(ComponentType::Utility) {
            return u.name.as_ref();
        }
    }
    ""
}

fn read_phys(
    world: &World,
    lh: LevelHandle,
    sh: StageHandle,
    ah: ActorHandle,
) -> Option<(f32, f32)> {
    let actor = world.levels[lh].stages[sh].actors.get(ah)?;
    for sub in actor.sub_entities.iter().flatten() {
        if let Some(Component::Physics(p)) = sub.component(ComponentType::Physics) {
            return Some((p.mass, p.velocity.0));
        }
    }
    None
}

fn mutate_phys(
    world: &mut World,
    lh: LevelHandle,
    sh: StageHandle,
    ah: ActorHandle,
    f: impl FnOnce(&mut PhysicsComponent),
) {
    let stage = &mut world.levels[lh].stages[sh];
    if let Some(actor) = stage.actors.get_mut(ah) {
        for sub in actor.sub_entities.iter_mut().flatten() {
            if let Some(Component::Physics(p)) = sub.component_mut(ComponentType::Physics) {
                f(p);
                return;
            }
        }
    }
}

fn gold_value(world: &World, lh: LevelHandle, sh: StageHandle, ah: ActorHandle) -> i32 {
    let actor = &world.levels[lh].stages[sh].actors[ah];
    for sub in actor.sub_entities.iter().flatten() {
        if let ActorType::Item(item) = &sub.actor_type {
            return item.quantity.0 as i32;
        }
    }
    0
}

// Find the first non-meta actor at grid (x,y).  Every actor in this game
// carries a UtilityComponent, so `cache[Utility]` is the universal index.
fn actor_at(
    world: &World,
    lh: LevelHandle,
    sh: StageHandle,
    x: i32,
    y: i32,
    meta: ActorHandle,
) -> Option<ActorHandle> {
    let stage = &world.levels[lh].stages[sh];
    for &ah in &stage.cache[ComponentType::Utility.index()] {
        if ah == meta {
            continue;
        }
        let p = stage.worlds[ah.idx as usize].translation;
        if p.x.round() as i32 == x && p.y.round() as i32 == y {
            return Some(ah);
        }
    }
    None
}

// ── spawners — every map glyph becomes an Actor + SubEntity + Components ────

fn next_id(id: &mut i64) -> i64 {
    *id += 1;
    *id
}

fn spawn_wall(world: &mut World, lh: LevelHandle, sh: StageHandle, id: &mut i64, x: i32, y: i32) {
    let ah = world
        .spawn_actor(lh, sh, ActorId::new(next_id(id)), at(x, y))
        .unwrap();
    let vi = world
        .spawn_sub_entity(
            lh,
            sh,
            ah,
            ActorType::Environment(Environment {
                id: EnvironmentId::new(next_id(id)),
                name: "wall".into(),
                visible: true,
                physical: true,
            }),
            Affine3A::IDENTITY,
        )
        .unwrap();
    world.add_component(
        lh,
        sh,
        ah,
        vi,
        CollisionComponent {
            shape: CollisionShape::Box,
            position: (x as f32, y as f32, 0.0),
            rotation: (0.0, 0.0, 0.0),
            scale: (1.0, 1.0, 1.0),
            collision: true,
        },
    );
    world.add_component(
        lh,
        sh,
        ah,
        vi,
        UtilityComponent {
            name: "wall".into(),
            description: Arc::from(""),
        },
    );
}

fn spawn_player(
    world: &mut World,
    lh: LevelHandle,
    sh: StageHandle,
    id: &mut i64,
    x: i32,
    y: i32,
) -> ActorHandle {
    let ah = world
        .spawn_actor(lh, sh, ActorId::new(next_id(id)), at(x, y))
        .unwrap();
    let vi = world
        .spawn_sub_entity(
            lh,
            sh,
            ah,
            ActorType::Character(Character {
                id: CharacterId::new(next_id(id)),
                name: "player".into(),
                visible: true,
                physical: true,
                playable: true,
            }),
            Affine3A::IDENTITY,
        )
        .unwrap();
    // mass = max HP, velocity.0 = current HP
    world.add_component(
        lh,
        sh,
        ah,
        vi,
        PhysicsComponent {
            mass: 5.0,
            velocity: (5.0, 0.0, 0.0),
            acceleration: (0.0, 0.0, 0.0),
        },
    );
    world.add_component(
        lh,
        sh,
        ah,
        vi,
        UtilityComponent {
            name: "player".into(),
            description: Arc::from(""),
        },
    );
    ah
}

fn spawn_enemy(world: &mut World, lh: LevelHandle, sh: StageHandle, id: &mut i64, x: i32, y: i32) {
    let ah = world
        .spawn_actor(lh, sh, ActorId::new(next_id(id)), at(x, y))
        .unwrap();
    let vi = world
        .spawn_sub_entity(
            lh,
            sh,
            ah,
            ActorType::Character(Character {
                id: CharacterId::new(next_id(id)),
                name: "enemy".into(),
                visible: true,
                physical: true,
                playable: false,
            }),
            Affine3A::IDENTITY,
        )
        .unwrap();
    world.add_component(
        lh,
        sh,
        ah,
        vi,
        PhysicsComponent {
            mass: 2.0,
            velocity: (2.0, 0.0, 0.0),
            acceleration: (0.0, 0.0, 0.0),
        },
    );
    world.add_component(
        lh,
        sh,
        ah,
        vi,
        UtilityComponent {
            name: "enemy".into(),
            description: Arc::from(""),
        },
    );
}

fn spawn_gold(
    world: &mut World,
    lh: LevelHandle,
    sh: StageHandle,
    id: &mut i64,
    x: i32,
    y: i32,
    value: i32,
) {
    let ah = world
        .spawn_actor(lh, sh, ActorId::new(next_id(id)), at(x, y))
        .unwrap();
    let vi = world
        .spawn_sub_entity(
            lh,
            sh,
            ah,
            ActorType::Item(Item {
                id:          ItemId::new(next_id(id)),
                name:        "gold".into(),
                quantity:    (value as u32, value as u32, 0), // current/max/stack
                description: Arc::from("A pile of gold coins"),
                stackable:   false,
                visible:     true,
                physical:    false,
        }),
            Affine3A::IDENTITY,
        )
        .unwrap();
        
        world.add_component(
        lh,
        sh,
        ah,
        vi,
        UtilityComponent {
            name:        "gold".into(),
            description: Arc::from(""),
        },
    );
}

fn spawn_exit(world: &mut World, lh: LevelHandle, sh: StageHandle, id: &mut i64, x: i32, y: i32) {
    let ah = world
        .spawn_actor(lh, sh, ActorId::new(next_id(id)), at(x, y))
        .unwrap();
    let vi = world
        .spawn_sub_entity(
            lh,
            sh,
            ah,
            ActorType::Environment(Environment {
                id: EnvironmentId::new(next_id(id)),
                name: "exit".into(),
                visible: true,
                physical: false,
            }),
            Affine3A::IDENTITY,
        )
        .unwrap();
    world.add_component(
        lh,
        sh,
        ah,
        vi,
        UtilityComponent {
            name: "exit".into(),
            description: Arc::from(""),
        },
    );
}

// Off-map utility actor that holds score (PhysicsComponent.mass) and turn
// counter (PhysicsComponent.velocity.0).  Lives at (-1,-1) so it never
// shows up in spatial queries.
fn spawn_meta(world: &mut World, lh: LevelHandle, sh: StageHandle, id: &mut i64) -> ActorHandle {
    let ah = world
        .spawn_actor(lh, sh, ActorId::new(next_id(id)), at(-1, -1))
        .unwrap();
    let vi = world
        .spawn_sub_entity(
            lh,
            sh,
            ah,
            ActorType::Utility(Utility {
                id: UtilityId::new(next_id(id)),
                name: "meta".into(),
                visible: false,
                toggle: false,
            }),
            Affine3A::IDENTITY,
        )
        .unwrap();
    world.add_component(
        lh,
        sh,
        ah,
        vi,
        PhysicsComponent {
            mass: 0.0,
            velocity: (0.0, 0.0, 0.0),
            acceleration: (0.0, 0.0, 0.0),
        },
    );
    world.add_component(
        lh,
        sh,
        ah,
        vi,
        UtilityComponent {
            name: "meta".into(),
            description: Arc::from(""),
        },
    );
    ah
}

// ── display ─────────────────────────────────────────────────────────────────

fn display(world: &World, lh: LevelHandle, sh: StageHandle, player: ActorHandle, meta: ActorHandle) {
    let mut grid = [['.'; W]; H];
    let stage = &world.levels[lh].stages[sh];

    // Iterate the universal Utility cache; every actor (except meta) is in it.
    for &ah in &stage.cache[ComponentType::Utility.index()] {
        if ah == meta {
            continue;
        }
        let p = stage.worlds[ah.idx as usize].translation;
        let (x, y) = (p.x.round() as i32, p.y.round() as i32);
        if x < 0 || x >= W as i32 || y < 0 || y >= H as i32 {
            continue;
        }
        let glyph = match tag(world, lh, sh, ah) {
            "player" => '1',
            "enemy" => '!',
            "gold" => '$',
            "wall" => '+',
            "exit" => '>',
            _ => continue,
        };
        
        grid[y as usize][x as usize] = glyph;
    }

    let (max_hp, hp) = read_phys(world, lh, sh, player).unwrap_or((0.0, 0.0));
    let (score, turn) = read_phys(world, lh, sh, meta).unwrap_or((0.0, 0.0));

    println!();
    for row in &grid {
        print!("    ");
        for &c in row {
            print!("{}", c);
        }
        println!();
    }
    println!(
        "\n    HP: {}/{}    Score: {}    Turn: {}",
        hp as i32, max_hp as i32, score as i32, turn as i32
    );
}

fn read_cmd() -> String {
    print!("    > ");
    io::stdout().flush().ok();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).ok();
    buf.trim().to_lowercase()
}

// ── main loop ───────────────────────────────────────────────────────────────

fn main() {
    let mut world = World::new(WorldId::new(1));
    let lh = world.spawn_level(LevelId::new(1), "dungeon");
    let sh = world.spawn_stage(lh, StageId::new(1), "floor").unwrap();
    let mut id = 0i64;

    let mut player_h: Option<ActorHandle> = None;
    for (y, row) in MAP.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            let (xi, yi) = (x as i32, y as i32);
            match ch {
                '+' => spawn_wall(&mut world, lh, sh, &mut id, xi, yi),
                '1' => player_h = Some(spawn_player(&mut world, lh, sh, &mut id, xi, yi)),
                '!' => spawn_enemy(&mut world, lh, sh, &mut id, xi, yi),
                '$' => spawn_gold(&mut world, lh, sh, &mut id, xi, yi, 10),
                '>' => spawn_exit(&mut world, lh, sh, &mut id, xi, yi),
                _ => {}
            }
        }
    }
    let player_h = player_h.expect("map must contain '1'");
    let meta_h = spawn_meta(&mut world, lh, sh, &mut id);

    world.propagate_transforms();

    println!("\n  ╔═══════════════════════╗");
    println!("  ║      EMBER DELVE      ║");
    println!("  ╚═══════════════════════╝");
    println!("    wasd = move    q = quit");
    println!("    bump enemies to attack, walk over gold to grab, reach X to escape");

    loop {
        display(&world, lh, sh, player_h, meta_h);

        let dir: (i32, i32) = match read_cmd().as_str() {
            "w" => (0, -1),
            "s" => (0, 1),
            "a" => (-1, 0),
            "d" => (1, 0),
            "q" => {
                println!("    Goodbye.");
                return;
            }
            _ => continue,
        };

        let (px, py) = cell(&world, lh, sh, player_h);
        let (nx, ny) = (px + dir.0, py + dir.1);

        // Resolve what the player walks into.  Clone the tag so we can
        // release the immutable borrow on `world` before mutating.
        let target = actor_at(&world, lh, sh, nx, ny, meta_h)
            .map(|ah| (ah, tag(&world, lh, sh, ah).to_string()));

        match target.as_ref().map(|(ah, t)| (*ah, t.as_str())) {
            Some((_, "wall")) => {
                // blocked — turn does not advance
                continue;
            }
            Some((ah, "enemy")) => {
                mutate_phys(&mut world, lh, sh, ah, |p| p.velocity.0 -= 1.0);
                let (_, hp) = read_phys(&world, lh, sh, ah).unwrap_or((0.0, 0.0));
                if hp <= 0.0 {
                    world.despawn_actor(lh, sh, ah);
                }
            }
            Some((ah, "gold")) => {
                let v = gold_value(&world, lh, sh, ah);
                mutate_phys(&mut world, lh, sh, meta_h, |p| p.mass += v as f32);
                world.despawn_actor(lh, sh, ah);
                world.set_actor_local(lh, sh, player_h, at(nx, ny));
            }
            Some((_, "exit")) => {
                world.set_actor_local(lh, sh, player_h, at(nx, ny));
                world.propagate_transforms();
                display(&world, lh, sh, player_h, meta_h);
                let (score, turn) = read_phys(&world, lh, sh, meta_h).unwrap_or((0.0, 0.0));
                println!(
                    "\n    *** You escaped on turn {}! Final score: {} ***\n",
                    turn as i32, score as i32
                );
                return;
            }
            None => {
                world.set_actor_local(lh, sh, player_h, at(nx, ny));
            }
            _ => {}
        }

        // ── enemy turns ────────────────────────────────────────────────────
        // Pull handles into a scratch Vec so we can mutate the world inside
        // the loop.  Cache contains player, enemies, and meta — filter them.
        let mut enemy_handles: Vec<ActorHandle> = Vec::new();
        for &ah in &world.levels[lh].stages[sh].cache[ComponentType::Physics.index()] {
            if ah != player_h && ah != meta_h {
                enemy_handles.push(ah);
            }
        }

        let (px2, py2) = cell(&world, lh, sh, player_h);
        for eh in enemy_handles {
            // Re-validate; a previous enemy's move can't have killed this one
            // (enemies don't fight each other), but the handle could be stale
            // if despawned this same turn.  Defensive guard.
            if !world.levels[lh].stages[sh].actors.contains(eh) {
                continue;
            }
            let (ex, ey) = cell(&world, lh, sh, eh);
            let dx = (px2 - ex).signum();
            let dy = (py2 - ey).signum();

            for step in [(dx, 0), (0, dy)] {
                if step == (0, 0) {
                    continue;
                }
                let (tx, ty) = (ex + step.0, ey + step.1);
                if (tx, ty) == (px2, py2) {
                    // Bumping the player damages them.
                    mutate_phys(&mut world, lh, sh, player_h, |p| p.velocity.0 -= 1.0);
                    break;
                }
                if actor_at(&world, lh, sh, tx, ty, meta_h).is_some() {
                    continue; // wall, other enemy, gold — blocked
                }
                world.set_actor_local(lh, sh, eh, at(tx, ty));
                break;
            }
        }

        mutate_phys(&mut world, lh, sh, meta_h, |p| p.velocity.0 += 1.0);
        world.propagate_transforms();

        let (_, hp) = read_phys(&world, lh, sh, player_h).unwrap_or((0.0, 0.0));
        if hp <= 0.0 {
            display(&world, lh, sh, player_h, meta_h);
            let (score, turn) = read_phys(&world, lh, sh, meta_h).unwrap_or((0.0, 0.0));
            println!(
                "\n    *** You fell on turn {}.  Final score: {} ***\n",
                turn as i32, score as i32
            );
            return;
        }
    }
}
