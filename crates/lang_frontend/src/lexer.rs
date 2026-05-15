//! Handwritten lexer. Produces a `ThinVec<Token>`.

use std::sync::Arc;
use thin_vec::ThinVec;

// ── Token ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Ident(Arc<str>),
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),
    StringLit(Arc<str>),
    HexLit(u64),

    // Keywords
    Script, State, Migrate, From,
    Scene, Behavior, Condition, Action, Transition, When,
    OnEnter, OnExit,
    Selector, Sequence, Parallel, Repeat, Inverter, Guard, Cooldown,
    True, False,
    And, Or, Not,

    // Type names
    TyI32, TyF64, TyBool, TyActorHandle, TySceneId,

    // Punctuation
    LBrace, RBrace, LParen, RParen, LBracket, RBracket,
    Comma, Colon, Semi, Dot,
    Arrow,      // ->
    FatArrow,   // =>
    Eq, EqEq, NotEq, Lt, Le, Gt, Ge,
    Plus, Minus, Star, Slash,

    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: u32,
    pub col:  u32,
}

// ── Lexer ─────────────────────────────────────────────────────────────────────

pub struct Lexer<'src> {
    src:  &'src [u8],
    pos:  usize,
    line: u32,
    col:  u32,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Lexer { src: src.as_bytes(), pos: 0, line: 1, col: 1 }
    }

    pub fn tokenise(mut self) -> Result<ThinVec<Token>, LexError> {
        let mut out = ThinVec::new();
        loop {
            let tok = self.next_token()?;
            let done = matches!(tok.kind, TokenKind::Eof);
            out.push(tok);
            if done { break; }
        }
        Ok(out)
    }

    fn peek(&self)  -> Option<u8> { self.src.get(self.pos).copied() }
    fn peek2(&self) -> Option<u8> { self.src.get(self.pos + 1).copied() }

    fn advance(&mut self) -> Option<u8> {
        let b = self.src.get(self.pos).copied()?;
        self.pos += 1;
        if b == b'\n' { self.line += 1; self.col = 1; } else { self.col += 1; }
        Some(b)
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => { self.advance(); }
                Some(b'/') if self.peek2() == Some(b'/') => {
                    while self.peek().is_some_and(|b| b != b'\n') { self.advance(); }
                }
                Some(b'/') if self.peek2() == Some(b'*') => {
                    self.advance(); self.advance();
                    loop {
                        match (self.peek(), self.peek2()) {
                            (Some(b'*'), Some(b'/')) => { self.advance(); self.advance(); break; }
                            (Some(_), _) => { self.advance(); }
                            (None, _) => break,
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_ws_and_comments();
        let line = self.line;
        let col  = self.col;
        let Some(b) = self.peek() else {
            return Ok(Token { kind: TokenKind::Eof, line, col });
        };

        let kind = match b {
            b'{' => { self.advance(); TokenKind::LBrace }
            b'}' => { self.advance(); TokenKind::RBrace }
            b'(' => { self.advance(); TokenKind::LParen }
            b')' => { self.advance(); TokenKind::RParen }
            b'[' => { self.advance(); TokenKind::LBracket }
            b']' => { self.advance(); TokenKind::RBracket }
            b',' => { self.advance(); TokenKind::Comma }
            b':' => { self.advance(); TokenKind::Colon }
            b';' => { self.advance(); TokenKind::Semi }
            b'.' => { self.advance(); TokenKind::Dot }
            b'+' => { self.advance(); TokenKind::Plus }
            b'*' => { self.advance(); TokenKind::Star }
            b'/' => { self.advance(); TokenKind::Slash }
            b'<' if self.peek2() == Some(b'=') => { self.advance(); self.advance(); TokenKind::Le }
            b'>' if self.peek2() == Some(b'=') => { self.advance(); self.advance(); TokenKind::Ge }
            b'<' => { self.advance(); TokenKind::Lt }
            b'>' => { self.advance(); TokenKind::Gt }
            b'!' if self.peek2() == Some(b'=') => { self.advance(); self.advance(); TokenKind::NotEq }
            b'=' if self.peek2() == Some(b'>') => { self.advance(); self.advance(); TokenKind::FatArrow }
            b'=' if self.peek2() == Some(b'=') => { self.advance(); self.advance(); TokenKind::EqEq }
            b'=' => { self.advance(); TokenKind::Eq }
            b'-' if self.peek2() == Some(b'>') => { self.advance(); self.advance(); TokenKind::Arrow }
            b'-' if self.peek2().is_some_and(|b| b.is_ascii_digit()) => self.lex_number()?,
            b'-' => { self.advance(); TokenKind::Minus }
            b'"' => self.lex_string()?,
            b'0' if self.peek2() == Some(b'x') || self.peek2() == Some(b'X') => self.lex_hex()?,
            b'0'..=b'9' => self.lex_number()?,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_word(),
            other => return Err(LexError {
                msg: format_unexpected(other), line, col,
            }),
        };
        Ok(Token { kind, line, col })
    }

    fn lex_string(&mut self) -> Result<TokenKind, LexError> {
        let line = self.line;
        let col  = self.col;
        self.advance(); // opening "
        let mut buf: ThinVec<u8> = ThinVec::new();
        loop {
            match self.advance() {
                Some(b'"') => break,
                Some(b'\\') => match self.advance() {
                    Some(b'n')  => buf.push(b'\n'),
                    Some(b't')  => buf.push(b'\t'),
                    Some(b'r')  => buf.push(b'\r'),
                    Some(b'"')  => buf.push(b'"'),
                    Some(b'\\') => buf.push(b'\\'),
                    Some(b'0')  => buf.push(0),
                    _ => return Err(LexError {
                        msg: Arc::from("invalid escape sequence"), line, col,
                    }),
                },
                Some(c) => buf.push(c),
                None => return Err(LexError {
                    msg: Arc::from("unterminated string"), line, col,
                }),
            }
        }
        let text = core::str::from_utf8(&buf)
            .map_err(|_| LexError { msg: Arc::from("non-utf8 string"), line, col })?;
        Ok(TokenKind::StringLit(Arc::<str>::from(text)))
    }

    fn lex_hex(&mut self) -> Result<TokenKind, LexError> {
        let line = self.line;
        let col  = self.col;
        self.advance(); // '0'
        self.advance(); // 'x' or 'X'
        let mut val: u64 = 0;
        let mut digits = 0u32;
        while let Some(b) = self.peek() {
            let d = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                b'_' => { self.advance(); continue; }
                _ => break,
            } as u64;
            val = val.wrapping_mul(16).wrapping_add(d);
            digits += 1;
            self.advance();
        }
        if digits == 0 {
            return Err(LexError { msg: "empty hex literal".into(), line, col });
        }
        Ok(TokenKind::HexLit(val))
    }

    fn lex_number(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        let line  = self.line;
        let col   = self.col;
        if self.peek() == Some(b'-') { self.advance(); }
        while self.peek().is_some_and(|b| b.is_ascii_digit() || b == b'_') { self.advance(); }
        let is_float = self.peek() == Some(b'.')
            && self.peek2().is_some_and(|b| b.is_ascii_digit());
        if is_float {
            self.advance(); // '.'
            while self.peek().is_some_and(|b| b.is_ascii_digit() || b == b'_') { self.advance(); }
        }
        let raw_slice = &self.src[start..self.pos];
        let mut cleaned: ThinVec<u8> = ThinVec::with_capacity(raw_slice.len());
        for &b in raw_slice { if b != b'_' { cleaned.push(b); } }
        let s = core::str::from_utf8(&cleaned).unwrap();
        if is_float {
            s.parse::<f64>().map(TokenKind::FloatLit).map_err(|_| LexError {
                msg: Arc::<str>::from(format!("invalid float literal `{s}`").as_str()),
                line, col,
            })
        } else {
            s.parse::<i64>().map(TokenKind::IntLit).map_err(|_| LexError {
                msg: Arc::<str>::from(format!("invalid integer literal `{s}`").as_str()),
                line, col,
            })
        }
    }

    fn lex_word(&mut self) -> TokenKind {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
            self.advance();
        }
        let w = core::str::from_utf8(&self.src[start..self.pos]).unwrap();
        keyword_or_ident(w)
    }
}

