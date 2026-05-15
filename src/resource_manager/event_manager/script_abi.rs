//! C-ABI types shared with compiled `.lang` shared libraries.
//!
//! The byte layout below MUST match `tools/langc/src/engine_api.rs`.  Any
//! drift will be caught at link-load time by the offset assertions at the
//! bottom of this file.

use core::sync::atomic::{AtomicU64, Ordering};

/// Wide-form ActorHandle packed for FFI: `(generation << 32) | idx`.
pub type ActorHandlePacked = u64;

#[repr(C)]
pub struct ComponentCacheSlice {
    pub data: *const ActorHandlePacked,
    pub len:  u32,
    pub _pad: u32,
}

/// The C-ABI engine API descriptor passed into every script function.
///
/// Field order, sizes, and offsets are part of the public ABI (see the
/// `static_assertions` below).  Adding fields means bumping the SO ABI version.
#[repr(C)]
pub struct EngineAPI {
    /// Stage SoA: actor-local transforms (`Affine3A` = `[f32;12]`).
    pub locals:      *mut [f32; 12],
    /// Stage SoA: actor world transforms (read-only for scripts).
    pub worlds:      *const [f32; 12],
    /// Dirty flag parallel to `locals` / `worlds`.
    pub dirty_flags: *mut bool,
    /// Number of valid slots in the above arrays.
    pub actor_count: u32,
    pub _pad0:       u32,
    /// One slice per `ComponentType` (`ComponentType::COUNT == 5`).
    pub caches:      [ComponentCacheSlice; 5],
    /// `push_effect(api, effect)` — pushes a dynamic effect to the buffer.
    pub push_effect: unsafe extern "C" fn(*const EngineAPI, *const EffectAbi),
    /// `cue_troupe(api, troupe_id)` — fires an identity cue on the named troupe.
    pub cue_troupe:  unsafe extern "C" fn(*const EngineAPI, i64),
    /// Scene elapsed time in seconds (f32 for cache fit; codegen widens to f64).
    pub elapsed:     f32,
    pub _pad1:       u32,
    /// Engine tick counter (monotonic per-Play).
    pub tick_count:  u64,
    pub _pad2:       u64,
}

#[repr(C)]
pub struct EffectAbi {
    pub kind: u8,
    pub _pad: [u8; 7],
    pub arg0: i64,
    pub arg1: i64,
}

#[repr(C)]
pub struct SceneEntry {
    pub raw_id:   i64,
    pub on_enter: unsafe extern "C" fn(*const EngineAPI, *mut u8),
    pub on_exit:  unsafe extern "C" fn(*const EngineAPI, *mut u8),
    pub tick:     unsafe extern "C" fn(*const EngineAPI, *mut u8) -> i64,
}

#[repr(C)]
pub struct SceneDefArray {
    pub scene_count: u32,
    pub _pad:        u32,
    pub scenes:      *const SceneEntry,
}

// ── ABI byte-offset assertions ────────────────────────────────────────────────
//
// Const-evaluated at every build.  A mismatch here makes the engine fail to
// build, catching any drift between this file and the langc codegen module
// before runtime.

const _: () = {
    assert!(core::mem::size_of::<ComponentCacheSlice>() == 16);
    assert!(core::mem::size_of::<EngineAPI>()           == 152);
    assert!(core::mem::size_of::<EffectAbi>()           == 24);
    assert!(core::mem::size_of::<SceneEntry>()          == 32);
    assert!(core::mem::size_of::<SceneDefArray>()       == 16);

    // EngineAPI field offsets (must match engine_api.rs in tools/langc).
    assert!(core::mem::offset_of!(EngineAPI, locals)      ==   0);
    assert!(core::mem::offset_of!(EngineAPI, worlds)      ==   8);
    assert!(core::mem::offset_of!(EngineAPI, dirty_flags) ==  16);
    assert!(core::mem::offset_of!(EngineAPI, actor_count) ==  24);
    assert!(core::mem::offset_of!(EngineAPI, caches)      ==  32);
    assert!(core::mem::offset_of!(EngineAPI, push_effect) == 112);
    assert!(core::mem::offset_of!(EngineAPI, cue_troupe)  == 120);
    assert!(core::mem::offset_of!(EngineAPI, elapsed)     == 128);
    assert!(core::mem::offset_of!(EngineAPI, tick_count)  == 136);
};

