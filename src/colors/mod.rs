//! Folder colors, stored by accent slot name and keyed by absolute path.
//!
//! Two hard rules from the build spec shape this module:
//!
//! * **Nothing is ever written into the user's folders.** No `.directory`, no
//!   dotfiles, no xattrs. The whole mapping lives in one file of Hive's own,
//!   `$XDG_CONFIG_HOME/hive/folder-colors.toml`.
//! * **Colors survive a flavor switch**, which is why the stored value is a slot
//!   name like `mauve` rather than a hex string. A mauve folder is Mocha mauve
//!   in Mocha and Latte mauve in Latte, and it keeps working under a user theme
//!   that maps its own colors onto the same fourteen slots.
//!
//! Stale paths — renamed, moved, deleted — are ignored on read and pruned
//! lazily by [`Store::paths_in`] plus [`Store::forget_all`] as directories are
//! visited. There is no startup scan: a store full of dead paths costs nothing
//! but a few bytes on disk, and walking the filesystem to tidy it would cost
//! the launch.
//!
//! Like `config`, this module uses `std::fs` rather than gio so that it stays
//! GTK-free and unit-testable without a display. It touches only Hive's own
//! config directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::atomic;
use crate::theme::palette::Accent;

/// Current schema version of `folder-colors.toml`.
pub const CURRENT_VERSION: u32 = 1;

/// The on-disk shape. Kept separate from [`Store`] so that reading can be
/// lenient about individual entries without the map itself being stringly typed.
#[derive(Debug, Serialize, Deserialize)]
struct Document {
    version: u32,
    #[serde(default)]
    colors: BTreeMap<String, String>,
}

/// Absolute path to accent slot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Store {
    colors: BTreeMap<PathBuf, Accent>,
}

/// Why a path could not be stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// Relative paths are meaningless once Hive has changed directory.
    NotAbsolute,
    /// TOML keys are text. A name that is not valid UTF-8 cannot be one, and
    /// writing a lossy approximation would colour a *different* folder.
    NotUtf8,
}

/// Why the loaded store is not simply the file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Unreadable or malformed: set aside, started empty.
    Recovered { backup: PathBuf, reason: String },
    /// Could not even be set aside; started empty, original untouched.
    RecoveryFailed { reason: String },
    /// Written by a newer Hive: set aside rather than rewriting fields we
    /// cannot round-trip.
    FromFuture { backup: PathBuf, found: u32 },
}

