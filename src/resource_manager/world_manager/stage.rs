use std::sync::Arc;
use slotmap::new_key_type;
use super::level::LevelKey;
use crate::resource_manager::manager::ActorKey;

new_key_type! { pub struct StageKey; }

pub struct Stage {
    pub scenes: Vec<ActorKey>,
    pub sub_entities: Vec<SubEntityKey>,
    pub components: Vec<ComponentType>,
    pub interactions: Vec<Interaction>,
    pub sparse_set: Vec<SparseSet>,
}

pub struct SparseSet<T> {
    pub sparse: Vec<Option<u32>>,
    pub dense:  Vec<T>,
}

let troupe = actors.iter().map(|a| a.id).collect();


pub struct Scene {
    pub id: (StageKey, isize),
    pub parent: StageKey,
    pub children: Vec<ActorKey>,
}