//! Experiment 2 — Per-frame bump arena (Clay-style) vs general allocator.
//!
//! Clay resets a single bump pointer at Clay_BeginLayout(); the engine's
//! immediate builder instead does `path.to_string()` + `Signal::new(text.to_owned())`
//! (heap alloc) per widget per frame. This isolates the allocation codegen.
//!
//! Build: rustc -O --emit asm --crate-type=lib exp2_arena.rs

// ---- bump arena: reset is a single store; alloc is a pointer add ---------
pub struct Bump {
    base: *mut u8,
    head: usize,
    cap: usize,
}

impl Bump {
    #[inline]
    pub fn reset(&mut self) {
        self.head = 0; // O(1) — no per-object drop, no free list
    }

    #[inline]
    pub fn alloc(&mut self, bytes: usize, align: usize) -> *mut u8 {
        let aligned = (self.head + align - 1) & !(align - 1);
        let new_head = aligned + bytes;
        if new_head > self.cap {
            return core::ptr::null_mut();
        }
        self.head = new_head;
        unsafe { self.base.add(aligned) }
    }
}

#[no_mangle]
pub fn bump_alloc16(b: &mut Bump) -> *mut u8 {
    b.alloc(16, 8)
}

#[no_mangle]
pub fn bump_reset(b: &mut Bump) {
    b.reset()
}

// ---- general allocator path: what the IM builder does today --------------
// `String::from` -> __rust_alloc + memcpy + length bookkeeping every call.
#[no_mangle]
pub fn alloc_string(src: &str) -> String {
    src.to_owned()
}

// Allocating N transient strings (one per widget label) per frame.
#[no_mangle]
pub fn alloc_many_strings(labels: &[&str]) -> usize {
    let mut total = 0;
    for s in labels {
        let owned = s.to_owned(); // heap alloc per widget per frame
        total += owned.len();
        core::hint::black_box(&owned);
    }
    total
}
