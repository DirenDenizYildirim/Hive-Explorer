//! Back/forward navigation history.

/// How many locations to remember. Old entries fall off the front.
pub const CAPACITY: usize = 100;

/// A bounded back/forward stack.
///
/// Generic over the location type so it tests with plain strings; the UI stores
/// URIs, which cover both `file://` paths and locations like `trash://` that
/// have no path at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History<T> {
    entries: Vec<T>,
    cursor: usize,
    capacity: usize,
}

impl<T: Clone + PartialEq> History<T> {
    pub fn new(initial: T) -> Self {
        Self::with_capacity(initial, CAPACITY)
    }

    /// A history with no locations yet.
    ///
    /// The window is built before it knows where it will open, so seeding it
    /// with a placeholder would make Back look available and then step onto a
    /// location that was never visited.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            capacity: CAPACITY,
        }
    }

    pub fn with_capacity(initial: T, capacity: usize) -> Self {
        Self {
            entries: vec![initial],
            cursor: 0,
            capacity: capacity.max(1),
        }
    }

    /// Record a new location.
    ///
    /// Navigating somewhere new after going back discards the forward entries,
    /// which is what every browser and file manager does. Re-entering the
    /// current location is a no-op rather than a duplicate entry, so holding a
    /// refresh key cannot bury the real history.
    pub fn push(&mut self, location: T) {
        if self.current() == Some(&location) {
            return;
        }

        if self.entries.is_empty() {
            self.entries.push(location);
            self.cursor = 0;
            return;
        }

        self.entries.truncate(self.cursor + 1);
        self.entries.push(location);

        if self.entries.len() > self.capacity {
            let excess = self.entries.len() - self.capacity;
            self.entries.drain(..excess);
        }

        self.cursor = self.entries.len() - 1;
    }

    /// Step back, returning the location now current.
    pub fn back(&mut self) -> Option<&T> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.entries.get(self.cursor)
    }

    /// Step forward, returning the location now current.
    pub fn forward(&mut self) -> Option<&T> {
        if self.cursor + 1 >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        self.entries.get(self.cursor)
    }

    pub fn current(&self) -> Option<&T> {
        self.entries.get(self.cursor)
    }

    pub fn can_go_back(&self) -> bool {
        !self.entries.is_empty() && self.cursor > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn history() -> History<&'static str> {
        History::new("/home")
    }

    #[test]
    fn a_fresh_history_cannot_move() {
        let mut h = history();
        assert_eq!(h.current(), Some(&"/home"));
        assert!(!h.can_go_back());
        assert!(!h.can_go_forward());
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), None);
        assert_eq!(h.current(), Some(&"/home"), "failed moves must not shift");
    }

    #[test]
    fn back_and_forward_walk_the_trail() {
        let mut h = history();
        h.push("/home/a");
        h.push("/home/a/b");

        assert_eq!(h.back(), Some(&"/home/a"));
        assert_eq!(h.back(), Some(&"/home"));
        assert_eq!(h.back(), None);
        assert_eq!(h.current(), Some(&"/home"));

        assert_eq!(h.forward(), Some(&"/home/a"));
        assert_eq!(h.forward(), Some(&"/home/a/b"));
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn navigating_after_going_back_discards_the_forward_trail() {
        let mut h = history();
        h.push("/home/a");
        h.push("/home/b");
        h.back();

        assert!(h.can_go_forward());
        h.push("/home/c");

        assert!(!h.can_go_forward(), "forward trail must be discarded");
        assert_eq!(h.current(), Some(&"/home/c"));
        assert_eq!(h.back(), Some(&"/home/a"));
        assert_eq!(h.back(), Some(&"/home"));
    }

    #[test]
    fn re_entering_the_current_location_is_a_no_op() {
        // A directory monitor or a repeated click must not bury real history
        // under a run of identical entries.
        let mut h = history();
        h.push("/home/a");
        for _ in 0..10 {
            h.push("/home/a");
        }
        assert_eq!(h.len(), 2);
        assert_eq!(h.back(), Some(&"/home"));
    }

    #[test]
    fn revisiting_a_location_that_is_not_current_still_records_it() {
        let mut h = history();
        h.push("/home/a");
        h.push("/home");
        assert_eq!(h.len(), 3);
        assert_eq!(h.back(), Some(&"/home/a"));
    }

    #[test]
    fn capacity_drops_the_oldest_entries() {
        let mut h = History::with_capacity("0", 3);
        for entry in ["1", "2", "3", "4"] {
            h.push(entry);
        }
        assert_eq!(h.len(), 3);
        assert_eq!(h.current(), Some(&"4"));
        assert_eq!(h.back(), Some(&"3"));
        assert_eq!(h.back(), Some(&"2"));
        assert_eq!(h.back(), None, "older entries have fallen off");
    }

    #[test]
    fn capacity_of_zero_is_clamped_to_one() {
        let mut h = History::with_capacity("only", 0);
        h.push("next");
        assert_eq!(h.current(), Some(&"next"));
        assert!(!h.can_go_back());
    }

    #[test]
    fn cursor_stays_valid_after_eviction() {
        // Eviction shifts every index; the cursor must follow or back/forward
        // would silently land on the wrong location.
        let mut h = History::with_capacity("0", 3);
        h.push("1");
        h.push("2");
        h.back();
        assert_eq!(h.current(), Some(&"1"));
        h.push("3");
        assert_eq!(h.current(), Some(&"3"));
        assert_eq!(h.back(), Some(&"1"));
        assert_eq!(h.back(), Some(&"0"));
    }

    #[test]
    fn an_empty_history_has_nowhere_to_go() {
        let mut h: History<&str> = History::empty();
        assert_eq!(h.current(), None);
        assert!(!h.can_go_back());
        assert!(!h.can_go_forward());
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn the_first_push_into_an_empty_history_is_not_a_step() {
        // Opening the first directory must not make Back available; there is
        // nothing behind it.
        let mut h: History<&str> = History::empty();
        h.push("/home");
        assert_eq!(h.current(), Some(&"/home"));
        assert!(!h.can_go_back());
        assert_eq!(h.len(), 1);

        h.push("/home/a");
        assert!(h.can_go_back());
        assert_eq!(h.back(), Some(&"/home"));
    }

    #[test]
    fn works_with_owned_uri_strings() {
        let mut h = History::new("file:///home/diren".to_owned());
        h.push("trash:///".to_owned());
        assert_eq!(h.current().map(String::as_str), Some("trash:///"));
        assert_eq!(h.back().map(String::as_str), Some("file:///home/diren"));
    }
}
