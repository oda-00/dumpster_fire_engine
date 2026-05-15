use std::sync::Arc;
use thin_vec::ThinVec;
use crate::resource_manager::manager::{Arena, Handle, Id};
use super::scene::{Handler, SceneDef, SceneId};
use super::script_abi::{EngineAPI, SceneDefArray, SceneEntry};

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

// ── Compiled-script entry points ──────────────────────────────────────────────

/// The five symbols every compiled `.lang` `.so` must export, plus the table
/// of per-scene function pointers it produces at load time.
pub struct ScriptEntryPoints {
    pub state_size:        unsafe extern "C" fn() -> u32,
    pub state_version:     unsafe extern "C" fn() -> u32,
    pub init_state:        unsafe extern "C" fn(*mut u8),
    pub migrate_state:     unsafe extern "C" fn(u32, *const u8, *mut u8),
    pub create_scene_defs: unsafe extern "C" fn(*const EngineAPI, *mut SceneDefArray),
    /// Materialised by calling `create_scene_defs` at load time.  Lifetime
    /// tied to the loaded library — owned by `LoadedScript`.
    pub scenes: ThinVec<SceneEntry>,
}

impl ScriptEntryPoints {
    pub fn state_size(&self)    -> u32 { unsafe { (self.state_size)() } }
    pub fn state_version(&self) -> u32 { unsafe { (self.state_version)() } }

    pub fn scene(&self, raw_id: i64) -> Option<&SceneEntry> {
        self.scenes.iter().find(|s| s.raw_id == raw_id)
    }
}

// ── LoadedScript / ScriptManager ──────────────────────────────────────────────

/// One compiled `.lang` shared library that has been loaded into the process.
///
/// `lib` MUST outlive every function pointer in `entry`.  The arena enforces
/// this by holding both together and dropping the library only via
/// `ScriptManager::unload`.
pub struct LoadedScript {
    pub id:          ScriptId,
    pub source_path: Arc<str>,
    pub lib:         libloading::Library,
    pub entry:       ScriptEntryPoints,
}

#[derive(Debug)]
pub enum ScriptLoadError {
    Io(String),
    MissingSymbol(&'static str),
}

impl core::fmt::Display for ScriptLoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScriptLoadError::Io(s)            => write!(f, "io: {s}"),
            ScriptLoadError::MissingSymbol(s) => write!(f, "missing symbol `{s}`"),
        }
    }
}

pub struct ScriptManager {
    arena: Arena<ScriptTag, LoadedScript>,
    /// Sorted ascending by `ScriptId.raw()`; binary-search lookups.
    id_to_handle: ThinVec<(ScriptId, ScriptHandle)>,
    next_raw_id:  i64,
}

impl ScriptManager {
    pub fn new() -> Self {
        ScriptManager {
            arena:        Arena::new(),
            id_to_handle: ThinVec::new(),
            next_raw_id:  1,
        }
    }

    pub fn len(&self) -> usize { self.id_to_handle.len() }
    pub fn is_empty(&self) -> bool { self.id_to_handle.is_empty() }

    /// Load a compiled `.lang` shared library from `path`.  Assigns a fresh
    /// `ScriptId` and registers the library in the arena.
    pub fn load_from_file(&mut self, path: Arc<str>) -> Result<ScriptId, ScriptLoadError> {
        // SAFETY: opening a shared library is fundamentally unsafe; we trust
        // the caller that `path` references a `.so` produced by `langc`.
        let lib = unsafe { libloading::Library::new(path.as_ref()) }
            .map_err(|e| ScriptLoadError::Io(e.to_string()))?;

        let entry = unsafe { read_entry_points(&lib) }?;

        let id = ScriptId::new(self.next_raw_id);
        self.next_raw_id += 1;
        let loaded = LoadedScript { id, source_path: path, lib, entry };
        let h = self.arena.insert(loaded);
        let pos = self.id_to_handle.partition_point(|(sid, _)| sid.raw() < id.raw());
        self.id_to_handle.insert(pos, (id, h));
        Ok(id)
    }

