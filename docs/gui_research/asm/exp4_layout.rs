//! Experiment 4 — Flex distribution loop: branchy match vs branchless.
//!
//! Models the Fill/Fixed/Hug sizing decision (the engine's `ui_manager`
//! layout has Sizing::{Fill,Fixed,Hug}; Clay/Yoga do the same grow/shrink).
//! Question: does the per-child `match` vectorize, and can the grow pass be
//! made branchless?
//!
//! Build: rustc -O --emit asm --crate-type=lib exp4_layout.rs

#[derive(Copy, Clone)]
pub enum Sizing { Fill, Fixed(f32), Hug }

// Branchy: a match per child (the natural translation).
#[no_mangle]
pub fn distribute_branchy(items: &[Sizing], hugs: &[f32], avail: f32) -> f32 {
    let mut used = 0.0f32;
    let mut fills = 0u32;
    for (i, s) in items.iter().enumerate() {
        match s {
            Sizing::Fixed(px) => used += *px,
            Sizing::Hug => used += hugs[i],
            Sizing::Fill => fills += 1,
        }
    }
    let remaining = (avail - used).max(0.0);
    if fills == 0 { remaining } else { remaining / fills as f32 }
}

// Branchless first pass: precompute fixed/hug contribution as a flat f32
// stream, count fills with a mask. This is the shape that auto-vectorizes.
#[derive(Copy, Clone, Default)]
pub struct FlatSize { pub fixed_px: f32, pub is_fill: f32 } // is_fill in {0.0,1.0}

#[no_mangle]
pub fn distribute_branchless(items: &[FlatSize], avail: f32) -> f32 {
    let mut used = 0.0f32;
    let mut fills = 0.0f32;
    for it in items {
        used += it.fixed_px;     // hug folded into fixed_px ahead of time
        fills += it.is_fill;     // sum of mask = fill count, no branch
    }
    let remaining = (avail - used).max(0.0);
    if fills == 0.0 { remaining } else { remaining / fills }
}
