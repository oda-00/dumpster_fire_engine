# `dumpster_fire_engine` Code Review

Scope: Rust ~3,800 LoC across `src/` (scene-graph + HSM/BT runtime) and `benches/`.
Focus area for the deep dive: **`src/resource_manager/event_manager/`** — the HSM + BT
machinery is the heart of the engine and where most of the load-bearing invariants live.

Severity legend: **🔴 Bug / unsound** · **🟠 Perf or design risk** · **🟡 Style / nit**

---

## TL;DR

The architecture is *good*. The ownership chain `World → Level → Stage → Actor →
SubEntity → Component`, generational handles, hot/cold SoA split for transforms, and
two‑pass effect collection → application are all genuinely solid choices. The bench
data shows the warmed‑up steady-state tick runs in ~28 µs at the medium scale and
scales roughly linearly to ~9 ms at 160k actors — that's a real engine, not a toy.

The problems are in the runtime layer:

1. `event_manager/scene.rs` **does not currently compile** (Mutex used as Cell, `Cell`
   referenced without import, stray missing comma in `Effect::Clone`, derive(Clone) on
   a struct holding Mutex).
2. `Condition::ActorMovedThisTick` will never fire — nothing sets `ActiveActor::cued`.
3. `Effect::Clone` for `SpawnSubEntity` panics; for `AddComponent` it deep-clones a
   payload that's already wrapped in `Arc`.
4. The hot Pass-1 walk (`Play::collect_effects`) allocates a fresh `Vec<SceneHandle>`
   per active leaf per tick (`ancestors()`), which is the easiest perf win on the table.

Everything else is gravy: typed-ID separation, the static-troupe analysis, the 4-way
unrolled SoA propagate kernel — these are nicely engineered.

---

## Architecture & Design

### What's working

The engine is structured around three orthogonal layers, and each layer owns one job:

`manager.rs` defines the primitives: a flat-Vec generational `Arena<Tag, T>`, typed
`Handle<Tag>` and `Id<T>` so a `LevelHandle` cannot be passed where a `StageHandle` is
expected (compile-time, zero runtime cost — `PhantomData<fn() -> Tag>`). The
`Option<Handle<Tag>>` size optimisation via `NonZeroU32` for the generation is the
right call.

`world_manager/` enforces the ownership chain. `World` knows about `Level`, `Level`
knows about `Stage`, `Stage` knows about `Actor`. Mutations are routed top-down so
caches at each level stay consistent (`Stage::cache` for in-stage component lookups,
`Level::cache` for cross-stage rollups). This is the kind of design that pays back
the moment you start adding features; it would be expensive to retrofit.

`event_manager/` is layered: `Script` (authored) → `Play` (instantiated runtime) →
`Scene` (per-frame state). The four-pass tick (drain Mealy → collect effects readonly
→ apply effects → post_tick bookkeeping → propagate transforms) cleanly separates
read and write, which is what makes the read-only `&World` borrow during effect
collection sound. The static-troupe analysis at `Play::instantiate` is a nice
optimisation: troupes that only ever receive identity cues get fast-pathed in
`Stage::cue_troupe_direct`.

### Design risks

**🟠 The 5-level chain is rigid.** Every mutation that needs to update both Stage and
Level caches goes `World → Level → Stage`. If you ever want a Stage to talk to its
sibling, or a Level to skip a Stage, you'll need a back-channel. The current
delegation (e.g. `World::spawn_actor` → `Level::spawn_actor` → `Stage::spawn_actor`)
is fine for now but starts to feel like ceremony. Worth keeping an eye on as the API
grows.

**🟠 `pub` field exposure on hot data.** `Stage::locals`, `Stage::worlds`,
`Stage::dirty_flags`, `Play::active_leaves`, `Play::scenes` are all `pub`. The smoke
test in `main.rs` and the benches lean on this for direct indexing
(`world.levels[lh].stages[sh].worlds[ah.idx as usize]`). It's ergonomic but
encourages callers to bypass the cache-maintaining mutators. Once you have more than
the smoke test using it, consider a `view()` accessor that returns `(&[Affine3A],
&[Affine3A])` so the invariant ("locals/worlds parallel to actors arena") is harder
to break by mistake.

