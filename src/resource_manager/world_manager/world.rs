use std::sync::Arc;
use glam::Affine3A;
use thin_vec::ThinVec;
use crate::resource_manager::component::{Component, ComponentData};
use crate::resource_manager::manager::{
    ActorHandle, ActorId, ActorType, Arena, Id, LevelHandle, LevelTag, StageHandle,
};
use super::level::{Level, LevelId};
use super::stage::StageId;

pub struct WorldMarker;
pub type WorldId = Id<WorldMarker>;

// ── World ────────────────────────────────────────────────────────────────────
//
// World is the source of truth for its Levels. It has no direct knowledge of
// Stages, Actors, or SubEntities — those are owned by their respective parents.
//
// All mutations below the Level boundary are delegated into Level, which
// delegates further into Stage. This enforces the strict ownership chain:
//   World → Level → Stage → Actor → SubEntity → Component

pub struct World {
    pub id:     WorldId,
    pub levels: Arena<LevelTag, Level>,
    pub roots:  ThinVec<LevelHandle>,
    /// Reusable per-tick effect buffer — keeps allocation out of the hot path.
    tick_effects: Vec<crate::resource_manager::event_manager::Effect>,
}

impl World {
    pub fn new(id: WorldId) -> Self {
        Self {
            id,
            levels: Arena::new(),
            roots: ThinVec::new(),
            tick_effects: Vec::new(),
        }
    }

    // ── Levels ────────────────────────────────────────────────────────────

    pub fn spawn_level(&mut self, id: LevelId, name: impl Into<Arc<str>>) -> LevelHandle {
        let h = self.levels.insert(Level::new(id, name));
        self.roots.push(h);
        h
    }

    pub fn despawn_level(&mut self, level_h: LevelHandle) {
        // Dropping the Level drops all its Stages, Actors, and SubEntities.
        self.levels.remove(level_h);
        if let Some(pos) = self.roots.iter().position(|&h| h == level_h) {
            self.roots.swap_remove(pos);
        }
    }

    // ── Stages ────────────────────────────────────────────────────────────

    pub fn spawn_stage(
        &mut self,
        level_h: LevelHandle,
        id: StageId,
        name: impl Into<Arc<str>>,
    ) -> Option<StageHandle> {
        Some(self.levels.get_mut(level_h)?.spawn_stage(id, name))
    }

    pub fn despawn_stage(&mut self, level_h: LevelHandle, stage_h: StageHandle) {
        if let Some(level) = self.levels.get_mut(level_h) {
            level.despawn_stage(stage_h);
        }
    }

    // ── Actors ────────────────────────────────────────────────────────────

    pub fn spawn_actor(
        &mut self,
        level_h: LevelHandle,
        stage_h: StageHandle,
        id: ActorId,
        local: Affine3A,
    ) -> Option<ActorHandle> {
        self.levels.get_mut(level_h)?.spawn_actor(stage_h, id, local)
    }

    pub fn despawn_actor(
        &mut self,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
    ) {
        if let Some(level) = self.levels.get_mut(level_h) {
            level.despawn_actor(stage_h, actor_h);
        }
    }

    // ── SubEntities ───────────────────────────────────────────────────────

    /// Returns the variant index (0–3) that identifies the sub-entity slot.
    pub fn spawn_sub_entity(
        &mut self,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        actor_type: ActorType,
        local: Affine3A,
    ) -> Option<usize> {
        self.levels.get_mut(level_h)?.spawn_sub_entity(stage_h, actor_h, actor_type, local)
    }

    pub fn despawn_sub_entity(
        &mut self,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        variant_idx: usize,
    ) {
        if let Some(level) = self.levels.get_mut(level_h) {
            level.despawn_sub_entity(stage_h, actor_h, variant_idx);
        }
    }

    // ── Components ────────────────────────────────────────────────────────

    /// Adds a component, maintaining Level and Stage caches.
    /// Use this instead of direct SubEntity access to keep caches consistent.
    pub fn add_component<T: ComponentData>(
        &mut self,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        variant_idx: usize,
        data: T,
    ) -> bool
    where
        Component: From<T>,
    {
        let Some(level) = self.levels.get_mut(level_h) else { return false };
        level.add_component(stage_h, actor_h, variant_idx, Component::from(data))
    }

    /// Removes a component, maintaining Level and Stage caches.
    pub fn remove_component<T: ComponentData + TryFrom<Component>>(
        &mut self,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        variant_idx: usize,
    ) -> Option<T> {
        let level = self.levels.get_mut(level_h)?;
        let comp = level.remove_component(stage_h, actor_h, variant_idx, T::TYPE)?;
        comp.try_into().ok()
    }

    // ── Transforms ───────────────────────────────────────────────────────

    #[inline]
    pub fn set_actor_local(
        &mut self,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        t: Affine3A,
    ) {
        if let Some(level) = self.levels.get_mut(level_h) {
            level.set_actor_local(stage_h, actor_h, t);
        }
    }

    #[inline]
    pub fn set_sub_entity_local(
        &mut self,
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        variant_idx: usize,
        t: Affine3A,
    ) {
        if let Some(level) = self.levels.get_mut(level_h) {
            level.set_sub_entity_local(stage_h, actor_h, variant_idx, t);
        }
    }

    /// Flushes all dirty transforms in all levels. O(dirty actors across all stages).
    pub fn propagate_transforms(&mut self) {
        for level in self.levels.values_mut() {
            level.propagate_transforms();
        }
    }