/// Always a usable store, plus anything worth telling the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub store: Store,
    pub notice: Option<Notice>,
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("could not serialize folder colors: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error(transparent)]
    Write(#[from] atomic::WriteError),
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.colors.len()
    }

    /// The colour for `path`, if it has one.
    pub fn get(&self, path: &Path) -> Option<Accent> {
        self.colors.get(path).copied()
    }

    /// Colour `path`, or clear it when `accent` is `None`.
    ///
    /// Returns whether anything changed, or why the path cannot be stored.
    pub fn set(&mut self, path: &Path, accent: Option<Accent>) -> Result<bool, Rejected> {
        if !path.is_absolute() {
            return Err(Rejected::NotAbsolute);
        }
        if path.to_str().is_none() {
            return Err(Rejected::NotUtf8);
        }

        Ok(match accent {
            Some(accent) => self.colors.insert(path.to_path_buf(), accent) != Some(accent),
            None => self.colors.remove(path).is_some(),
        })
    }

    /// Apply one colour to a whole selection at once.
    ///
    /// Returns how many entries changed and every path that could not be
    /// stored, so the caller can say so rather than failing silently.
    pub fn set_all<'a>(
        &mut self,
        paths: impl IntoIterator<Item = &'a Path>,
        accent: Option<Accent>,
    ) -> (usize, Vec<PathBuf>) {
        let mut changed = 0usize;
        let mut rejected = Vec::new();

        for path in paths {
            match self.set(path, accent) {
                Ok(true) => changed += 1,
                Ok(false) => {}
                Err(_) => rejected.push(path.to_path_buf()),
            }
        }

        (changed, rejected)
    }

    /// Stored paths whose parent is `directory`.
    ///
    /// This is the lazy-prune candidate list: the caller checks these few paths
    /// for existence off the main thread when a directory finishes listing, and
    /// hands what is gone to [`Store::forget_all`]. Nothing walks the tree.
    pub fn paths_in(&self, directory: &Path) -> Vec<PathBuf> {
        self.colors
            .keys()
            .filter(|path| path.parent() == Some(directory))
            .cloned()
            .collect()
    }

    /// Drop entries for paths that no longer exist. Returns how many went.
    pub fn forget_all<'a>(&mut self, paths: impl IntoIterator<Item = &'a Path>) -> usize {
        paths
            .into_iter()
            .filter(|path| self.colors.remove(*path).is_some())
            .count()
    }

    fn to_document(&self) -> Document {
        Document {
            version: CURRENT_VERSION,
            colors: self
                .colors
                .iter()
                .filter_map(|(path, accent)| {
                    Some((path.to_str()?.to_owned(), accent.id().to_owned()))
                })
                .collect(),
        }
    }

    fn from_document(document: Document) -> Self {
        let mut colors = BTreeMap::new();

        for (key, value) in document.colors {
            let path = PathBuf::from(&key);
            if !path.is_absolute() {
                tracing::warn!(path = %key, "ignoring relative path in folder colors");
                continue;
            }
            let Some(accent) = Accent::from_id(&value) else {
                tracing::warn!(path = %key, value = %value, "ignoring unknown folder color");
                continue;
            };
            colors.insert(path, accent);
        }

        Self { colors }
    }
}

/// Load the store at `path`. Never fails: a bad file becomes an empty store.
pub fn load(path: &Path) -> Loaded {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                store: Store::new(),
                notice: None,
            };
        }
        Err(error) => {
            return Loaded {
                store: Store::new(),
                notice: Some(Notice::RecoveryFailed {
                    reason: format!("could not read {}: {error}", path.display()),
                }),
            };
        }
    };

    let document: Document = match toml::from_str(&text) {
        Ok(document) => document,
        Err(error) => return recover(path, &text, error.message().to_owned()),
    };

    if document.version > CURRENT_VERSION {
        let found = document.version;
        return match atomic::back_up(path, &text) {
            Ok(backup) => Loaded {
                store: Store::new(),
                notice: Some(Notice::FromFuture { backup, found }),
            },
            Err(reason) => Loaded {
                store: Store::new(),
                notice: Some(Notice::RecoveryFailed { reason }),
            },
        };
    }

    Loaded {
        store: Store::from_document(document),
        notice: None,
    }
}

/// Write the store to `path` atomically.
pub fn save(path: &Path, store: &Store) -> Result<(), SaveError> {
    let text = toml::to_string_pretty(&store.to_document())?;
    atomic::write(path, &text)?;
    Ok(())
}

