//! The undo stack and its re-validation rules.
//!
//! This is the highest-value safety feature in the app, so treat a bug here as a
//! data-loss bug. Two properties matter more than convenience:
//!
//! * There is no `Delete` variant. Permanent delete is not undoable and must not
//!   appear to be, so it cannot be represented, let alone pushed.
//! * An inverse that would destroy data is refused, not attempted. Undoing a
//!   copy deletes the copies; if one of them has been edited since, deleting it
//!   would discard those edits, so the whole entry is refused and dropped.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::model::path::display_name;

/// Bounded, in-session only, never persisted across restarts.
pub const CAPACITY: usize = 20;

/// Size and modification time, for noticing that a file changed after the fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stamp {
    pub size: u64,
    /// Nanoseconds since the Unix epoch; negative before it.
    pub modified: i64,
}

impl Stamp {
    pub fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            size: metadata.len(),
            modified: modified_nanos(metadata),
        }
    }
}

fn modified_nanos(metadata: &std::fs::Metadata) -> i64 {
    use std::time::UNIX_EPOCH;

    let Ok(modified) = metadata.modified() else {
        return 0;
    };

    match modified.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_nanos()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_nanos())
            .map(|nanos| -nanos)
            .unwrap_or(i64::MIN),
    }
}

/// A file Hive created, which undo would remove again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub path: PathBuf,
    /// Size and mtime as Hive left it.
    pub stamp: Stamp,
    pub is_dir: bool,
    /// Wall-clock nanoseconds at which Hive finished writing it.
    ///
    /// A directory's own stamp says nothing about what is nested inside it, and
    /// copied files keep the source's mtime, so this is the reference point for
    /// "was anything under here written after Hive put it there".
    pub completed_at: i64,
}

/// A file Hive relocated, which undo would put back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// A file Hive trashed, with where it landed so undo need not guess by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed {
    pub original: PathBuf,
    /// The entry under the trash directory's `files/`, when it could be located.
    pub trashed: Option<PathBuf>,
    /// Its `.trashinfo` sidecar, removed on restore.
    pub info: Option<PathBuf>,
}

/// Something that happened, paired with a known inverse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Trash(Vec<Trashed>),
    Rename { from: PathBuf, to: PathBuf },
    Move(Vec<Moved>),
    Copy(Vec<Created>),
    Create(Created),
}

impl Operation {
    /// True when nothing actually completed, so there is nothing to record.
    pub fn is_empty(&self) -> bool {
        match self {
            Operation::Trash(items) => items.is_empty(),
            Operation::Move(items) => items.is_empty(),
            Operation::Copy(items) => items.is_empty(),
            Operation::Rename { .. } | Operation::Create(_) => false,
        }
    }

    /// What the toast says after the inverse has been applied.
    pub fn describe_undo(&self) -> String {
        match self {
            Operation::Trash(items) => match items.as_slice() {
                [only] => format!(
                    "Undid: restored “{}” from Trash",
                    display_name(&only.original)
                ),
                many => format!("Undid: restored {} items from Trash", many.len()),
            },
            Operation::Rename { from, to } => format!(
                "Undid: renamed “{}” back to “{}”",
                display_name(to),
                display_name(from)
            ),
            Operation::Move(items) => {
                let target = items
                    .first()
                    .and_then(|item| item.from.parent().map(display_name))
                    .unwrap_or_else(|| "their original folder".to_owned());
                match items.as_slice() {
                    [only] => format!("Undid: moved “{}” back to {target}", display_name(&only.to)),
                    many => format!("Undid: moved {} items back to {target}", many.len()),
                }
            }
            Operation::Copy(items) => match items.as_slice() {
                [only] => format!("Undid: removed the copy of “{}”", display_name(&only.path)),
                many => format!("Undid: removed {} copied items", many.len()),
            },
            Operation::Create(item) => {
                format!("Undid: removed “{}”", display_name(&item.path))
            }
        }
    }
}

/// Why an inverse will not be attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NothingToUndo,
    /// What the inverse would act on is gone.
    Missing(PathBuf),
    /// Where the inverse would put it is no longer free.
    Occupied(PathBuf),
    /// It changed since the operation, so undoing would discard that change.
    Modified(PathBuf),
    /// Trashed, but the trash entry was never located, so restoring is a guess.
    Untracked(PathBuf),
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Refusal::NothingToUndo => "Nothing to undo".to_owned(),
            Refusal::Missing(path) => {
                format!("Cannot undo: “{}” is no longer there", display_name(path))
            }
            Refusal::Occupied(path) => format!(
                "Cannot undo: something else is now at “{}”",
                display_name(path)
            ),
            Refusal::Modified(path) => format!(
                "Cannot undo: “{}” has changed since, and undoing would discard that",
                display_name(path)
            ),
            Refusal::Untracked(path) => format!(
                "Cannot undo: Hive could not find “{}” in the Trash",
                display_name(path)
            ),
        }
    }
}

