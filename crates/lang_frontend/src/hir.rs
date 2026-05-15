//! HIR — flat, fully-resolved program representation. `ThinVec` + `Arc<str>` only.

use std::sync::Arc;
use thin_vec::ThinVec;
use crate::ast::{BinOp, CmpOp, Ty};

pub struct HirScript {
    pub name:          Arc<str>,
    pub state_size:    u32,
    pub state_align:   u32,
    pub state_version: u32,
    pub fields:        ThinVec<HirField>,
    pub migrations:    ThinVec<HirMigration>,
    pub scenes:        ThinVec<HirScene>,
    /// Raw `SceneId` of the entry scene (FNV-1a of `name::entry_scene_name`).
    pub entry_raw_id:  i64,
}

pub struct HirField {
    pub name:    Arc<str>,
    pub ty:      Ty,
    pub offset:  u32,
    pub default: Option<HirExpr>,
}

pub struct HirMigration {
    pub from_version: u32,
    pub stmts:        ThinVec<HirAssign>,
}

pub struct HirAssign {
    pub new_offset: u32,
    pub ty:         Ty,
    pub value:      HirExpr,
}

pub struct HirScene {
    pub name:        Arc<str>,
    pub raw_id:      i64,
    pub on_enter:    ThinVec<HirEffect>,
    pub on_exit:     ThinVec<HirEffect>,
    pub transitions: ThinVec<HirTransition>,
    pub behavior:    Option<HirBtNode>,
}

pub struct HirTransition {
    pub target_raw_id: i64,
    pub condition:     HirCondition,
}

pub enum HirBtNode {
    Sequence(ThinVec<HirBtNode>),
    Selector(ThinVec<HirBtNode>),
    Parallel(ThinVec<HirBtNode>),
    Repeat   { count: u32, child: Box<HirBtNode> },
    Inverter { child: Box<HirBtNode> },
    Guard    { cond: HirCondition, child: Box<HirBtNode> },
    Cooldown { duration: f32, child: Box<HirBtNode> },
    Leaf     { condition: Option<HirCondition>, action: Option<HirEffect> },
}

pub enum HirCondition {
    /// Built-in or user predicate call.  Resolved to one of the well-known
    /// intrinsic IDs in `sema::INTRINSIC_PREDICATES` or treated as `false`
    /// when unknown (diagnostic emitted at sema time).
    Intrinsic(IntrinsicPredicate, ThinVec<HirExpr>),
    And(Box<HirCondition>, Box<HirCondition>),
    Or (Box<HirCondition>, Box<HirCondition>),
    Not(Box<HirCondition>),
    Cmp(Box<HirExpr>, CmpOp, Box<HirExpr>),
    Bool(bool),
}

pub enum HirEffect {
    /// Whole-troupe cue, troupe id derived as `fnv1a(name) as i64`.
    CueTroupe(i64),
    /// `push_effect` via the engine callback — `EffectKind` discriminates.
    Intrinsic(IntrinsicEffect, ThinVec<HirExpr>),
    /// Store into state field at `offset` with the inner expression.
    AssignState { offset: u32, ty: Ty, value: HirExpr },
}

pub enum HirExpr {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// State-field read.  `offset` + `ty` baked at compile time.
    StateLoad { offset: u32, ty: Ty },
    /// `old.<offset>` read inside a migration block.
    OldStateLoad { offset: u32, ty: Ty },
    Neg(Box<HirExpr>),
    Bin(Box<HirExpr>, BinOp, Box<HirExpr>),
    /// Intrinsic that returns a numeric value.
    Intrinsic(IntrinsicValue, ThinVec<HirExpr>),
}

// ── Intrinsic catalogue ───────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IntrinsicPredicate {
    /// `enemy_in_range(f64)` — true when nearest enemy within radius.
    EnemyInRange,
    /// `see_player(f64)` — true when player within radius.
    SeePlayer,
    /// `actor_near(i64 actor_id, f64 radius)` — true when stage's first actor is near another.
    ActorNear,
    /// `after_seconds(f64)` — true when `elapsed >= arg`.
    AfterSeconds,
    /// `event_fired(i64 event_id)` — true when matching custom event drained this tick.
    EventFired,
    /// User-defined; the compiler emits a stub returning `false` and a diagnostic.
    Unknown,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IntrinsicEffect {
    /// `attack` / generic action invoking emit_event with a fixed event id.
    EmitEvent,
    /// `patrol_path()` — placeholder action emitting a tagged event.
    PatrolPath,
    /// `attack()` — placeholder action emitting a tagged event.
    Attack,
    Unknown,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IntrinsicValue {
    /// `tick_count()` — engine tick counter (u64 truncated to i64).
    TickCount,
    /// `elapsed()` — scene elapsed time in seconds (f64).
    Elapsed,
    Unknown,
}
