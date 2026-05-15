//! AST types — all collections are `ThinVec`, all strings are `Arc<str>`.

use std::sync::Arc;
use thin_vec::ThinVec;

// ── Top level ────────────────────────────────────────────────────────────────

pub struct LangScript {
    pub name:       Arc<str>,
    pub state:      Option<StateBlock>,
    pub migrations: ThinVec<Migration>,
    pub scenes:     ThinVec<SceneDef>,
}

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty { I32, F64, Bool, ActorHandle, SceneId }

// ── State ────────────────────────────────────────────────────────────────────

pub struct StateBlock { pub fields: ThinVec<StateField> }

pub struct StateField {
    pub name:    Arc<str>,
    pub ty:      Ty,
    pub default: Option<Expr>,
}

// ── Migration ────────────────────────────────────────────────────────────────

pub struct Migration {
    pub from_version: u32,
    pub body:         ThinVec<MigrateStmt>,
}

pub enum MigrateStmt {
    /// `new.<field> = <expr>;` where the expression may reference `old.<field>`.
    Assign { field: Arc<str>, value: Expr },
}

// ── Scene ────────────────────────────────────────────────────────────────────

pub struct SceneDef {
    pub name:        Arc<str>,
    pub on_enter:    ThinVec<EffectStmt>,
    pub on_exit:     ThinVec<EffectStmt>,
    pub transitions: ThinVec<Transition>,
    pub behavior:    Option<BtNode>,
}

pub struct Transition {
    pub target:    Arc<str>,
    pub condition: CondExpr,
}

// ── Behavior tree ────────────────────────────────────────────────────────────

pub enum BtNode {
    Sequence(ThinVec<BtNode>),
    Selector(ThinVec<BtNode>),
    Parallel(ThinVec<BtNode>),
    Repeat   { count: u32,  child: Box<BtNode> },
    Inverter { child: Box<BtNode> },
    Guard    { cond: CondExpr,  child: Box<BtNode> },
    Cooldown { duration: f32,  child: Box<BtNode> },
    Leaf     { condition: Option<CondExpr>, action: Option<EffectStmt> },
}

// ── Conditions ───────────────────────────────────────────────────────────────

pub enum CondExpr {
    Call(Arc<str>, ThinVec<Expr>),
    And(Box<CondExpr>, Box<CondExpr>),
    Or (Box<CondExpr>, Box<CondExpr>),
    Not(Box<CondExpr>),
    Cmp(Box<Expr>, CmpOp, Box<Expr>),
    Bool(bool),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CmpOp { Eq, Ne, Lt, Le, Gt, Ge }

// ── Effect statements ────────────────────────────────────────────────────────

pub enum EffectStmt {
    /// `cue_troupe("name")` — fires whole-troupe transform delta = identity (named).
    CueTroupe(Arc<str>),
    /// `<intrinsic>(<args>);` — call a free intrinsic, no return value.
    Call(Arc<str>, ThinVec<Expr>),
    /// `<field> = <expr>;` — assignment into a state field.
    Assign { field: Arc<str>, value: Expr },
}

// ── Expressions ──────────────────────────────────────────────────────────────

pub enum Expr {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Arc<str>),
    Ident(Arc<str>),
    /// `<scope>.<field>` — currently used for `old.x` / `new.x` inside migrate.
    QualIdent { scope: Arc<str>, field: Arc<str> },
    Call(Arc<str>, ThinVec<Expr>),
    Neg(Box<Expr>),
    Bin(Box<Expr>, BinOp, Box<Expr>),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BinOp { Add, Sub, Mul, Div }
