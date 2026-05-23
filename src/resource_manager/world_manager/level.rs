use std::sync::Arc;
use glam::Affine3A;
use thin_vec::ThinVec;
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
    pub cache:  [ThinVec<(StageHandle, ActorHandle)>; ComponentType::COUNT],
}

impl Level {
    pub fn new(id: LevelId, name: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            name:   name.into(),
            stages: Arena::new(),
            cache:  std::array::from_fn(|_| ThinVec::new()),
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
        let actor_idx = actor_h.idx as usize;
        let Some(stage) = self.stages.get_mut(stage_h) else { return };
        stage.despawn_actor(actor_h);
        // Copy level_cache_pos before the borrow ends (NLL).
        let lpos = stage.level_cache_pos[actor_idx];
        // Borrow of `stage` ends here.
        for ct_idx in 0..ComponentType::COUNT {
            let pos = lpos[ct_idx];
            if pos == u16::MAX { continue; }
            self.cache[ct_idx].swap_remove(pos as usize);
            if (pos as usize) < self.cache[ct_idx].len() {
                let (d_sh, d_ah) = self.cache[ct_idx][pos as usize];
                self.stages.get_mut(d_sh).unwrap()
                    .level_cache_pos[d_ah.idx as usize][ct_idx] = pos;
            }
        }
        // Reset (slot may be reused by the next spawn_actor).
        self.stages.get_mut(stage_h).unwrap()
            .level_cache_pos[actor_idx] = [u16::MAX; ComponentType::COUNT];
    }

    pub fn despawn_sub_entity(
        &mut self,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        variant_idx: usize,
    ) {
        let actor_idx = actor_h.idx as usize;
        let Some(stage) = self.stages.get_mut(stage_h) else { return };
        stage.despawn_sub_entity(actor_h, variant_idx);
        // stage.cache_pos[idx][ct] is u16::MAX if actor was evicted from stage cache.
        // stage.level_cache_pos[idx][ct] tracks position in Level.cache.
        let cpos = stage.cache_pos[actor_idx];
        let lpos = stage.level_cache_pos[actor_idx];
        // Borrow of `stage` ends here.
        for ct_idx in 0..ComponentType::COUNT {
            if cpos[ct_idx] != u16::MAX { continue; } // still in stage cache
            let pos = lpos[ct_idx];
            if pos == u16::MAX { continue; }           // not in level cache
            self.cache[ct_idx].swap_remove(pos as usize);
            if (pos as usize) < self.cache[ct_idx].len() {
                let (d_sh, d_ah) = self.cache[ct_idx][pos as usize];
                self.stages.get_mut(d_sh).unwrap()
                    .level_cache_pos[d_ah.idx as usize][ct_idx] = pos;
            }
            self.stages.get_mut(stage_h).unwrap()
                .level_cache_pos[actor_idx][ct_idx] = u16::MAX;
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
        let ct_idx = comp.component_type().index();
        let actor_idx = actor_h.idx as usize;
        let Some(stage) = self.stages.get_mut(stage_h) else { return false };
        if !stage.add_component(actor_h, variant_idx, comp) { return false }
        // O(1) dedup using level_cache_pos: if u16::MAX, actor not yet in level cache.
        let needs_level = stage.level_cache_pos[actor_idx][ct_idx] == u16::MAX;
        // Borrow of `stage` ends here.
        if needs_level {
            let pos = self.cache[ct_idx].len() as u16;
            self.cache[ct_idx].push((stage_h, actor_h));
            self.stages.get_mut(stage_h).unwrap()
                .level_cache_pos[actor_idx][ct_idx] = pos;
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
        let ct_idx = ct.index();
        let actor_idx = actor_h.idx as usize;
        let stage = self.stages.get_mut(stage_h)?;
        let removed = stage.remove_component(actor_h, variant_idx, ct)?;
        // After remove_component, stage.cache_pos[actor_idx][ct_idx] is u16::MAX
        // if the actor no longer has this component anywhere in the stage.
        let still_in_stage = stage.cache_pos[actor_idx][ct_idx] != u16::MAX;
        let level_pos = stage.level_cache_pos[actor_idx][ct_idx];
        // Borrow of `stage` ends here.
        if !still_in_stage && level_pos != u16::MAX {
            self.cache[ct_idx].swap_remove(level_pos as usize);
            if (level_pos as usize) < self.cache[ct_idx].len() {
                let (d_sh, d_ah) = self.cache[ct_idx][level_pos as usize];
                self.stages.get_mut(d_sh).unwrap()
                    .level_cache_pos[d_ah.idx as usize][ct_idx] = level_pos;
            }
            self.stages.get_mut(stage_h).unwrap()
                .level_cache_pos[actor_idx][ct_idx] = u16::MAX;
        }
        Some(removed)
    }

    // ── Transform propagation ─────────────────────────────────────────────

    #[inline]
    pub fn set_actor_local(&mut self, stage_h: StageHandle, actor_h: ActorHandle, t: Affine3A) {
        if let Some(stage) = self.stages.get_mut(stage_h) {
            stage.set_actor_local(actor_h, t);
        }
    }

    #[inline]
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
        sink: &mut ThinVec<crate::resource_manager::event_manager::Effect>,
        chain: &mut ThinVec<crate::resource_manager::event_manager::SceneHandle>,
    ) {
        for stage in self.stages.values() {
            stage.collect_effects(dt, world, sink, chain);
        }
    }

    pub fn post_tick(&mut self, dt: f32) {
        for stage in self.stages.values_mut() {
            stage.post_tick(dt);
        }
    }

    pub fn drain_pending_mealy(
        &mut self,
        sink: &mut ThinVec<crate::resource_manager::event_manager::Effect>,
    ) {
        for stage in self.stages.values_mut() {
            stage.drain_pending_mealy(sink);
        }
    }
}
