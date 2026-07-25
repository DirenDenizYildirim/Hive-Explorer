//! Filename generation and validation for copy conflicts and renames.

use crate::model::path::split_extension;

/// Longest name most Linux filesystems accept, in bytes.
pub const MAX_NAME_BYTES: usize = 255;

/// Why a name typed into a rename or new-folder dialog cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    /// `.` and `..` are directory entries that already exist everywhere.
    Reserved,
    /// `/` is the only byte a filename may never contain.
    Separator,
    TooLong,
}

impl NameError {
    pub const fn message(self) -> &'static str {
        match self {
            NameError::Empty => "Name cannot be empty",
            NameError::Reserved => "“.” and “..” are reserved",
            NameError::Separator => "Name cannot contain “/”",
            NameError::TooLong => "Name is too long",
        }
    }
}

/// Check a user-typed name before it reaches the filesystem.
pub fn validate(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameError::Reserved);
    }
    if name.contains('/') {
        return Err(NameError::Separator);
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(NameError::TooLong);
    }
    Ok(())
}

/// True when only letter case differs.
///
/// On a case-insensitive filesystem `rename("foo", "Foo")` either fails or, on
/// some implementations, unlinks the file. Callers must route this through a
/// temporary name instead. See §10.1 hazard 6.
pub fn is_case_only_rename(from: &str, to: &str) -> bool {
    from != to && from.to_lowercase() == to.to_lowercase()
}

/// A temporary name for the middle of a two-step case-only rename.
///
/// Kept inside the same directory so the rename stays atomic and never crosses
/// a filesystem boundary.
pub fn case_rename_staging(name: &str, taken: impl Fn(&str) -> bool) -> String {
    for attempt in 0..1000u32 {
        let candidate = format!(".hive-rename-{name}-{attempt}");
        if !taken(&candidate) && validate(&candidate).is_ok() {
            return candidate;
        }
    }
    format!(".hive-rename-{name}-overflow")
}

/// Strip a `(copy)` / `(copy N)` suffix so duplicates do not stack.
fn strip_copy_suffix(stem: &str) -> &str {
    let Some(open) = stem.rfind(" (copy") else {
        return stem;
    };
    let Some(inner) = stem[open..].strip_suffix(')') else {
        return stem;
    };
    let rest = &inner[" (copy".len()..];

    if rest.is_empty() || rest.strip_prefix(' ').is_some_and(is_positive_number) {
        &stem[..open]
    } else {
        stem
    }
}

fn is_positive_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// `photo.png` at attempt 1 -> `photo (copy).png`, at 2 -> `photo (copy 2).png`.
pub fn copy_name(name: &str, attempt: u32) -> String {
    let (stem, extension) = split_extension(name);
    let base = strip_copy_suffix(stem);

    let suffix = if attempt <= 1 {
        " (copy)".to_owned()
    } else {
        format!(" (copy {attempt})")
    };

    match extension {
        Some(extension) => format!("{base}{suffix}.{extension}"),
        None => format!("{base}{suffix}"),
    }
}

