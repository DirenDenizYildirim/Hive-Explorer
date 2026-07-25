//! Visibility and search predicates.
//!
//! Plain Rust, no GTK. The `gtk::CustomFilter` shim reads a `gio::FileInfo` into
//! [`FilterInput`] and calls [`matches`].

/// The state the filter is evaluated against.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilterSpec {
    /// `Ctrl+H`.
    pub show_hidden: bool,
    /// `Ctrl+F` substring query. Empty means no filtering.
    pub query: String,
}

impl FilterSpec {
    pub fn new(show_hidden: bool, query: impl Into<String>) -> Self {
        Self {
            show_hidden,
            query: query.into(),
        }
    }

    pub fn has_query(&self) -> bool {
        !self.query.trim().is_empty()
    }
}

/// The per-entry facts the predicate needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterInput<'a> {
    pub name: &'a str,
    /// gio's `standard::is-hidden` — a leading dot, or a name listed in the
    /// directory's `.hidden` file.
    pub is_hidden: bool,
    /// gio's `standard::is-backup` — trailing `~` and similar editor leftovers.
    pub is_backup: bool,
}

impl<'a> FilterInput<'a> {
    pub fn new(name: &'a str, is_hidden: bool, is_backup: bool) -> Self {
        Self {
            name,
            is_hidden,
            is_backup,
        }
    }

    /// Derive hidden-ness from the name alone, for callers without a
    /// `gio::FileInfo`.
    pub fn from_name(name: &'a str) -> Self {
        Self {
            name,
            is_hidden: name.starts_with('.'),
            is_backup: name.ends_with('~'),
        }
    }
}

/// Whether an entry survives the current filter.
pub fn matches(entry: &FilterInput<'_>, spec: &FilterSpec) -> bool {
    if !spec.show_hidden && (entry.is_hidden || entry.is_backup) {
        return false;
    }

    if !spec.has_query() {
        return true;
    }

    contains_ignore_ascii_case(entry.name, spec.query.trim())
}

/// Case-insensitive substring test.
///
/// Deliberately allocation-free on the common path and tolerant of non-ASCII:
/// ASCII letters fold, everything else compares as-is. That is the right
/// tradeoff for type-as-you-filter, where predictability beats full Unicode
/// case folding and the predicate runs on every visible row per keystroke.
pub fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }

    let hay = haystack.as_bytes();
    let pin = needle.as_bytes();

    hay.windows(pin.len())
        .any(|window| window.eq_ignore_ascii_case(pin))
}

/// Type-ahead jump: the first entry whose name starts with `prefix`.
///
/// Returns an index into `names`, searching from `start` and wrapping, so
/// repeatedly typing the same letter cycles through matches.
pub fn type_ahead_match(names: &[&str], prefix: &str, start: usize) -> Option<usize> {
    if prefix.is_empty() || names.is_empty() {
        return None;
    }

    let count = names.len();
    (0..count)
        .map(|offset| (start + offset) % count)
        .find(|&index| starts_with_ignore_ascii_case(names[index], prefix))
}

