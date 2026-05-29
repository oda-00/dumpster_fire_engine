//! Experiment 5 — Reactive read path: Rc<RefCell> Signal vs epoch counter.
//!
//! Mirrors `Signal<T>::get` (src/render/ui_core/signal.rs), which does
//! `self.inner.borrow().value.clone()` — a runtime borrow-flag check + clone
//! on every read. Compose/Slint instead compare a u64 version/epoch.
//!
//! Build: rustc -O --emit asm --crate-type=lib exp5_reactivity.rs

use std::cell::RefCell;
use std::rc::Rc;

pub struct Signal<T> { inner: Rc<RefCell<T>> }

#[no_mangle]
pub fn signal_get_f32(s: &Signal<f32>) -> f32 {
    s.inner.borrow().clone() // borrow-flag branch + panic path + clone
}

// Epoch reactivity: a dirty check is a single u64 compare, no borrow flag.
pub struct Versioned<T> { value: T, version: u64 }

#[no_mangle]
pub fn epoch_changed(v: &Versioned<f32>, last_seen: u64) -> bool {
    v.version != last_seen
}

#[no_mangle]
pub fn epoch_read(v: &Versioned<f32>) -> f32 {
    v.value // plain field load, no indirection, no borrow flag
}

// Realistic: scan widgets, recompute only the changed ones. Epoch lets the
// whole scan stay branch-predictable and the load stay a direct field access.
#[no_mangle]
pub fn scan_dirty_epoch(versions: &[u64], last: &[u64]) -> u32 {
    let mut dirty = 0u32;
    for (v, l) in versions.iter().zip(last) {
        dirty += (v != l) as u32;
    }
    dirty
}
