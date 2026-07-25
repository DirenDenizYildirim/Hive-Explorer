//! Path normalization and containment checks.
//!
//! Plain Rust, no GTK. The lexical functions here never touch the filesystem, so
//! they behave identically in tests and on a path that has since vanished.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// Resolve `.` and `..` lexically, without consulting the filesystem.
///
/// This is deliberately *not* `std::fs::canonicalize`: it does not resolve
/// symlinks and does not require the path to exist. Use it to tidy a
/// user-supplied path for display and comparison; use
/// [`canonicalize_existing`] when symlink identity actually matters.
///
/// Leading `..` components on a relative path are preserved, since there is no
/// filesystem context in which to resolve them.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut pending_parents = 0usize;

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                // Popping past the root is a no-op: `/..` is `/`.
                let popped = match out.components().next_back() {
                    Some(Component::Normal(_)) => out.pop(),
                    _ => false,
                };
                if !popped && !out.has_root() {
                    pending_parents += 1;
                }
            }
            Component::Normal(part) => out.push(part),
        }
    }

    if pending_parents > 0 {
        let mut prefixed = PathBuf::new();
        for _ in 0..pending_parents {
            prefixed.push("..");
        }
        prefixed.push(&out);
        return prefixed;
    }

    if out.as_os_str().is_empty() {
        return PathBuf::from(".");
    }

    out
}

/// Resolve `input` against `base` when relative, then normalize.
///
/// This is what `hive .` depends on: the working directory must be the
/// *invoking shell's*, captured before the path is handed to an already-running
/// instance. Resolving it in the running process would silently open whatever
/// directory that process happened to start in.
pub fn resolve_against(base: &Path, input: &Path) -> PathBuf {
    if input.is_absolute() {
        normalize(input)
    } else {
        normalize(&base.join(input))
    }
}

/// Expand a leading `~` or `~/…` against `home`. Other users' `~name` forms are
/// left alone — resolving them needs passwd lookups and is not worth the
/// surface area.
pub fn expand_tilde(input: &Path, home: &Path) -> PathBuf {
    let Some(text) = input.to_str() else {
        return input.to_path_buf();
    };

    if text == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return home.join(rest);
    }
    input.to_path_buf()
}

/// True when `ancestor` is `descendant` or contains it.
///
/// Compares whole path components, so `/home/di` is **not** treated as an
/// ancestor of `/home/diren` — a prefix-string check would get that wrong and,
/// in the copy pre-flight, would refuse legitimate operations while a subtly
/// different bug let real recursive copies through.
pub fn is_ancestor(ancestor: &Path, descendant: &Path) -> bool {
    let ancestor = normalize(ancestor);
    let descendant = normalize(descendant);
    descendant.starts_with(&ancestor)
}

/// True when copying or moving `source` into `destination` would place a
/// directory inside its own subtree, or onto itself.
///
/// This is the classic recursive data-eater, so the check is intentionally
/// conservative: it is purely lexical and does not need either path to exist.
/// The pre-flight in `fs::preflight` runs this against canonicalized paths as
/// well, to catch the symlinked-into-itself case that lexical comparison misses.
pub fn would_recurse(source: &Path, destination: &Path) -> bool {
    let source = normalize(source);
    let destination = normalize(destination);
    source == destination || is_ancestor(&source, &destination)
}

