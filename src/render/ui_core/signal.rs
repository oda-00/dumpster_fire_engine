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

    pub fn get(&self) -> T {
        self.inner.borrow().value.clone()
    }

    pub fn set(&self, new: T) {
        let mut inner = self.inner.borrow_mut();
        if inner.value != new {
            inner.value = new.clone();
        }
    }

    pub fn subscribe(&self, wid: WidgetId) {
        self.inner.borrow_mut().subscribers.push(wid);
    }

    pub fn map<U: Clone + PartialEq, F: Fn(&T) -> U + 'static>(
        &self,
        f: F,
    ) -> Signal<U> {
        let derived = Signal::new(f(&self.get()));
        let source_inner = Rc::clone(&self.inner);
        let derived_inner = Rc::clone(&derived.inner);
        derived
    }
}

impl<T: Clone + PartialEq> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Signal {
            inner: Rc::clone(&self.inner),
        }
    }
}
