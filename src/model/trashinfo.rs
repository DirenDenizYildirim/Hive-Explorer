//! The freedesktop trash spec's `.trashinfo` sidecar files.
//!
//! `g_file_trash` writes these but tells the caller nothing about where the file
//! landed, so undoing a trash means finding the entry again afterwards: match on
//! the recorded original path, and among several take the newest deletion date.
//! Reading the spec directories directly rather than `trash:///` keeps this
//! working on a machine with no gvfs, where the URI scheme is unavailable.

use std::path::{Path, PathBuf};

use crate::model::uri;

const GROUP: &str = "[Trash Info]";
const INFO_SUFFIX: &str = ".trashinfo";

/// The two fields Hive needs out of a `.trashinfo` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Where the file was before it was trashed.
    pub original: PathBuf,
    /// `YYYY-MM-DDThh:mm:ss`, local time, as written by the trashing application.
    pub deleted_at: String,
}

/// One trashed item: its sidecar, its contents, and what it used to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub info_path: PathBuf,
    pub files_path: PathBuf,
    pub info: Info,
}

/// Parse a `.trashinfo` body.
///
/// Lenient about the group header and about unknown keys: the goal is to
/// recognise entries written by any trashing implementation, not to validate.
pub fn parse(text: &str) -> Option<Info> {
    let mut original = None;
    let mut deleted_at = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.eq_ignore_ascii_case(GROUP) {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        match key.trim() {
            "Path" if original.is_none() => original = Some(uri::decode_path(value.trim())),
            "DeletionDate" if deleted_at.is_none() => deleted_at = Some(value.trim().to_owned()),
            _ => {}
        }
    }

    let original = original?;
    if original.as_os_str().is_empty() {
        return None;
    }

    Some(Info {
        original,
        deleted_at: deleted_at.unwrap_or_default(),
    })
}

/// The `files/` counterpart of an `info/` sidecar path.
pub fn files_path_for(info_path: &Path, trash_dir: &Path) -> Option<PathBuf> {
    let name = info_path.file_name()?.to_str()?;
    let stem = name.strip_suffix(INFO_SUFFIX)?;
    Some(trash_dir.join("files").join(stem))
}

/// Does this directory entry look like a sidecar?
pub fn is_info_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(INFO_SUFFIX) && name.len() > INFO_SUFFIX.len())
}

