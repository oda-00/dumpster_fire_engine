use std::sync::Arc;
use glam::Affine3A;
use thin_vec::ThinVec;
use crate::resource_manager::component::{Component, ComponentType};
use crate::resource_manager::manager::{
    Actor, ActorHandle, ActorId, ActorTag, ActorType, Arena, Id, SubEntity,
};

pub struct StageMarker;
pub type StageId = Id<StageMarker>;

/// Cache-friendly initial capacity for `Stage::actors`, `dirty_actors`, and
/// `cue_scratch`. Chosen so the underlying `Vec`s start with a power-of-two
/// multiple of a 64-byte cache line and avoid reallocation on the common path.
/// Stages with more actors will still grow dynamically, but the initial
/// allocation covers the typical case without thrash.
pub const MAX_ACTORS_PER_STAGE: usize = 1024;

// ── Stage ────────────────────────────────────────────────────────────────────
//
// Stage is the source of truth for its Actors. It owns them in a flat Arena so
// insertion and removal are O(1) with generational safety.
//
// Component cache: for each ComponentType, cache holds the ActorHandles of every
// actor in this Stage that has at least one sub-entity carrying that component.
// "Give me all Physics actors in this Stage" = O(matching) slice read.
//
// Dirty list: set_actor_local and spawn_actor push handles here so
// propagate_transforms only visits actors that actually changed.

pub struct Stage {
    pub id:       StageId,
    pub name:     Arc<str>,
    pub actors:   Arena<ActorTag, Actor>,
    // cache[i] = handles of actors that have ≥1 sub-entity with ComponentType index i
    pub cache:    [Vec<ActorHandle>; ComponentType::COUNT],
    dirty_actors: ThinVec<ActorHandle>,
    pub play:     Option<crate::resource_manager::event_manager::Play>,
    /// Reusable scratch buffer for cue_troupe_direct so per-cue fan-out
    /// stays allocation-free in the hot path.
    cue_scratch:  ThinVec<ActorHandle>,

    // ── SoA hot transform arrays — parallel to `actors`, indexed by `ah.idx`
    //
    // Two parallel arrays: `locals` (actor-local space) and `worlds` (global
    // space). propagate_transforms processes them with a 4-way unroll so the
    // OOO core can keep multiple independent loads/stores in flight at once.
    //
    // Slot lifecycle: Stage::spawn_actor synchronizes these with the Arena
    // (push-on-grow, overwrite on free-list reuse). Stage::despawn_actor leaves
    // the entries as garbage; next reuse overwrites. Validity is gated by the
    // corresponding Arena slot's generation, same as Actor itself.
    pub locals:      Vec<Affine3A>,
    pub worlds:      Vec<Affine3A>,
    pub dirty_flags: Vec<bool>,
}

impl Stage {
    pub fn new(id: StageId, name: impl Into<Arc<str>>) -> Self {
        let mut dirty_actors = ThinVec::with_capacity(MAX_ACTORS_PER_STAGE);
        let mut cue_scratch  = ThinVec::with_capacity(MAX_ACTORS_PER_STAGE);
        // ThinVec::with_capacity is a request, not a guarantee on a fresh vec.
        // Force the allocation now so the first cue/dirty push doesn't trigger one.
        dirty_actors.reserve(MAX_ACTORS_PER_STAGE);
        cue_scratch.reserve(MAX_ACTORS_PER_STAGE);

        Self {
            id,
            name:         name.into(),
            actors:       Arena::with_capacity(MAX_ACTORS_PER_STAGE),
            cache:        std::array::from_fn(|_| Vec::with_capacity(MAX_ACTORS_PER_STAGE)),
            dirty_actors,
            play:         None,
            cue_scratch,
            locals:       Vec::with_capacity(MAX_ACTORS_PER_STAGE),
            worlds:       Vec::with_capacity(MAX_ACTORS_PER_STAGE),
            dirty_flags:  Vec::with_capacity(MAX_ACTORS_PER_STAGE),
        }
    }