**🟠 `clone_component` in `scene.rs` exists because `Component` doesn't implement
`Clone`.** Comment says `component.rs` is "owned by `resource_manager` — we don't
touch it." That's a load-bearing comment (it's preventing a one-line fix from being
considered). If the reason `Component` isn't `Clone` is that `UtilityComponent`
contains `String`, that's not a real obstacle. Just `#[derive(Clone)]` and delete the
80 lines of `clone_component` / `clone_shape`. If there's a deeper reason (Component
is meant to be exclusive-ownership-only), then write that down — the current state
suggests an artificial wall.

**🟡 `arch/comps.rs` is a duplicate of `component.rs` minus the macro.** Keep one,
delete the other, or move it to `docs/` and rename. Right now it's a landmine for
"which one is actually used."

**🟡 `pub use module::*;` cascades from `lib.rs`.** Every public name in every
submodule is re-exported flat. The smoke test does `use dumpster_fire_engine::resource_manager::*;`
and gets ~80 symbols. Useful for now, but soon you'll get name collisions (you
already have two `entered` flags — one on `Scene`, one on `ActiveActor`). Consider
re-exporting only the public surface and keeping internals namespaced.

---

## Correctness & Bugs

### 🔴 `scene.rs` does not compile (Mutex / Cell mix-up)

`SceneOperation::fired` is typed `Mutex<bool>` (line 443) but used as `Cell<bool>`:

```
src/resource_manager/event_manager/scene.rs:569
    if op.once && op.fired.get() { ... }
src/resource_manager/event_manager/scene.rs:574
    if op.once { op.fired.set(true); }
src/resource_manager/event_manager/scene.rs:594
    BtNode::Leaf(op) => op.fired.set(false),
```

`Mutex<bool>` has neither `.get()` nor `.set(_)`. The doc comment on the field at
line 441 even says "Cell so pass 1 (read-only against the Scene) can mark a once-op
as fired" — clearly the original was `Cell<bool>` and someone began swapping in
`Mutex` without finishing. Compounding the issue:

- `BtNode::Repeat { ..., current: Cell<u32> }` (line 467) references `Cell` — but
  `std::cell::Cell` is never imported. The only `use` at the top of the file is
  `std::sync::Mutex` and `std::sync::Arc`.
- `#[derive(Clone)] pub struct SceneOperation` (line 437) with a `Mutex<bool>` field
  fails because `Mutex<T>` doesn't implement `Clone` regardless of `T`.

**Fix**: revert `fired: Mutex<bool>` to `fired: Cell<bool>`, add
`use std::cell::Cell;`. The "single-threaded BT, &self tick with interior mutability"
contract is exactly what `Cell` is for. `Mutex` would force a per-leaf lock acquisition
on every BT walk for no concurrency benefit.

### 🔴 Missing comma in `Effect::Clone` impl

```
src/resource_manager/event_manager/scene.rs:224-230
            Effect::AddComponent(b) =>
                Effect::AddComponent(Arc::new(AddComponentEffect {
                    ...
                    component: clone_component(&b.component),
                }))                       // ← needs trailing comma
            Effect::RemoveComponent { ... } =>
```

The match arm body is a function-call expression, not a block, so it needs a comma
before the next arm. Will not parse.

### 🔴 `Condition::ActorMovedThisTick` is dead

```
src/resource_manager/event_manager/scene.rs:339-341
    Condition::ActorMovedThisTick(id) => {
        ctx.actors.iter_all().any(|a| a.actor_id == *id && a.cued)
    }
```

