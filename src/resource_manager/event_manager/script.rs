use std::sync::Arc;
use thin_vec::ThinVec;
use crate::resource_manager::manager::{Handle, Id};
use super::scene::{Handler, SceneDef, SceneId};

// ── Tags / markers / Ids owned by script.rs ─────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq)] pub struct ScriptTag;
pub type ScriptHandle = Handle<ScriptTag>;

pub struct ScriptMarker;
pub type ScriptId = Id<ScriptMarker>;

// ── Script ──────────────────────────────────────────────────────────────────
//
// A Script is the static, authored hierarchy of SceneDefs (HSM nodes) plus a
// set of play-global handlers. Reusable; consumed by Play::instantiate to
// produce a runtime Play with materialized Scenes.

pub struct Script {
    pub id:       ScriptId,
    pub name:     Arc<str>,
    pub scenes:   ThinVec<SceneDef>,
    pub entry:    SceneId,
    pub handlers: ThinVec<Handler>,
}

impl Script {
    pub fn new(id: ScriptId, name: impl Into<Arc<str>>, entry: SceneId) -> Self {
        Script {
            id,
            name: name.into(),
            scenes: ThinVec::new(),
            entry,
            handlers: ThinVec::new(),
        }
    }

    pub fn add_scene(&mut self, def: SceneDef) {
        self.scenes.push(def);
    }

    pub fn add_handler(&mut self, h: Handler) {
        self.handlers.push(h);
    }

    pub fn find_scene(&self, id: SceneId) -> Option<&SceneDef> {
        self.scenes.iter().find(|s| s.id == id)
    }
}
