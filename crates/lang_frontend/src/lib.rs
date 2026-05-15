//! `.lang` frontend: lexer, parser, semantic analysis, HIR lowering.
//!
//! Strict invariants:
//! * No `HashMap`, no `String` field in any AST/HIR type, no `std::Vec` field.
//! * `ThinVec<T>` for collections, `Arc<str>` for owned strings.
//! * Symbol tables and lookup maps are `ThinVec<(Arc<str>, T)>` kept sorted by
//!   key; resolution uses binary search.

pub mod ast;
pub mod hir;
pub mod lexer;
pub mod parser;
pub mod sema;

pub use ast::{LangScript, SceneDef, StateBlock, StateField, Migration, Ty};
pub use hir::{HirScript, HirScene, HirBtNode, HirCondition, HirEffect, HirField, HirMigration};
pub use lexer::{Lexer, LexError, Token, TokenKind};
pub use parser::{Parser, ParseError};
pub use sema::{lower, SemaError, fnv1a};
