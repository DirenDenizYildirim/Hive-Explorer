//! Tab completion for the `Ctrl+L` path entry.
//!
//! The directory listing is injected, so the rules are tested without touching
//! the filesystem and without a display.

use std::path::{Path, PathBuf};

/// One entry a directory listing can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub is_dir: bool,
}

impl Candidate {
    pub fn dir(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            is_dir: true,
        }
    }

    pub fn file(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            is_dir: false,
        }
    }
}

/// The result of pressing Tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The text the entry should now contain.
    pub text: String,
    /// Everything that matched, for a hint popup. Empty when nothing matched.
    pub matches: Vec<String>,
    /// True when `text` names exactly one directory, so it ends in `/` and the
    /// user can keep typing straight into it.
    pub is_unique_dir: bool,
}

/// Split `input` into the directory to list and the prefix to match within it.
///
/// A trailing separator means "list this directory", not "match a sibling".
pub fn split(input: &str, home: &Path) -> (PathBuf, String) {
    let expanded = expand(input, home);

    if expanded.ends_with('/') {
        return (directory(&expanded), String::new());
    }

    match expanded.rfind('/') {
        Some(0) => (PathBuf::from("/"), expanded[1..].to_owned()),
        Some(index) => (
            directory(&expanded[..index]),
            expanded[index + 1..].to_owned(),
        ),
        // No separator at all: complete against the working directory the
        // caller passes in as `home`.
        None => (home.to_path_buf(), expanded),
    }
}

/// A directory path without a trailing separator, so callers always receive the
/// same spelling for the same directory. The root keeps its slash.
fn directory(text: &str) -> PathBuf {
    let trimmed = text.trim_end_matches('/');
    if trimmed.is_empty() {
        PathBuf::from("/")
    } else {
        PathBuf::from(trimmed)
    }
}

/// Expand a leading `~` and collapse nothing else — the user is mid-type, so
/// normalizing `..` here would fight what they are writing.
pub fn expand(input: &str, home: &Path) -> String {
    let home = home.to_string_lossy();
    if input == "~" {
        return home.into_owned();
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return format!("{}/{rest}", home.trim_end_matches('/'));
    }
    input.to_owned()
}

/// Complete `input` against a directory listing.
///
/// `list` receives the directory to enumerate and returns its entries; it is
/// only ever called for one directory, so the caller can keep it cheap.
pub fn complete(input: &str, home: &Path, list: impl Fn(&Path) -> Vec<Candidate>) -> Completion {
    let (dir, prefix) = split(input, home);

    let mut matches: Vec<Candidate> = list(&dir)
        .into_iter()
        .filter(|candidate| candidate.name.starts_with(&prefix))
        .collect();

    // Hidden entries only appear once the user has typed the dot, so completion
    // does not flood the popup with dotfiles on an empty prefix.
    if !prefix.starts_with('.') {
        matches.retain(|candidate| !candidate.name.starts_with('.'));
    }

    matches.sort_by(|a, b| a.name.cmp(&b.name));

    if matches.is_empty() {
        return Completion {
            text: input.to_owned(),
            matches: Vec::new(),
            is_unique_dir: false,
        };
    }

    let names: Vec<String> = matches.iter().map(|c| c.name.clone()).collect();
    let shared = longest_common_prefix(&names);

    let base = dir.to_string_lossy();
    let separator = if base.ends_with('/') { "" } else { "/" };

    // A single match completes fully; several complete as far as they agree,
    // which is what makes repeated Tab feel like a shell.
    let unique_dir = matches.len() == 1 && matches[0].is_dir;
    let suffix = if unique_dir { "/" } else { "" };

    Completion {
        text: format!("{base}{separator}{shared}{suffix}"),
        matches: names,
        is_unique_dir: unique_dir,
    }
}

