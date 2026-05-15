use std::sync::Arc;
use thin_vec::ThinVec;
use rayon::prelude::*;
use crate::resource_manager::manager::{Arena, Handle, Id};
use super::scene::{Handler, SceneDef, SceneId};
use super::script_abi::{EngineAPI, SceneDefArray, SceneEntry, ENGINE_ABI_VERSION};
use super::object_loader::LoadedObject;

/// Batch sizes at or above this fan out across rayon's pool; below it the
/// sequential 4-way unrolled path wins because rayon's fork/join overhead
/// (~1–5 µs) dominates short ticks (~50–100 ns of real work each).
///
/// Mirrors `world_manager::world::propagate_transforms`'s 1024-actor threshold
/// — scaled down because per-script work is ~10× a transform copy.
pub const PARALLEL_TICK_THRESHOLD: usize = 64;

/// Same idea for object loading — each `LoadedObject::from_file` is ~10 µs of
/// disk-read + parse + mmap + relocate, comfortably above rayon's fork cost.
pub const PARALLEL_LOAD_THRESHOLD: usize = 4;

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

/// One compiled `.lang` object file loaded into executable memory.
///
/// `obj` MUST outlive every function pointer in `entry`.  The arena enforces
/// this by holding both together and dropping the object only via
/// `ScriptManager::unload`.
pub struct LoadedScript {
    pub id:          ScriptId,
    pub source_path: Arc<str>,
    pub obj:         Arc<LoadedObject>,
    pub entry:       ScriptEntryPoints,
}

#[derive(Debug)]
pub enum ScriptLoadError {
    Io(Arc<str>),
    MissingSymbol(&'static str),
    AbiMismatch { expected: u32, got: u32 },
}

impl core::fmt::Display for ScriptLoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScriptLoadError::Io(s)            => write!(f, "io: {s}"),
            ScriptLoadError::MissingSymbol(s) => write!(f, "missing symbol `{s}`"),
            ScriptLoadError::AbiMismatch { expected, got } =>
                write!(f, "ABI version mismatch: engine={expected}, script={got}"),
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

    /// Load a compiled `.lang` object file from `path`.  Assigns a fresh
    /// `ScriptId` and registers the object in the arena.
    pub fn load_from_file(&mut self, path: Arc<str>) -> Result<ScriptId, ScriptLoadError> {
        let obj = Arc::new(
            LoadedObject::from_file(std::path::Path::new(path.as_ref()))
                .map_err(|e| ScriptLoadError::Io(Arc::<str>::from(format!("{e}").as_str())))?
        );
        let entry = read_entry_points(&obj)?;
        let id = ScriptId::new(self.next_raw_id);
        self.next_raw_id += 1;
        let loaded = LoadedScript { id, source_path: path, obj, entry };
        let h = self.arena.insert(loaded);
        let pos = self.id_to_handle.partition_point(|(sid, _)| sid.raw() < id.raw());
        self.id_to_handle.insert(pos, (id, h));
        Ok(id)
    }

    /// Bulk-load `paths` in parallel.  Above `PARALLEL_LOAD_THRESHOLD` we
    /// dispatch each `LoadedObject::from_file` + entry-point materialisation
    /// onto rayon's pool — each load is independent (its own mmap region,
    /// its own relocation pass), so this is embarrassingly parallel.  The
    /// arena registration step at the end is single-threaded so `ScriptId`s
    /// stay in deterministic order.
    ///
    /// Returns one `Result` per input path, in input order.  A failure on
    /// path *i* does not abort the others — partial loads are preserved.
    pub fn load_many(
        &mut self,
        paths: &[Arc<str>],
    ) -> ThinVec<Result<ScriptId, ScriptLoadError>> {
        if paths.is_empty() {
            return ThinVec::new();
        }

        // Phase 1: parallel parse + relocate + entry-point probe.
        // `LoadedObject::from_file` does its own IO + mmap + relocations and
        // produces a fully-constructed `Arc<LoadedObject>` that we hand to
        // `read_entry_points` (which itself calls `df_create_scene_defs`).
        type Loaded = Result<(Arc<LoadedObject>, ScriptEntryPoints), ScriptLoadError>;
        let load_one = |p: &Arc<str>| -> Loaded {
            let obj = LoadedObject::from_file(std::path::Path::new(p.as_ref()))
                .map_err(|e| ScriptLoadError::Io(Arc::<str>::from(format!("{e}").as_str())))?;
            let obj = Arc::new(obj);
            let entry = read_entry_points(&obj)?;
            Ok((obj, entry))
        };

        let mut loaded: ThinVec<Loaded> = ThinVec::with_capacity(paths.len());
        if paths.len() >= PARALLEL_LOAD_THRESHOLD {
            // rayon doesn't directly collect into ThinVec; use std::Vec as the
            // intermediate then move element-by-element.  The Vec is dropped
            // immediately so this allocation never escapes the function.
            let v: Vec<Loaded> = paths.par_iter().map(load_one).collect();
            for r in v { loaded.push(r); }
        } else {
            for p in paths { loaded.push(load_one(p)); }
        }

        // Phase 2: single-threaded arena registration so IDs stay in order.
        let mut out: ThinVec<Result<ScriptId, ScriptLoadError>> =
            ThinVec::with_capacity(paths.len());
        for (path, res) in paths.iter().zip(loaded.into_iter()) {
            match res {
                Ok((obj, entry)) => {
                    let id = ScriptId::new(self.next_raw_id);
                    self.next_raw_id += 1;
                    let loaded = LoadedScript {
                        id, source_path: Arc::clone(path), obj, entry,
                    };
                    let h = self.arena.insert(loaded);
                    let pos = self.id_to_handle
                        .partition_point(|(sid, _)| sid.raw() < id.raw());
                    self.id_to_handle.insert(pos, (id, h));
                    out.push(Ok(id));
                }
                Err(e) => out.push(Err(e)),
            }
        }
        out
    }

