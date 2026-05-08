pub mod scene;
pub mod script;
pub mod play;

pub use scene::*;
pub use script::*;
pub use play::*;

// ── EffectArena ──────────────────────────────────────────────────────────────
//
// Newtype over Vec<Effect> that formalises the reuse pattern: the backing
// allocation is preserved across ticks (via `std::mem::take` + `clear`) so
// the per-tick effect accumulation path stays allocation-free in steady state.
// `as_vec_mut` exposes the inner Vec so lower-level functions whose signatures
// predate the arena type keep compiling without change.

#[derive(Default)]
pub struct EffectArena {
    buf: Vec<Effect>,
}

impl EffectArena {
    pub fn with_capacity(cap: usize) -> Self { Self { buf: Vec::with_capacity(cap) } }
    #[inline] pub fn push(&mut self, e: Effect)                      { self.buf.push(e); }
    #[inline] pub fn drain(&mut self) -> std::vec::Drain<'_, Effect> { self.buf.drain(..) }
    #[inline] pub fn clear(&mut self)                                { self.buf.clear(); }
    #[inline] pub fn as_vec_mut(&mut self) -> &mut Vec<Effect>       { &mut self.buf }
    #[inline] pub fn len(&self) -> usize                             { self.buf.len() }
    #[inline] pub fn is_empty(&self) -> bool                         { self.buf.is_empty() }
}

