use std::sync::Arc;
use slotmap::new_key_type;
use super::level::LevelKey;
use crate::resource_manager::manager::ActorKey;

new_key_type! { pub struct StageKey; }

pub struct Stage {
    pub name: Arc<str>,
    pub parent: LevelKey,
    pub children: Vec<ActorKey>,
}