fn starts_with_ignore_ascii_case(haystack: &str, prefix: &str) -> bool {
    haystack.len() >= prefix.len()
        && haystack.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn hidden_files_are_filtered_unless_shown() {
        let dotfile = FilterInput::from_name(".bashrc");
        let normal = FilterInput::from_name("notes.txt");

        let hiding = FilterSpec::new(false, "");
        assert!(!matches(&dotfile, &hiding));
        assert!(matches(&normal, &hiding));

        let showing = FilterSpec::new(true, "");
        assert!(matches(&dotfile, &showing));
        assert!(matches(&normal, &showing));
    }

    #[test]
    fn backup_files_follow_the_hidden_toggle() {
        let backup = FilterInput::from_name("draft.txt~");
        assert!(backup.is_backup);
        assert!(!matches(&backup, &FilterSpec::new(false, "")));
        assert!(matches(&backup, &FilterSpec::new(true, "")));
    }

    #[test]
    fn gio_hidden_flag_is_honored_even_without_a_leading_dot() {
        // A name listed in the directory's `.hidden` file has no leading dot but
        // must still hide.
        let entry = FilterInput::new("lost+found", true, false);
        assert!(!matches(&entry, &FilterSpec::new(false, "")));
        assert!(matches(&entry, &FilterSpec::new(true, "")));
    }

    #[test]
    fn query_matches_case_insensitively_anywhere_in_the_name() {
        let entry = FilterInput::from_name("Vacation Photo.JPEG");
        assert!(matches(&entry, &FilterSpec::new(false, "photo")));
        assert!(matches(&entry, &FilterSpec::new(false, "PHOTO")));
        assert!(matches(&entry, &FilterSpec::new(false, "jpeg")));
        assert!(matches(&entry, &FilterSpec::new(false, "vacation")));
        assert!(!matches(&entry, &FilterSpec::new(false, "video")));
    }

    #[test]
    fn empty_or_whitespace_query_matches_everything() {
        let entry = FilterInput::from_name("anything");
        assert!(matches(&entry, &FilterSpec::new(false, "")));
        assert!(matches(&entry, &FilterSpec::new(false, "   ")));
    }

    #[test]
    fn query_still_respects_the_hidden_toggle() {
        // Searching must not reveal hidden files while the toggle is off — the
        // filter is a conjunction, not a replacement.
        let dotfile = FilterInput::from_name(".config");
        assert!(!matches(&dotfile, &FilterSpec::new(false, "config")));
        assert!(matches(&dotfile, &FilterSpec::new(true, "config")));
    }

    #[test]
    fn substring_search_handles_edges() {
        assert!(contains_ignore_ascii_case("abc", "abc"));
        assert!(contains_ignore_ascii_case("abc", "a"));
        assert!(contains_ignore_ascii_case("abc", "c"));
        assert!(contains_ignore_ascii_case("abc", ""));
        assert!(!contains_ignore_ascii_case("ab", "abc"));
        assert!(!contains_ignore_ascii_case("", "a"));
    }

    #[test]
    fn non_ascii_names_match_exactly_and_never_panic() {
        // Byte-window comparison must never slice a multi-byte character in a
        // way that panics; eq_ignore_ascii_case on &[u8] is safe by construction.
        let entry = FilterInput::from_name("Straße-Übung-日本語.txt");

        // Non-ASCII substrings match at their own case.
        assert!(matches(&entry, &FilterSpec::new(false, "日本")));
        assert!(matches(&entry, &FilterSpec::new(false, "Straße")));
        assert!(matches(&entry, &FilterSpec::new(false, "Übung")));

        // ASCII within a non-ASCII name still folds.
        assert!(matches(&entry, &FilterSpec::new(false, "TXT")));
        assert!(matches(&entry, &FilterSpec::new(false, "bung")));

        // Documented limitation: folding is ASCII-only, so a lowercase "ü" does
        // not match an uppercase "Ü". Predictability beats full Unicode case
        // folding for a predicate that runs on every visible row per keystroke.
        assert!(!matches(&entry, &FilterSpec::new(false, "übung")));

        assert!(!matches(&entry, &FilterSpec::new(false, "zzz")));
    }

    #[test]
    fn names_with_newlines_filter_normally() {
        let entry = FilterInput::from_name("two\nlines.txt");
        assert!(matches(&entry, &FilterSpec::new(false, "lines")));
        assert!(matches(&entry, &FilterSpec::new(false, "two")));
    }

    #[test]
    fn type_ahead_finds_and_wraps() {
        let names = ["alpha", "beta", "Bravo", "gamma"];
        assert_eq!(type_ahead_match(&names, "b", 0), Some(1));
        // Starting past the first match continues to the next.
        assert_eq!(type_ahead_match(&names, "b", 2), Some(2));
        // Wraps around the end of the list.
        assert_eq!(type_ahead_match(&names, "a", 2), Some(0));
        assert_eq!(type_ahead_match(&names, "z", 0), None);
        assert_eq!(type_ahead_match(&names, "", 0), None);
        assert_eq!(type_ahead_match(&[], "a", 0), None);
    }

    #[test]
    fn type_ahead_is_case_insensitive() {
        let names = ["Documents", "downloads"];
        assert_eq!(type_ahead_match(&names, "do", 0), Some(0));
        assert_eq!(type_ahead_match(&names, "DO", 1), Some(1));
    }
}
