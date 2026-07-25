//! Decisions taken before a copy or move starts moving bytes.
//!
//! Everything here is plain logic over facts the caller has already gathered, so
//! the rules that stop a recursive copy can be tested without a filesystem.

use std::path::{Path, PathBuf};

use crate::model::format::human_bytes;
use crate::model::path::{display_name, normalize, would_recurse};

/// What the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Move,
}

impl Kind {
    pub const fn verb(self) -> &'static str {
        match self {
            Kind::Copy => "Copy",
            Kind::Move => "Move",
        }
    }

    pub const fn progress_title(self) -> &'static str {
        match self {
            Kind::Copy => "Copying",
            Kind::Move => "Moving",
        }
    }
}

/// A reason to stop before touching anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NoSources,
    /// The classic recursive data-eater: the target lives inside the source.
    IntoOwnSubtree {
        source: PathBuf,
        destination: PathBuf,
    },
    OntoItself(PathBuf),
    Missing(PathBuf),
    NotEnoughSpace {
        required: u64,
        available: u64,
    },
}

impl Refusal {
    pub fn title(&self) -> &'static str {
        match self {
            Refusal::NoSources => "Nothing to transfer",
            Refusal::IntoOwnSubtree { .. } => "Cannot copy a folder into itself",
            Refusal::OntoItself(_) => "Source and destination are the same",
            Refusal::Missing(_) => "File is no longer there",
            Refusal::NotEnoughSpace { .. } => "Not enough free space",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Refusal::NoSources => "No files were selected.".to_owned(),
            Refusal::IntoOwnSubtree {
                source,
                destination,
            } => format!(
                "“{}” contains the destination “{}”, so the transfer would never finish.",
                display_name(source),
                display_name(destination)
            ),
            Refusal::OntoItself(path) => {
                format!("“{}” is already in that folder.", display_name(path))
            }
            Refusal::Missing(path) => {
                format!("“{}” could not be found.", display_name(path))
            }
            Refusal::NotEnoughSpace {
                required,
                available,
            } => format!(
                "This needs {} but only {} is free.",
                human_bytes(*required),
                human_bytes(*available)
            ),
        }
    }
}

/// How the transfer will be carried out, which decides whether it is instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// A move within one filesystem: a rename, no bytes touched.
    Rename,
    /// A move across filesystems: copy everything, then delete the source —
    /// and only after the copy has fully and successfully completed.
    CopyThenDelete,
    Copy,
}

impl Strategy {
    /// Shown before the operation starts: it decides whether to wait or walk away.
    pub const fn describe(self) -> &'static str {
        match self {
            Strategy::Rename => "Moving within the same drive — this is instant",
            Strategy::CopyThenDelete => "Moving to a different drive — copying, then removing",
            Strategy::Copy => "Copying",
        }
    }

    /// True when the operation transfers bytes and so needs free space.
    pub const fn transfers_bytes(self) -> bool {
        !matches!(self, Strategy::Rename)
    }
}

/// The result of walking the sources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Survey {
    pub items: u64,
    pub bytes: u64,
}

impl Survey {
    pub fn add_file(&mut self, bytes: u64) {
        self.items += 1;
        self.bytes += bytes;
    }

    pub fn add_directory(&mut self) {
        self.items += 1;
    }
}

/// Everything decided before the first byte moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub kind: Kind,
    pub strategy: Strategy,
    pub survey: Survey,
    pub required: u64,
}

/// Refuse a transfer that would recurse into itself.
///
/// Paths must already be canonicalized by the caller; symlinks are resolved
/// there because deciding it here would need the filesystem.
pub fn validate(sources: &[PathBuf], destination: &Path) -> Result<(), Refusal> {
    if sources.is_empty() {
        return Err(Refusal::NoSources);
    }

    let destination = normalize(destination);

    for source in sources {
        let source = normalize(source);

        if source == destination {
            return Err(Refusal::OntoItself(source));
        }

        if would_recurse(&source, &destination) {
            return Err(Refusal::IntoOwnSubtree {
                source,
                destination,
            });
        }
    }

    Ok(())
}

/// A same-filesystem move is a rename; anything else transfers bytes.
pub const fn strategy(kind: Kind, same_filesystem: bool) -> Strategy {
    match (kind, same_filesystem) {
        (Kind::Copy, _) => Strategy::Copy,
        (Kind::Move, true) => Strategy::Rename,
        (Kind::Move, false) => Strategy::CopyThenDelete,
    }
}

/// Build the plan once the walk has finished.
pub const fn plan(kind: Kind, strategy: Strategy, survey: Survey) -> Plan {
    let required = if strategy.transfers_bytes() {
        survey.bytes
    } else {
        0
    };
    Plan {
        kind,
        strategy,
        survey,
        required,
    }
}