    // ── Event-manager tick cascade ─────────────────────────────────────────
    //
    // Four passes per tick:
    //   1. read-only: collect Effects from every Play in every Stage
    //   2. mut: apply those Effects (and any Mealy effects from the previous
    //          tick's transition) via existing World mutators
    //   3. mut: per-Play bookkeeping (queues, transitions, counters)
    //   4. propagate transforms (existing behavior)

    pub fn tick(&mut self, dt: f32) {
        // Take the pooled buffer out so we can hold &mut self while it's full.
        // Reuses prior capacity — fresh tick reads no allocator.
        let mut effects = std::mem::take(&mut self.tick_effects);
        effects.clear();

        // Pass 2-prelude: drain Mealy effects from the previous tick's transitions.
        for level in self.levels.values_mut() {
            level.drain_pending_mealy(&mut effects);
        }

        // Pass 1 — read-only collect. Reborrow self as &World so condition
        // evaluators can hold a &World while we iterate levels via shared access.
        {
            let world_view: &World = &*self;
            for level in world_view.levels.values() {
                level.collect_effects(dt, world_view, &mut effects);
            }
        }

        // Pass 2 — apply.
        for eff in effects.drain(..) {
            self.apply_effect(eff);
        }

        // Pass 3 — bookkeeping.
        for level in self.levels.values_mut() {
            level.post_tick(dt);
        }

        // Pass 4 — transforms.
        self.propagate_transforms();

        // Restore the (now-empty, but capacity-preserving) buffer.
        self.tick_effects = effects;
    }

    pub fn apply_effect(&mut self, eff: crate::resource_manager::event_manager::Effect) {
        use crate::resource_manager::event_manager::{Effect, EventTarget, Event};
        match eff {
            Effect::SetActorLocal { level_h, stage_h, actor_h, local } => {
                self.set_actor_local(level_h, stage_h, actor_h, local);
            }
            Effect::SetSubEntityLocal { level_h, stage_h, actor_h, variant_idx, local } => {
                self.set_sub_entity_local(level_h, stage_h, actor_h, variant_idx, local);
            }
            Effect::AddComponent(b) => {
                let crate::resource_manager::event_manager::AddComponentEffect {
                    level_h, stage_h, actor_h, variant_idx, component,
                } = *b;
                if let Some(level) = self.levels.get_mut(level_h) {
                    level.add_component(stage_h, actor_h, variant_idx, component);
                }
            }
            Effect::RemoveComponent { level_h, stage_h, actor_h, variant_idx, component_type } => {
                if let Some(level) = self.levels.get_mut(level_h) {
                    level.remove_component(stage_h, actor_h, variant_idx, component_type);
                }
            }
            Effect::SpawnActor { level_h, stage_h, id, local } => {
                self.spawn_actor(level_h, stage_h, id, local);
            }
            Effect::SpawnSubEntity(b) => {
                let crate::resource_manager::event_manager::SpawnSubEntityEffect {
                    level_h, stage_h, actor_h, actor_type, local,
                } = *b;
                self.spawn_sub_entity(level_h, stage_h, actor_h, actor_type, local);
            }
            Effect::DespawnActor { level_h, stage_h, actor_h } => {
                self.despawn_actor(level_h, stage_h, actor_h);
            }
            Effect::DespawnSubEntity { level_h, stage_h, actor_h, variant_idx } => {
                self.despawn_sub_entity(level_h, stage_h, actor_h, variant_idx);
            }
            Effect::CueTroupe { level_h, stage_h, troupe, delta } => {
                // Direct-write fast path — bypasses per-actor World→Level→Stage
                // routing. Stage holds a reusable scratch buffer to keep the
                // fan-out allocation-free.
                if let Some(level) = self.levels.get_mut(level_h) {
                    if let Some(stage) = level.stages.get_mut(stage_h) {
                        stage.cue_troupe_direct(troupe, delta);
                    }
                }
            }
            Effect::Emit { level_h, stage_h, target, event } => {
                let Some(level) = self.levels.get_mut(level_h) else { return };
                let Some(stage) = level.stages.get_mut(stage_h) else { return };
                let Some(play) = stage.play.as_mut() else { return };
                match target {
                    EventTarget::Play => play.queue.push(event),
                    EventTarget::CurrentScene => {
                        if let Some(&leaf) = play.active_leaves.first() {
                            play.scenes[leaf].queue.push(event);
                        }
                    }
                    EventTarget::Scene(sid) => {
                        if let Some(h) = play.handle_for(sid) {
                            play.scenes[h].queue.push(event);
                        }
                    }
                }
                let _ = Event::Tick { dt: 0.0 }; // keep Event in scope so future variants compile
            }
            Effect::ScheduleTransition { level_h, stage_h, source, target, mealy } => {
                if let Some(level) = self.levels.get_mut(level_h) {
                    if let Some(stage) = level.stages.get_mut(stage_h) {
                        if let Some(play) = stage.play.as_mut() {
                            // Last-write-wins if multiple transitions schedule
                            // in the same tick; pass 3 will pick this one.
                            play.pending_transition =
                                Some(crate::resource_manager::event_manager::TransitionRecord {
                                    source, target, mealy,
                                });
                        }
                    }
                }
            }
        }
    }
}

impl Default for World {
    fn default() -> Self { Self::new(WorldId::new(0)) }
}
