//! Semantic analysis: name resolution, type checking, state layout, HIR lowering.
//!
//! Symbol tables are sorted `ThinVec<(Arc<str>, T)>` and resolved with binary
//! search. No `HashMap`, no stored `String` or `std::Vec`.

use std::sync::Arc;
use thin_vec::ThinVec;
use crate::ast::{self, Ty};
use crate::hir::*;

// ── Public entry point ────────────────────────────────────────────────────────

pub fn lower(script: ast::LangScript) -> Result<HirScript, SemaError> {
    Sema::new(&script.name).lower(script)
}

// ── FNV-1a 64-bit hash ────────────────────────────────────────────────────────

#[inline]
pub fn fnv1a(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME:  u64 = 0x100000001b3;
    let mut h = OFFSET;
    for &b in data { h = (h ^ b as u64).wrapping_mul(PRIME); }
    h
}

#[inline]
fn type_tag(ty: Ty) -> &'static str {
    match ty {
        Ty::I32 => "i32", Ty::F64 => "f64", Ty::Bool => "bool",
        Ty::ActorHandle => "actor_handle", Ty::SceneId => "scene_id",
    }
}

#[inline]
fn type_size(ty: Ty) -> u32 {
    match ty {
        Ty::I32 | Ty::Bool => 4, // Bool stored as i32 for alignment regularity
        Ty::F64 | Ty::ActorHandle | Ty::SceneId => 8,
    }
}

#[inline]
fn type_align(ty: Ty) -> u32 { type_size(ty) }

/// Per-spec, the script's `state_version` is the FNV hash of the field-name
/// + type-tag sequence in declaration order, truncated to 32 bits. Adding,
/// removing, renaming, or retyping a field changes the version deterministically.
fn compute_state_version(fields: &[ast::StateField]) -> u32 {
    // Hash incrementally so no scratch buffer is needed — preserves the
    // "no std::Vec scratch" invariant.
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME:  u64 = 0x100000001b3;
    let mut h: u64 = OFFSET;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes { h = (h ^ b as u64).wrapping_mul(PRIME); }
    };
    for f in fields {
        mix(f.name.as_bytes());
        mix(b":");
        mix(type_tag(f.ty).as_bytes());
        mix(b"\n");
    }
    h as u32
}

fn scene_raw_id(script_name: &str, scene_name: &str) -> i64 {
    // Build the key without storing it: hash the script name, then a separator,
    // then the scene name.  Scratch String avoided by hashing piecewise.
    const SEP: &[u8] = b"::";
    let mut h = 0xcbf29ce484222325u64;
    const PRIME: u64 = 0x100000001b3;
    for &b in script_name.as_bytes() { h = (h ^ b as u64).wrapping_mul(PRIME); }
    for &b in SEP                    { h = (h ^ b as u64).wrapping_mul(PRIME); }
    for &b in scene_name.as_bytes()  { h = (h ^ b as u64).wrapping_mul(PRIME); }
    h as i64
}

// ── Sema ──────────────────────────────────────────────────────────────────────

struct Sema {
    script_name: Arc<str>,
    /// Sorted by name: (name, offset, ty).
    fields: ThinVec<(Arc<str>, u32, Ty)>,
    /// Old-layout fields when lowering a migration block: (name, offset, ty).
    old_fields: ThinVec<(Arc<str>, u32, Ty)>,
    /// Sorted by name: scene_name → raw_id.
    scenes: ThinVec<(Arc<str>, i64)>,
}

impl Sema {
    fn new(script_name: &Arc<str>) -> Self {
        Sema {
            script_name: script_name.clone(),
            fields:      ThinVec::new(),
            old_fields:  ThinVec::new(),
            scenes:      ThinVec::new(),
        }
    }

