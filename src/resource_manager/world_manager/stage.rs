use std::sync::Arc;
use glam::Affine3A;
use thin_vec::ThinVec;
use crate::resource_manager::component::{Component, ComponentType};
use crate::resource_manager::manager::{
    Actor, ActorHandle, ActorId, ActorTag, ActorType, Arena, Id, SubEntity,
};

pub struct StageMarker;
pub type StageId = Id<StageMarker>;

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
}

impl Stage {
    pub fn new(id: StageId, name: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            name:         name.into(),
            actors:       Arena::new(),
            cache:        std::array::from_fn(|_| Vec::new()),
            dirty_actors: ThinVec::new(),
            play:         None,
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
            local,
            world: Affine3A::IDENTITY,
            dirty: true,
            sub_entities: std::array::from_fn(|_| None),
        });
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
        if !actor.dirty {
            actor.dirty = true;
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

    pub fn set_actor_local(&mut self, actor_h: ActorHandle, t: Affine3A) {
        let Some(actor) = self.actors.get_mut(actor_h) else { return };
        actor.local = t;
        if !actor.dirty {
            actor.dirty = true;
            self.dirty_actors.push(actor_h);
        }
    }

    pub fn set_sub_entity_local(&mut self, actor_h: ActorHandle, variant_idx: usize, t: Affine3A) {
        let Some(actor) = self.actors.get_mut(actor_h) else { return };
        if let Some(sub) = actor.sub_entities[variant_idx].as_mut() {
            sub.local = t;
            sub.dirty = true;
        }
        if !actor.dirty {
            actor.dirty = true;
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

    /// Recomputes world transforms for dirty actors and their sub-entities.
    /// O(dirty count), not O(all actors).
    pub fn propagate_transforms(&mut self) {
        for &actor_h in &self.dirty_actors {
            let Some(actor) = self.actors.get_mut(actor_h) else { continue };
            // Actors sit directly in the Stage — no parent transform to compose with.
            actor.world = actor.local;
            actor.dirty = false;
            let actor_world = actor.world;
            for sub in actor.sub_entities.iter_mut().flatten() {
                sub.world = actor_world * sub.local;
                sub.dirty = false;
            }
        }
        self.dirty_actors.clear();
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn cache_remove_actor(cache_slot: &mut Vec<ActorHandle>, actor_h: ActorHandle) {
        if let Some(pos) = cache_slot.iter().position(|&h| h == actor_h) {
            cache_slot.swap_remove(pos);
        }
    }
}