    /// Hot-reload: replace the library for an existing `ScriptId` with a
    /// fresh compilation from `new_path`.  All function pointers in `entry`
    /// are rewritten in place; the previous library is dropped after the swap.
    pub fn hot_reload(
        &mut self,
        id: ScriptId,
        new_path: Arc<str>,
    ) -> Result<(), ScriptLoadError> {
        let h = self.handle_for(id).ok_or_else(|| ScriptLoadError::Io(
            format!("no script with id {}", id.raw())
        ))?;
        let lib = unsafe { libloading::Library::new(new_path.as_ref()) }
            .map_err(|e| ScriptLoadError::Io(e.to_string()))?;
        let entry = unsafe { read_entry_points(&lib) }?;
        if let Some(loaded) = self.arena.get_mut(h) {
            // Drop the old library AFTER replacing the pointers, so no live
            // function pointer references a freed code region.
            loaded.lib         = lib;
            loaded.entry       = entry;
            loaded.source_path = new_path;
        }
        Ok(())
    }

    pub fn unload(&mut self, id: ScriptId) {
        let Some(h) = self.handle_for(id) else { return };
        self.arena.remove(h);
        let pos = self.id_to_handle.partition_point(|(sid, _)| sid.raw() < id.raw());
        if self.id_to_handle.get(pos).is_some_and(|(sid, _)| sid.raw() == id.raw()) {
            self.id_to_handle.remove(pos);
        }
    }

    pub fn get(&self, id: ScriptId) -> Option<&LoadedScript> {
        let h = self.handle_for(id)?;
        self.arena.get(h)
    }

    pub fn get_entry_points(&self, id: ScriptId) -> Option<&ScriptEntryPoints> {
        self.get(id).map(|l| &l.entry)
    }

    fn handle_for(&self, id: ScriptId) -> Option<ScriptHandle> {
        let pos = self.id_to_handle.partition_point(|(sid, _)| sid.raw() < id.raw());
        self.id_to_handle.get(pos)
            .filter(|(sid, _)| sid.raw() == id.raw())
            .map(|(_, h)| *h)
    }
}

impl Default for ScriptManager {
    fn default() -> Self { Self::new() }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

unsafe fn read_entry_points(lib: &libloading::Library) -> Result<ScriptEntryPoints, ScriptLoadError> {
    unsafe {
        let state_size: libloading::Symbol<unsafe extern "C" fn() -> u32> =
            lib.get(b"df_state_size\0").map_err(|_| ScriptLoadError::MissingSymbol("df_state_size"))?;
        let state_version: libloading::Symbol<unsafe extern "C" fn() -> u32> =
            lib.get(b"df_state_version\0").map_err(|_| ScriptLoadError::MissingSymbol("df_state_version"))?;
        let init_state: libloading::Symbol<unsafe extern "C" fn(*mut u8)> =
            lib.get(b"df_init_state\0").map_err(|_| ScriptLoadError::MissingSymbol("df_init_state"))?;
        let migrate_state: libloading::Symbol<unsafe extern "C" fn(u32, *const u8, *mut u8)> =
            lib.get(b"df_migrate_state\0").map_err(|_| ScriptLoadError::MissingSymbol("df_migrate_state"))?;
        let create_scene_defs: libloading::Symbol<unsafe extern "C" fn(*const EngineAPI, *mut SceneDefArray)> =
            lib.get(b"df_create_scene_defs\0").map_err(|_| ScriptLoadError::MissingSymbol("df_create_scene_defs"))?;

        // Materialise the scene table by calling create_scene_defs with a
        // null `api` — the codegen only uses the api in script-side functions,
        // not in the scene-defs constructor itself.
        let mut arr = SceneDefArray {
            scene_count: 0,
            _pad: 0,
            scenes: core::ptr::null(),
        };
        (create_scene_defs)(core::ptr::null(), &mut arr);

        let mut scenes = ThinVec::with_capacity(arr.scene_count as usize);
        if !arr.scenes.is_null() && arr.scene_count > 0 {
            let raw = core::slice::from_raw_parts(arr.scenes, arr.scene_count as usize);
            for e in raw {
                scenes.push(SceneEntry {
                    raw_id:   e.raw_id,
                    on_enter: e.on_enter,
                    on_exit:  e.on_exit,
                    tick:     e.tick,
                });
            }
        }

        Ok(ScriptEntryPoints {
            state_size:        *state_size.into_raw(),
            state_version:     *state_version.into_raw(),
            init_state:        *init_state.into_raw(),
            migrate_state:     *migrate_state.into_raw(),
            create_scene_defs: *create_scene_defs.into_raw(),
            scenes,
        })
    }
}