    fn lower(mut self, script: ast::LangScript) -> Result<HirScript, SemaError> {
        // ── Build field table & layout ───────────────────────────────────────
        let raw_fields: ThinVec<ast::StateField> = script.state
            .map(|s| s.fields).unwrap_or_default();
        let state_version = compute_state_version(&raw_fields);

        let mut hir_fields = ThinVec::new();
        let mut cursor = 0u32;
        let mut max_align = 1u32;
        for f in raw_fields.iter() {
            let align = type_align(f.ty);
            if align > max_align { max_align = align; }
            let pad = (align - cursor % align) % align;
            cursor += pad;
            let offset = cursor;
            sorted_insert(&mut self.fields, f.name.clone(), (offset, f.ty));
            cursor += type_size(f.ty);

            let default = match f.default.as_ref() {
                Some(e) => Some(self.lower_expr(e, f.ty, false)?),
                None    => None,
            };
            hir_fields.push(HirField {
                name: f.name.clone(), ty: f.ty, offset, default,
            });
        }
        let state_size = (cursor + max_align - 1) & !(max_align - 1); // round up

        // ── Build scene table ────────────────────────────────────────────────
        for s in &script.scenes {
            let raw = scene_raw_id(&self.script_name, &s.name);
            let pos = self.scenes.partition_point(|(n, _)| n.as_ref() < s.name.as_ref());
            if self.scenes.get(pos).is_some_and(|(n, _)| n.as_ref() == s.name.as_ref()) {
                return Err(SemaError {
                    msg: Arc::<str>::from(format!("duplicate scene `{}`", s.name).as_str()),
                });
            }
            self.scenes.insert(pos, (s.name.clone(), raw));
        }

        let entry_raw_id = script.scenes.first()
            .map(|s| scene_raw_id(&self.script_name, &s.name))
            .ok_or_else(|| SemaError { msg: "script has no scenes".into() })?;

        // ── Lower migrations ─────────────────────────────────────────────────
        let mut migrations = ThinVec::new();
        for m in script.migrations {
            // Migration semantics: `old.<f>` reads the previous layout, `new.<f>`
            // (implicit) writes the new one.  For a v1 implementation, we treat
            // old_fields as identical to current — the user is responsible for
            // expressing the actual layout shift via explicit assignments.
            self.old_fields = self.fields.clone();
            let mut stmts = ThinVec::new();
            for s in m.body {
                match s {
                    ast::MigrateStmt::Assign { field, value } => {
                        let (offset, ty) = self.lookup_field(&field)?;
                        let v = self.lower_expr(&value, ty, true)?;
                        stmts.push(HirAssign { new_offset: offset, ty, value: v });
                    }
                }
            }
            migrations.push(HirMigration { from_version: m.from_version, stmts });
        }

        // ── Lower scenes ─────────────────────────────────────────────────────
        let mut hir_scenes = ThinVec::new();
        for s in script.scenes {
            hir_scenes.push(self.lower_scene(s)?);
        }

        Ok(HirScript {
            name: self.script_name.clone(),
            state_size,
            state_align: max_align,
            state_version,
            fields: hir_fields,
            migrations,
            scenes: hir_scenes,
            entry_raw_id,
        })
    }

    fn lower_scene(&self, s: ast::SceneDef) -> Result<HirScene, SemaError> {
        let raw_id = scene_raw_id(&self.script_name, &s.name);

        let mut on_enter = ThinVec::new();
        for e in s.on_enter { on_enter.push(self.lower_effect(&e)?); }
        let mut on_exit = ThinVec::new();
        for e in s.on_exit  { on_exit.push(self.lower_effect(&e)?); }

        let mut transitions = ThinVec::new();
        for t in s.transitions {
            let target_raw = self.lookup_scene(&t.target)?;
            let condition  = self.lower_cond(&t.condition)?;
            transitions.push(HirTransition { target_raw_id: target_raw, condition });
        }

        let behavior = match s.behavior {
            Some(b) => Some(self.lower_bt(&b)?),
            None    => None,
        };

        Ok(HirScene { name: s.name, raw_id, on_enter, on_exit, transitions, behavior })
    }