/// The longest prefix shared by every string, by character.
pub fn longest_common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };
    if values.len() == 1 {
        return first.clone();
    }

    let mut end = first.len();
    for other in &values[1..] {
        let shared = first
            .char_indices()
            .zip(other.chars())
            .take_while(|((_, a), b)| a == b)
            .map(|((index, a), _)| index + a.len_utf8())
            .last()
            .unwrap_or(0);
        end = end.min(shared);
    }

    first[..end].to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const HOME: &str = "/home/diren";

    fn home() -> &'static Path {
        Path::new(HOME)
    }

    fn listing(dir: &Path) -> Vec<Candidate> {
        match dir.to_string_lossy().as_ref() {
            "/home/diren" => vec![
                Candidate::dir("Downloads"),
                Candidate::dir("Documents"),
                Candidate::dir("Desktop"),
                Candidate::file("notes.txt"),
                Candidate::dir(".config"),
                Candidate::file(".bashrc"),
            ],
            "/home/diren/Downloads" => {
                vec![Candidate::file("report.pdf"), Candidate::dir("archive")]
            }
            "/" => vec![Candidate::dir("usr"), Candidate::dir("var")],
            _ => Vec::new(),
        }
    }

    fn complete_str(input: &str) -> Completion {
        complete(input, home(), listing)
    }

    #[test]
    fn splits_directory_from_prefix() {
        assert_eq!(
            split("/home/diren/Down", home()),
            (PathBuf::from("/home/diren"), "Down".to_owned())
        );
        assert_eq!(
            split("/home/diren/", home()),
            (PathBuf::from("/home/diren"), String::new())
        );
        assert_eq!(split("/us", home()), (PathBuf::from("/"), "us".to_owned()));
        // The root keeps its slash rather than collapsing to nothing.
        assert_eq!(split("/", home()), (PathBuf::from("/"), String::new()));
        // Repeated separators name the same directory.
        assert_eq!(
            split("/home/diren//", home()),
            (PathBuf::from("/home/diren"), String::new())
        );
    }

    #[test]
    fn tilde_expands_to_home() {
        assert_eq!(expand("~", home()), HOME);
        assert_eq!(expand("~/Downloads", home()), "/home/diren/Downloads");
        assert_eq!(expand("/etc", home()), "/etc");
        assert_eq!(expand("~root/x", home()), "~root/x");
    }

    #[test]
    fn a_unique_match_completes_fully_and_appends_a_separator() {
        let result = complete_str("/home/diren/Dow");
        assert_eq!(result.text, "/home/diren/Downloads/");
        assert_eq!(result.matches, vec!["Downloads"]);
        assert!(result.is_unique_dir);
    }

    #[test]
    fn a_unique_file_match_does_not_get_a_separator() {
        let result = complete_str("/home/diren/not");
        assert_eq!(result.text, "/home/diren/notes.txt");
        assert!(!result.is_unique_dir);
    }

    #[test]
    fn several_matches_complete_to_their_common_prefix() {
        // "Downloads", "Documents", "Desktop" agree only on "D".
        let result = complete_str("/home/diren/D");
        assert_eq!(result.text, "/home/diren/D");
        assert_eq!(result.matches, vec!["Desktop", "Documents", "Downloads"]);
        assert!(!result.is_unique_dir);

        // "Documents" and "Downloads" agree on "Do".
        let result = complete_str("/home/diren/Do");
        assert_eq!(result.text, "/home/diren/Do");
        assert_eq!(result.matches, vec!["Documents", "Downloads"]);
    }

    #[test]
    fn a_trailing_separator_lists_the_directory() {
        let result = complete_str("/home/diren/Downloads/");
        assert_eq!(result.matches, vec!["archive", "report.pdf"]);
        // They share nothing, so the text is unchanged.
        assert_eq!(result.text, "/home/diren/Downloads/");
    }

    #[test]
    fn no_match_leaves_the_input_untouched() {
        let result = complete_str("/home/diren/zzz");
        assert_eq!(result.text, "/home/diren/zzz");
        assert!(result.matches.is_empty());
        assert!(!result.is_unique_dir);
    }

    #[test]
    fn an_unknown_directory_is_not_an_error() {
        let result = complete_str("/nowhere/at/all");
        assert_eq!(result.text, "/nowhere/at/all");
        assert!(result.matches.is_empty());
    }

    #[test]
    fn hidden_entries_appear_only_once_the_dot_is_typed() {
        // Empty prefix must not flood the popup with dotfiles.
        let result = complete_str("/home/diren/");
        assert!(!result.matches.iter().any(|m| m.starts_with('.')));

        let result = complete_str("/home/diren/.");
        assert_eq!(result.matches, vec![".bashrc", ".config"]);
    }

    #[test]
    fn completes_at_the_filesystem_root() {
        let result = complete_str("/u");
        assert_eq!(result.text, "/usr/");
        assert!(result.is_unique_dir);
    }

    #[test]
    fn tilde_paths_complete_to_absolute_text() {
        let result = complete_str("~/Dow");
        assert_eq!(result.text, "/home/diren/Downloads/");
    }

    #[test]
    fn repeated_completion_is_stable() {
        // Tab twice on a unique match must not append a second separator or
        // start completing inside the wrong directory.
        let once = complete_str("/home/diren/Dow");
        let twice = complete(&once.text, home(), listing);
        assert_eq!(twice.text, "/home/diren/Downloads/");
    }

    #[test]
    fn common_prefix_handles_edges() {
        assert_eq!(longest_common_prefix(&[]), "");
        assert_eq!(longest_common_prefix(&["solo".to_owned()]), "solo");
        assert_eq!(
            longest_common_prefix(&["abc".to_owned(), "abd".to_owned()]),
            "ab"
        );
        assert_eq!(
            longest_common_prefix(&["abc".to_owned(), "xyz".to_owned()]),
            ""
        );
        assert_eq!(
            longest_common_prefix(&["abc".to_owned(), "abc".to_owned()]),
            "abc"
        );
        assert_eq!(
            longest_common_prefix(&["a".to_owned(), "abc".to_owned()]),
            "a"
        );
    }

    #[test]
    fn common_prefix_does_not_split_multibyte_characters() {
        // Slicing at a byte index inside a multi-byte character would panic.
        let values = vec!["日本語".to_owned(), "日本".to_owned()];
        assert_eq!(longest_common_prefix(&values), "日本");

        let values = vec!["éa".to_owned(), "éb".to_owned()];
        assert_eq!(longest_common_prefix(&values), "é");

        let values = vec!["🐝x".to_owned(), "🐝y".to_owned()];
        assert_eq!(longest_common_prefix(&values), "🐝");
    }

    #[test]
    fn names_with_spaces_complete_normally() {
        let result = complete("/space/my", Path::new("/space"), |_| {
            vec![Candidate::dir("my documents")]
        });
        assert_eq!(result.text, "/space/my documents/");
    }
}
