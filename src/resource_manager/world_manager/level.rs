use std::sync::Arc;
use slotmap::new_key_type;
use super::stage::StageKey;

new_key_type! { pub struct LevelKey; }

pub struct Level {
    pub name: Arc<str>,
    pub children: Vec<StageKey>,
}