    fn lower_bt(&self, n: &ast::BtNode) -> Result<HirBtNode, SemaError> {
        Ok(match n {
            ast::BtNode::Sequence(cs) => HirBtNode::Sequence(self.lower_bt_list(cs)?),
            ast::BtNode::Selector(cs) => HirBtNode::Selector(self.lower_bt_list(cs)?),
            ast::BtNode::Parallel(cs) => HirBtNode::Parallel(self.lower_bt_list(cs)?),
            ast::BtNode::Repeat { count, child } => HirBtNode::Repeat {
                count: *count, child: Box::new(self.lower_bt(child)?),
            },
            ast::BtNode::Inverter { child } => HirBtNode::Inverter {
                child: Box::new(self.lower_bt(child)?),
            },
            ast::BtNode::Guard { cond, child } => HirBtNode::Guard {
                cond: self.lower_cond(cond)?,
                child: Box::new(self.lower_bt(child)?),
            },
            ast::BtNode::Cooldown { duration, child } => HirBtNode::Cooldown {
                duration: *duration, child: Box::new(self.lower_bt(child)?),
            },
            ast::BtNode::Leaf { condition, action } => HirBtNode::Leaf {
                condition: condition.as_ref().map(|c| self.lower_cond(c)).transpose()?,
                action:    action.as_ref().map(|a| self.lower_effect(a)).transpose()?,
            },
        })
    }

    fn lower_bt_list(&self, list: &ThinVec<ast::BtNode>) -> Result<ThinVec<HirBtNode>, SemaError> {
        let mut v = ThinVec::new();
        for n in list { v.push(self.lower_bt(n)?); }
        Ok(v)
    }

    fn lower_cond(&self, c: &ast::CondExpr) -> Result<HirCondition, SemaError> {
        Ok(match c {
            ast::CondExpr::Bool(b) => HirCondition::Bool(*b),
            ast::CondExpr::Not(a)  => HirCondition::Not(Box::new(self.lower_cond(a)?)),
            ast::CondExpr::And(a, b) => HirCondition::And(
                Box::new(self.lower_cond(a)?), Box::new(self.lower_cond(b)?),
            ),
            ast::CondExpr::Or(a, b)  => HirCondition::Or(
                Box::new(self.lower_cond(a)?), Box::new(self.lower_cond(b)?),
            ),
            ast::CondExpr::Cmp(a, op, b) => HirCondition::Cmp(
                Box::new(self.lower_expr(a, Ty::F64, false)?),
                *op,
                Box::new(self.lower_expr(b, Ty::F64, false)?),
            ),
            ast::CondExpr::Call(name, args) => {
                let kind = match name.as_ref() {
                    "enemy_in_range" => IntrinsicPredicate::EnemyInRange,
                    "see_player"     => IntrinsicPredicate::SeePlayer,
                    "actor_near"     => IntrinsicPredicate::ActorNear,
                    "after_seconds"  => IntrinsicPredicate::AfterSeconds,
                    "event_fired"    => IntrinsicPredicate::EventFired,
                    _                => IntrinsicPredicate::Unknown,
                };
                let mut hir_args = ThinVec::new();
                for a in args { hir_args.push(self.lower_expr(a, Ty::F64, false)?); }
                HirCondition::Intrinsic(kind, hir_args)
            }
        })
    }

    fn lower_effect(&self, e: &ast::EffectStmt) -> Result<HirEffect, SemaError> {
        Ok(match e {
            ast::EffectStmt::CueTroupe(name) => {
                let troupe_id = fnv1a(name.as_bytes()) as i64;
                HirEffect::CueTroupe(troupe_id)
            }
            ast::EffectStmt::Call(name, args) => {
                let kind = match name.as_ref() {
                    "attack"        => IntrinsicEffect::Attack,
                    "patrol_path"   => IntrinsicEffect::PatrolPath,
                    "emit"          => IntrinsicEffect::EmitEvent,
                    _               => IntrinsicEffect::Unknown,
                };
                let mut hir_args = ThinVec::new();
                for a in args { hir_args.push(self.lower_expr(a, Ty::F64, false)?); }
                HirEffect::Intrinsic(kind, hir_args)
            }
            ast::EffectStmt::Assign { field, value } => {
                let (offset, ty) = self.lookup_field(field)?;
                let v = self.lower_expr(value, ty, false)?;
                HirEffect::AssignState { offset, ty, value: v }
            }
        })
    }

