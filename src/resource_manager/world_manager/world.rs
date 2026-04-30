use std::sync::Arc;
use std::collections::HashMap;
use slotmap::SlotMap;
use crate::resource_manager::manager::*;
use crate::resource_manager::comps::ComponentType;
use super::level::{Level, LevelKey};
use super::stage::{Stage, StageKey};

pub struct World {
    pub levels:       SlotMap<LevelKey, Level>,
    pub stages:       SlotMap<StageKey, Stage>,
    pub actors:       SlotMap<ActorKey, Actor>,
    pub sub_entities: SlotMap<SubEntityKey, SubEntity>,
    pub roots:        Vec<LevelKey>,
}

impl World {
    pub fn new() -> Self {
        Self {
            levels:       SlotMap::with_key(),
            stages:       SlotMap::with_key(),
            actors:       SlotMap::with_key(),
            sub_entities: SlotMap::with_key(),
            roots:        Vec::new(),
        }
    }

    pub fn spawn_level(&mut self, name: impl Into<Arc<str>>) -> LevelKey {
        let key = self.levels.insert(Level {
            name: name.into(),
            children: Vec::new(),
        });
        self.roots.push(key);
        key
    }

    pub fn spawn_stage(&mut self, level: LevelKey, name: impl Into<Arc<str>>) -> Option<StageKey> {
        if !self.levels.contains_key(level) { return None; }
        let key = self.stages.insert(Stage {
            name: name.into(),
            parent: level,
            children: Vec::new(),
        });
        self.levels[level].children.push(key);
        Some(key)
    }

    pub fn spawn_actor(
        &mut self,
        stage: StageKey,
        id: ActorId,
        local: Transform,
    ) -> Option<ActorKey> {
        if !self.stages.contains_key(stage) { return None; }
        let key = self.actors.insert(Actor {
            id,
            local,
            world: local,
            dirty: false,
            parent: stage,
            children: HashMap::new(),
        });
        self.stages[stage].children.push(key);
        Some(key)
    }

    pub fn spawn_sub_entity(
        &mut self,
        actor: ActorKey,
        actor_type: ActorType,
        local: Transform,
    ) -> Option<SubEntityKey> {
        if !self.actors.contains_key(actor) { return None; }
        let subtype_id = actor_type.subtype_id();
        let key = self.sub_entities.insert(SubEntity {
            actor_type,
            local,
            world: Transform::IDENTITY,
            dirty: true,
            components: [const { None }; ComponentType::COUNT],
            parent: actor,
        });
        self.actors[actor].children.insert(subtype_id, key);
        Some(key)
    }

    pub fn set_actor_local(&mut self, actor: ActorKey, t: Transform) {
        let child_keys: Vec<SubEntityKey> = match self.actors.get_mut(actor) {
            Some(a) => {
                a.local = t;
                a.dirty = true;
                a.children.values().copied().collect()
            }
            None => return,
        };
        for k in child_keys {
            if let Some(c) = self.sub_entities.get_mut(k) {
                c.dirty = true;
            }
        }
    }

    pub fn set_sub_entity_local(&mut self, key: SubEntityKey, t: Transform) {
        if let Some(s) = self.sub_entities.get_mut(key) {
            s.local = t;
            s.dirty = true;
        }
    }

    pub fn propagate_transforms(&mut self) {
        for actor in self.actors.values_mut() {
            if actor.dirty {
                actor.world = actor.local;
                actor.dirty = false;
            }
        }
        for sub in self.sub_entities.values_mut() {
            if sub.dirty {
                if let Some(actor) = self.actors.get(sub.parent) {
                    sub.world = Transform::compose(&actor.world, &sub.local);
                }
                sub.dirty = false;
            }
        }
    }

    pub fn despawn_sub_entity(&mut self, key: SubEntityKey) {
        if let Some(sub) = self.sub_entities.remove(key) {
            if let Some(actor) = self.actors.get_mut(sub.parent) {
                actor.children.retain(|_, v| *v != key);
            }
        }
    }

    pub fn despawn_actor(&mut self, key: ActorKey) {
        if let Some(actor) = self.actors.remove(key) {
            for &sub_key in actor.children.values() {
                self.sub_entities.remove(sub_key);
            }
            if let Some(stage) = self.stages.get_mut(actor.parent) {
                stage.children.retain(|&k| k != key);
            }
        }
    }

    pub fn despawn_stage(&mut self, key: StageKey) {
        if let Some(stage) = self.stages.remove(key) {
            for &actor_key in &stage.children {
                if let Some(actor) = self.actors.remove(actor_key) {
                    for &sub_key in actor.children.values() {
                        self.sub_entities.remove(sub_key);
                    }
                }
            }
            if let Some(level) = self.levels.get_mut(stage.parent) {
                level.children.retain(|&k| k != key);
            }
        }
    }

    pub fn despawn_level(&mut self, key: LevelKey) {
        if let Some(level) = self.levels.remove(key) {
            for &stage_key in &level.children {
                if let Some(stage) = self.stages.remove(stage_key) {
                    for &actor_key in &stage.children {
                        if let Some(actor) = self.actors.remove(actor_key) {
                            for &sub_key in actor.children.values() {
                                self.sub_entities.remove(sub_key);
                            }
                        }
                    }
                }
            }
            self.roots.retain(|&k| k != key);
        }
    }
}

impl Default for World {
    fn default() -> Self { Self::new() }
}
