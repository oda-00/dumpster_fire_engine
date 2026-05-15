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

// ── ActiveScript: one running instance of a compiled .lang script ────────────
//
// Plan §6.3 — held inside `Play::active_scripts`.  State is preserved across
// ticks (and across hot-reloads when layout matches via `state_version`).

pub struct ActiveScript {
    pub script_id:     ScriptId,
    pub state_buffer:  ThinVec<u8>,
    pub state_version: u32,
    /// Raw_id of the currently-active scene.  Zero means "uninitialised";
    /// `from_entry` defaults this to the first scene in the table.
    pub active_scene:  i64,
    pub tick_count:    u64,
    pub elapsed:       f32,
}

impl ActiveScript {
    /// Initialise from a freshly-loaded entry table.  Allocates the state
    /// buffer, calls `init_state` for defaults, and sets `active_scene` to
    /// the first scene in the table.
    pub fn from_entry(script_id: ScriptId, entry: &ScriptEntryPoints) -> Self {
        let state_size    = entry.state_size() as usize;
        let state_version = entry.state_version();
        let mut buf: ThinVec<u8> = ThinVec::with_capacity(state_size);
        buf.resize(state_size, 0);
        unsafe { (entry.init_state)(buf.as_mut_ptr()); }
        let active_scene = entry.scenes.first().map(|s| s.raw_id).unwrap_or(0);
        ActiveScript {
            script_id, state_buffer: buf, state_version,
            active_scene, tick_count: 0, elapsed: 0.0,
        }
    }

    /// Migrate the state buffer into the layout described by `new_entry`.
    /// When the version matches, the buffer is reused (zero-copy).  Otherwise
    /// a fresh buffer is allocated and `migrate_state(old_version, old, new)`
    /// from the new library is invoked.  `active_scene` falls back to the
    /// first scene when its raw_id is no longer present in the new table.
    pub fn migrate_into(&mut self, new_entry: &ScriptEntryPoints) {
        let new_size    = new_entry.state_size() as usize;
        let new_version = new_entry.state_version();
        if new_version == self.state_version && new_size == self.state_buffer.len() {
            return;
        }
        let mut new_buf: ThinVec<u8> = ThinVec::with_capacity(new_size);
        new_buf.resize(new_size, 0);
        unsafe {
            (new_entry.migrate_state)(
                self.state_version,
                self.state_buffer.as_ptr(),
                new_buf.as_mut_ptr(),
            );
        }
        self.state_buffer  = new_buf;
        self.state_version = new_version;

        if !new_entry.scenes.iter().any(|s| s.raw_id == self.active_scene) {
            self.active_scene = new_entry.scenes.first().map(|s| s.raw_id).unwrap_or(0);
        }
    }

    /// Tick the active scene against `entry`'s scene table.  Returns the
    /// transition target (zero ⇒ no transition).  When a transition fires the
    /// previous scene's `on_exit` and the new scene's `on_enter` are invoked
    /// in order so state-buffer side effects from those handlers land.
    ///
    /// `api.elapsed` and `api.tick_count` are overwritten with this
    /// `ActiveScript`'s tracked values before each call.
    pub fn tick(
        &mut self,
        entry: &ScriptEntryPoints,
        api: &mut super::script_abi::EngineAPI,
        dt: f32,
    ) -> i64 {
        self.elapsed     += dt;
        self.tick_count  += 1;
        api.elapsed       = self.elapsed;
        api.tick_count    = self.tick_count;

        let Some(scene) = entry.scene(self.active_scene) else { return 0; };
        let next = unsafe { (scene.tick)(api, self.state_buffer.as_mut_ptr()) };
        if next != 0 && next != self.active_scene {
            unsafe { (scene.on_exit)(api, self.state_buffer.as_mut_ptr()); }
            if let Some(target) = entry.scene(next) {
                unsafe { (target.on_enter)(api, self.state_buffer.as_mut_ptr()); }
                self.active_scene = next;
                self.elapsed = 0.0;  // reset per-scene elapsed
            }
        }
        next
    }

