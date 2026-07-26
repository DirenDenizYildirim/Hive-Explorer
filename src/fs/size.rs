//! Recursive directory size: the walk behind the Properties dialog's
//! **Calculate** button.
//!
//! §10.1 hazard 1 says this is the classic file-manager freeze, so it is never
//! started on its own: a Properties dialog opening computes nothing. When the
//! user does ask, the walk runs on its own thread, reports a running total as
//! it goes, and stops the moment it is cancelled.
//!
//! Like the transfer engine next door in [`crate::fs::ops`], the walk uses
//! `std::fs` metadata rather than gio. It is the same job — stat every entry
//! under a tree, on a worker thread — and having the two disagree about how
//! they read a directory would be worse than either choice.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// How often the walker is allowed to push an update. Matches the transfer
/// engine, and is well under the eye's threshold for "it is still going".
const REPORT_INTERVAL: Duration = Duration::from_millis(80);

/// Entries between cancellation checks and time checks.
///
/// Checking the clock on every entry costs more than the stat does on a warm
/// cache; a directory of small files would spend its time in `Instant::now`.
const CHECK_EVERY: u64 = 256;

/// What the walk has found so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub bytes: u64,
    pub files: u64,
    pub directories: u64,
    /// Entries that could not be read: permission denied, or vanished mid-walk.
    pub unreadable: u64,
}

impl Tally {
    /// Files plus directories, not counting the roots' own container.
    pub fn items(&self) -> u64 {
        self.files + self.directories
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A running total. Sent at most every [`REPORT_INTERVAL`].
    Progress(Tally),
    /// The walk stopped, either because it ran out of tree or was cancelled.
    Finished { tally: Tally, cancelled: bool },
}

/// Start walking `roots`. The receiver must be drained from the main loop.
///
/// Cancellation is a `gio::Cancellable` so the caller can wire it to the same
/// dialog machinery as everything else; the walk itself only ever asks whether
/// it has been cancelled.
pub fn spawn(roots: Vec<PathBuf>, cancellable: gio::Cancellable) -> Receiver<Event> {
    let (events, receiver) = std::sync::mpsc::channel();
    let fallback = events.clone();

    if let Err(error) = std::thread::Builder::new()
        .name("hive-size".to_owned())
        .spawn(move || run(&roots, &cancellable, &events))
    {
        // Out of threads. Report it the way a finished walk reports, so the
        // dialog has one path to handle rather than two.
        tracing::error!(%error, "could not start the size walk");
        let _ = fallback.send(Event::Finished {
            tally: Tally::default(),
            cancelled: true,
        });
    }

    receiver
}

fn run(roots: &[PathBuf], cancellable: &gio::Cancellable, events: &Sender<Event>) {
    use gio::prelude::CancellableExt;

    let mut last_report = Instant::now();
    let mut report = |tally: Tally| {
        if last_report.elapsed() >= REPORT_INTERVAL {
            last_report = Instant::now();
            let _ = events.send(Event::Progress(tally));
        }
    };

    let tally = walk(roots, &|| cancellable.is_cancelled(), &mut report);
    let _ = events.send(Event::Finished {
        tally,
        cancelled: cancellable.is_cancelled(),
    });
}

/// Walk `roots`, calling `report` with the running total as it goes.
///
/// An explicit stack rather than recursion: a pathologically deep tree would
/// otherwise decide how much stack Hive needs, and a walk that overflows takes
/// the whole process with it.
///
/// Symlinks are counted but never followed — the same rule `du` uses. That is
/// what makes a symlink loop a non-event here rather than something needing a
/// depth guard, and it stops a link to `/` from reporting the size of the disk.
/// Hard links are counted once per name, so a tree of links to one large file
/// reads larger than the space it occupies.
pub fn walk(
    roots: &[PathBuf],
    cancelled: &dyn Fn() -> bool,
    report: &mut dyn FnMut(Tally),
) -> Tally {
    let mut tally = Tally::default();
    let mut pending: Vec<PathBuf> = Vec::new();
    let mut seen: u64 = 0;

    for root in roots {
        match std::fs::symlink_metadata(root) {
            Ok(metadata) if metadata.is_dir() => {
                tally.directories += 1;
                pending.push(root.clone());
            }
            Ok(metadata) => {
                tally.files += 1;
                tally.bytes += metadata.len();
            }
            Err(_) => tally.unreadable += 1,
        }
    }

    while let Some(directory) = pending.pop() {
        if cancelled() {
            break;
        }

        let listing = match std::fs::read_dir(&directory) {
            Ok(listing) => listing,
            Err(error) => {
                tracing::debug!(path = %directory.display(), %error, "unreadable during size walk");
                tally.unreadable += 1;
                continue;
            }
        };

        for entry in listing {
            let Ok(entry) = entry else {
                tally.unreadable += 1;
                continue;
            };

            match entry.metadata() {
                // `DirEntry::metadata` does not follow symlinks, so a link is
                // counted at its own size and its target is left alone.
                Ok(metadata) if metadata.is_dir() => {
                    tally.directories += 1;
                    pending.push(entry.path());
                }
                Ok(metadata) => {
                    tally.files += 1;
                    tally.bytes += metadata.len();
                }
                Err(_) => tally.unreadable += 1,
            }

            seen += 1;
            if seen.is_multiple_of(CHECK_EVERY) {
                if cancelled() {
                    return tally;
                }
                report(tally);
            }
        }
    }

    tally
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::Path;

    use super::*;

    fn never() -> bool {
        false
    }

    fn total(root: &Path) -> Tally {
        walk(&[root.to_path_buf()], &never, &mut |_| {})
    }

    #[test]
    fn an_empty_directory_is_itself_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let tally = total(dir.path());
        assert_eq!(tally.bytes, 0);
        assert_eq!(tally.files, 0);
        assert_eq!(tally.directories, 1, "the root counts as one directory");
        assert_eq!(tally.unreadable, 0);
    }