    pub fn set_play(&mut self, play: crate::resource_manager::event_manager::Play) {
        self.play = Some(play);
    }

    pub fn take_play(&mut self) -> Option<crate::resource_manager::event_manager::Play> {
        self.play.take()
    }

    // ── Spawn / despawn ───────────────────────────────────────────────────

    pub fn spawn_actor(&mut self, id: ActorId, local: Affine3A) -> ActorHandle {
        let h = self.actors.insert(Actor {
            id,
            sub_entities: std::array::from_fn(|_| None),
        });
        let idx = h.idx as usize;
        // Either grow (fresh slot at end) or overwrite a freed slot reused by Arena.
        if idx == self.locals.len() {
            self.locals.push(local);
            self.worlds.push(Affine3A::IDENTITY);
            self.dirty_flags.push(true);
        } else {
            self.locals[idx] = local;
            self.worlds[idx] = Affine3A::IDENTITY;
            self.dirty_flags[idx] = true;
        }
        self.dirty_actors.push(h);
        h
    }

    /// Returns the variant index (0–3) on success, None if the actor is unknown
    /// or the slot is already occupied by another sub-entity of the same type.
    pub fn spawn_sub_entity(
        &mut self,
        actor_h: ActorHandle,
        actor_type: ActorType,
        local: Affine3A,
    ) -> Option<usize> {
        let variant_idx = actor_type.index();
        let actor = self.actors.get_mut(actor_h)?;
        if actor.sub_entities[variant_idx].is_some() {
            return None;
        }
        actor.sub_entities[variant_idx] = Some(SubEntity {
            actor_type,
            local,
            world: Affine3A::IDENTITY,
            dirty: true,
            components: std::array::from_fn(|_| None),
        });
        let idx = actor_h.idx as usize;
        if !self.dirty_flags[idx] {
            self.dirty_flags[idx] = true;
            self.dirty_actors.push(actor_h);
        }
        Some(variant_idx)
    }

    pub fn despawn_actor(&mut self, actor_h: ActorHandle) {
        if self.actors.remove(actor_h).is_none() { return }
        // Broad sweep: evict from all cache slots (safe and simple for ≤5 slots).
        for cache_slot in self.cache.iter_mut() {
            Self::cache_remove_actor(cache_slot, actor_h);
        }
        if let Some(pos) = self.dirty_actors.iter().position(|&h| h == actor_h) {
            self.dirty_actors.swap_remove(pos);
        }
    }

    /// Removes one sub-entity slot from an actor. Returns the removed sub-entity
    /// so callers (e.g. Level::despawn_sub_entity) can inspect what it held.
    pub fn despawn_sub_entity(
        &mut self,
        actor_h: ActorHandle,
        variant_idx: usize,
    ) -> Option<SubEntity> {
        let actor = self.actors.get_mut(actor_h)?;
        let sub = actor.sub_entities[variant_idx].take()?;
        // Per component the sub held: evict actor from cache if no other sub carries it.
        for ct_idx in 0..ComponentType::COUNT {
            if sub.components[ct_idx].is_some() {
                let still_has = actor.sub_entities.iter()
                    .filter_map(|s| s.as_ref())
                    .any(|s| s.components[ct_idx].is_some());
                if !still_has {
                    Self::cache_remove_actor(&mut self.cache[ct_idx], actor_h);
                }
            }
        }
        Some(sub)
    }

    // ── Component access ──────────────────────────────────────────────────

    /// Writes a component into the sub-entity at variant_idx and updates cache.
    pub fn add_component(
        &mut self,
        actor_h: ActorHandle,
        variant_idx: usize,
        comp: Component,
    ) -> bool {
        let ct = comp.component_type();
        let Some(actor) = self.actors.get_mut(actor_h) else { return false };
        let Some(sub) = actor.sub_entities[variant_idx].as_mut() else { return false };
        sub.components[ct.index()] = Some(comp);
        if !self.cache[ct.index()].contains(&actor_h) {
            self.cache[ct.index()].push(actor_h);
        }
        true
    }

