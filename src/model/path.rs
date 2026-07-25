//! Path normalization and containment checks.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// Resolve `.` and `..` lexically, without consulting the filesystem.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut pending_parents = 0usize;

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
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
pub fn resolve_against(base: &Path, input: &Path) -> PathBuf {
    if input.is_absolute() {
        normalize(input)
    } else {
        normalize(&base.join(input))
    }
}

/// Expand a leading `~` against `home`. Other users' `~name` is left alone.
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
/// Compares whole components, so `/home/di` is not an ancestor of `/home/diren`.
/// A string-prefix check would refuse legitimate copies and, worse, its inverse
/// bug would let real recursive copies through.
pub fn is_ancestor(ancestor: &Path, descendant: &Path) -> bool {
    let ancestor = normalize(ancestor);
    let descendant = normalize(descendant);
    descendant.starts_with(&ancestor)
}

/// True when copying or moving `source` into `destination` would recurse.
pub fn would_recurse(source: &Path, destination: &Path) -> bool {
    let source = normalize(source);
    let destination = normalize(destination);
    source == destination || is_ancestor(&source, &destination)
}

/// The final component, as a lossy string suitable for display.
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
pub fn canonicalize_existing(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// Split a filename into stem and extension for conflict-resolution renaming.
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
        assert!(!is_ancestor(
            Path::new("/home/di"),
            Path::new("/home/diren")
        ));
        assert!(!is_ancestor(Path::new("/home/diren"), Path::new("/home")));
        assert!(!is_ancestor(Path::new("/var"), Path::new("/home/diren")));
    }

    #[test]
    fn recursion_check_catches_self_and_subtree() {
        assert!(would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/data")
        ));
        assert!(would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/data/backup")
        ));
        assert!(would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/data/a/b/c")
        ));
        assert!(!would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren/other")
        ));
        assert!(!would_recurse(
            Path::new("/home/diren/data"),
            Path::new("/home/diren")
        ));
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
        assert_eq!(split_extension(".bashrc"), (".bashrc", None));
        assert_eq!(split_extension(".config.toml"), (".config", Some("toml")));
        assert_eq!(split_extension("trailing."), ("trailing", Some("")));
    }
}