    #[test]
    fn nested_files_are_summed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"0123456789").unwrap();
        std::fs::create_dir(dir.path().join("inner")).unwrap();
        std::fs::write(dir.path().join("inner/b"), b"012345").unwrap();
        std::fs::create_dir(dir.path().join("inner/deeper")).unwrap();
        std::fs::write(dir.path().join("inner/deeper/c"), b"01").unwrap();

        let tally = total(dir.path());
        assert_eq!(tally.bytes, 18);
        assert_eq!(tally.files, 3);
        assert_eq!(tally.directories, 3);
        assert_eq!(tally.items(), 6);
    }

    #[test]
    fn a_zero_byte_file_still_counts_as_an_item() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("empty"), b"").unwrap();
        let tally = total(dir.path());
        assert_eq!(tally.bytes, 0);
        assert_eq!(tally.files, 1);
    }

    #[test]
    fn several_roots_are_added_together() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"1234").unwrap();
        std::fs::write(dir.path().join("b"), b"567").unwrap();

        let tally = walk(
            &[dir.path().join("a"), dir.path().join("b")],
            &never,
            &mut |_| {},
        );
        assert_eq!(tally.bytes, 7);
        assert_eq!(tally.files, 2);
        assert_eq!(tally.directories, 0, "neither root is a directory");
    }

    #[test]
    fn a_symlink_loop_terminates_because_links_are_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        std::os::unix::fs::symlink(dir.path(), inner.join("loop")).unwrap();

        let tally = total(dir.path());
        assert_eq!(tally.directories, 2, "the loop is not walked into");
        assert_eq!(tally.files, 1, "the link itself is one entry");
    }

    #[test]
    fn a_broken_symlink_is_counted_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join("dangling"))
            .unwrap();

        let tally = total(dir.path());
        assert_eq!(tally.files, 1);
        assert_eq!(tally.unreadable, 0, "a dangling link is not a failure");
    }

    #[test]
    fn a_missing_root_is_reported_rather_than_panicking() {
        let tally = walk(
            &[PathBuf::from("/definitely/not/here/at/all")],
            &never,
            &mut |_| {},
        );
        assert_eq!(tally.unreadable, 1);
        assert_eq!(tally.bytes, 0);
    }

    #[test]
    fn an_unreadable_directory_is_counted_and_the_walk_continues() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readable"), b"1234").unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::write(locked.join("hidden"), b"12345678").unwrap();

        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let tally = total(dir.path());

        // Restore before the assertions, so a failure still cleans up.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        // Running as root ignores the permission bits entirely, so accept either
        // outcome rather than failing the suite depending on who runs it.
        assert!(tally.bytes >= 4, "the readable file is still counted");
        assert!(tally.unreadable <= 1);
    }

    #[test]
    fn cancelling_stops_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..64 {
            std::fs::write(dir.path().join(format!("f{index}")), b"1234").unwrap();
        }

        let tally = walk(&[dir.path().to_path_buf()], &|| true, &mut |_| {});
        assert_eq!(tally.files, 0, "cancelled before reading the first listing");
        assert_eq!(tally.directories, 1, "the root was already stat'ed");
    }

    #[test]
    fn progress_is_reported_for_a_large_directory() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..(CHECK_EVERY * 2 + 8) {
            std::fs::write(dir.path().join(format!("f{index}")), b"1").unwrap();
        }

        let mut reports = Vec::new();
        let tally = walk(&[dir.path().to_path_buf()], &never, &mut |running| {
            reports.push(running)
        });

        assert!(reports.len() >= 2, "got {} reports", reports.len());
        assert!(
            reports
                .windows(2)
                .all(|pair| pair[0].files <= pair[1].files),
            "the running total must never go backwards"
        );
        assert_eq!(tally.files, CHECK_EVERY * 2 + 8);
    }

    #[test]
    fn a_file_root_needs_no_directory_walk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("solo");
        std::fs::write(&file, b"12345").unwrap();

        let tally = walk(&[file], &never, &mut |_| {});
        assert_eq!(tally.bytes, 5);
        assert_eq!(tally.files, 1);
        assert_eq!(tally.directories, 0);
    }
}