This checks `ActiveActor::cued`, but **nothing in the codebase ever sets
`a.cued = true`**. `Stage::cue_troupe_direct` updates `dirty_flags` and `locals` but
never the `cued` flag on the `ActiveActor` records inside `Scene::actors`. The flag is
unconditionally reset in `Play::post_tick_bookkeeping:290-292`:

```
for a in scene.actors.iter_all_mut() { a.cued = false; }
```

So the condition is permanently false. Either the cue path needs to flip `cued = true`
on every member of the cued troupe, or the condition needs to read from a different
source (e.g. `Stage::dirty_flags`). Given that `cue_troupe_direct` operates with
disjoint-field borrows on Stage, threading `cued` into it would require reaching into
the Play's Scene array — easiest fix is probably to record cued actor-ids in a
per-tick scratch buffer on Stage and have the condition consult it.

### 🔴 `Effect::Clone` panics for `SpawnSubEntity`

```
src/resource_manager/event_manager/scene.rs:234-237
    Effect::SpawnSubEntity(_) => panic!(
        "Effect::SpawnSubEntity is not Clone — ActorType holds non-Clone fields. ..."
    ),
```

`SpawnSubEntity` already wraps its payload in `Arc<SpawnSubEntityEffect>`. There's no
need to clone the payload — `Arc::clone(b)` is sound, cheap, and removes the panic.
The same is true of `Effect::AddComponent`: the current arm allocates a fresh
`Arc<AddComponentEffect>` and *deep-clones the component*. `Arc::clone(b)` is correct
and ~100× faster.

The `panic!` is a footgun: as soon as someone authors a `SpawnSubEntity` inside a
`Transition::effects` (Mealy outputs), the engine deadlocks the play on the next
transition. Tests don't exercise this path because the smoke test only authors
SpawnSubEntity outside transitions.

### 🟠 `Arena::remove` generation wraparound is unsound

```
src/resource_manager/manager.rs:72-73
    slot.generation = NonZeroU32::new(slot.generation.get().wrapping_add(1))
        .unwrap_or(NonZeroU32::new(1).unwrap());
```

When generation hits `u32::MAX`, `wrapping_add(1)` produces `0`, `NonZeroU32::new(0)`
is `None`, and we reset to `1`. But generation `1` is the initial value for fresh
slots — any extant `Handle<T>` from the original generation-1 slot would now
incorrectly validate as live. This is genuinely sound for handles whose generation
wrap around within the same slot lifetime (32-bit roundtrip — billions of removes
per slot — practically unreachable), but it's worth either documenting or fixing
properly: skip generation `1` on wrap (next is `2`), or saturate at `u32::MAX` and
mark the slot permanently dead.

### 🟠 Play queue dispatched only after first leaf

```
src/resource_manager/event_manager/play.rs:214-275
    let mut play_queue_dispatched = false;
    for &leaf in self.active_leaves.iter() {
        ...
        if !play_queue_dispatched {
            for ev in self.queue.iter() { ... }
        }
        ...
        play_queue_dispatched = true;
    }
```