    /// Removes a component from the sub-entity at variant_idx and updates cache.
    pub fn remove_component(
        &mut self,
        actor_h: ActorHandle,
        variant_idx: usize,
        ct: ComponentType,
    ) -> Option<Component> {
        let actor = self.actors.get_mut(actor_h)?;
        let removed = actor.sub_entities[variant_idx].as_mut()?.components[ct.index()].take()?;
        let still_has = actor.sub_entities.iter()
            .filter_map(|s| s.as_ref())
            .any(|s| s.components[ct.index()].is_some());
        if !still_has {
            Self::cache_remove_actor(&mut self.cache[ct.index()], actor_h);
        }
        Some(removed)
    }

    // ── Transform propagation ─────────────────────────────────────────────

    #[inline]
    pub fn set_actor_local(&mut self, actor_h: ActorHandle, t: Affine3A) {
        // Generation gate via the Arena, then directly index the SoA arrays.
        if !self.actors.contains(actor_h) { return }
        let idx = actor_h.idx as usize;
        self.locals[idx] = t;
        if !self.dirty_flags[idx] {
            self.dirty_flags[idx] = true;
            self.dirty_actors.push(actor_h);
        }
    }

    #[inline]
    pub fn set_sub_entity_local(&mut self, actor_h: ActorHandle, variant_idx: usize, t: Affine3A) {
        let Some(actor) = self.actors.get_mut(actor_h) else { return };
        if let Some(sub) = actor.sub_entities[variant_idx].as_mut() {
            sub.local = t;
            sub.dirty = true;
        }
        let idx = actor_h.idx as usize;
        if !self.dirty_flags[idx] {
            self.dirty_flags[idx] = true;
            self.dirty_actors.push(actor_h);
        }
    }

    // ── Event-manager tick gears ──────────────────────────────────────────
    //
    // Stage drives its owned Play. Pass 1 is read-only (collect_effects);
    // Pass 3 is mut bookkeeping. Effects target World mutators and are applied
    // by World::apply_effect between passes.

    pub fn collect_effects(
        &self,
        dt: f32,
        world: &crate::resource_manager::world_manager::world::World,
        sink: &mut Vec<crate::resource_manager::event_manager::Effect>,
    ) {
        if let Some(play) = self.play.as_ref() {
            play.collect_effects(dt, world, sink);
        }
    }

    pub fn post_tick(&mut self, dt: f32) {
        if let Some(play) = self.play.as_mut() {
            play.post_tick_bookkeeping(dt);
        }
    }

    pub fn drain_pending_mealy(
        &mut self,
        sink: &mut Vec<crate::resource_manager::event_manager::Effect>,
    ) {
        if let Some(play) = self.play.as_mut() {
            play.drain_pending_mealy(sink);
        }
    }

