//! Recursive-descent parser. Produces `ast::LangScript`.

use std::sync::Arc;
use thin_vec::ThinVec;
use crate::ast::*;
use crate::lexer::{Token, TokenKind};

pub struct Parser {
    toks: ThinVec<Token>,
    pos:  usize,
}

impl Parser {
    pub fn new(toks: ThinVec<Token>) -> Self { Parser { toks, pos: 0 } }

    // ── Top level ─────────────────────────────────────────────────────────────

    pub fn parse_script(mut self) -> Result<LangScript, ParseError> {
        self.expect(TokenKind::Script)?;
        let name = self.expect_string()?;
        self.expect(TokenKind::LBrace)?;
        let mut state      = None;
        let mut migrations = ThinVec::new();
        let mut scenes     = ThinVec::new();
        while !self.check_kind(&TokenKind::RBrace) {
            match self.peek_kind() {
                TokenKind::State   => { state = Some(self.parse_state()?); }
                TokenKind::Migrate => { migrations.push(self.parse_migration()?); }
                TokenKind::Scene   => { scenes.push(self.parse_scene()?); }
                _ => return Err(self.err("expected `state`, `migrate`, or `scene`")),
            }
        }
        self.expect(TokenKind::RBrace)?;
        self.expect(TokenKind::Eof)?;
        Ok(LangScript { name, state, migrations, scenes })
    }

    // ── State block ───────────────────────────────────────────────────────────

    fn parse_state(&mut self) -> Result<StateBlock, ParseError> {
        self.expect(TokenKind::State)?;
        self.expect(TokenKind::LBrace)?;
        let mut fields = ThinVec::new();
        while !self.check_kind(&TokenKind::RBrace) {
            let name = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_ty()?;
            let default = if self.consume(&TokenKind::Eq) {
                Some(self.parse_expr()?)
            } else { None };
            self.consume(&TokenKind::Comma);
            self.consume(&TokenKind::Semi);
            fields.push(StateField { name, ty, default });
        }
        self.expect(TokenKind::RBrace)?;
        Ok(StateBlock { fields })
    }

    fn parse_ty(&mut self) -> Result<Ty, ParseError> {
        let tok = self.advance();
        Ok(match tok.kind {
            TokenKind::TyI32         => Ty::I32,
            TokenKind::TyF64         => Ty::F64,
            TokenKind::TyBool        => Ty::Bool,
            TokenKind::TyActorHandle => Ty::ActorHandle,
            TokenKind::TySceneId     => Ty::SceneId,
            _ => return Err(ParseError {
                msg: "expected type".into(), line: tok.line, col: tok.col,
            }),
        })
    }

    // ── Migration ─────────────────────────────────────────────────────────────

    fn parse_migration(&mut self) -> Result<Migration, ParseError> {
        self.expect(TokenKind::Migrate)?;
        self.expect(TokenKind::From)?;
        let v_tok = self.advance();
        let from_version = match v_tok.kind {
            TokenKind::IntLit(n) if n >= 0 && n <= u32::MAX as i64 => n as u32,
            TokenKind::HexLit(h) if h <= u32::MAX as u64 => h as u32,
            _ => return Err(ParseError {
                msg: "expected version number".into(), line: v_tok.line, col: v_tok.col,
            }),
        };
        self.expect(TokenKind::LBrace)?;
        let mut body = ThinVec::new();
        while !self.check_kind(&TokenKind::RBrace) {
            // <field> = <expr>;
            let field = self.expect_ident()?;
            self.expect(TokenKind::Eq)?;
            let value = self.parse_expr()?;
            self.expect(TokenKind::Semi)?;
            body.push(MigrateStmt::Assign { field, value });
        }
        self.expect(TokenKind::RBrace)?;
        Ok(Migration { from_version, body })
    }

    // ── Scene ─────────────────────────────────────────────────────────────────

