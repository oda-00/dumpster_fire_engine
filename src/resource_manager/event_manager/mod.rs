pub mod scene;
pub mod script;
pub mod script_abi;
pub mod script_ipc;
pub mod script_client;
pub mod play;

pub use scene::*;
pub use script::*;
pub use play::*;

use thin_vec::ThinVec;

// ── EffectArena ──────────────────────────────────────────────────────────────
//
// Newtype over ThinVec<Effect> that formalises the reuse pattern: the backing
// allocation is preserved across ticks (via `std::mem::take` + `clear`) so
// the per-tick effect accumulation path stays allocation-free in steady state.
// `as_thin_vec_mut` exposes the inner ThinVec so the per-tick `&mut ThinVec<Effect>`
// plumbing (Play::collect_effects, BtNode::tick, etc.) can borrow it directly.

#[derive(Default)]
pub struct EffectArena {
    buf: ThinVec<Effect>,
}

impl EffectArena {
    pub fn with_capacity(cap: usize) -> Self { Self { buf: ThinVec::with_capacity(cap) } }
    #[inline] pub fn push(&mut self, e: Effect)                          { self.buf.push(e); }
    #[inline] pub fn drain(&mut self) -> thin_vec::Drain<'_, Effect>     { self.buf.drain(..) }
    #[inline] pub fn clear(&mut self)                                    { self.buf.clear(); }
    #[inline] pub fn as_thin_vec_mut(&mut self) -> &mut ThinVec<Effect>  { &mut self.buf }
    #[inline] pub fn len(&self) -> usize                                 { self.buf.len() }
    #[inline] pub fn is_empty(&self) -> bool                             { self.buf.is_empty() }
}