    /// Direct-write troupe cue. Walks the active scenes' rosters for the named
    /// troupe and applies `delta` to every member's local transform in place,
    /// bypassing the per-actor World→Level→Stage routing chain.
    ///
    /// The inner loop is unrolled 4× so the OOO core can keep multiple
    /// `Affine3A` SIMD multiplies in flight at once (each is independent across
    /// actors). When `delta` equals identity we skip the multiply entirely
    /// and only update dirty bookkeeping.
    pub fn cue_troupe_direct(
        &mut self,
        troupe: crate::resource_manager::event_manager::TroupeId,
        delta: Affine3A,
    ) {
        // Disjoint-field borrow: we mutate `cue_scratch` while reading/mutating `play`.
        let Self { play, cue_scratch, locals, dirty_flags, dirty_actors, .. } = self;
        let is_identity = delta == Affine3A::IDENTITY;

        // Static-troupe fast path: if the script never moves this troupe AND
        // this particular cue's delta is identity, the only observable effect
        // would be a redundant dirty mark. Skip the whole walk.
        if is_identity
            && let Some(play) = play.as_ref()
            && play.static_troupes.contains(&troupe)
        {
            return;
        }

        cue_scratch.clear();

        // TODO: Condition::ActorMovedThisTick (scene.rs) reads ActiveActor.cued,
        // but nothing currently sets it. Setting it here would mean mutating
        // every troupe member's ActiveActor every tick — measured ~5MB/tick of
        // extra cache traffic at xlarge scales. Until ActorMovedThisTick has
        // a real consumer, leave `cued` dead. When wiring it up, prefer a
        // tick-local Vec<ActorId> on the Stage that the condition consults,
        // rather than mutating per-actor state in the cue hot path.
        if let Some(play) = play.as_ref() {
            for &leaf in play.active_leaves.iter() {
                let scene = &play.scenes[leaf];
                if let Some(idx) = scene.troupe_idx(troupe)
                    && let Some(group) = scene.actors.group(idx)
                {
                    for a in group { cue_scratch.push(a.actor_h); }
                }
            }
        }

        // Identity short-circuit: ambient cues that re-publish the current
        // pose still want every member marked dirty/cued, but pay no math.
        if is_identity {
            apply_identity_block_soa(locals, dirty_flags, dirty_actors, cue_scratch);
        } else {
            apply_delta_block_soa(locals, dirty_flags, dirty_actors, cue_scratch, delta);
        }
    }

    /// Recomputes world transforms for dirty actors and their sub-entities.
    /// O(dirty count), not O(all actors). 4-way unrolled inner loop so the
    /// OOO core can keep multiple actor publishes (locals → worlds copy,
    /// dirty_flag clear, sub-entity composition) in flight at once.
    pub fn propagate_transforms(&mut self) {
        let Self { dirty_actors, locals, worlds, dirty_flags, actors, .. } = self;
        let n = dirty_actors.len();
        let cap = locals.len();

        let mut i = 0;
        while i + 4 <= n {
            let h0 = dirty_actors[i];
            let h1 = dirty_actors[i + 1];
            let h2 = dirty_actors[i + 2];
            let h3 = dirty_actors[i + 3];
            propagate_one(h0, locals, worlds, dirty_flags, actors, cap);
            propagate_one(h1, locals, worlds, dirty_flags, actors, cap);
            propagate_one(h2, locals, worlds, dirty_flags, actors, cap);
            propagate_one(h3, locals, worlds, dirty_flags, actors, cap);
            i += 4;
        }
        while i < n {
            propagate_one(dirty_actors[i], locals, worlds, dirty_flags, actors, cap);
            i += 1;
        }

        dirty_actors.clear();
    }

    /// Number of actors with a pending transform update. Used by the parallel
    /// propagate threshold in `World::propagate_transforms`.
    #[inline]
    pub fn dirty_count(&self) -> usize { self.dirty_actors.len() }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn cache_remove_actor(cache_slot: &mut Vec<ActorHandle>, actor_h: ActorHandle) {
        if let Some(pos) = cache_slot.iter().position(|&h| h == actor_h) {
            cache_slot.swap_remove(pos);
        }
    }
}

// ── Propagate inner kernel ──────────────────────────────────────────────────
//
// One actor's full publish: copy local → world, clear dirty_flag, walk
// sub-entities composing each one against the just-published actor world.
// Marked `#[inline(always)]` so the 4-way unrolled call site inlines it and
// LLVM sees four independent dependency chains to schedule.

#[inline(always)]
fn propagate_one(
    actor_h: ActorHandle,
    locals: &[Affine3A],
    worlds: &mut [Affine3A],
    dirty_flags: &mut [bool],
    actors: &mut Arena<ActorTag, Actor>,
    cap: usize,
) {
    let idx = actor_h.idx as usize;
    if idx >= cap { return }
    // Note: the `actors.get_mut` below gates sub-entity composition behind a
    // generation check. The two SoA writes that follow execute unconditionally,
    // which is intentional: in normal operation `dirty_actors` only contains
    // live handles (spawn pushes, despawn drops). The pathological case (a
    // stale handle stranded in dirty_actors) writes correct data to the slot
    // because spawn_actor overwrites `locals[idx]` before any tick can read it.
    let actor_world = locals[idx];
    worlds[idx] = actor_world;
    dirty_flags[idx] = false;
    if let Some(actor) = actors.get_mut(actor_h) {
        for sub in actor.sub_entities.iter_mut().flatten() {
            sub.world = actor_world * sub.local;
            sub.dirty = false;
        }
    }
}