    fn parse_scene(&mut self) -> Result<SceneDef, ParseError> {
        self.expect(TokenKind::Scene)?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::LBrace)?;
        let mut on_enter    = ThinVec::new();
        let mut on_exit     = ThinVec::new();
        let mut transitions = ThinVec::new();
        let mut behavior    = None;
        while !self.check_kind(&TokenKind::RBrace) {
            match self.peek_kind() {
                TokenKind::OnEnter => {
                    self.advance();
                    if self.consume(&TokenKind::FatArrow) {
                        // `on_enter => <stmt>;`
                        on_enter.push(self.parse_effect_stmt()?);
                        self.consume(&TokenKind::Semi);
                    } else {
                        self.expect(TokenKind::LBrace)?;
                        while !self.check_kind(&TokenKind::RBrace) {
                            on_enter.push(self.parse_effect_stmt()?);
                            self.consume(&TokenKind::Semi);
                        }
                        self.expect(TokenKind::RBrace)?;
                    }
                }
                TokenKind::OnExit => {
                    self.advance();
                    if self.consume(&TokenKind::FatArrow) {
                        on_exit.push(self.parse_effect_stmt()?);
                        self.consume(&TokenKind::Semi);
                    } else {
                        self.expect(TokenKind::LBrace)?;
                        while !self.check_kind(&TokenKind::RBrace) {
                            on_exit.push(self.parse_effect_stmt()?);
                            self.consume(&TokenKind::Semi);
                        }
                        self.expect(TokenKind::RBrace)?;
                    }
                }
                TokenKind::Transition => {
                    self.advance();
                    let target = self.expect_ident()?;
                    self.expect(TokenKind::When)?;
                    let condition = self.parse_cond()?;
                    self.consume(&TokenKind::Semi);
                    transitions.push(Transition { target, condition });
                }
                TokenKind::Behavior => {
                    self.advance();
                    self.expect(TokenKind::LBrace)?;
                    behavior = Some(self.parse_bt_node()?);
                    self.expect(TokenKind::RBrace)?;
                }
                _ => return Err(self.err("expected scene body element")),
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(SceneDef { name, on_enter, on_exit, transitions, behavior })
    }

    // ── Behavior tree ─────────────────────────────────────────────────────────

    fn parse_bt_node(&mut self) -> Result<BtNode, ParseError> {
        match self.peek_kind() {
            TokenKind::Sequence => { self.advance(); Ok(BtNode::Sequence(self.parse_bt_children()?)) }
            TokenKind::Selector => { self.advance(); Ok(BtNode::Selector(self.parse_bt_children()?)) }
            TokenKind::Parallel => { self.advance(); Ok(BtNode::Parallel(self.parse_bt_children()?)) }
            TokenKind::Repeat => {
                self.advance();
                self.expect(TokenKind::LParen)?;
                let count = self.expect_uint()? as u32;
                self.expect(TokenKind::RParen)?;
                self.expect(TokenKind::LBrace)?;
                let child = Box::new(self.parse_bt_node()?);
                self.expect(TokenKind::RBrace)?;
                Ok(BtNode::Repeat { count, child })
            }
            TokenKind::Inverter => {
                self.advance();
                self.expect(TokenKind::LBrace)?;
                let child = Box::new(self.parse_bt_node()?);
                self.expect(TokenKind::RBrace)?;
                Ok(BtNode::Inverter { child })
            }
            TokenKind::Guard => {
                self.advance();
                self.expect(TokenKind::LParen)?;
                let cond = self.parse_cond()?;
                self.expect(TokenKind::RParen)?;
                self.expect(TokenKind::LBrace)?;
                let child = Box::new(self.parse_bt_node()?);
                self.expect(TokenKind::RBrace)?;
                Ok(BtNode::Guard { cond, child })
            }
            TokenKind::Cooldown => {
                self.advance();
                self.expect(TokenKind::LParen)?;
                let duration = self.expect_float()? as f32;
                self.expect(TokenKind::RParen)?;
                self.expect(TokenKind::LBrace)?;
                let child = Box::new(self.parse_bt_node()?);
                self.expect(TokenKind::RBrace)?;
                Ok(BtNode::Cooldown { duration, child })
            }
            TokenKind::Condition | TokenKind::Action => {
                let condition = if self.consume(&TokenKind::Condition) {
                    let c = self.parse_cond()?;
                    self.consume(&TokenKind::Comma);
                    Some(c)
                } else { None };
                let action = if self.consume(&TokenKind::Action) {
                    Some(self.parse_effect_stmt()?)
                } else { None };
                Ok(BtNode::Leaf { condition, action })
            }
            _ => Err(self.err("expected behavior-tree node")),
        }
    }

    fn parse_bt_children(&mut self) -> Result<ThinVec<BtNode>, ParseError> {
        self.expect(TokenKind::LBrace)?;
        let mut v = ThinVec::new();
        while !self.check_kind(&TokenKind::RBrace) {
            v.push(self.parse_bt_node()?);
            self.consume(&TokenKind::Comma);
        }
        self.expect(TokenKind::RBrace)?;
        Ok(v)
    }

    // ── Conditions ────────────────────────────────────────────────────────────

    fn parse_cond(&mut self) -> Result<CondExpr, ParseError> {
        let lhs = self.parse_cond_and()?;
        if self.consume(&TokenKind::Or) {
            let rhs = self.parse_cond()?;
            return Ok(CondExpr::Or(Box::new(lhs), Box::new(rhs)));
        }
        Ok(lhs)
    }

    fn parse_cond_and(&mut self) -> Result<CondExpr, ParseError> {
        let lhs = self.parse_cond_not()?;
        if self.consume(&TokenKind::And) {
            let rhs = self.parse_cond_and()?;
            return Ok(CondExpr::And(Box::new(lhs), Box::new(rhs)));
        }
        Ok(lhs)
    }

    fn parse_cond_not(&mut self) -> Result<CondExpr, ParseError> {
        if self.consume(&TokenKind::Not) {
            return Ok(CondExpr::Not(Box::new(self.parse_cond_not()?)));
        }
        self.parse_cond_atom()
    }

    fn parse_cond_atom(&mut self) -> Result<CondExpr, ParseError> {
        // Boolean literal short-circuit
        if let TokenKind::BoolLit(b) = self.peek_kind().clone() {
            self.advance();
            return Ok(CondExpr::Bool(b));
        }
        if self.consume(&TokenKind::LParen) {
            let c = self.parse_cond()?;
            self.expect(TokenKind::RParen)?;
            return Ok(c);
        }
        // Try a comparison `<expr> <op> <expr>` by lookahead-snapshotting the parser.
        let snapshot = self.pos;
        if let Ok(lhs) = self.parse_expr() {
            if let Some(op) = self.try_consume_cmp_op() {
                let rhs = self.parse_expr()?;
                return Ok(CondExpr::Cmp(Box::new(lhs), op, Box::new(rhs)));
            }
            // Otherwise the consumed expr should be an Ident(name)(args) call.
            if let Expr::Call(name, args) = lhs {
                return Ok(CondExpr::Call(name, args));
            }
            // Bare identifier means zero-arg predicate.
            if let Expr::Ident(name) = lhs {
                return Ok(CondExpr::Call(name, ThinVec::new()));
            }
        }
        self.pos = snapshot;
        Err(self.err("expected condition"))
    }

    fn try_consume_cmp_op(&mut self) -> Option<CmpOp> {
        let op = match self.peek_kind() {
            TokenKind::EqEq  => CmpOp::Eq,
            TokenKind::NotEq => CmpOp::Ne,
            TokenKind::Lt    => CmpOp::Lt,
            TokenKind::Le    => CmpOp::Le,
            TokenKind::Gt    => CmpOp::Gt,
            TokenKind::Ge    => CmpOp::Ge,
            _ => return None,
        };
        self.advance();
        Some(op)
    }

    // ── Effect statements ─────────────────────────────────────────────────────

    fn parse_effect_stmt(&mut self) -> Result<EffectStmt, ParseError> {
        // Two shapes: `<ident>(<args>)` or `<ident> = <expr>` (assignment).
        let name = self.expect_ident()?;
        if self.consume(&TokenKind::Eq) {
            let value = self.parse_expr()?;
            return Ok(EffectStmt::Assign { field: name, value });
        }
        if name.as_ref() == "cue_troupe" {
            self.expect(TokenKind::LParen)?;
            let n = self.expect_string()?;
            self.expect(TokenKind::RParen)?;
            return Ok(EffectStmt::CueTroupe(n));
        }
        // Generic call: `<name>(<args>)`.  Arguments are optional for actions
        // referenced bare (e.g. `action attack`).
        let args = if self.consume(&TokenKind::LParen) {
            let mut a = ThinVec::new();
            while !self.check_kind(&TokenKind::RParen) {
                a.push(self.parse_expr()?);
                self.consume(&TokenKind::Comma);
            }
            self.expect(TokenKind::RParen)?;
            a
        } else {
            ThinVec::new()
        };
        Ok(EffectStmt::Call(name, args))
    }

    // ── Expressions ───────────────────────────────────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_add()
    }

    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus  => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_mul()?;
            lhs = Expr::Bin(Box::new(lhs), op, Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star  => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_unary()?;
            lhs = Expr::Bin(Box::new(lhs), op, Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.consume(&TokenKind::Minus) {
            return Ok(Expr::Neg(Box::new(self.parse_primary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let tok = self.advance();
        match tok.kind {
            TokenKind::IntLit(n)    => Ok(Expr::Int(n)),
            TokenKind::FloatLit(f)  => Ok(Expr::Float(f)),
            TokenKind::BoolLit(b)   => Ok(Expr::Bool(b)),
            TokenKind::StringLit(s) => Ok(Expr::Str(s)),
            TokenKind::LParen => {
                let e = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                Ok(e)
            }
            TokenKind::Ident(name) => {
                if self.consume(&TokenKind::Dot) {
                    let field = self.expect_ident()?;
                    return Ok(Expr::QualIdent { scope: name, field });
                }
                if self.consume(&TokenKind::LParen) {
                    let mut args = ThinVec::new();
                    while !self.check_kind(&TokenKind::RParen) {
                        args.push(self.parse_expr()?);
                        self.consume(&TokenKind::Comma);
                    }
                    self.expect(TokenKind::RParen)?;
                    return Ok(Expr::Call(name, args));
                }
                Ok(Expr::Ident(name))
            }
            _ => Err(ParseError {
                msg: "expected expression".into(), line: tok.line, col: tok.col,
            }),
        }
    }

    // ── Cursor helpers ────────────────────────────────────────────────────────

    fn peek(&self) -> &Token { &self.toks[self.pos] }
    fn peek_kind(&self) -> &TokenKind { &self.peek().kind }

    fn advance(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() { self.pos += 1; }
        t
    }

    fn check_kind(&self, k: &TokenKind) -> bool {
        same_kind(self.peek_kind(), k)
    }

    fn consume(&mut self, k: &TokenKind) -> bool {
        if self.check_kind(k) { self.advance(); true } else { false }
    }

    fn expect(&mut self, k: TokenKind) -> Result<Token, ParseError> {
        if self.check_kind(&k) { Ok(self.advance()) }
        else {
            let t = self.peek();
            Err(ParseError {
                msg: format!("expected {:?}, got {:?}", k, t.kind),
                line: t.line, col: t.col,
            })
        }
    }

    fn expect_ident(&mut self) -> Result<Arc<str>, ParseError> {
        let t = self.advance();
        match t.kind {
            TokenKind::Ident(s) => Ok(s),
            _ => Err(ParseError {
                msg: format!("expected identifier, got {:?}", t.kind),
                line: t.line, col: t.col,
            }),
        }
    }

    fn expect_string(&mut self) -> Result<Arc<str>, ParseError> {
        let t = self.advance();
        match t.kind {
            TokenKind::StringLit(s) => Ok(s),
            _ => Err(ParseError {
                msg: format!("expected string literal, got {:?}", t.kind),
                line: t.line, col: t.col,
            }),
        }
    }

    fn expect_uint(&mut self) -> Result<u64, ParseError> {
        let t = self.advance();
        match t.kind {
            TokenKind::IntLit(n) if n >= 0 => Ok(n as u64),
            TokenKind::HexLit(h) => Ok(h),
            _ => Err(ParseError {
                msg: "expected non-negative integer".into(),
                line: t.line, col: t.col,
            }),
        }
    }

    fn expect_float(&mut self) -> Result<f64, ParseError> {
        let t = self.advance();
        match t.kind {
            TokenKind::FloatLit(f) => Ok(f),
            TokenKind::IntLit(n)   => Ok(n as f64),
            _ => Err(ParseError {
                msg: "expected float".into(), line: t.line, col: t.col,
            }),
        }
    }

    fn err(&self, msg: &str) -> ParseError {
        let t = self.peek();
        ParseError { msg: msg.into(), line: t.line, col: t.col }
    }
}

fn same_kind(a: &TokenKind, b: &TokenKind) -> bool {
    core::mem::discriminant(a) == core::mem::discriminant(b)
}

#[derive(Debug)]
pub struct ParseError {
    pub msg:  String,
    pub line: u32,
    pub col:  u32,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.msg)
    }
}
