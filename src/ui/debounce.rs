//! Coalescing timer for view-derived UI.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// Fires `action` at most once per `window`, on the trailing edge.
#[derive(Clone)]
pub struct Debouncer {
    pending: Rc<Cell<bool>>,
    window: Duration,
    action: Rc<dyn Fn()>,
}

impl Debouncer {
    pub fn new(window_ms: u32, action: impl Fn() + 'static) -> Self {
        Self {
            pending: Rc::new(Cell::new(false)),
            window: Duration::from_millis(u64::from(window_ms)),
            action: Rc::new(action),
        }
    }

    /// Request a run. Repeated calls inside the window collapse into one.
    pub fn trigger(&self) {
        if self.pending.replace(true) {
            return;
        }

        let pending = Rc::clone(&self.pending);
        let action = Rc::clone(&self.action);
        glib::timeout_add_local_once(self.window, move || {
            pending.set(false);
            action();
        });
    }

    /// Run immediately, cancelling any pending trailing run.
    pub fn flush(&self) {
        self.pending.set(false);
        (self.action)();
    }
}