    /// Drive the active scene's `on_enter` once — call this immediately after
    /// `from_entry` to seed initial effects (the entry-scene's on_enter never
    /// fires automatically).
    pub fn run_initial_on_enter(
        &mut self,
        entry: &ScriptEntryPoints,
        api: &mut super::script_abi::EngineAPI,
    ) {
        let Some(scene) = entry.scene(self.active_scene) else { return; };
        unsafe { (scene.on_enter)(api, self.state_buffer.as_mut_ptr()); }
    }
}

#[cfg(test)]
mod active_script_tests {
    use super::*;
    use super::super::script_abi::EngineAPI;

    extern "C" fn ss_v1()           -> u32 { 4 }
    extern "C" fn ss_v2()           -> u32 { 8 }
    extern "C" fn ver_v1()          -> u32 { 0x1111_1111 }
    extern "C" fn ver_v2()          -> u32 { 0x2222_2222 }
    extern "C" fn init_v1(p: *mut u8) {
        unsafe { *(p as *mut i32) = 7; }
    }
    extern "C" fn init_v2(p: *mut u8) {
        unsafe {
            *(p as *mut i32) = 0;
            *(p.add(4) as *mut i32) = 0;
        }
    }
    extern "C" fn migrate_v2(old_ver: u32, old: *const u8, new: *mut u8) {
        // From v1 (4 B i32) into v2 (4 B i32 + 4 B i32): copy first word, set second to old_ver.
        unsafe {
            let x = *(old as *const i32);
            *(new as *mut i32) = x;
            *(new.add(4) as *mut i32) = old_ver as i32;
        }
    }
    extern "C" fn create_defs_noop(_api: *const EngineAPI, out: *mut super::super::script_abi::SceneDefArray) {
        unsafe {
            (*out).scene_count = 0;
            (*out).scenes      = core::ptr::null();
        }
    }

    extern "C" fn migrate_v1_unused(_old_ver: u32, _old: *const u8, _new: *mut u8) {
        panic!("v1 migrate unused in test");
    }

    fn v1_entry() -> ScriptEntryPoints {
        ScriptEntryPoints {
            state_size:        ss_v1,
            state_version:     ver_v1,
            init_state:        init_v1,
            migrate_state:     migrate_v1_unused,
            create_scene_defs: create_defs_noop,
            scenes:            ThinVec::new(),
        }
    }

    fn v2_entry() -> ScriptEntryPoints {
        ScriptEntryPoints {
            state_size:        ss_v2,
            state_version:     ver_v2,
            init_state:        init_v2,
            migrate_state:     migrate_v2,
            create_scene_defs: create_defs_noop,
            scenes:            ThinVec::new(),
        }
    }

    #[test]
    fn from_entry_runs_init_state() {
        let v1 = v1_entry();
        let s = ActiveScript::from_entry(ScriptId::new(1), &v1);
        assert_eq!(s.state_buffer.len(), 4);
        assert_eq!(s.state_version, 0x1111_1111);
        assert_eq!(i32::from_ne_bytes(s.state_buffer[..].try_into().unwrap()), 7);
    }

    #[test]
    fn migrate_into_runs_migrate_state_when_layout_changes() {
        let v1 = v1_entry();
        let v2 = v2_entry();
        let mut s = ActiveScript::from_entry(ScriptId::new(1), &v1);

        s.migrate_into(&v2);
        assert_eq!(s.state_buffer.len(), 8);
        assert_eq!(s.state_version, 0x2222_2222);
        // First word: x = 7 (carried over from old).
        let x = i32::from_ne_bytes(s.state_buffer[0..4].try_into().unwrap());
        assert_eq!(x, 7);
        // Second word: old_version that migrate_v2 stamped in.
        let y = i32::from_ne_bytes(s.state_buffer[4..8].try_into().unwrap());
        assert_eq!(y as u32, 0x1111_1111);
    }

    #[test]
    fn migrate_into_is_noop_when_layout_matches() {
        let v1a = v1_entry();
        let v1b = v1_entry(); // same layout, same version
        let mut s = ActiveScript::from_entry(ScriptId::new(1), &v1a);
        let original_ptr = s.state_buffer.as_ptr();
        s.migrate_into(&v1b);
        // Buffer must not be re-allocated.
        assert_eq!(s.state_buffer.as_ptr(), original_ptr);
        assert_eq!(s.state_version, 0x1111_1111);
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