/// The final component, as a lossy string suitable for display.
///
/// Filenames are bytes on Linux and need not be valid UTF-8. Invalid sequences
/// become U+FFFD rather than being rejected, so such a file is still visible and
/// selectable instead of vanishing from the listing.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(OsStr::to_string_lossy)
        .map(|name| name.into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// The parent directory, or `None` at the filesystem root.
pub fn parent_of(path: &Path) -> Option<PathBuf> {
    let normalized = normalize(path);
    normalized.parent().map(Path::to_path_buf)
}

/// Every ancestor from the root down to `path`, for the breadcrumb bar.
///
/// The result always starts at the root for an absolute path, and each entry is
/// a real navigable path rather than a display fragment.
pub fn breadcrumb_segments(path: &Path) -> Vec<PathBuf> {
    let normalized = normalize(path);
    let mut segments: Vec<PathBuf> = normalized
        .ancestors()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    segments.reverse();
    segments
}

/// Resolve symlinks and `..` for real. Requires the path to exist.
///
/// Only used where symlink identity genuinely matters — the copy/move
/// pre-flight, and undo re-validation.
pub fn canonicalize_existing(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// Split a filename into stem and extension for conflict-resolution renaming.
///
/// Dotfiles are treated as having no extension: `.bashrc` is a stem, not an
/// extension, so a conflicting copy becomes `.bashrc (copy)` rather than
/// `. (copy)bashrc`.
pub fn split_extension(name: &str) -> (&str, Option<&str>) {
    let trimmed = name.strip_prefix('.').unwrap_or(name);
    let leading_dot = name.len() - trimmed.len();

    match trimmed.rfind('.') {
        Some(index) if index > 0 => {
            let split = leading_dot + index;
            (&name[..split], Some(&name[split + 1..]))
        }
        _ => (name, None),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn normalize_removes_dot_and_parent_components() {
        assert_eq!(
            normalize(Path::new("/home/diren/./docs")),
            PathBuf::from("/home/diren/docs")
        );
        assert_eq!(
            normalize(Path::new("/home/diren/../diren")),
            PathBuf::from("/home/diren")
        );
        assert_eq!(
            normalize(Path::new("/a/b/c/../../d")),
            PathBuf::from("/a/d")
        );
        assert_eq!(normalize(Path::new("/a//b///c")), PathBuf::from("/a/b/c"));
    }

    #[test]
    fn normalize_cannot_escape_the_root() {
        assert_eq!(normalize(Path::new("/..")), PathBuf::from("/"));
        assert_eq!(normalize(Path::new("/../../..")), PathBuf::from("/"));
        assert_eq!(normalize(Path::new("/../home")), PathBuf::from("/home"));
    }

    #[test]
    fn normalize_keeps_unresolvable_relative_parents() {
        assert_eq!(
            normalize(Path::new("../sibling")),
            PathBuf::from("../sibling")
        );
        assert_eq!(normalize(Path::new("a/../../b")), PathBuf::from("../b"));
        assert_eq!(normalize(Path::new(".")), PathBuf::from("."));
        assert_eq!(normalize(Path::new("")), PathBuf::from("."));
    }

    #[test]
    fn relative_paths_resolve_against_the_invoking_directory() {
        // This is the `hive .` case: resolving against the caller's cwd, not the
        // running instance's.
        let cwd = Path::new("/home/diren/projects/hive");
        assert_eq!(
            resolve_against(cwd, Path::new(".")),
            PathBuf::from("/home/diren/projects/hive")
        );
        assert_eq!(
            resolve_against(cwd, Path::new("..")),
            PathBuf::from("/home/diren/projects")
        );
        assert_eq!(
            resolve_against(cwd, Path::new("src")),
            PathBuf::from("/home/diren/projects/hive/src")
        );
        assert_eq!(
            resolve_against(cwd, Path::new("/etc")),
            PathBuf::from("/etc")
        );
    }

    #[test]
    fn tilde_expands_only_for_the_current_user() {
        let home = Path::new("/home/diren");
        assert_eq!(
            expand_tilde(Path::new("~"), home),
            PathBuf::from("/home/diren")
        );
        assert_eq!(
            expand_tilde(Path::new("~/Downloads"), home),
            PathBuf::from("/home/diren/Downloads")
        );
        // Another user's home needs a passwd lookup; leave it untouched.
        assert_eq!(
            expand_tilde(Path::new("~root/x"), home),
            PathBuf::from("~root/x")
        );
        assert_eq!(
            expand_tilde(Path::new("/tmp/~"), home),
            PathBuf::from("/tmp/~")
        );
    }

    #[test]
    fn ancestry_compares_components_not_string_prefixes() {
        assert!(is_ancestor(Path::new("/home"), Path::new("/home/diren")));
        assert!(is_ancestor(
            Path::new("/home/diren"),
            Path::new("/home/diren")
        ));
        // The bug this guards against: "/home/di" is a string prefix of
        // "/home/diren" but is not an ancestor of it.
        assert!(!is_ancestor(
            Path::new("/home/di"),
            Path::new("/home/diren")
        ));
        assert!(!is_ancestor(Path::new("/home/diren"), Path::new("/home")));
        assert!(!is_ancestor(Path::new("/var"), Path::new("/home/diren")));
    }

    #[test]
    fn recursion_check_catches_self_and_subtree() {
        // Copying a directory onto itself.
        assert!(would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/data")
        ));
        // Copying a directory into its own child — the data-eater.
        assert!(would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/data/backup")
        ));
        assert!(would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/data/a/b/c")
        ));
        // Unrelated destinations are fine.
        assert!(!would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/other")
        ));
        assert!(!would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren")
        ));
        // Sibling with a shared string prefix must not be refused.
        assert!(!would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/data2")
        ));
    }

    #[test]
    fn recursion_check_sees_through_dot_components() {
        assert!(would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/./data/./sub")
        ));
        assert!(would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/other/../data/sub")
        ));
    }

    #[test]
    fn display_name_survives_invalid_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let raw = OsStr::from_bytes(b"/tmp/bad\xff\xfename");
        let name = display_name(Path::new(raw));
        assert!(name.contains("bad"));
        assert!(name.contains("name"));
        assert!(!name.is_empty());
    }

    #[test]
    fn display_name_handles_newlines_in_filenames() {
        assert_eq!(display_name(Path::new("/tmp/two\nlines")), "two\nlines");
    }

    #[test]
    fn display_name_at_root() {
        assert_eq!(display_name(Path::new("/")), "/");
    }

    #[test]
    fn parent_stops_at_root() {
        assert_eq!(
            parent_of(Path::new("/home/diren")),
            Some(PathBuf::from("/home"))
        );
        assert_eq!(parent_of(Path::new("/home")), Some(PathBuf::from("/")));
        assert_eq!(parent_of(Path::new("/")), None);
    }

    #[test]
    fn breadcrumbs_run_root_first() {
        let segments = breadcrumb_segments(Path::new("/home/diren/docs"));
        assert_eq!(
            segments,
            vec![
                PathBuf::from("/"),
                PathBuf::from("/home"),
                PathBuf::from("/home/diren"),
                PathBuf::from("/home/diren/docs"),
            ]
        );
        assert_eq!(
            breadcrumb_segments(Path::new("/")),
            vec![PathBuf::from("/")]
        );
    }

    #[test]
    fn extension_split_treats_dotfiles_as_stems() {
        assert_eq!(split_extension("photo.png"), ("photo", Some("png")));
        assert_eq!(
            split_extension("archive.tar.gz"),
            ("archive.tar", Some("gz"))
        );
        assert_eq!(split_extension("README"), ("README", None));
        // A dotfile has no extension.
        assert_eq!(split_extension(".bashrc"), (".bashrc", None));
        assert_eq!(split_extension(".config.toml"), (".config", Some("toml")));
        assert_eq!(split_extension("trailing."), ("trailing", Some("")));
    }
}