// ── SoA cue-batch helpers ───────────────────────────────────────────────────
//
// Free functions so disjoint-field borrows on Stage pass through cleanly.
// Iterates a tightly-packed `Vec<Affine3A>` (the SoA hot array) — one cache
// line of work per actor instead of pulling 1-2 KB Actor cache lines.
// 4-way unrolled so LLVM can keep multiple `Affine3A * Affine3A` SSE multiplies
// in flight (each multiply is independent across actors).

#[inline(always)]
fn apply_delta_block_soa(
    locals: &mut [Affine3A],
    dirty_flags: &mut [bool],
    dirty_actors: &mut ThinVec<ActorHandle>,
    handles: &[ActorHandle],
    delta: Affine3A,
) {
    let mut i = 0;
    let n = handles.len();
    let cap = locals.len();

    while i + 4 <= n {
        let h0 = handles[i];
        let h1 = handles[i + 1];
        let h2 = handles[i + 2];
        let h3 = handles[i + 3];
        let i0 = h0.idx as usize;
        let i1 = h1.idx as usize;
        let i2 = h2.idx as usize;
        let i3 = h3.idx as usize;

        if i0 < cap {
            locals[i0] = delta * locals[i0];
            if !dirty_flags[i0] { dirty_flags[i0] = true; dirty_actors.push(h0); }
        }
        if i1 < cap {
            locals[i1] = delta * locals[i1];
            if !dirty_flags[i1] { dirty_flags[i1] = true; dirty_actors.push(h1); }
        }
        if i2 < cap {
            locals[i2] = delta * locals[i2];
            if !dirty_flags[i2] { dirty_flags[i2] = true; dirty_actors.push(h2); }
        }
        if i3 < cap {
            locals[i3] = delta * locals[i3];
            if !dirty_flags[i3] { dirty_flags[i3] = true; dirty_actors.push(h3); }
        }
        i += 4;
    }

    while i < n {
        let h = handles[i];
        let idx = h.idx as usize;
        if idx < cap {
            locals[idx] = delta * locals[idx];
            if !dirty_flags[idx] { dirty_flags[idx] = true; dirty_actors.push(h); }
        }
        i += 1;
    }
}

#[inline(always)]
fn apply_identity_block_soa(
    _locals: &mut [Affine3A],
    dirty_flags: &mut [bool],
    dirty_actors: &mut ThinVec<ActorHandle>,
    handles: &[ActorHandle],
) {
    let mut i = 0;
    let n = handles.len();
    let cap = dirty_flags.len();

    while i + 4 <= n {
        let h0 = handles[i];
        let h1 = handles[i + 1];
        let h2 = handles[i + 2];
        let h3 = handles[i + 3];
        let i0 = h0.idx as usize;
        let i1 = h1.idx as usize;
        let i2 = h2.idx as usize;
        let i3 = h3.idx as usize;

        if i0 < cap && !dirty_flags[i0] { dirty_flags[i0] = true; dirty_actors.push(h0); }
        if i1 < cap && !dirty_flags[i1] { dirty_flags[i1] = true; dirty_actors.push(h1); }
        if i2 < cap && !dirty_flags[i2] { dirty_flags[i2] = true; dirty_actors.push(h2); }
        if i3 < cap && !dirty_flags[i3] { dirty_flags[i3] = true; dirty_actors.push(h3); }
        i += 4;
    }

    while i < n {
        let h = handles[i];
        let idx = h.idx as usize;
        if idx < cap && !dirty_flags[idx] { dirty_flags[idx] = true; dirty_actors.push(h); }
        i += 1;
    }
}