/// The filesystem facts validation needs, injected so the rules stay testable.
pub trait Probe {
    fn exists(&self, path: &Path) -> bool;
    fn stamp(&self, path: &Path) -> Option<Stamp>;
    /// The newest modification time anywhere under `path`, for directories.
    fn newest_within(&self, path: &Path) -> Option<i64>;
}

/// Can this inverse be applied without destroying anything?
pub fn validate(operation: &Operation, probe: &impl Probe) -> Result<(), Refusal> {
    match operation {
        Operation::Trash(items) => {
            for item in items {
                let Some(trashed) = &item.trashed else {
                    return Err(Refusal::Untracked(item.original.clone()));
                };
                if !probe.exists(trashed) {
                    return Err(Refusal::Missing(item.original.clone()));
                }
                if probe.exists(&item.original) {
                    return Err(Refusal::Occupied(item.original.clone()));
                }
            }
            Ok(())
        }

        Operation::Rename { from, to } => {
            if !probe.exists(to) {
                return Err(Refusal::Missing(to.clone()));
            }
            if probe.exists(from) {
                return Err(Refusal::Occupied(from.clone()));
            }
            Ok(())
        }

        Operation::Move(items) => {
            for item in items {
                if !probe.exists(&item.to) {
                    return Err(Refusal::Missing(item.to.clone()));
                }
                if probe.exists(&item.from) {
                    return Err(Refusal::Occupied(item.from.clone()));
                }
            }
            Ok(())
        }

        // Deleting is the inverse here, so an edit since the copy is fatal.
        Operation::Copy(items) => {
            for item in items {
                validate_deletable(item, probe)?;
            }
            Ok(())
        }

        Operation::Create(item) => validate_deletable(item, probe),
    }
}

fn validate_deletable(item: &Created, probe: &impl Probe) -> Result<(), Refusal> {
    let Some(stamp) = probe.stamp(&item.path) else {
        return Err(Refusal::Missing(item.path.clone()));
    };

    // Removing a directory removes everything under it, so the check has to
    // look under it too: a file written there after the copy is someone's work,
    // and the directory's own stamp would not notice it below the first level.
    if item.is_dir {
        let newest = probe.newest_within(&item.path).unwrap_or(i64::MAX);
        if newest > item.completed_at {
            return Err(Refusal::Modified(item.path.clone()));
        }
        return Ok(());
    }

    if stamp != item.stamp {
        return Err(Refusal::Modified(item.path.clone()));
    }
    Ok(())
}

/// A bounded stack of inverses.
#[derive(Debug, Default)]
pub struct Stack {
    entries: VecDeque<Operation>,
}