If `active_leaves` is empty for a tick (e.g. all leaves were transitioned out at the
end of the prior tick and the new chain hasn't been entered yet), the play queue
never gets dispatched. In practice this is hard to hit because `apply_transition`
always re-descends to leaves before `post_tick_bookkeeping` returns, but the loop
shouldn't depend on the leaf set being non-empty. Hoist the play-queue dispatch out
of the leaf loop entirely.

### 🟠 `Tick { dt }` event is never queued

`World::apply_effect` line 301: `let _ = Event::Tick { dt: 0.0 };` — labelled "keep
Event in scope so future variants compile." That's a code smell. The `Tick` variant
of `Event` is never actually emitted by the runtime, so a `Handler` matching
`EventMatcher::Tick` will never fire. Either (a) actually push `Event::Tick { dt }`
into `play.queue` at the start of pass 1, or (b) remove the `Tick` event variant and
the `EventMatcher::Tick` matcher.

### 🟠 `propagate_one` writes through a stale-handle path

```
src/resource_manager/world_manager/stage.rs:373-383
fn propagate_one(actor_h, locals, worlds, dirty_flags, actors, cap) {
    let idx = actor_h.idx as usize;
    if idx >= cap { return }
    let actor_world = locals[idx];
    worlds[idx] = actor_world;        // ← writes regardless of generation
    dirty_flags[idx] = false;          // ← clears flag regardless
    if let Some(actor) = actors.get_mut(actor_h) {
        for sub in actor.sub_entities.iter_mut().flatten() { ... }
    }
}
```

The `Arena` generation check happens only inside the `if let`. The two writes above
it execute even for a stale handle. In the path I traced (despawn, then re-spawn into
the same slot, no tick in between) this is benign — `locals[idx]` was already
overwritten by `spawn_actor`, so we publish the right value to `worlds[idx]`. But if
you ever introduce a path where a handle in `dirty_actors` outlives its slot (e.g.
re-using `dirty_actors` across a `Stage::clear()` or similar), you'll silently
corrupt transforms. Cheap fix: check `actors.contains(actor_h)` once at the top.

### 🟡 `if delta == Affine3A::IDENTITY` checked twice in `cue_troupe_direct`

`stage.rs:289` and `stage.rs:312`. Pull it into a `let is_identity = ...` once.

### 🟡 `_rendered`, `_transform`, `_playing`, `entered` (on `Scene`) — unread fields

Several `pub` fields are written but never read for any control-flow decision.
`Scene::entered` is set to `true` in post_tick (line 289) but never consulted; same
for `Scene::_rendered` (set on exit, never read). Either remove or mark `#[allow(dead_code)]`
with a comment explaining why.

---

## Performance

The criterion data in `target/criterion/` (warmed-up, post-transition) gives a clear
breakdown for the medium scale (2 levels × 2 stages × 500 actors = 2k actors):

| phase                | mean    | per-actor |
|----------------------|---------|-----------|
| `collect_effects`    | 738 ns  | 0.4 ns/A  |
| `post_tick`          | 1.6 µs  | 0.8 ns/A  |
| `propagate_transforms` | 14 ns | ~0 (nothing dirty post-warmup) |
| `full_tick`          | 28.5 µs | 14 ns/A   |

The `full_tick - sum-of-phases` gap (~26 µs) is dominated by `apply_effect` —
unsurprising, since BT leaves with `Condition::Always` push effects every frame and
the medium bench has 4 stages × (~6 leaves) = ~24 effects/tick that each route
through `World → Level → Stage`. Scaling is roughly linear: 4L×4S×10000A=160k
actors hits 9.3 ms (≈58 ns/actor). For a 60 Hz frame budget of 16.7 ms, that gives
you ~280k actors before tick alone consumes the frame.

### 🟠 `ancestors()` allocates a `Vec<SceneHandle>` per leaf per tick

```
src/resource_manager/event_manager/play.rs:191-207
fn ancestors(&self, leaf: SceneHandle) -> Vec<SceneHandle> {
    let mut chain = Vec::new();
    ...
}
```

Called inside `collect_effects` (line 217) and `apply_transition` (lines 315, 316).
At medium scale that's 4 stages × 1–2 active leaves × ~3 ancestors = small per call,
but it's an allocation per leaf per tick. Two cheap fixes:

1. Pass a reusable `&mut Vec<SceneHandle>` scratch buffer (live on `Play`).
2. Use `SmallVec<[SceneHandle; 8]>` — HSM trees are shallow, ~all chains fit on the
   stack.

Either should claw back ~5–10% of the steady-state tick time.

### 🟠 `active_configuration()` does the same thing — twice per tick

`play.rs:144-152` builds a `Vec` then `O(n²)`-dedupes via `.contains()`. Called from
`post_tick_bookkeeping:283` every tick. Same fix as above; for typical depths the
contains check is fine, the allocation isn't.

### 🟠 `Effect::Clone` for `AddComponent` deep-clones the payload

`scene.rs:224-229` allocates a fresh `Arc<AddComponentEffect>` and runs
`clone_component()` on the inner. The whole point of the `Arc` is to make this cheap.
Just `Effect::AddComponent(Arc::clone(b))`. Same change for `SpawnSubEntity` (and
remove the panic).

### 🟠 `mealy: t.effects.clone()` on every transition fire

```
src/resource_manager/event_manager/play.rs:268
    out.push(Effect::ScheduleTransition {
        ...,
        mealy: t.effects.clone(),
    });
```

`t.effects` is `ThinVec<Effect>`. Cloning it deep-clones every effect, with the
per-variant penalties above. Wrap mealy in `Arc<[Effect]>` (or `Arc<ThinVec<Effect>>`)
on `Transition` so the clone is a refcount bump. This matters more than it sounds —
once you have many simultaneous transitions, this is a per-fire heap allocation.

### 🟠 `Stage::cache` and `Level::cache` use `Vec::contains` for membership

`stage.rs:186` (`add_component`), `level.rs:91-92` (`despawn_sub_entity`),
`level.rs:110` (`add_component`). Linear scan per add. Fine at 100 actors, painful at
10k. The bench data shows `add_component` isn't on the hot tick path, so this is a
*spawn-time* concern, not a per-tick one. Worth a note in code: "add is O(n) in
component-bearing actors."

### 🟡 `Affine3A::IDENTITY` comparison in `compute_static_troupes`

`play.rs:426, 453`: `*delta != Affine3A::IDENTITY`. Float equality on a 3×4 affine
matrix is exact-bitwise — IDENTITY happens to be all-zero/all-one bit patterns, so
this works, but a single `0.0f32` that arrived via arithmetic vs a literal would
break the fast path silently. Worth a doc comment that this comparison is bit-exact
by design.

### 🟡 `World::tick_effects` reuse via `mem::take`

`world.rs:203-235` is a clever pattern — borrow the buffer out so we can hold
`&mut self`, then put it back with the preserved capacity. Worth a one-line comment
explaining the borrow-checker workaround so the next reader doesn't "simplify" it.

---

## Idiomatic Rust & Style

### 🟡 `_loop`, `_playing`, `_transform`, `_rendered` — leading-underscore public fields

`_loop` is justified (`loop` is a keyword). The others aren't. `playing`, `transform`,
`rendered` would be fine. The leading underscore conventionally marks "intentionally
unused" — but these are public fields the user is presumably meant to read.

### 🟡 `pub use ...::*;` re-export cascade in `lib.rs` and submodule `mod.rs` files

Already noted in Architecture — repeating because it's also a style/idiomatic concern.
Prefer explicit `pub use` of named items.

### 🟡 `wrapping_add(1).unwrap_or(NonZeroU32::new(1).unwrap())`

`manager.rs:72-73`. Use `NonZeroU32::MIN` (= 1) instead of `NonZeroU32::new(1).unwrap()`.
Or better: `slot.generation = slot.generation.checked_add(1).unwrap_or(NonZeroU32::MIN);`
— same behavior, no nested `unwrap_or`.

### 🟡 `Arena::values_mut` — orphan-handle iteration

`manager.rs:94-96`: yields `&mut T` but no handle. Useful for blanket sweeps. Not a
problem; just note that this leaves the door open to mutating an actor without going
through `Stage`'s cache-maintaining mutators. Low priority.

### 🟡 `arch/` directory parallels `src/resource_manager/component.rs`

If `arch/` is meant to be design notes / pre-macro reference, rename to `docs/` and
mark non-buildable. As-is it's confusing — `Cargo.toml` doesn't include it as a
target, but it lives next to source.

### 🟡 `Mutex`-only-import in `scene.rs`

Already covered above. After the fix, line 1 should be `use std::cell::Cell;` —
that's the only interior-mutability primitive needed in this file.

### 🟡 `clone_component`, `clone_shape` — boilerplate that disappears with `#[derive(Clone)]`

Adding `#[derive(Clone)]` to `Component`, the per-component structs, and
`CollisionShape` would delete ~80 LoC and remove a maintenance burden (every new
component type currently requires a new arm in `clone_component`).

### 🟡 The `Mutex<bool>` typo as evidence of incomplete refactor

The doc comment at `scene.rs:441` says "Cell" but the type is `Mutex`. Plus the
`Cell<u32>` reference at line 467 has no import. Plus `derive(Clone)` is broken. Plus
the `Effect::Clone` arm is missing a comma. These four together suggest someone
started a "make Scene Send + Sync" pass and got pulled away. Worth either finishing
the pass (different story — would need different choices: probably an `AtomicBool`
not `Mutex<bool>`) or reverting it cleanly back to `Cell`.

---

## Deep Dive: `event_manager/`

### Pass model

`World::tick(dt)` orchestrates five things (in order):

1. **Drain pending Mealy** outputs from the *previous* tick's transitions (so Mealy
   effects observe a one-tick latency rule). Walks every Level → Stage → Play and
   appends `play.pending_mealy` into the shared effects buffer.