    fn lower_expr(&self, e: &ast::Expr, _expected: Ty, in_migration: bool) -> Result<HirExpr, SemaError> {
        Ok(match e {
            ast::Expr::Int(n)   => HirExpr::Int(*n),
            ast::Expr::Float(f) => HirExpr::Float(*f),
            ast::Expr::Bool(b)  => HirExpr::Bool(*b),
            ast::Expr::Str(_)   => return Err(SemaError {
                msg: "string literals are not valid in numeric expression context".into(),
            }),
            ast::Expr::Ident(name) => {
                let (offset, ty) = self.lookup_field(name)?;
                HirExpr::StateLoad { offset, ty }
            }
            ast::Expr::QualIdent { scope, field } => {
                if scope.as_ref() == "old" {
                    if !in_migration {
                        return Err(SemaError { msg: "`old.<field>` only valid in `migrate from ...`".into() });
                    }
                    let (offset, ty) = self.lookup_old_field(field)?;
                    HirExpr::OldStateLoad { offset, ty }
                } else if scope.as_ref() == "new" {
                    let (offset, ty) = self.lookup_field(field)?;
                    HirExpr::StateLoad { offset, ty }
                } else {
                    return Err(SemaError {
                        msg: Arc::<str>::from(format!("unknown scope `{scope}`; expected `old` or `new`").as_str()),
                    });
                }
            }
            ast::Expr::Call(name, args) => {
                let kind = match name.as_ref() {
                    "tick_count" => IntrinsicValue::TickCount,
                    "elapsed"    => IntrinsicValue::Elapsed,
                    _            => IntrinsicValue::Unknown,
                };
                let mut hir_args = ThinVec::new();
                for a in args { hir_args.push(self.lower_expr(a, Ty::F64, in_migration)?); }
                HirExpr::Intrinsic(kind, hir_args)
            }
            ast::Expr::Neg(inner) => HirExpr::Neg(Box::new(self.lower_expr(inner, Ty::F64, in_migration)?)),
            ast::Expr::Bin(a, op, b) => HirExpr::Bin(
                Box::new(self.lower_expr(a, Ty::F64, in_migration)?),
                *op,
                Box::new(self.lower_expr(b, Ty::F64, in_migration)?),
            ),
        })
    }

    fn lookup_field(&self, name: &str) -> Result<(u32, Ty), SemaError> {
        binary_lookup(&self.fields, name)
            .ok_or_else(|| SemaError {
                msg: Arc::<str>::from(format!("unknown state field `{name}`").as_str()),
            })
    }

    fn lookup_old_field(&self, name: &str) -> Result<(u32, Ty), SemaError> {
        binary_lookup(&self.old_fields, name)
            .ok_or_else(|| SemaError {
                msg: Arc::<str>::from(format!("`old.{name}` not in previous layout").as_str()),
            })
    }

    fn lookup_scene(&self, name: &str) -> Result<i64, SemaError> {
        let pos = self.scenes.partition_point(|(n, _)| n.as_ref() < name);
        self.scenes.get(pos)
            .filter(|(n, _)| n.as_ref() == name)
            .map(|(_, id)| *id)
            .ok_or_else(|| SemaError {
                msg: Arc::<str>::from(format!("unknown scene `{name}`").as_str()),
            })
    }
}

// ── Sorted-array helpers ──────────────────────────────────────────────────────

fn sorted_insert(
    table: &mut ThinVec<(Arc<str>, u32, Ty)>,
    name: Arc<str>,
    val: (u32, Ty),
) {
    let pos = table.partition_point(|(n, _, _)| n.as_ref() < name.as_ref());
    table.insert(pos, (name, val.0, val.1));
}

