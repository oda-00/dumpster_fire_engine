use std::sync::Arc;
use glam::Affine3A;
use crate::resource_manager::component::{Component, ComponentType};
use crate::resource_manager::manager::{
    ActorHandle, ActorId, ActorType, Arena, Id, StageHandle, StageTag,
};
use super::stage::{Stage, StageId};

pub struct LevelMarker;
pub type LevelId = Id<LevelMarker>;

// ── Level ────────────────────────────────────────────────────────────────────
//
// Level is the source of truth for its Stages. Each Stage owns its Actors.
//
// Component cache: for each ComponentType, cache holds (StageHandle, ActorHandle)
// pairs for every actor in this Level that has ≥1 sub-entity with that component.
// Entries carry the StageHandle so callers can reach the actor without extra context.
// "Give me all Physics actors across all stages in this Level" = O(matching) read.

pub struct Level {
    pub id:     LevelId,
    pub name:   Arc<str>,
    pub stages: Arena<StageTag, Stage>,
    // cache[i] = (stage, actor) pairs where the actor has ≥1 sub-entity with ComponentType i
    pub cache:  [Vec<(StageHandle, ActorHandle)>; ComponentType::COUNT],
}

impl Level {
    pub fn new(id: LevelId, name: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            name:   name.into(),
            stages: Arena::new(),
            cache:  std::array::from_fn(|_| Vec::new()),
        }
    }

    // ── Spawn / despawn ───────────────────────────────────────────────────

    pub fn spawn_stage(&mut self, id: StageId, name: impl Into<Arc<str>>) -> StageHandle {
        self.stages.insert(Stage::new(id, name))
    }

    pub fn despawn_stage(&mut self, stage_h: StageHandle) {
        if self.stages.remove(stage_h).is_none() { return }
        // Drop all level-cache entries that belonged to this stage.
        for cache_slot in self.cache.iter_mut() {
            cache_slot.retain(|(sh, _)| *sh != stage_h);
        }
    }

    pub fn spawn_actor(
        &mut self,
        stage_h: StageHandle,
        id: ActorId,
        local: Affine3A,
    ) -> Option<ActorHandle> {
        Some(self.stages.get_mut(stage_h)?.spawn_actor(id, local))
    }

    pub fn spawn_sub_entity(
        &mut self,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        actor_type: ActorType,
        local: Affine3A,
    ) -> Option<usize> {
        self.stages.get_mut(stage_h)?.spawn_sub_entity(actor_h, actor_type, local)
    }

    pub fn despawn_actor(&mut self, stage_h: StageHandle, actor_h: ActorHandle) {
        let Some(stage) = self.stages.get_mut(stage_h) else { return };
        stage.despawn_actor(actor_h);
        for cache_slot in self.cache.iter_mut() {
            cache_slot.retain(|(sh, ah)| !(*sh == stage_h && *ah == actor_h));
        }
    }

    pub fn despawn_sub_entity(
        &mut self,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        variant_idx: usize,
    ) {
        let Some(stage) = self.stages.get_mut(stage_h) else { return };
        stage.despawn_sub_entity(actor_h, variant_idx);
        // After Stage updated its own cache, sync the level cache: remove entries
        // where the actor no longer appears in Stage's cache for that component.
        for ct_idx in 0..ComponentType::COUNT {
            if !stage.cache[ct_idx].contains(&actor_h) {
                self.cache[ct_idx].retain(|(sh, ah)| !(*sh == stage_h && *ah == actor_h));
            }
        }
    }

    // ── Component access ──────────────────────────────────────────────────

    pub fn add_component(
        &mut self,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        variant_idx: usize,
        comp: Component,
    ) -> bool {
        let ct = comp.component_type();
        let Some(stage) = self.stages.get_mut(stage_h) else { return false };
        if !stage.add_component(actor_h, variant_idx, comp) { return false }
        let entry = (stage_h, actor_h);
        if !self.cache[ct.index()].contains(&entry) {
            self.cache[ct.index()].push(entry);
        }
        true
    }

    pub fn remove_component(
        &mut self,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        variant_idx: usize,
        ct: ComponentType,
    ) -> Option<Component> {
        let stage = self.stages.get_mut(stage_h)?;
        let removed = stage.remove_component(actor_h, variant_idx, ct)?;
        // If Stage evicted actor from its cache, evict from Level's cache too.
        if !stage.cache[ct.index()].contains(&actor_h) {
            self.cache[ct.index()].retain(|(sh, ah)| !(*sh == stage_h && *ah == actor_h));
        }
        Some(removed)
    }

    // ── Transform propagation ─────────────────────────────────────────────

    pub fn set_actor_local(&mut self, stage_h: StageHandle, actor_h: ActorHandle, t: Affine3A) {
        if let Some(stage) = self.stages.get_mut(stage_h) {
            stage.set_actor_local(actor_h, t);
        }
    }

    pub fn set_sub_entity_local(
        &mut self,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        variant_idx: usize,
        t: Affine3A,
    ) {
        if let Some(stage) = self.stages.get_mut(stage_h) {
            stage.set_sub_entity_local(actor_h, variant_idx, t);
        }
    }

    pub fn propagate_transforms(&mut self) {
        for stage in self.stages.values_mut() {
            stage.propagate_transforms();
        }
    }

    // ── Event-manager tick gears ──────────────────────────────────────────

    pub fn collect_effects(
        &self,
        dt: f32,
        world: &crate::resource_manager::world_manager::world::World,
        sink: &mut Vec<crate::resource_manager::event_manager::Effect>,
    ) {
        for stage in self.stages.values() {
            stage.collect_effects(dt, world, sink);
        }
    }

    pub fn post_tick(&mut self, dt: f32) {
        for stage in self.stages.values_mut() {
            stage.post_tick(dt);
        }
    }

    pub fn drain_pending_mealy(
        &mut self,
        sink: &mut Vec<crate::resource_manager::event_manager::Effect>,
    ) {
        for stage in self.stages.values_mut() {
            stage.drain_pending_mealy(sink);
        }
    }
}