/// The entry that most recently held `original`.
///
/// Deletion dates are ISO-8601 without a zone, so they order lexically. Entries
/// with no date sort oldest, which is the safe direction: a dateless sidecar is
/// malformed and should not win over a well-formed one.
pub fn best_match<'a>(entries: &'a [Entry], original: &Path) -> Option<&'a Entry> {
    entries
        .iter()
        .filter(|entry| entry.info.original == original)
        .max_by(|a, b| {
            a.info
                .deleted_at
                .cmp(&b.info.deleted_at)
                .then_with(|| a.info_path.cmp(&b.info_path))
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    fn entry(name: &str, original: &str, deleted_at: &str) -> Entry {
        let trash = Path::new("/home/diren/.local/share/Trash");
        Entry {
            info_path: trash.join("info").join(format!("{name}{INFO_SUFFIX}")),
            files_path: trash.join("files").join(name),
            info: Info {
                original: PathBuf::from(original),
                deleted_at: deleted_at.to_owned(),
            },
        }
    }

    #[test]
    fn a_well_formed_sidecar_parses() {
        let text = "[Trash Info]\nPath=/home/diren/notes.txt\nDeletionDate=2026-07-25T14:33:01\n";
        assert_eq!(
            parse(text),
            Some(Info {
                original: PathBuf::from("/home/diren/notes.txt"),
                deleted_at: "2026-07-25T14:33:01".to_owned(),
            })
        );
    }

    #[test]
    fn the_path_field_is_percent_decoded() {
        let text = "[Trash Info]\nPath=/home/diren/a%20b%23c\nDeletionDate=2026-01-01T00:00:00\n";
        assert_eq!(
            parse(text).unwrap().original,
            PathBuf::from("/home/diren/a b#c")
        );
    }

    #[test]
    fn invalid_utf8_original_paths_survive() {
        let text = "[Trash Info]\nPath=/tmp/bad%FFname\nDeletionDate=2026-01-01T00:00:00\n";
        let expected = PathBuf::from(OsString::from_vec(b"/tmp/bad\xffname".to_vec()));
        assert_eq!(parse(text).unwrap().original, expected);
    }

    #[test]
    fn a_newline_in_the_original_name_does_not_split_the_entry() {
        let text = "[Trash Info]\nPath=/tmp/two%0Alines\nDeletionDate=2026-01-01T00:00:00\n";
        assert_eq!(
            parse(text).unwrap().original,
            PathBuf::from("/tmp/two\nlines")
        );
    }

    #[test]
    fn a_missing_group_header_is_tolerated() {
        let text = "Path=/tmp/a\nDeletionDate=2026-01-01T00:00:00\n";
        assert_eq!(parse(text).unwrap().original, PathBuf::from("/tmp/a"));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let text = "[Trash Info]\nSize=1234\nPath=/tmp/a\nDeletionDate=2026-01-01T00:00:00\n";
        assert!(parse(text).is_some());
    }

    #[test]
    fn a_sidecar_with_no_path_is_not_an_entry() {
        assert_eq!(
            parse("[Trash Info]\nDeletionDate=2026-01-01T00:00:00\n"),
            None
        );
        assert_eq!(parse(""), None);
        assert_eq!(parse("nonsense"), None);
    }

    #[test]
    fn a_sidecar_with_no_date_still_names_its_original() {
        let info = parse("[Trash Info]\nPath=/tmp/a\n").unwrap();
        assert_eq!(info.original, PathBuf::from("/tmp/a"));
        assert!(info.deleted_at.is_empty());
    }

    #[test]
    fn the_first_value_wins_when_a_key_repeats() {
        let text = "[Trash Info]\nPath=/tmp/first\nPath=/tmp/second\n";
        assert_eq!(parse(text).unwrap().original, PathBuf::from("/tmp/first"));
    }

    #[test]
    fn the_files_entry_is_the_sidecar_name_without_its_suffix() {
        let trash = Path::new("/home/diren/.local/share/Trash");
        let info = trash.join("info").join("notes.txt.trashinfo");
        assert_eq!(
            files_path_for(&info, trash),
            Some(trash.join("files").join("notes.txt"))
        );
    }

    #[test]
    fn a_file_without_the_suffix_is_not_a_sidecar() {
        assert!(is_info_file(Path::new("/t/info/a.trashinfo")));
        assert!(!is_info_file(Path::new("/t/info/a.txt")));
        assert!(!is_info_file(Path::new("/t/info/.trashinfo")));
        assert_eq!(
            files_path_for(Path::new("/t/info/a.txt"), Path::new("/t")),
            None
        );
    }

    #[test]
    fn the_newest_deletion_of_a_path_wins() {
        let entries = vec![
            entry("notes.txt", "/home/diren/notes.txt", "2026-01-01T09:00:00"),
            entry(
                "notes.txt.2",
                "/home/diren/notes.txt",
                "2026-07-25T14:33:01",
            ),
            entry(
                "notes.txt.3",
                "/home/diren/notes.txt",
                "2026-03-14T00:00:00",
            ),
        ];
        let best = best_match(&entries, Path::new("/home/diren/notes.txt")).unwrap();
        assert_eq!(best.info.deleted_at, "2026-07-25T14:33:01");
        assert!(best.files_path.ends_with("files/notes.txt.2"));
    }

    #[test]
    fn a_different_original_path_never_matches() {
        let entries = vec![entry("a", "/home/diren/a", "2026-01-01T00:00:00")];
        assert!(best_match(&entries, Path::new("/home/diren/b")).is_none());
        assert!(best_match(&[], Path::new("/home/diren/a")).is_none());
    }

    #[test]
    fn a_dateless_entry_loses_to_a_dated_one() {
        let entries = vec![
            entry("a", "/tmp/a", ""),
            entry("a.2", "/tmp/a", "2020-01-01T00:00:00"),
        ];
        let best = best_match(&entries, Path::new("/tmp/a")).unwrap();
        assert_eq!(best.info.deleted_at, "2020-01-01T00:00:00");
    }

    #[test]
    fn identical_dates_resolve_deterministically() {
        let entries = vec![
            entry("b", "/tmp/a", "2026-01-01T00:00:00"),
            entry("a", "/tmp/a", "2026-01-01T00:00:00"),
        ];
        let first = best_match(&entries, Path::new("/tmp/a")).unwrap();
        let second = best_match(&entries, Path::new("/tmp/a")).unwrap();
        assert_eq!(first, second);
    }
}