    /// Hot-reload: replace the object for an existing `ScriptId` with a
    /// fresh compilation from `new_path`.  All function pointers in `entry`
    /// are rewritten in place; the previous object is dropped after the swap.
    pub fn hot_reload(
        &mut self,
        id: ScriptId,
        new_path: Arc<str>,
    ) -> Result<(), ScriptLoadError> {
        let h = self.handle_for(id).ok_or_else(|| ScriptLoadError::Io(
            Arc::<str>::from(format!("no script with id {}", id.raw()).as_str())
        ))?;
        let obj = Arc::new(
            LoadedObject::from_file(std::path::Path::new(new_path.as_ref()))
                .map_err(|e| ScriptLoadError::Io(Arc::<str>::from(format!("{e}").as_str())))?
        );
        let entry = read_entry_points(&obj)?;
        if let Some(loaded) = self.arena.get_mut(h) {
            // Drop the old object AFTER replacing the pointers, so no live
            // function pointer references freed memory.
            loaded.obj         = obj;
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
    /// Cached index of `active_scene` inside `entry.scenes` — avoids the
    /// linear search in the per-tick hot path.  `u32::MAX` means "stale",
    /// triggering a one-time re-resolve on the next `tick`.
    pub active_scene_idx: u32,
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
        let active_scene_idx = if entry.scenes.is_empty() { u32::MAX } else { 0 };
        ActiveScript {
            script_id, state_buffer: buf, state_version,
            active_scene, active_scene_idx, tick_count: 0, elapsed: 0.0,
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
            // Layout matches but scene table may have been reordered/replaced.
            self.active_scene_idx = u32::MAX;
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
        // Hot-reload always invalidates the cached scene index — entry.scenes
        // may have been reordered between compiles even when layout matches.
        self.active_scene_idx = u32::MAX;
    }

    /// Tick the active scene against `entry`'s scene table.  Returns the
    /// transition target (zero ⇒ no transition).  When a transition fires the
    /// previous scene's `on_exit` and the new scene's `on_enter` are invoked
    /// in order so state-buffer side effects from those handlers land.
    ///
    /// `api.elapsed` and `api.tick_count` are overwritten with this
    /// `ActiveScript`'s tracked values before each call.
    #[inline]
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

        // Cached-index fast path: avoids the per-tick linear search through
        // `entry.scenes`.  `u32::MAX` means "stale" (post-construction, post-
        // migrate, or post-transition); fall back to a one-shot resolve.
        let scene = match entry.scenes.get(self.active_scene_idx as usize) {
            Some(s) if s.raw_id == self.active_scene => s,
            _ => {
                let Some((idx, s)) = entry.scenes.iter().enumerate()
                    .find(|(_, s)| s.raw_id == self.active_scene) else { return 0; };
                self.active_scene_idx = idx as u32;
                s
            }
        };
        let next = unsafe { (scene.tick)(api, self.state_buffer.as_mut_ptr()) };
        if next != 0 && next != self.active_scene {
            unsafe { (scene.on_exit)(api, self.state_buffer.as_mut_ptr()); }
            if let Some((idx, target)) = entry.scenes.iter().enumerate()
                .find(|(_, s)| s.raw_id == next)
            {
                unsafe { (target.on_enter)(api, self.state_buffer.as_mut_ptr()); }
                self.active_scene     = next;
                self.active_scene_idx = idx as u32;
                self.elapsed          = 0.0;  // reset per-scene elapsed
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

// ── Batch tick ────────────────────────────────────────────────────────────────
//
// Mirror of `world_manager::stage::propagate_transforms`: every script's tick
// is independent of every other (each owns its own state buffer + EngineAPI +
// EffectSink), so above `PARALLEL_TICK_THRESHOLD` we fan out across rayon's
// pool; below it we run a 4-way unrolled sequential loop so the OOO core can
// keep four indirect JIT calls (and the dependent `push_effect` callbacks) in
// flight at once.
//
// SAFETY: callers must ensure the three slices are equal length and that no
// two `EngineAPI`s alias the same `EffectSink` — rayon will tick them on
// different worker threads.

/// Tick `scripts[i]` against `entries[i]` with `apis[i]` for every `i`.
///
/// Returns `()`; per-script transition targets are reflected in
/// `scripts[i].active_scene`.  Callers who need the transition vector should
/// fall back to ticking individually.
pub fn tick_batch(
    scripts: &mut [ActiveScript],
    entries: &[&ScriptEntryPoints],
    apis:    &mut [EngineAPI],
    dt:      f32,
) {
    assert_eq!(scripts.len(), entries.len(), "tick_batch: scripts/entries length mismatch");
    assert_eq!(scripts.len(), apis.len(),    "tick_batch: scripts/apis length mismatch");
    let n = scripts.len();
    if n == 0 { return; }

    if n >= PARALLEL_TICK_THRESHOLD {
        // Parallel: zip three disjoint mutable/immutable slices via rayon.
        // `entries` is `&[&ScriptEntryPoints]` — Copy of `&_`, so cloning the
        // outer slice into thread-locals is just pointer-sized.
        scripts.par_iter_mut()
            .zip(apis.par_iter_mut())
            .zip(entries.par_iter())
            .for_each(|((s, api), entry)| {
                let _ = s.tick(*entry, api, dt);
            });
        return;
    }

    // Sequential 4-way unroll.  Four independent script ticks per loop iter
    // — LLVM keeps the indirect calls and dependent EffectSink writes in
    // flight on the OOO core.  Tail loop handles `n % 4`.
    let mut i = 0;
    while i + 4 <= n {
        // SAFETY: indices are disjoint and within bounds; the assertions
        // above prove the three slices match in length.  `split_at_mut` is
        // the safe analog but adds bounds-check IR that LLVM struggles to
        // hoist out of the hot loop.
        let (s0, s1, s2, s3) = unsafe {
            let p = scripts.as_mut_ptr();
            (&mut *p.add(i), &mut *p.add(i + 1), &mut *p.add(i + 2), &mut *p.add(i + 3))
        };
        let (a0, a1, a2, a3) = unsafe {
            let p = apis.as_mut_ptr();
            (&mut *p.add(i), &mut *p.add(i + 1), &mut *p.add(i + 2), &mut *p.add(i + 3))
        };
        let e0 = entries[i];     let e1 = entries[i + 1];
        let e2 = entries[i + 2]; let e3 = entries[i + 3];
        let _ = s0.tick(e0, a0, dt);
        let _ = s1.tick(e1, a1, dt);
        let _ = s2.tick(e2, a2, dt);
        let _ = s3.tick(e3, a3, dt);
        i += 4;
    }
    while i < n {
        let _ = scripts[i].tick(entries[i], &mut apis[i], dt);
        i += 1;
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

fn read_entry_points(obj: &LoadedObject) -> Result<ScriptEntryPoints, ScriptLoadError> {
    // Check ABI version before touching any other entry point.
    let abi_version_fn: unsafe extern "C" fn() -> u32 = unsafe {
        obj.fn_ptr("df_abi_version")
            .ok_or(ScriptLoadError::MissingSymbol("df_abi_version"))?
    };
    let got = unsafe { abi_version_fn() };
    if got != ENGINE_ABI_VERSION {
        return Err(ScriptLoadError::AbiMismatch { expected: ENGINE_ABI_VERSION, got });
    }

    let state_size: unsafe extern "C" fn() -> u32 = unsafe {
        obj.fn_ptr("df_state_size").ok_or(ScriptLoadError::MissingSymbol("df_state_size"))?
    };
    let state_version: unsafe extern "C" fn() -> u32 = unsafe {
        obj.fn_ptr("df_state_version").ok_or(ScriptLoadError::MissingSymbol("df_state_version"))?
    };
    let init_state: unsafe extern "C" fn(*mut u8) = unsafe {
        obj.fn_ptr("df_init_state").ok_or(ScriptLoadError::MissingSymbol("df_init_state"))?
    };
    let migrate_state: unsafe extern "C" fn(u32, *const u8, *mut u8) = unsafe {
        obj.fn_ptr("df_migrate_state").ok_or(ScriptLoadError::MissingSymbol("df_migrate_state"))?
    };
    let create_scene_defs: unsafe extern "C" fn(*const EngineAPI, *mut SceneDefArray) = unsafe {
        obj.fn_ptr("df_create_scene_defs").ok_or(ScriptLoadError::MissingSymbol("df_create_scene_defs"))?
    };

    // Materialise the scene table.  Null `api` is safe — codegen only reads
    // the API pointer inside tick/on_enter/on_exit, not in create_scene_defs.
    let mut arr = SceneDefArray { scene_count: 0, _pad: 0, scenes: core::ptr::null() };
    unsafe { create_scene_defs(core::ptr::null(), &mut arr) };

    let mut scenes = ThinVec::with_capacity(arr.scene_count as usize);
    if !arr.scenes.is_null() && arr.scene_count > 0 {
        let raw = unsafe { core::slice::from_raw_parts(arr.scenes, arr.scene_count as usize) };
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
        state_size,
        state_version,
        init_state,
        migrate_state,
        create_scene_defs,
        scenes,
    })
}