2. **Pass 1 — collect effects (read-only)**. With `&self` borrowed shared, walks
   every Play and ticks its BT against the active configuration. Writes into a
   `Vec<Effect>` scratch buffer.
3. **Pass 2 — apply effects (`&mut self`)**. Drains the buffer; each effect routes
   through `World::apply_effect` to the appropriate mutator.
4. **Pass 3 — post-tick bookkeeping**. Per Play, drains queues, advances
   `elapsed`/`tick_count`, applies any `pending_transition` that pass 2 may have
   stashed.
5. **Pass 4 — propagate transforms**. Visits dirty actors, copies `local → world`,
   composes sub-entity worlds.

This is correct — there's no pass where we read and write the same data through
different borrows — but the invariants are subtle. Worth documenting in
`event_manager/mod.rs`:

- *Effects are deferred*: nothing produced in pass 1 takes effect until pass 2.
- *Transitions have a tick of latency*: schedule in pass 1, apply in pass 3, Mealy
  outputs visible to listeners in next tick's pass 2.
- *`pending_transition` is single-slot*: last-writer-wins per tick. Multiple
  transitions firing in the same tick silently lose all but one. (Document, or change
  the type to `ThinVec<TransitionRecord>`.)

### `apply_transition` — the LCA dance

`play.rs:310-382` computes the lowest common ancestor of source and target by walking
both ancestor chains from the root and finding the divergence point:

```rust
let src_chain = self.ancestors(src_h); // root → src
let tgt_chain = self.ancestors(tgt_h); // root → tgt
let mut lca_idx = 0usize;
let max = src_chain.len().min(tgt_chain.len());
while lca_idx < max && src_chain[lca_idx] == tgt_chain[lca_idx] {
    lca_idx += 1;
}
```

This is correct UML statechart semantics — exit `src..LCA` leaf-first, enter
`LCA..target` root-first. But there are three rough edges:

1. **Self-transition (LCA == src == tgt)** is handled in the `else` branch at
   line 363-371 by descending from the *parent* of src, which would *re-enter src*.
   For a true UML self-transition the spec is "exit src, enter src" — but this code
   exits src via the main path, then enters src's parent's children (potentially
   including src). Probably fine for v1 but the comment "for v1 simplicity" is doing
   load-bearing work; flag this as TODO.
2. **AndParallel exit semantics**: when `src` is inside one region of an
   `AndParallel`, only the leaves descended from `src` are dropped (line 343-348).
   This is correct ("transitions inside one region don't kick concurrent regions").
   Worth a test.
3. **History pseudostate**: written in exit (line 329-331), read in
   `descend_to_leaves:165-167`. Looks correct, but only Compound's `history` is
   honored — `Region::history` (used by AndParallel) is read on initial descent
   (line 177) but never *written*. So AndParallel regions will always re-enter via
   `initial`, never via history. That's a correctness gap if anyone authors a Region
   with `history: Some(_)` expecting it to remember.