fn binary_lookup(table: &ThinVec<(Arc<str>, u32, Ty)>, key: &str) -> Option<(u32, Ty)> {
    let pos = table.partition_point(|(n, _, _)| n.as_ref() < key);
    table.get(pos)
        .filter(|(n, _, _)| n.as_ref() == key)
        .map(|(_, off, ty)| (*off, *ty))
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct SemaError {
    pub msg: Arc<str>,
}

impl core::fmt::Display for SemaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "semantic error: {}", self.msg)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer::Lexer, parser::Parser};

    fn lower_str(s: &str) -> HirScript {
        let toks = Lexer::new(s).tokenise().expect("lex");
        let ast  = Parser::new(toks).parse_script().expect("parse");
        super::lower(ast).expect("lower")
    }

    #[test]
    fn state_layout_and_version_deterministic() {
        let src = r#"
            script "g" {
                state {
                    a: i32 = 0
                    b: f64 = 0.0
                }
                scene s { }
            }
        "#;
        let h1 = lower_str(src);
        let h2 = lower_str(src);
        assert_eq!(h1.state_size, 16);
        assert_eq!(h1.state_version, h2.state_version);
    }

    #[test]
    fn missing_scene_in_transition_errors() {
        let src = r#"
            script "g" {
                scene a {
                    transition zzz when true;
                }
            }
        "#;
        let toks = Lexer::new(src).tokenise().unwrap();
        let ast  = Parser::new(toks).parse_script().unwrap();
        let err = match super::lower(ast) {
            Ok(_) => panic!("expected sema error"),
            Err(e) => e,
        };
        assert!(err.msg.contains("unknown scene"));
    }

    #[test]
    fn fnv_hash_stable() {
        assert_eq!(fnv1a(b""),     0xcbf29ce484222325);
        assert_eq!(fnv1a(b"abc"),  0xe71fa2190541574b);
    }

    #[test]
    fn migration_old_state_load_resolves() {
        let src = r#"
            script "m" {
                state {
                    x: i32 = 0
                    y: f64 = 0.0
                }
                migrate from 123 {
                    x = old.x
                    y = old.y + 1.0
                }
                scene s { }
            }
        "#;
        let h = lower_str(src);
        assert_eq!(h.migrations.len(), 1);
        let m = &h.migrations[0];
        assert_eq!(m.from_version, 123);
        assert_eq!(m.stmts.len(), 2);
        // y at offset 8 — first migration stmt writes x (offset 0), second y (offset 8).
        assert_eq!(m.stmts[0].new_offset, 0);
        assert_eq!(m.stmts[1].new_offset, 8);
    }

    #[test]
    fn migration_old_field_outside_block_errors() {
        let src = r#"
            script "m" {
                state { x: i32 = 0 }
                scene s {
                    on_enter => x = old.x;
                }
            }
        "#;
        let toks = Lexer::new(src).tokenise().unwrap();
        let ast  = Parser::new(toks).parse_script().unwrap();
        match super::lower(ast) {
            Ok(_) => panic!("expected `old.x` outside migrate to fail"),
            Err(e) => assert!(e.msg.contains("only valid in `migrate")),
        }
    }

    #[test]
    fn duplicate_scene_errors() {
        let src = r#"
            script "m" {
                scene s { }
                scene s { }
            }
        "#;
        let toks = Lexer::new(src).tokenise().unwrap();
        let ast  = Parser::new(toks).parse_script().unwrap();
        match super::lower(ast) {
            Ok(_) => panic!("expected duplicate-scene error"),
            Err(e) => assert!(e.msg.contains("duplicate scene")),
        }
    }

    #[test]
    fn state_version_changes_with_layout() {
        let v_a = lower_str(r#"script "a" { state { x: i32 = 0 } scene s {} }"#).state_version;
        let v_b = lower_str(r#"script "a" { state { x: f64 = 0.0 } scene s {} }"#).state_version;
        let v_c = lower_str(r#"script "a" { state { x: i32 = 0, y: i32 = 0 } scene s {} }"#).state_version;
        assert_ne!(v_a, v_b, "type change must change version");
        assert_ne!(v_a, v_c, "adding a field must change version");
    }
}
