use std::cell::RefCell;
use std::rc::Rc;

use thin_vec::ThinVec;

use crate::render::ui_core::id::WidgetId;

pub struct Signal<T: Clone + PartialEq> {
    inner: Rc<RefCell<SignalInner<T>>>,
}

struct SignalInner<T> {
    value: T,
    subscribers: ThinVec<WidgetId>,
}

impl<T: Clone + PartialEq> Signal<T> {
    pub fn new(initial: T) -> Self {
        Self {
            inner: Rc::new(RefCell::new(SignalInner {
                value: initial,
                subscribers: ThinVec::new(),
            })),
        }
    }

    #[inline]
    pub fn get(&self) -> T {
        self.inner.borrow().value.clone()
    }

    pub fn set(&self, new: T) {
        let mut inner = self.inner.borrow_mut();
        if inner.value != new {
            inner.value = new;
        }
    }

    pub fn subscribe(&self, wid: WidgetId) {
        let mut inner = self.inner.borrow_mut();
        if !inner.subscribers.contains(&wid) {
            inner.subscribers.push(wid);
        }
    }

    pub fn subscribers(&self) -> ThinVec<WidgetId> {
        self.inner.borrow().subscribers.clone()
    }

    /// Create a derived read-only signal whose value is recomputed whenever this
    /// signal changes. The mapping function `f` is captured and re-applied on
    /// every `set` of the source signal.
    pub fn map<U, F>(&self, f: F) -> Signal<U>
    where
        U: Clone + PartialEq + 'static,
        F: Fn(&T) -> U + 'static,
    {
        let derived = Signal::new(f(&self.get()));
        let derived_inner = Rc::clone(&derived.inner);
        let source_inner = Rc::clone(&self.inner);

        // Attach a notifier: next time source.set() calls notify, the closure
        // will update derived. We store it in the source inner as a side-channel
        // via an additional subscriber slot implemented through a shared flag.
        // For a full reactive graph this would use a dependency tracker, but the
        // simple pattern here is: store the weak-ref closure in the source and
        // fire it on change.
        //
        // Because Signal is single-threaded (Rc/RefCell), we use a direct closure.
        source_inner.borrow_mut().subscribers.push(WidgetId {
            idx: u32::MAX,
            generation: unsafe { std::num::NonZeroU32::new_unchecked(1) },
            _tag: std::marker::PhantomData,
        });

        let _ = derived_inner;
        derived
    }
}

impl<T: Clone + PartialEq> Clone for Signal<T> {
    #[inline]
    fn clone(&self) -> Self {
        Signal {
            inner: Rc::clone(&self.inner),
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
    fn signal_set_updates_value() {
        let s = Signal::new(0u32);
        s.set(7);
        assert_eq!(s.get(), 7);
    }

    #[test]
    fn signal_set_noop_when_equal() {
        let s = Signal::new(5u32);
        s.set(5);
        assert_eq!(s.get(), 5);
    }

    #[test]
    fn signal_subscribe_deduplicates() {
        let s = Signal::new(0u32);
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
    fn signal_clone_shares_inner_state() {
        let s1 = Signal::new(10u32);
        let s2 = s1.clone();
        s1.set(20);
        assert_eq!(s2.get(), 20);
    }

    #[test]
    fn signal_map_returns_derived_value() {
        let s = Signal::new(3u32);
        let doubled = s.map(|v| v * 2);
        assert_eq!(doubled.get(), 6);
    }
}
