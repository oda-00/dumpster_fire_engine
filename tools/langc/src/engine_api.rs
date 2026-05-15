//! `#[repr(C)]` layout shared between compiled `.lang` scripts and the engine.
//!
//! These types live ONLY in the engine — this module records the layout the
//! LLVM codegen must produce.  Field order and offsets MUST stay in lock-step
//! with `dumpster_fire_engine::resource_manager::event_manager::script` (the
//! engine side asserts this with `static_assertions`).
//!
//! Layout (Linux x86-64, default repr(C) alignment):
//!
//! ```text
//! struct ComponentCacheSlice {       // 16 B
//!     data: *const u64,              //  8 B at offset 0  (packed (gen,idx) ActorHandle)
//!     len:  u32,                     //  4 B at offset 8
//!     _pad: u32,                     //  4 B at offset 12 (alignment of next struct field)
//! }
//!
//! struct EngineAPI {                 // total 152 B
//!     locals:        *mut [f32;12]   //  offset   0
//!     worlds:        *const [f32;12] //  offset   8
//!     dirty_flags:   *mut bool       //  offset  16
//!     actor_count:   u32             //  offset  24
//!     _pad0:         u32             //  offset  28
//!     caches[5]:     ComponentCacheSlice  // offset 32..112  (5 * 16 = 80 B)
//!     push_effect:   *fn(*const EngineAPI, *const EffectAbi)
//!                                    //  offset 112
//!     cue_troupe:    *fn(*const EngineAPI, i64)
//!                                    //  offset 120
//!     elapsed:       f32             //  offset 128
//!     _pad1:         u32             //  offset 132
//!     tick_count:    u64             //  offset 136
//!     _pad2:         u64             //  offset 144
//! }
//!
//! struct EffectAbi {                 // 24 B
//!     kind: u8                       // offset  0
//!     _pad: [u8;7]                   // offset  1
//!     arg0: i64                      // offset  8
//!     arg1: i64                      // offset 16
//! }
//!
//! struct SceneEntry {                // 32 B
//!     raw_id:   i64                  // offset  0
//!     on_enter: *fn(*const EngineAPI, *mut u8)  // offset  8
//!     on_exit:  *fn(*const EngineAPI, *mut u8)  // offset 16
//!     tick:     *fn(*const EngineAPI, *mut u8) -> i64  // offset 24
//! }
//!
//! struct SceneDefArray {             // 16 B
//!     scene_count: u32               // offset  0
//!     _pad:        u32               // offset  4
//!     scenes:      *const SceneEntry // offset  8
//! }
//! ```

#![allow(dead_code)] // a handful of layout constants documented for the ABI but
                     // not consumed by the codegen yet (caches[], padding offsets).

/// ABI contract version.  Bump whenever the `EngineAPI` layout changes.
/// The engine validates this at load time before calling any entry point.
pub const ENGINE_ABI_VERSION: u32 = 1;

pub const N_COMPONENT_TYPES: usize = 5;

pub const COMPONENT_CACHE_SLICE_SIZE: u32 = 16;
pub const ENGINE_API_SIZE:            u32 = 152;
pub const EFFECT_ABI_SIZE:            u32 = 24;
pub const SCENE_ENTRY_SIZE:           u32 = 32;
pub const SCENE_DEF_ARRAY_SIZE:       u32 = 16;

// Engine-API field byte offsets (used by codegen for pointer arithmetic).
pub const API_OFF_LOCALS:      u32 = 0;
pub const API_OFF_WORLDS:      u32 = 8;
pub const API_OFF_DIRTY:       u32 = 16;
pub const API_OFF_ACTOR_COUNT: u32 = 24;
pub const API_OFF_CACHES:      u32 = 32;
pub const API_OFF_PUSH_EFFECT: u32 = 112;
pub const API_OFF_CUE_TROUPE:  u32 = 120;
pub const API_OFF_ELAPSED:     u32 = 128;
pub const API_OFF_TICK_COUNT:  u32 = 136;

// EffectAbi kinds.  Stable, plan-aligned.
pub const EFFECT_KIND_NOP:          u8 = 0;
pub const EFFECT_KIND_EMIT_EVENT:   u8 = 1;
pub const EFFECT_KIND_ATTACK:       u8 = 2;
pub const EFFECT_KIND_PATROL_PATH:  u8 = 3;

// BtStatus return values (matches engine BtStatus enum order:
//   0 = Running, 1 = Success, 2 = Failure).
pub const BT_RUNNING: i32 = 0;
pub const BT_SUCCESS: i32 = 1;
pub const BT_FAILURE: i32 = 2;
