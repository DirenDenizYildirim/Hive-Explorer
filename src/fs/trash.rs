//! Trashing and restoring, per the freedesktop trash spec.
//!
//! `g_file_trash` implements the spec itself, so this works with no gvfs — but
//! it reports nothing about where the file landed, and undo must not guess by
//! name. So after trashing, Hive reads the spec's own `info/` directories and
//! matches on the recorded original path, newest deletion date first.
//!
//! Reading those directories rather than enumerating `trash:///` is deliberate:
//! the URI scheme is a gvfs backend and simply does not exist on a machine
//! without it, whereas the directories are always there.

use std::path::{Path, PathBuf};

use gio::prelude::*;

use crate::model::trashinfo::{self, Entry};
use crate::model::undo::Trashed;

/// Why a file could not be trashed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrashError {
    /// No trash is reachable for this file — the classic case is a FAT or
    /// exFAT removable drive, where the per-mount `.Trash-$uid` cannot be
    /// created. §10.1 hazard 4: offer permanent delete, never silently do
    /// nothing and never silently delete instead.
    #[error("this location has no trash")]
    NotSupported,
    #[error("{0}")]
    Failed(String),
}

/// Move `path` to the trash and record where it went, for undo.
pub fn trash(path: &Path, cancellable: &gio::Cancellable) -> Result<Trashed, TrashError> {
    // The candidate directories depend on which mount the file is on, so they
    // have to be worked out while the file is still there.
    let candidates = trash_directories(path);

    gio::File::for_path(path)
        .trash(Some(cancellable))
        .map_err(|error| {
            if error.matches(gio::IOErrorEnum::NotSupported) {
                TrashError::NotSupported
            } else {
                TrashError::Failed(error.message().to_owned())
            }
        })?;

    let located = locate(path, &candidates);
    if located.is_none() {
        tracing::warn!(path = %path.display(), "trashed but no matching trashinfo found");
    }

    Ok(Trashed {
        original: path.to_path_buf(),
        trashed: located.as_ref().map(|entry| entry.files_path.clone()),
        info: located.map(|entry| entry.info_path),
    })
}

/// Put a trashed file back, then drop its sidecar.
pub fn restore(item: &Trashed) -> Result<(), String> {
    let Some(trashed) = &item.trashed else {
        return Err(format!(
            "Hive did not record where “{}” went",
            crate::model::path::display_name(&item.original)
        ));
    };

    if let Some(parent) = item.original.parent()
        && !parent.exists()
    {
        return Err(format!("{} no longer exists", parent.display()));
    }

    std::fs::rename(trashed, &item.original).map_err(|error| {
        format!(
            "could not restore {}: {error}",
            crate::model::path::display_name(&item.original)
        )
    })?;

    // The sidecar is bookkeeping; a stale one is untidy, not dangerous.
    if let Some(info) = &item.info
        && let Err(error) = std::fs::remove_file(info)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %info.display(), "could not remove trashinfo sidecar");
    }

    Ok(())
}

/// The trash directories that could hold something trashed from `path`.
///
/// The home trash first, then the per-mount directories the spec defines for
/// files that live on another filesystem.
pub fn trash_directories(path: &Path) -> Vec<PathBuf> {
    let mut directories = vec![glib::user_data_dir().join("Trash")];

    if let Some(top) = mount_point_of(path) {
        let uid = current_uid();
        directories.push(top.join(".Trash").join(uid.to_string()));
        directories.push(top.join(format!(".Trash-{uid}")));
    }

    directories.dedup();
    directories
}

/// Walk up until the device number changes: that boundary is the mount point.
fn mount_point_of(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;

    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };

    let device = std::fs::metadata(&current).ok()?.dev();

    while let Some(parent) = current.parent() {
        match std::fs::metadata(parent) {
            Ok(metadata) if metadata.dev() == device => current = parent.to_path_buf(),
            _ => break,
        }
    }

    Some(current)
}

/// This process's user id, read from the filesystem to avoid a libc dependency.
fn current_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;

    std::fs::metadata("/proc/self")
        .or_else(|_| std::fs::metadata(glib::home_dir()))
        .map(|metadata| metadata.uid())
        .unwrap_or(0)
}

/// Find the trash entry that most recently held `original`.
fn locate(original: &Path, candidates: &[PathBuf]) -> Option<Entry> {
    let mut entries = Vec::new();
    for directory in candidates {
        collect(directory, &mut entries);
    }
    trashinfo::best_match(&entries, original).cloned()
}