fn keyword_or_ident(w: &str) -> TokenKind {
    match w {
        "script"       => TokenKind::Script,
        "state"        => TokenKind::State,
        "migrate"      => TokenKind::Migrate,
        "from"         => TokenKind::From,
        "scene"        => TokenKind::Scene,
        "behavior"     => TokenKind::Behavior,
        "condition"    => TokenKind::Condition,
        "action"       => TokenKind::Action,
        "transition"   => TokenKind::Transition,
        "when"         => TokenKind::When,
        "on_enter"     => TokenKind::OnEnter,
        "on_exit"      => TokenKind::OnExit,
        "selector"     => TokenKind::Selector,
        "sequence"     => TokenKind::Sequence,
        "parallel"     => TokenKind::Parallel,
        "repeat"       => TokenKind::Repeat,
        "inverter"     => TokenKind::Inverter,
        "guard"        => TokenKind::Guard,
        "cooldown"     => TokenKind::Cooldown,
        "true"         => TokenKind::BoolLit(true),
        "false"        => TokenKind::BoolLit(false),
        "and"          => TokenKind::And,
        "or"           => TokenKind::Or,
        "not"          => TokenKind::Not,
        "i32"          => TokenKind::TyI32,
        "f64"          => TokenKind::TyF64,
        "bool"         => TokenKind::TyBool,
        "actor_handle" => TokenKind::TyActorHandle,
        "scene_id"     => TokenKind::TySceneId,
        other          => TokenKind::Ident(Arc::<str>::from(other)),
    }
}

