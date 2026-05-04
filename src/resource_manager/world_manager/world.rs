use std::sync::Arc;
use glam::Affine3A;
use thin_vec::ThinVec;
use crate::resource_manager::component::{Component, ComponentData};
use crate::resource_manager::manager::{
    ActorHandle, ActorId, ActorType, Arena, LevelHandle, LevelTag, StageHandle,
};
use super::level::Level;

// ── World ────────────────────────────────────────────────────────────────────
//
// World is the source of truth for its Levels. It has no direct knowledge of
// Stages, Actors, or SubEntities — those are owned by their respective parents.
//
// All mutations below the Level boundary are delegated into Level, which
// delegates further into Stage. This enforces the strict ownership chain:
//   World → Level → Stage → Actor → SubEntity → Component

pub struct World {
    pub levels: Arena<LevelTag, Level>,
    pub roots:  ThinVec<LevelHandle>,
}

impl World {
    pub fn new() -> Self {
        Self { levels: Arena::new(), roots: ThinVec::new() }
    }

    // ── Levels ────────────────────────────────────────────────────────────

    pub fn spawn_level(&mut self, name: impl Into<Arc<str>>) -> LevelHandle {
        let h = self.levels.insert(Level::new(name));
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
        name: impl Into<Arc<str>>,
    ) -> Option<StageHandle> {
        Some(self.levels.get_mut(level_h)?.spawn_stage(name))
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
}

impl Default for World {
    fn default() -> Self { Self::new() }
}