/// Read one trash directory's sidecars.
fn collect(trash_dir: &Path, entries: &mut Vec<Entry>) {
    let Ok(listing) = std::fs::read_dir(trash_dir.join("info")) else {
        return;
    };

    for info_path in listing.flatten().map(|entry| entry.path()) {
        if !trashinfo::is_info_file(&info_path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&info_path) else {
            continue;
        };
        let Some(info) = trashinfo::parse(&text) else {
            continue;
        };
        let Some(files_path) = trashinfo::files_path_for(&info_path, trash_dir) else {
            continue;
        };
        entries.push(Entry {
            info_path,
            files_path,
            info,
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Build a trash directory holding one entry, the way g_file_trash would.
    fn make_trash(root: &Path, name: &str, original: &str, deleted_at: &str) {
        let info = root.join("info");
        let files = root.join("files");
        std::fs::create_dir_all(&info).unwrap();
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(
            info.join(format!("{name}.trashinfo")),
            format!("[Trash Info]\nPath={original}\nDeletionDate={deleted_at}\n"),
        )
        .unwrap();
        std::fs::write(files.join(name), b"contents").unwrap();
    }

    #[test]
    fn a_sidecar_on_disk_is_matched_back_to_its_original() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        make_trash(
            &trash,
            "notes.txt",
            "/home/diren/notes.txt",
            "2026-07-25T10:00:00",
        );

        let found = locate(
            Path::new("/home/diren/notes.txt"),
            std::slice::from_ref(&trash),
        )
        .unwrap();
        assert_eq!(found.files_path, trash.join("files").join("notes.txt"));
        assert_eq!(
            found.info_path,
            trash.join("info").join("notes.txt.trashinfo")
        );
    }

    #[test]
    fn the_most_recent_of_several_deletions_wins() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        make_trash(&trash, "a", "/home/diren/a", "2020-01-01T00:00:00");
        make_trash(&trash, "a.2", "/home/diren/a", "2026-07-25T10:00:00");

        let found = locate(Path::new("/home/diren/a"), std::slice::from_ref(&trash)).unwrap();
        assert_eq!(found.files_path, trash.join("files").join("a.2"));
    }

    #[test]
    fn several_trash_directories_are_searched() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home/Trash");
        let mount_trash = dir.path().join("mnt/.Trash-1000");
        make_trash(&home_trash, "a", "/home/diren/a", "2026-01-01T00:00:00");
        make_trash(&mount_trash, "b", "/mnt/usb/b", "2026-01-01T00:00:00");

        let candidates = vec![home_trash, mount_trash.clone()];
        let found = locate(Path::new("/mnt/usb/b"), &candidates).unwrap();
        assert_eq!(found.files_path, mount_trash.join("files").join("b"));
    }

    #[test]
    fn a_path_that_was_never_trashed_is_not_matched() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        make_trash(&trash, "a", "/home/diren/a", "2026-01-01T00:00:00");

        assert!(locate(Path::new("/home/diren/b"), &[trash]).is_none());
    }

    #[test]
    fn a_missing_trash_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(locate(Path::new("/home/diren/a"), &[dir.path().join("nope")]).is_none());
    }

    #[test]
    fn unreadable_and_malformed_sidecars_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        make_trash(&trash, "good", "/home/diren/good", "2026-01-01T00:00:00");

        std::fs::write(trash.join("info/broken.trashinfo"), b"\xff\xfe not text").unwrap();
        std::fs::write(trash.join("info/empty.trashinfo"), "").unwrap();
        std::fs::write(trash.join("info/notasidecar.txt"), "Path=/x\n").unwrap();

        let found = locate(Path::new("/home/diren/good"), &[trash]).unwrap();
        assert!(found.files_path.ends_with("files/good"));
    }

    #[test]
    fn names_needing_escapes_round_trip_through_the_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        make_trash(
            &trash,
            "odd name",
            "/home/diren/odd%20name%0Aline",
            "2026-01-01T00:00:00",
        );

        let found = locate(Path::new("/home/diren/odd name\nline"), &[trash]).unwrap();
        assert!(found.files_path.ends_with("files/odd name"));
    }

    #[test]
    fn restoring_puts_the_file_back_and_removes_the_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        let original = dir.path().join("home").join("notes.txt");
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        make_trash(&trash, "notes.txt", "/ignored", "2026-01-01T00:00:00");

        let item = Trashed {
            original: original.clone(),
            trashed: Some(trash.join("files/notes.txt")),
            info: Some(trash.join("info/notes.txt.trashinfo")),
        };

        restore(&item).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"contents");
        assert!(!trash.join("files/notes.txt").exists());
        assert!(!trash.join("info/notes.txt.trashinfo").exists());
    }

    #[test]
    fn restoring_into_a_vanished_folder_reports_rather_than_creating_it() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        make_trash(&trash, "a", "/ignored", "2026-01-01T00:00:00");

        let item = Trashed {
            original: dir.path().join("gone").join("a"),
            trashed: Some(trash.join("files/a")),
            info: None,
        };

        assert!(restore(&item).is_err());
        assert!(trash.join("files/a").exists(), "nothing was moved");
    }

    #[test]
    fn restoring_something_never_tracked_is_refused() {
        let item = Trashed {
            original: PathBuf::from("/tmp/a"),
            trashed: None,
            info: None,
        };
        assert!(restore(&item).is_err());
    }

    #[test]
    fn the_home_trash_is_always_a_candidate() {
        let directories = trash_directories(Path::new("/tmp"));
        assert!(directories.contains(&glib::user_data_dir().join("Trash")));
    }

    #[test]
    fn the_mount_point_of_a_temp_file_is_a_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();

        let mount = mount_point_of(&file).unwrap();
        assert!(mount.is_dir());
        assert!(file.starts_with(&mount));
    }

    #[test]
    fn the_uid_is_a_real_account_not_a_placeholder() {
        let uid = current_uid();
        let home_uid = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(glib::home_dir()).unwrap().uid()
        };
        assert_eq!(uid, home_uid);
    }
}