/// Refuse before starting rather than dying at 90% with half a tree written.
///
/// An unknown free-space figure is not treated as zero: some filesystems do not
/// report one, and refusing every transfer there would be worse than trying.
pub fn check_space(required: u64, available: Option<u64>) -> Result<(), Refusal> {
    match available {
        Some(available) if available < required => Err(Refusal::NotEnoughSpace {
            required,
            available,
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn copying_a_folder_into_its_own_subtree_is_refused() {
        let result = validate(
            &paths(&["/home/diren/data"]),
            Path::new("/home/diren/data/backup"),
        );
        assert!(matches!(result, Err(Refusal::IntoOwnSubtree { .. })));
    }

    #[test]
    fn copying_a_folder_deep_into_its_own_subtree_is_refused() {
        let result = validate(
            &paths(&["/home/diren/data"]),
            Path::new("/home/diren/data/a/b/c/d"),
        );
        assert!(matches!(result, Err(Refusal::IntoOwnSubtree { .. })));
    }

    #[test]
    fn copying_a_folder_onto_itself_is_refused() {
        let result = validate(&paths(&["/home/diren/data"]), Path::new("/home/diren/data"));
        assert_eq!(
            result,
            Err(Refusal::OntoItself(PathBuf::from("/home/diren/data")))
        );
    }

    #[test]
    fn dot_components_cannot_smuggle_a_recursive_copy_through() {
        let result = validate(
            &paths(&["/home/diren/data"]),
            Path::new("/home/diren/other/../data/backup"),
        );
        assert!(matches!(result, Err(Refusal::IntoOwnSubtree { .. })));

        let result = validate(
            &paths(&["/home/diren/data/"]),
            Path::new("/home/diren/./data"),
        );
        assert!(matches!(result, Err(Refusal::OntoItself(_))));
    }

    #[test]
    fn a_sibling_with_a_shared_name_prefix_is_allowed() {
        assert!(
            validate(
                &paths(&["/home/diren/data"]),
                Path::new("/home/diren/data2")
            )
            .is_ok()
        );
        assert!(validate(&paths(&["/home/di"]), Path::new("/home/diren")).is_ok());
    }

    #[test]
    fn copying_up_into_a_parent_is_allowed() {
        assert!(validate(&paths(&["/home/diren/data/sub"]), Path::new("/home/diren")).is_ok());
    }

    #[test]
    fn one_bad_source_in_a_selection_stops_the_whole_transfer() {
        let result = validate(
            &paths(&["/home/diren/ok", "/home/diren/data"]),
            Path::new("/home/diren/data/backup"),
        );
        assert!(matches!(result, Err(Refusal::IntoOwnSubtree { .. })));
    }

    #[test]
    fn an_empty_selection_is_refused() {
        assert_eq!(validate(&[], Path::new("/tmp")), Err(Refusal::NoSources));
    }

    #[test]
    fn a_same_filesystem_move_is_a_rename() {
        assert_eq!(strategy(Kind::Move, true), Strategy::Rename);
        assert!(!Strategy::Rename.transfers_bytes());
    }

    #[test]
    fn a_cross_filesystem_move_copies_then_deletes() {
        assert_eq!(strategy(Kind::Move, false), Strategy::CopyThenDelete);
        assert!(Strategy::CopyThenDelete.transfers_bytes());
    }

    #[test]
    fn a_copy_stays_a_copy_on_either_filesystem() {
        assert_eq!(strategy(Kind::Copy, true), Strategy::Copy);
        assert_eq!(strategy(Kind::Copy, false), Strategy::Copy);
        assert!(Strategy::Copy.transfers_bytes());
    }

    #[test]
    fn a_rename_needs_no_free_space() {
        let survey = Survey {
            items: 3,
            bytes: 9_000_000_000,
        };
        let plan = plan(Kind::Move, Strategy::Rename, survey);
        assert_eq!(plan.required, 0);
        assert!(check_space(plan.required, Some(0)).is_ok());
    }

    #[test]
    fn a_copy_needs_the_whole_size() {
        let survey = Survey {
            items: 3,
            bytes: 4096,
        };
        let plan = plan(Kind::Copy, Strategy::Copy, survey);
        assert_eq!(plan.required, 4096);
    }

    #[test]
    fn insufficient_space_is_refused_with_both_figures() {
        let result = check_space(4096, Some(1024));
        assert_eq!(
            result,
            Err(Refusal::NotEnoughSpace {
                required: 4096,
                available: 1024
            })
        );
        let message = result.unwrap_err().message();
        assert!(message.contains("4.0 KiB"), "{message}");
        assert!(message.contains("1.0 KiB"), "{message}");
    }

    #[test]
    fn exactly_enough_space_is_allowed() {
        assert!(check_space(4096, Some(4096)).is_ok());
    }

    #[test]
    fn an_unknown_free_space_figure_does_not_block_the_transfer() {
        assert!(check_space(u64::MAX, None).is_ok());
    }

    #[test]
    fn a_survey_counts_directories_as_items_with_no_bytes() {
        let mut survey = Survey::default();
        survey.add_directory();
        survey.add_file(100);
        survey.add_file(0);
        assert_eq!(
            survey,
            Survey {
                items: 3,
                bytes: 100
            }
        );
    }

    #[test]
    fn every_refusal_explains_itself() {
        let refusals = [
            Refusal::NoSources,
            Refusal::IntoOwnSubtree {
                source: PathBuf::from("/a"),
                destination: PathBuf::from("/a/b"),
            },
            Refusal::OntoItself(PathBuf::from("/a")),
            Refusal::Missing(PathBuf::from("/a")),
            Refusal::NotEnoughSpace {
                required: 2,
                available: 1,
            },
        ];
        for refusal in refusals {
            assert!(!refusal.title().is_empty());
            assert!(!refusal.message().is_empty(), "{refusal:?}");
        }
    }

    #[test]
    fn every_strategy_says_whether_to_wait() {
        for strategy in [Strategy::Rename, Strategy::CopyThenDelete, Strategy::Copy] {
            assert!(!strategy.describe().is_empty());
        }
    }
}