// ── Engine-side callbacks invoked by compiled scripts ─────────────────────────
//
// These are the `push_effect` / `cue_troupe` function pointers wired into
// `EngineAPI` before calling any script function.  They route into a
// per-EngineAPI scratch buffer (see `EffectSink` in `script.rs`).

/// Discriminants for `EffectAbi.kind`, stable across the ABI surface.
pub mod effect_kind {
    pub const NOP:           u8 = 0;
    pub const EMIT_EVENT:    u8 = 1;
    pub const ATTACK:        u8 = 2;
    pub const PATROL_PATH:   u8 = 3;
}

// ── Effect sink: where the engine routes pushed effects ──────────────────────

/// Heap-allocated EffectAbi log shared between an `EngineAPI` and the engine
/// code that constructs it.  Scripts push effects into the log via the C-ABI
/// `push_effect` callback; the engine drains the log after each tick.
///
/// Synchronisation: the log is single-threaded — the engine constructs one
/// `EngineAPI`, calls a script's `tick`, then drains.  No locks needed.
pub struct EffectSink {
    /// Per-effect entries in arrival order.  Reused across ticks via `clear`.
    pub entries: thin_vec::ThinVec<EffectAbi>,
    /// Cue-troupe entries collected via `cue_troupe(api, troupe_id)`.
    pub cues:    thin_vec::ThinVec<i64>,
    /// Last-observed tick counter (bumped by the engine before each tick).
    pub tick:    AtomicU64,
}

impl EffectSink {
    pub fn new() -> Self {
        EffectSink {
            entries: thin_vec::ThinVec::new(),
            cues:    thin_vec::ThinVec::new(),
            tick:    AtomicU64::new(0),
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.cues.clear();
    }
}

impl Default for EffectSink {
    fn default() -> Self { Self::new() }
}

// The C-ABI callbacks that scripts invoke through `EngineAPI`.  The opaque
// `*const EngineAPI` first argument carries a back-pointer to the `EffectSink`
// via the `caches` array's first entry — see `engine_api_with_sink` below.

pub unsafe extern "C" fn cb_push_effect(api: *const EngineAPI, e: *const EffectAbi) {
    unsafe {
        let sink_ptr = (*api).caches[0].data as *mut EffectSink;
        if sink_ptr.is_null() { return; }
        (*sink_ptr).entries.push(EffectAbi {
            kind: (*e).kind,
            _pad: (*e)._pad,
            arg0: (*e).arg0,
            arg1: (*e).arg1,
        });
    }
}

pub unsafe extern "C" fn cb_cue_troupe(api: *const EngineAPI, troupe_id: i64) {
    unsafe {
        let sink_ptr = (*api).caches[0].data as *mut EffectSink;
        if sink_ptr.is_null() { return; }
        (*sink_ptr).cues.push(troupe_id);
    }
}

/// Construct an `EngineAPI` instance that routes `push_effect`/`cue_troupe`
/// into `sink`.  The returned struct holds raw pointers into the sink and
/// must not outlive it.
///
/// `caches[0].data` is overloaded to carry a `*mut EffectSink` — the other
/// four cache slices are empty (`len = 0`).  This avoids an extra indirection
/// field on the wire-level `EngineAPI` struct.
pub fn engine_api_for_sink(sink: &mut EffectSink) -> EngineAPI {
    let mut caches = [
        ComponentCacheSlice { data: core::ptr::null(), len: 0, _pad: 0 },
        ComponentCacheSlice { data: core::ptr::null(), len: 0, _pad: 0 },
        ComponentCacheSlice { data: core::ptr::null(), len: 0, _pad: 0 },
        ComponentCacheSlice { data: core::ptr::null(), len: 0, _pad: 0 },
        ComponentCacheSlice { data: core::ptr::null(), len: 0, _pad: 0 },
    ];
    caches[0].data = (sink as *mut EffectSink) as *const ActorHandlePacked;

    EngineAPI {
        locals:      core::ptr::null_mut(),
        worlds:      core::ptr::null(),
        dirty_flags: core::ptr::null_mut(),
        actor_count: 0,
        _pad0:       0,
        caches,
        push_effect: cb_push_effect,
        cue_troupe:  cb_cue_troupe,
        elapsed:     0.0,
        _pad1:       0,
        tick_count:  sink.tick.load(Ordering::Relaxed),
        _pad2:       0,
    }
}