impl Stack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an operation. Empty ones — a transfer where nothing completed —
    /// are dropped rather than stored as an inverse that does nothing.
    pub fn push(&mut self, operation: Operation) {
        if operation.is_empty() {
            return;
        }
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(operation);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn peek(&self) -> Option<&Operation> {
        self.entries.back()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Take the newest entry if its inverse still applies.
    ///
    /// The entry leaves the stack either way: an inverse that failed validation
    /// will not start passing later, and leaving it there would put a refusal
    /// between the user and every older entry.
    pub fn take_next(&mut self, probe: &impl Probe) -> Result<Operation, Refusal> {
        let Some(operation) = self.entries.pop_back() else {
            return Err(Refusal::NothingToUndo);
        };
        validate(&operation, probe).map(|()| operation)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeFs {
        stamps: HashMap<PathBuf, Stamp>,
    }

    impl FakeFs {
        fn with(paths: &[(&str, u64, i64)]) -> Self {
            let mut fs = Self::default();
            for (path, size, modified) in paths {
                fs.stamps.insert(
                    PathBuf::from(path),
                    Stamp {
                        size: *size,
                        modified: *modified,
                    },
                );
            }
            fs
        }

        fn touch(&mut self, path: &str, size: u64, modified: i64) {
            self.stamps
                .insert(PathBuf::from(path), Stamp { size, modified });
        }

        fn remove(&mut self, path: &str) {
            self.stamps.remove(Path::new(path));
        }
    }

    impl Probe for FakeFs {
        fn exists(&self, path: &Path) -> bool {
            self.stamps.contains_key(path)
        }

        fn stamp(&self, path: &Path) -> Option<Stamp> {
            self.stamps.get(path).copied()
        }

        /// The newest mtime of anything at or under `path`, by path prefix.
        fn newest_within(&self, path: &Path) -> Option<i64> {
            self.stamps
                .iter()
                .filter(|(candidate, _)| candidate.starts_with(path))
                .map(|(_, stamp)| stamp.modified)
                .max()
        }
    }

    fn created(path: &str, size: u64, modified: i64) -> Created {
        Created {
            path: PathBuf::from(path),
            stamp: Stamp { size, modified },
            is_dir: false,
            completed_at: modified,
        }
    }

    fn created_dir(path: &str, completed_at: i64) -> Created {
        Created {
            path: PathBuf::from(path),
            stamp: Stamp {
                size: 4096,
                modified: completed_at,
            },
            is_dir: true,
            completed_at,
        }
    }

    #[test]
    fn a_fresh_stack_has_nothing_to_undo() {
        let mut stack = Stack::new();
        let fs = FakeFs::default();
        assert_eq!(stack.take_next(&fs), Err(Refusal::NothingToUndo));
        assert!(stack.is_empty());
    }

    #[test]
    fn the_stack_is_bounded_at_twenty_entries() {
        let mut stack = Stack::new();
        for index in 0..30 {
            stack.push(Operation::Create(created(&format!("/tmp/{index}"), 0, 0)));
        }
        assert_eq!(stack.len(), CAPACITY);

        // The oldest ten fell off the bottom; the newest is still on top.
        let Some(Operation::Create(top)) = stack.peek() else {
            panic!("expected a Create on top");
        };
        assert_eq!(top.path, PathBuf::from("/tmp/29"));
    }

    #[test]
    fn an_operation_where_nothing_completed_is_not_recorded() {
        let mut stack = Stack::new();
        stack.push(Operation::Copy(Vec::new()));
        stack.push(Operation::Move(Vec::new()));
        stack.push(Operation::Trash(Vec::new()));
        assert!(stack.is_empty(), "empty inverses are not undoable");
    }

    #[test]
    fn a_partially_completed_transfer_records_only_what_finished() {
        // The engine reports two of five copies as completed; the inverse must
        // remove exactly those two.
        let mut stack = Stack::new();
        stack.push(Operation::Copy(vec![
            created("/dst/a", 10, 1),
            created("/dst/b", 20, 2),
        ]));

        let fs = FakeFs::with(&[("/dst/a", 10, 1), ("/dst/b", 20, 2)]);
        let Ok(Operation::Copy(items)) = stack.take_next(&fs) else {
            panic!("expected the copy back");
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn undoing_a_copy_refuses_when_a_copy_has_been_edited() {
        let mut stack = Stack::new();
        stack.push(Operation::Copy(vec![created("/dst/report.txt", 100, 500)]));

        let mut fs = FakeFs::with(&[("/dst/report.txt", 100, 500)]);
        fs.touch("/dst/report.txt", 140, 900);

        assert_eq!(
            stack.take_next(&fs),
            Err(Refusal::Modified(PathBuf::from("/dst/report.txt")))
        );
    }

    #[test]
    fn a_size_preserving_edit_is_still_caught_by_the_timestamp() {
        let mut stack = Stack::new();
        stack.push(Operation::Copy(vec![created("/dst/a", 100, 500)]));

        let mut fs = FakeFs::with(&[("/dst/a", 100, 500)]);
        fs.touch("/dst/a", 100, 501);

        assert!(matches!(stack.take_next(&fs), Err(Refusal::Modified(_))));
    }

    #[test]
    fn one_edited_file_refuses_the_whole_entry() {
        let mut stack = Stack::new();
        stack.push(Operation::Copy(vec![
            created("/dst/a", 1, 1),
            created("/dst/b", 2, 2),
        ]));

        let mut fs = FakeFs::with(&[("/dst/a", 1, 1), ("/dst/b", 2, 2)]);
        fs.touch("/dst/b", 9, 9);

        assert!(matches!(stack.take_next(&fs), Err(Refusal::Modified(_))));
    }

    #[test]
    fn a_refused_entry_is_dropped_rather_than_left_to_block_the_stack() {
        let mut stack = Stack::new();
        stack.push(Operation::Create(created("/tmp/older", 0, 0)));
        stack.push(Operation::Copy(vec![created("/dst/edited", 1, 1)]));

        let mut fs = FakeFs::with(&[("/tmp/older", 0, 0), ("/dst/edited", 1, 1)]);
        fs.touch("/dst/edited", 5, 5);

        assert!(stack.take_next(&fs).is_err());
        assert_eq!(stack.len(), 1);
        assert!(stack.take_next(&fs).is_ok(), "the older entry is reachable");
    }

    #[test]
    fn undoing_a_copy_refuses_when_the_copy_is_already_gone() {
        let mut stack = Stack::new();
        stack.push(Operation::Copy(vec![created("/dst/a", 1, 1)]));

        let fs = FakeFs::default();
        assert_eq!(
            stack.take_next(&fs),
            Err(Refusal::Missing(PathBuf::from("/dst/a")))
        );
    }

    #[test]
    fn undoing_a_new_folder_refuses_once_something_is_inside_it() {
        let mut stack = Stack::new();
        stack.push(Operation::Create(created_dir("/tmp/New Folder", 100)));

        let mut fs = FakeFs::with(&[("/tmp/New Folder", 4096, 100)]);
        fs.touch("/tmp/New Folder/notes.txt", 12, 200);

        assert!(matches!(stack.take_next(&fs), Err(Refusal::Modified(_))));
    }

    #[test]
    fn undoing_an_untouched_new_folder_is_allowed() {
        let mut stack = Stack::new();
        stack.push(Operation::Create(created_dir("/tmp/New Folder", 100)));

        let fs = FakeFs::with(&[("/tmp/New Folder", 4096, 100)]);
        assert!(stack.take_next(&fs).is_ok());
    }

    #[test]
    fn undoing_a_copied_folder_looks_below_the_top_level() {
        // The folder's own mtime never moves when a file three levels down is
        // edited, so a stamp check on the directory alone would delete the edit.
        let mut stack = Stack::new();
        stack.push(Operation::Copy(vec![created_dir("/dst/project", 1_000)]));

        let mut fs = FakeFs::with(&[
            ("/dst/project", 4096, 1_000),
            ("/dst/project/src", 4096, 900),
            ("/dst/project/src/main.rs", 200, 900),
        ]);
        fs.touch("/dst/project/src/main.rs", 260, 5_000);

        assert_eq!(
            stack.take_next(&fs),
            Err(Refusal::Modified(PathBuf::from("/dst/project")))
        );
    }

    #[test]
    fn a_copied_folder_whose_contents_kept_their_original_times_can_be_undone() {
        // Copies preserve mtimes, so every entry inside is older than the copy.
        let mut stack = Stack::new();
        stack.push(Operation::Copy(vec![created_dir("/dst/project", 9_000)]));

        let fs = FakeFs::with(&[
            ("/dst/project", 4096, 9_000),
            ("/dst/project/src", 4096, 120),
            ("/dst/project/src/main.rs", 200, 120),
        ]);
        assert!(stack.take_next(&fs).is_ok());
    }

    #[test]
    fn undoing_a_rename_needs_the_old_name_to_be_free() {
        let operation = Operation::Rename {
            from: PathBuf::from("/tmp/a"),
            to: PathBuf::from("/tmp/b"),
        };

        let fs = FakeFs::with(&[("/tmp/b", 0, 0)]);
        assert!(validate(&operation, &fs).is_ok());

        let occupied = FakeFs::with(&[("/tmp/a", 0, 0), ("/tmp/b", 0, 0)]);
        assert_eq!(
            validate(&operation, &occupied),
            Err(Refusal::Occupied(PathBuf::from("/tmp/a")))
        );

        let vanished = FakeFs::default();
        assert_eq!(
            validate(&operation, &vanished),
            Err(Refusal::Missing(PathBuf::from("/tmp/b")))
        );
    }

    #[test]
    fn undoing_a_rename_is_allowed_even_after_the_file_was_edited() {
        // Renaming back does not destroy the edit, so the modification check
        // that guards deletions would only refuse a safe undo here.
        let operation = Operation::Rename {
            from: PathBuf::from("/tmp/a"),
            to: PathBuf::from("/tmp/b"),
        };
        let fs = FakeFs::with(&[("/tmp/b", 999, 999)]);
        assert!(validate(&operation, &fs).is_ok());
    }

    #[test]
    fn undoing_a_move_needs_every_original_slot_to_be_free() {
        let operation = Operation::Move(vec![
            Moved {
                from: PathBuf::from("/src/a"),
                to: PathBuf::from("/dst/a"),
            },
            Moved {
                from: PathBuf::from("/src/b"),
                to: PathBuf::from("/dst/b"),
            },
        ]);

        let fs = FakeFs::with(&[("/dst/a", 0, 0), ("/dst/b", 0, 0)]);
        assert!(validate(&operation, &fs).is_ok());

        let refilled = FakeFs::with(&[("/dst/a", 0, 0), ("/dst/b", 0, 0), ("/src/b", 0, 0)]);
        assert_eq!(
            validate(&operation, &refilled),
            Err(Refusal::Occupied(PathBuf::from("/src/b")))
        );
    }

    #[test]
    fn undoing_a_trash_needs_the_trash_entry_to_have_been_tracked() {
        let untracked = Operation::Trash(vec![Trashed {
            original: PathBuf::from("/tmp/a"),
            trashed: None,
            info: None,
        }]);
        assert_eq!(
            validate(&untracked, &FakeFs::default()),
            Err(Refusal::Untracked(PathBuf::from("/tmp/a")))
        );
    }

    #[test]
    fn undoing_a_trash_needs_the_trashed_copy_to_still_exist() {
        let operation = Operation::Trash(vec![Trashed {
            original: PathBuf::from("/home/diren/a"),
            trashed: Some(PathBuf::from("/trash/files/a")),
            info: Some(PathBuf::from("/trash/info/a.trashinfo")),
        }]);

        let fs = FakeFs::with(&[("/trash/files/a", 0, 0)]);
        assert!(validate(&operation, &fs).is_ok());

        // Emptied the trash in between.
        let mut emptied = fs;
        emptied.remove("/trash/files/a");
        assert_eq!(
            validate(&operation, &emptied),
            Err(Refusal::Missing(PathBuf::from("/home/diren/a")))
        );
    }

    #[test]
    fn undoing_a_trash_refuses_to_overwrite_a_file_recreated_at_the_original_path() {
        let operation = Operation::Trash(vec![Trashed {
            original: PathBuf::from("/home/diren/a"),
            trashed: Some(PathBuf::from("/trash/files/a")),
            info: None,
        }]);

        let fs = FakeFs::with(&[("/trash/files/a", 0, 0), ("/home/diren/a", 0, 0)]);
        assert_eq!(
            validate(&operation, &fs),
            Err(Refusal::Occupied(PathBuf::from("/home/diren/a")))
        );
    }

    #[test]
    fn the_toast_names_what_was_reversed() {
        let single = Operation::Trash(vec![Trashed {
            original: PathBuf::from("/home/diren/notes.txt"),
            trashed: None,
            info: None,
        }]);
        assert_eq!(
            single.describe_undo(),
            "Undid: restored “notes.txt” from Trash"
        );

        let many = Operation::Move(
            (0..12)
                .map(|index| Moved {
                    from: PathBuf::from(format!("/home/diren/Downloads/{index}")),
                    to: PathBuf::from(format!("/tmp/{index}")),
                })
                .collect(),
        );
        assert_eq!(
            many.describe_undo(),
            "Undid: moved 12 items back to Downloads"
        );

        let rename = Operation::Rename {
            from: PathBuf::from("/tmp/old.txt"),
            to: PathBuf::from("/tmp/new.txt"),
        };
        assert_eq!(
            rename.describe_undo(),
            "Undid: renamed “new.txt” back to “old.txt”"
        );

        assert_eq!(
            Operation::Create(created("/tmp/New Folder", 0, 0)).describe_undo(),
            "Undid: removed “New Folder”"
        );
    }

    #[test]
    fn every_refusal_explains_itself() {
        let refusals = [
            Refusal::NothingToUndo,
            Refusal::Missing(PathBuf::from("/a")),
            Refusal::Occupied(PathBuf::from("/a")),
            Refusal::Modified(PathBuf::from("/a")),
            Refusal::Untracked(PathBuf::from("/a")),
        ];
        for refusal in refusals {
            assert!(!refusal.message().is_empty(), "{refusal:?}");
        }
    }

    #[test]
    fn a_stamp_reads_size_and_mtime_from_real_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();

        let stamp = Stamp::of(&std::fs::metadata(&path).unwrap());
        assert_eq!(stamp.size, 5);
        assert!(stamp.modified > 0, "mtime should be after the epoch");

        std::fs::write(&path, b"hello world").unwrap();
        let after = Stamp::of(&std::fs::metadata(&path).unwrap());
        assert_ne!(stamp, after, "an edit must change the stamp");
    }
}
