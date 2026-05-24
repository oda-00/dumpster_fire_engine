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

pub use ast::{LangScript, Migration, SceneDef, StateBlock, StateField, Ty};
pub use hir::{HirBtNode, HirCondition, HirEffect, HirField, HirMigration, HirScene, HirScript};
pub use lexer::{LexError, Lexer, Token, TokenKind};
pub use parser::{ParseError, Parser};
pub use sema::{SemaError, fnv1a, lower};
