# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build               # compile
cargo run                 # run the scene-graph smoke test in src/main.rs
cargo test                # run all tests
cargo test <test_name>    # run a single test by name
cargo bench               # run criterion benchmarks
cargo clippy              # lint
```

## Architecture

`dumpster_fire_engine` is an early-stage Rust game engine (edition 2024). The only fully-implemented subsystem is the scene graph / resource manager (`src/resource_manager/`). All other modules (`render`, `audio`, `physics`, `logic`, `network`, `user`, `util`) are stubs.

### Scene graph hierarchy

```
World
 └─ Level(s)       (LevelKey)
     └─ Stage(s)   (StageKey)
         └─ Actor(s)  (ActorKey)
             └─ SubEntity(s)  (SubEntityKey)
```

`World` (`world_manager/world.rs`) owns four flat `SlotMap`s — one per node type — rather than using nested ownership. Every spawn/despawn method lives on `World` and keeps parent/child bookkeeping consistent across maps. Despawn cascades downward (despawning a Level removes all Stages, Actors, and SubEntities within it).

### Typed ID system

`Id<T>` (`manager.rs`) is a `PhantomData`-branded `i64`. The marker types (`ActorMarker`, `CharacterMarker`, etc.) produce distinct types `ActorId`, `CharacterId`, `ItemId`, `EnvironmentId`, `UtilityId` that are incompatible at compile time. `SubtypeId` is a discriminated enum over the four sub-entity ID types and is the key used in `Actor.children: HashMap<SubtypeId, SubEntityKey>`.

`ActorAddress` (`actor_id.subtype_id`) provides a human-readable, globally-scoped address for any SubEntity.

### SubEntity and ActorType

A `SubEntity` wraps one of four `ActorType` variants: `Character`, `Environment`, `Item`, `Utility`. Each variant carries its own typed ID, name, and flags. The variant's `SubtypeId` is what indexes it in its parent `Actor`'s children map — an actor can hold at most one sub-entity of each `SubtypeId`.

### Component storage

Components are stored on `SubEntity` as a fixed-size array `[Option<Component>; 5]`, not a `HashMap`. `ComponentType` is `#[repr(u8)]` with `const fn index()` so lookup is a direct array index. The five component types are: `Transform`, `Audio`, `Physics`, `Collision`, `Utility`. Benchmarks in `benches/component_storage.rs` validate that the array outperforms `HashMap` for this use case.

### Transform propagation

Both `Actor` and `SubEntity` carry a `local` transform (relative to parent) and a `world` transform (absolute). A `dirty` flag is set whenever `set_actor_local` or `set_sub_entity_local` is called; marking an actor dirty also marks all its child sub-entities dirty. `World::propagate_transforms()` flushes dirty world transforms in two passes (actors first, then sub-entities).

`Transform::compose` uses component-wise Euler addition for rotation — only correct for axis-aligned rotations. A comment in the source flags this for replacement with quaternions before arbitrary orientations are needed.

### Benchmarks

`benches/arena_iteration.rs` — compares `SlotMap` vs `DenseSlotMap` for full iteration, sparse iteration (50 % deleted), and mutable transform propagation across 10 k nodes.

`benches/component_storage.rs` — compares `HashMap`-backed vs fixed-array-backed component storage for single lookup and 10 k entity scan.

### Key dependency

`slotmap` is the only data-structure dependency. `cpal` is pulled in for future audio work but unused. All SlotMap key types are declared with `new_key_type!` so they are strongly typed and non-interchangeable.