### BT semantics

`BtNode::tick` is a textbook BT walker. Three observations:

- `Decorator::Cooldown(_s)` is a no-op (`stage.rs` line 566 just delegates to child).
  Either implement it (track `last_fired` in a Cell) or remove the variant.
- `BtNode::Repeat` shares `child: Arc<BtNode>` across clones, but `current: Cell<u32>`
  is per-clone. So a Repeat over a single BT cloned across two scenes will share the
  *child* (including any Leaf's `fired` Cell, if those Cells live inside the Arc!),
  but each Repeat has its own counter. Mixed sharing — confusing.
- `BtNode::Leaf::fired` (after the Mutex→Cell fix) is shared across clones for any
  Leaf that lives behind an `Arc<BtNode>` (i.e. inside `Repeat::child` or
  `Decorator::child`). Two scenes that both contain a Decorator wrapping the same
  Arc'd leaf will share the `fired` flag. This is almost certainly a bug — once one
  scene fires it, the other can never fire it. Either deep-clone in `Scene::from_def`
  (currently the impl uses `.clone()` on `BtNode`, which Arc-clones the children) or
  push `fired` out of `SceneOperation` into a Scene-local table.

### Static-troupe analysis

`play.rs:394-459` is a nice piece of work: at instantiation, walks every effect that
could ever fire (BT leaves, on_enter, on_exit, transition.effects, recursing into
Mealy chains), buckets `CueTroupe` effects by whether they have a non-identity delta,
and produces the set of "static" troupes. `Stage::cue_troupe_direct` then short-
circuits identity cues against the static set.

One small bug: `compute_static_troupes` walks `def.on_enter`, `def.on_exit`,
`def.transitions`, and `def.root` — but it does *not* walk effects emitted by
`Handler::action` callbacks. Those are function pointers (`fn(&Event, &EvalCtx<'_>,
&mut Vec<Effect>)`), so static analysis can't see what they do. If a handler ever
calls `out.push(Effect::CueTroupe { troupe: T, delta: NON_IDENTITY })`, troupe T
might be incorrectly classified as "static" and have its cues dropped. Worth a doc
comment on the Handler type: "do not emit CueTroupe with non-identity delta from
handlers, or pessimistically declare the troupe non-static elsewhere."

---

## Recommended priority order

1. **Fix the compile errors** in `scene.rs` (Mutex→Cell, import Cell, comma in
   Effect::Clone, derive(Clone) on SceneOperation). The code as committed does not
   build; this is the gating issue for everything else.
2. **Fix `Condition::ActorMovedThisTick`** — currently dead. Decide whether to set
   `cued=true` in `cue_troupe_direct` or to consult `dirty_flags` directly.
3. **Replace `Effect::SpawnSubEntity` panic with `Arc::clone`** and likewise for
   `AddComponent` (perf + removes a runtime bomb).
4. **Pre-allocated scratch buffers on `Play`** for `ancestors()` and
   `active_configuration()` — single biggest steady-state perf win.
5. **`Component: Clone`** + delete `clone_component` / `clone_shape`.
6. **Document the four-pass tick model** and the one-tick Mealy latency rule in
   `event_manager/mod.rs`.
7. **Fold `arch/comps.rs`** into either `component.rs` or `docs/`.
8. **Fix `Region` history not being written** on exit (AndParallel will never
   restore from history).
9. **Audit `Vec::contains` cache lookups** — fine today, will hurt at 10k+ actors.

The architecture is sound. The runtime layer just needs the refactor that started in
`scene.rs` to actually finish.