fn recover(path: &Path, original: &str, reason: String) -> Loaded {
    match atomic::back_up(path, original) {
        Ok(backup) => Loaded {
            store: Store::new(),
            notice: Some(Notice::Recovered { backup, reason }),
        },
        Err(backup_error) => Loaded {
            store: Store::new(),
            notice: Some(Notice::RecoveryFailed {
                reason: format!("{reason}; backup also failed: {backup_error}"),
            }),
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn store_in(dir: &Path) -> PathBuf {
        dir.join("folder-colors.toml")
    }

    #[test]
    fn a_missing_file_is_an_empty_store_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(&store_in(dir.path()));
        assert!(loaded.store.is_empty());
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn colors_roundtrip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_in(dir.path());

        let mut store = Store::new();
        assert_eq!(
            store.set(Path::new("/home/diren/code"), Some(Accent::Mauve)),
            Ok(true)
        );
        assert_eq!(
            store.set(Path::new("/home/diren/notes"), Some(Accent::Teal)),
            Ok(true)
        );
        save(&path, &store).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.store, store);
        assert_eq!(
            loaded.store.get(Path::new("/home/diren/code")),
            Some(Accent::Mauve)
        );
        assert_eq!(
            loaded.store.get(Path::new("/home/diren/notes")),
            Some(Accent::Teal)
        );
    }

    /// The whole point of storing a slot name: the file must not contain hex,
    /// or a folder's colour would be frozen to the flavor that set it.
    #[test]
    fn the_file_stores_slot_names_not_hex() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_in(dir.path());

        let mut store = Store::new();
        store
            .set(Path::new("/home/diren/code"), Some(Accent::Mauve))
            .unwrap();
        save(&path, &store).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("version = 1"), "{text}");
        assert!(text.contains("\"mauve\""), "{text}");
        assert!(!text.contains('#'), "no hex may appear: {text}");
    }

    #[test]
    fn setting_none_clears_the_entry() {
        let mut store = Store::new();
        let path = Path::new("/home/diren/code");

        store.set(path, Some(Accent::Red)).unwrap();
        assert_eq!(store.get(path), Some(Accent::Red));

        assert_eq!(store.set(path, None), Ok(true));
        assert_eq!(store.get(path), None);
        assert!(store.is_empty());

        assert_eq!(
            store.set(path, None),
            Ok(false),
            "clearing twice changes nothing"
        );
    }

    #[test]
    fn setting_the_same_color_twice_is_not_a_change() {
        let mut store = Store::new();
        let path = Path::new("/home/diren/code");
        assert_eq!(store.set(path, Some(Accent::Sky)), Ok(true));
        assert_eq!(store.set(path, Some(Accent::Sky)), Ok(false));
        assert_eq!(store.set(path, Some(Accent::Pink)), Ok(true));
    }

    #[test]
    fn a_whole_selection_can_be_colored_at_once() {
        let mut store = Store::new();
        let paths = [
            PathBuf::from("/home/diren/a"),
            PathBuf::from("/home/diren/b"),
            PathBuf::from("/home/diren/c"),
        ];

        let (changed, rejected) =
            store.set_all(paths.iter().map(PathBuf::as_path), Some(Accent::Green));
        assert_eq!(changed, 3);
        assert!(rejected.is_empty());
        for path in &paths {
            assert_eq!(store.get(path), Some(Accent::Green));
        }

        let (cleared, _) = store.set_all(paths.iter().map(PathBuf::as_path), None);
        assert_eq!(cleared, 3);
        assert!(store.is_empty());
    }

    #[test]
    fn relative_paths_are_refused_rather_than_stored() {
        let mut store = Store::new();
        assert_eq!(
            store.set(Path::new("code/hive"), Some(Accent::Blue)),
            Err(Rejected::NotAbsolute)
        );
        assert!(store.is_empty());
    }

    /// A TOML key is text. Rather than write a lossy approximation — which
    /// would colour whichever folder happened to match it — such a path is
    /// refused and the caller says so.
    #[test]
    fn a_name_that_is_not_valid_utf8_is_refused() {
        use std::os::unix::ffi::OsStrExt;

        let mut store = Store::new();
        let raw = std::ffi::OsStr::from_bytes(b"/tmp/bad\xff\xfename");
        assert_eq!(
            store.set(Path::new(raw), Some(Accent::Blue)),
            Err(Rejected::NotUtf8)
        );

        let (changed, rejected) = store.set_all([Path::new(raw)], Some(Accent::Blue));
        assert_eq!(changed, 0);
        assert_eq!(rejected.len(), 1);
        assert!(store.is_empty());
    }

    #[test]
    fn an_unknown_accent_name_is_skipped_without_discarding_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_in(dir.path());
        std::fs::write(
            &path,
            "version = 1\n\
             [colors]\n\
             \"/home/diren/a\" = \"chartreuse\"\n\
             \"/home/diren/b\" = \"peach\"\n",
        )
        .unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.store.get(Path::new("/home/diren/a")), None);
        assert_eq!(
            loaded.store.get(Path::new("/home/diren/b")),
            Some(Accent::Peach)
        );
    }

    #[test]
    fn a_relative_key_in_the_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_in(dir.path());
        std::fs::write(&path, "version = 1\n[colors]\n\"code\" = \"peach\"\n").unwrap();

        assert!(load(&path).store.is_empty());
    }

    #[test]
    fn a_malformed_file_is_backed_up_and_the_store_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_in(dir.path());
        let garbage = "this is not = = toml [[[";
        std::fs::write(&path, garbage).unwrap();

        let loaded = load(&path);
        assert!(loaded.store.is_empty());

        let Some(Notice::Recovered { backup, .. }) = loaded.notice else {
            panic!("expected Recovered, got {:?}", loaded.notice);
        };
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), garbage);
    }

    #[test]
    fn a_file_from_a_newer_hive_is_set_aside_rather_than_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_in(dir.path());
        let future = "version = 99\n[colors]\n\"/home/diren/a\" = \"peach\"\n";
        std::fs::write(&path, future).unwrap();

        let loaded = load(&path);
        assert!(loaded.store.is_empty());
        let Some(Notice::FromFuture { backup, found }) = loaded.notice else {
            panic!("expected FromFuture, got {:?}", loaded.notice);
        };
        assert_eq!(found, 99);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), future);
    }

    #[test]
    fn a_directory_where_the_file_belongs_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_in(dir.path());
        std::fs::create_dir(&path).unwrap();

        let loaded = load(&path);
        assert!(loaded.store.is_empty());
        assert!(matches!(loaded.notice, Some(Notice::RecoveryFailed { .. })));
    }

    #[test]
    fn saving_reports_an_error_rather_than_panicking() {
        assert!(
            save(
                Path::new("/proc/hive-nope/folder-colors.toml"),
                &Store::new()
            )
            .is_err()
        );
    }

    #[test]
    fn prune_candidates_are_the_direct_children_of_one_directory() {
        let mut store = Store::new();
        for path in [
            "/home/diren/a",
            "/home/diren/b",
            "/home/diren/deeper/c",
            "/etc/d",
        ] {
            store.set(Path::new(path), Some(Accent::Yellow)).unwrap();
        }

        let mut found = store.paths_in(Path::new("/home/diren"));
        found.sort();
        assert_eq!(
            found,
            vec![
                PathBuf::from("/home/diren/a"),
                PathBuf::from("/home/diren/b")
            ],
            "neither a grandchild nor an unrelated directory is a candidate"
        );

        assert!(store.paths_in(Path::new("/home/nobody")).is_empty());
    }

    #[test]
    fn forgetting_removes_only_what_was_named() {
        let mut store = Store::new();
        store
            .set(Path::new("/home/diren/a"), Some(Accent::Sapphire))
            .unwrap();
        store
            .set(Path::new("/home/diren/b"), Some(Accent::Sapphire))
            .unwrap();

        let gone = store.forget_all([Path::new("/home/diren/a"), Path::new("/home/diren/never")]);
        assert_eq!(gone, 1, "a path that was not stored is not counted");
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.get(Path::new("/home/diren/b")),
            Some(Accent::Sapphire)
        );
    }

    /// Colours are stored by slot, so the same store read under two flavors
    /// yields the same slot and therefore that flavor's colour.
    #[test]
    fn a_stored_slot_resolves_through_whichever_palette_is_active() {
        use crate::theme::catppuccin::{LATTE, MOCHA};

        let mut store = Store::new();
        let path = Path::new("/home/diren/code");
        store.set(path, Some(Accent::Mauve)).unwrap();

        let slot = store.get(path).unwrap();
        assert_eq!(MOCHA.accent(slot), crate::theme::Color::rgb(0xcba6f7));
        assert_eq!(LATTE.accent(slot), crate::theme::Color::rgb(0x8839ef));
    }
}
