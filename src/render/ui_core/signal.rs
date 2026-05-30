use thin_vec::ThinVec;

use crate::render::ui_core::id::WidgetId;

/// Reactive value cell with epoch-based change tracking.
///
/// Previously backed by `Rc<RefCell<_>>`, which made every `get()` a pointer
/// chase plus a runtime borrow-flag check that branches to a panic path (see
/// `docs/gui_research/asm/exp5_reactivity.rs`). This version stores the value
/// **inline** alongside a monotonic `version` counter, so a read is a single
/// load and change detection is a `u64` compare. Writers bump `version` only
/// when the value actually changes; consumers cache a `last_seen` version and
/// recompute only when it differs (the Compose/SwiftUI model).
pub struct Signal<T> {
    value: T,
    version: u64,
    subscribers: ThinVec<WidgetId>,
}

impl<T: Clone + PartialEq> Signal<T> {
    pub fn new(initial: T) -> Self {
        Self {
            value: initial,
            version: 0,
            subscribers: ThinVec::new(),
        }
    }

    /// Clone-out read. Cheap for `Copy`/small `T`; prefer [`get_ref`] to avoid
    /// the clone when a borrow suffices.
    #[inline]
    pub fn get(&self) -> T {
        self.value.clone()
    }

    /// Borrowing read — a single field load, no clone.
    #[inline]
    pub fn get_ref(&self) -> &T {
        &self.value
    }

    /// Current epoch. Bumps on every value-changing `set`.
    #[inline]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Write a new value. No-op (and no version bump) when the value is
    /// unchanged, preserving the previous change-suppression semantics.
    pub fn set(&mut self, new: T) {
        if self.value != new {
            self.value = new;
            self.version = self.version.wrapping_add(1);
        }
    }

    /// Register a widget that depends on this signal, for write-side
    /// invalidation. De-duplicates.
    pub fn subscribe(&mut self, wid: WidgetId) {
        if !self.subscribers.contains(&wid) {
            self.subscribers.push(wid);
        }
    }

    /// Widgets currently subscribed to this signal.
    #[inline]
    pub fn subscribers(&self) -> &[WidgetId] {
        &self.subscribers
    }

    /// One-shot derived value: applies `f` to the current value and returns it
    /// in a fresh signal. This is a snapshot, not a live binding — recompute by
    /// calling `map` again after the source changes (tracked via `version`).
    pub fn map<U, F>(&self, f: F) -> Signal<U>
    where
        U: Clone + PartialEq,
        F: Fn(&T) -> U,
    {
        Signal::new(f(&self.value))
    }
}

impl<T: Clone + PartialEq> Clone for Signal<T> {
    /// Value-semantics clone (independent copy). The old `Rc`-backed `Signal`
    /// shared mutable state across clones; nothing in the engine relied on that
    /// (subscribe/subscribers were test-only), and shared interior mutability is
    /// exactly the hot-path cost this rewrite removes.
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            version: self.version,
            subscribers: self.subscribers.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_get_returns_initial_value() {
        let s = Signal::new(42u32);
        assert_eq!(s.get(), 42);
    }

    #[test]
    fn signal_set_updates_value_and_bumps_version() {
        let mut s = Signal::new(0u32);
        let v0 = s.version();
        s.set(7);
        assert_eq!(s.get(), 7);
        assert_eq!(s.version(), v0 + 1);
    }

    #[test]
    fn signal_set_noop_when_equal_does_not_bump_version() {
        let mut s = Signal::new(5u32);
        let v0 = s.version();
        s.set(5);
        assert_eq!(s.get(), 5);
        assert_eq!(s.version(), v0);
    }

    #[test]
    fn signal_subscribe_deduplicates() {
        let mut s = Signal::new(0u32);
        let id = WidgetId {
            idx: 1,
            generation: std::num::NonZeroU32::new(1).unwrap(),
            _tag: std::marker::PhantomData,
        };
        s.subscribe(id);
        s.subscribe(id);
        assert_eq!(s.subscribers().len(), 1);
    }

    #[test]
    fn signal_clone_is_independent() {
        let s1 = Signal::new(10u32);
        let mut s2 = s1.clone();
        s2.set(20);
        // Value semantics: mutating the clone does not affect the original.
        assert_eq!(s1.get(), 10);
        assert_eq!(s2.get(), 20);
    }

    #[test]
    fn signal_map_returns_derived_value() {
        let s = Signal::new(3u32);
        let doubled = s.map(|v| v * 2);
        assert_eq!(doubled.get(), 6);
    }

    #[test]
    fn signal_get_ref_avoids_clone() {
        let s = Signal::new(String::from("hi"));
        assert_eq!(s.get_ref(), "hi");
    }
}