fn format_unexpected(b: u8) -> Arc<str> {
    if b.is_ascii_graphic() {
        Arc::<str>::from(format!("unexpected character '{}'", b as char).as_str())
    } else {
        Arc::<str>::from(format!("unexpected byte 0x{b:02x}").as_str())
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct LexError {
    pub msg:  Arc<str>,
    pub line: u32,
    pub col:  u32,
}

impl core::fmt::Display for LexError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.msg)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(s: &str) -> ThinVec<Token> {
        Lexer::new(s).tokenise().expect("lex")
    }

    #[test]
    fn basic_tokens() {
        let t = lex("script my_thing { }");
        assert!(matches!(t[0].kind, TokenKind::Script));
        assert!(matches!(t[1].kind, TokenKind::Ident(ref s) if s.as_ref() == "my_thing"));
        assert!(matches!(t[2].kind, TokenKind::LBrace));
        assert!(matches!(t[3].kind, TokenKind::RBrace));
        assert!(matches!(t[4].kind, TokenKind::Eof));
    }

    #[test]
    fn literals_and_arrows() {
        let t = lex("42 3.14 0xff -> => == != <= >=");
        assert!(matches!(t[0].kind, TokenKind::IntLit(42)));
        assert!(matches!(t[1].kind, TokenKind::FloatLit(f) if (f - 3.14).abs() < 1e-9));
        assert!(matches!(t[2].kind, TokenKind::HexLit(0xff)));
        assert!(matches!(t[3].kind, TokenKind::Arrow));
        assert!(matches!(t[4].kind, TokenKind::FatArrow));
        assert!(matches!(t[5].kind, TokenKind::EqEq));
        assert!(matches!(t[6].kind, TokenKind::NotEq));
        assert!(matches!(t[7].kind, TokenKind::Le));
        assert!(matches!(t[8].kind, TokenKind::Ge));
    }

    #[test]
    fn string_with_escapes() {
        let t = lex(r#""hello\n\tworld""#);
        let TokenKind::StringLit(ref s) = t[0].kind else { panic!() };
        assert_eq!(s.as_ref(), "hello\n\tworld");
    }

    #[test]
    fn line_and_block_comments() {
        let t = lex("// hi\nscript /* x */ name");
        assert!(matches!(t[0].kind, TokenKind::Script));
        assert!(matches!(t[1].kind, TokenKind::Ident(ref s) if s.as_ref() == "name"));
    }
}