/// The first name in the `(copy N)` series that `taken` reports as free.
///
/// Returns `name` unchanged when nothing is in the way, so it is safe to call
/// on every paste rather than only on a conflict.
pub fn next_available(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }

    let mut candidate = String::new();
    for attempt in 1..=u32::MAX {
        candidate = copy_name(name, attempt);
        if !taken(&candidate) {
            return candidate;
        }
    }
    candidate
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn taken_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn a_free_name_is_returned_unchanged() {
        let taken = taken_set(&[]);
        assert_eq!(next_available("a.txt", |n| taken.contains(n)), "a.txt");
    }

    #[test]
    fn the_first_conflict_becomes_a_copy() {
        let taken = taken_set(&["a.txt"]);
        assert_eq!(
            next_available("a.txt", |n| taken.contains(n)),
            "a (copy).txt"
        );
    }

    #[test]
    fn further_conflicts_count_up() {
        let taken = taken_set(&["a.txt", "a (copy).txt", "a (copy 2).txt"]);
        assert_eq!(
            next_available("a.txt", |n| taken.contains(n)),
            "a (copy 3).txt"
        );
    }

    #[test]
    fn duplicating_a_duplicate_does_not_stack_suffixes() {
        let taken = taken_set(&["a (copy).txt"]);
        assert_eq!(
            next_available("a (copy).txt", |n| taken.contains(n)),
            "a (copy 2).txt"
        );

        let taken = taken_set(&["a (copy 7).txt", "a (copy).txt"]);
        assert_eq!(
            next_available("a (copy 7).txt", |n| taken.contains(n)),
            "a (copy 2).txt"
        );
    }

    #[test]
    fn a_name_that_merely_looks_like_a_copy_suffix_is_left_alone() {
        assert_eq!(
            copy_name("notes (copyright).txt", 1),
            "notes (copyright) (copy).txt"
        );
        assert_eq!(copy_name("a (copy x).txt", 1), "a (copy x) (copy).txt");
        assert_eq!(copy_name("(copy).txt", 1), "(copy) (copy).txt");
    }

    #[test]
    fn extensions_are_preserved_and_dotfiles_stay_whole() {
        assert_eq!(copy_name("photo.png", 1), "photo (copy).png");
        assert_eq!(copy_name("README", 2), "README (copy 2)");
        assert_eq!(copy_name(".bashrc", 1), ".bashrc (copy)");
        assert_eq!(copy_name("archive.tar.gz", 1), "archive.tar (copy).gz");
    }

    #[test]
    fn folders_without_extensions_still_get_a_suffix() {
        let taken = taken_set(&["Documents"]);
        assert_eq!(
            next_available("Documents", |n| taken.contains(n)),
            "Documents (copy)"
        );
    }

    #[test]
    fn names_with_newlines_and_invalid_looking_text_are_handled() {
        assert_eq!(copy_name("two\nlines.txt", 1), "two\nlines (copy).txt");
    }

    #[test]
    fn case_only_renames_are_detected() {
        assert!(is_case_only_rename("foo", "Foo"));
        assert!(is_case_only_rename("README.md", "readme.md"));
        assert!(!is_case_only_rename("foo", "foo"));
        assert!(!is_case_only_rename("foo", "bar"));
        assert!(!is_case_only_rename("foo", "Foo2"));
    }

    #[test]
    fn the_staging_name_avoids_names_already_present() {
        let taken = taken_set(&[".hive-rename-foo-0"]);
        let staged = case_rename_staging("foo", |n| taken.contains(n));
        assert_eq!(staged, ".hive-rename-foo-1");
        assert!(validate(&staged).is_ok());
    }

    #[test]
    fn validation_rejects_only_what_the_filesystem_rejects() {
        assert_eq!(validate(""), Err(NameError::Empty));
        assert_eq!(validate("."), Err(NameError::Reserved));
        assert_eq!(validate(".."), Err(NameError::Reserved));
        assert_eq!(validate("a/b"), Err(NameError::Separator));
        assert_eq!(validate(&"x".repeat(256)), Err(NameError::TooLong));

        assert!(validate("...").is_ok());
        assert!(validate(" leading space").is_ok());
        assert!(validate("trailing space ").is_ok());
        assert!(validate("with\nnewline").is_ok());
        assert!(validate("Ünïcödé 😀").is_ok());
        assert!(validate(&"x".repeat(255)).is_ok());
    }

    #[test]
    fn length_is_measured_in_bytes_not_characters() {
        // ext4's limit is 255 bytes, so 128 two-byte characters is too long
        // even though it is well under 255 chars.
        let name = "é".repeat(128);
        assert_eq!(name.chars().count(), 128);
        assert_eq!(validate(&name), Err(NameError::TooLong));
    }

    #[test]
    fn every_error_has_a_message() {
        for error in [
            NameError::Empty,
            NameError::Reserved,
            NameError::Separator,
            NameError::TooLong,
        ] {
            assert!(!error.message().is_empty());
        }
    }
}
