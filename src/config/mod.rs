//! Persistent configuration.
//!
//! # Design constraints
//!
//! * **No GTK, no gio.** This module is unit-tested without a display or a main
//!   context, so it takes a `&Path` rather than resolving `$XDG_CONFIG_HOME`
//!   itself. The caller in `app` does that resolution.
//! * **`std::fs` carve-out.** Per the build spec, config uses `std::fs` for its
//!   atomic temp-file-plus-`rename` write instead of `gio`. This is one of the
//!   two sanctioned exceptions (the other is the folder-color store, plus the
//!   read-only theme registry scan) and exists precisely so this module can stay
//!   gio-free and testable.
//! * **Never refuse to launch.** Every failure mode here degrades to defaults
//!   plus a user-visible notice. There is no path where a bad config file stops
//!   Hive from starting.

pub mod defaults;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use crate::model::sort::{SortKey, SortOrder};
use crate::theme::palette::Accent;

/// Current schema version. Bump when a field changes meaning, and add a
/// migration arm in [`migrate`].
pub const CURRENT_VERSION: u32 = 1;

/// How the file pane presents entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    List,
    Grid,
}

impl ViewMode {
    pub const fn id(self) -> &'static str {
        match self {
            ViewMode::List => "list",
            ViewMode::Grid => "grid",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "list" => Some(ViewMode::List),
            "grid" => Some(ViewMode::Grid),
            _ => None,
        }
    }

    pub const fn toggled(self) -> Self {
        match self {
            ViewMode::List => ViewMode::Grid,
            ViewMode::Grid => ViewMode::List,
        }
    }
}

/// Theme selection and window chrome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Theme id. Resolved through `theme::Registry`, which falls back to Mocha
    /// if this names a theme that no longer exists. An explicit default means
    /// first launch never waits on a portal reply.
    pub flavor: String,
    /// UI accent: selection, focus rings, active sidebar row.
    #[serde(deserialize_with = "lenient_accent")]
    pub accent: Accent,
    /// Best-effort light/dark following via the appearance portal. Off by
    /// default; an explicit flavor always wins and always works.
    pub follow_system: bool,
    pub light_flavor: String,
    pub dark_flavor: String,
    /// Off by default: Hyprland already rounds windows.
    pub client_side_rounding: bool,
    /// Off by default: Hyprland already shadows windows.
    pub client_side_shadow: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            flavor: defaults::FLAVOR.to_owned(),
            accent: defaults::ACCENT,
            follow_system: false,
            light_flavor: defaults::LIGHT_FLAVOR.to_owned(),
            dark_flavor: defaults::DARK_FLAVOR.to_owned(),
            client_side_rounding: false,
            client_side_shadow: false,
        }
    }
}

/// View state. One global choice, not per-directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    #[serde(deserialize_with = "lenient_view_mode")]
    pub mode: ViewMode,
    pub show_hidden: bool,
    #[serde(deserialize_with = "lenient_sort_key")]
    pub sort_key: SortKey,
    #[serde(deserialize_with = "lenient_sort_order")]
    pub sort_order: SortOrder,
    pub folders_first: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            mode: ViewMode::List,
            show_hidden: false,
            sort_key: SortKey::Name,
            sort_order: SortOrder::Ascending,
            folders_first: true,
        }
    }
}

/// Thumbnail limits. All three caps are configurable per the build spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Thumbnails {
    pub enabled: bool,
    /// Longest edge, in pixels.
    pub max_pixels: u32,
    /// Skip source files larger than this.
    pub max_file_bytes: u64,
    /// Disable thumbnailing entirely in directories larger than this.
    pub max_directory_entries: usize,
}

impl Default for Thumbnails {
    fn default() -> Self {
        Self {
            enabled: true,
            max_pixels: defaults::THUMBNAIL_MAX_PIXELS,
            max_file_bytes: defaults::THUMBNAIL_MAX_FILE_BYTES,
            max_directory_entries: defaults::THUMBNAIL_MAX_DIRECTORY_ENTRIES,
        }
    }
}

/// Operation semantics and opt-in input modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Behavior {
    /// Default is to copy the symlink itself, not its target.
    pub follow_symlinks_on_copy: bool,
    /// Warn on quit while Hive owns a file clipboard — Wayland clipboard
    /// contents die with the owning process.
    pub warn_clipboard_on_quit: bool,
    /// Off by default: `hjkl` navigation with `gg`/`G`.
    pub vim_keys: bool,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            follow_symlinks_on_copy: false,
            warn_clipboard_on_quit: true,
            vim_keys: false,
        }
    }
}

/// Sidebar state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Sidebar {
    /// Pinned folders, in user-defined order.
    pub pinned: Vec<PathBuf>,
}

/// The whole configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub appearance: Appearance,
    pub view: View,
    pub thumbnails: Thumbnails,
    pub behavior: Behavior,
    pub sidebar: Sidebar,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            appearance: Appearance::default(),
            view: View::default(),
            thumbnails: Thumbnails::default(),
            behavior: Behavior::default(),
            sidebar: Sidebar::default(),
        }
    }
}

/// Why the loaded config is not simply the file on disk.
///
/// Surfaced as an inline banner, never a modal, and never fatal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The file was unreadable or malformed. It was backed up and defaults were
    /// regenerated.
    Recovered { backup: PathBuf, reason: String },
    /// The file could not even be backed up; defaults are in use for this
    /// session and the original is untouched.
    RecoveryFailed { reason: String },
    /// The file was written by a newer Hive. Backed up, defaults regenerated,
    /// rather than silently discarding fields we do not understand.
    FromFuture { backup: PathBuf, found: u32 },
    /// Migrated forward from an older schema version.
    Migrated { from: u32, to: u32 },
}

/// The result of loading: always a usable config, plus anything worth telling
/// the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    pub notice: Option<Notice>,
}

impl Loaded {
    fn clean(config: Config) -> Self {
        Self {
            config,
            notice: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("could not create config directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("could not write config to {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Load the config at `path`.
///
/// A missing file yields defaults with no notice — that is first launch, not an
/// error. Anything unreadable or malformed is backed up to `<name>.bak` and
/// replaced by defaults, with a [`Notice`] for the banner.
pub fn load(path: &Path) -> Loaded {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Loaded::clean(Config::default());
        }
        Err(error) => {
            // Unreadable but present: do not clobber it, since the cause may be
            // a transient permission problem and the file may be valuable.
            return Loaded {
                config: Config::default(),
                notice: Some(Notice::RecoveryFailed {
                    reason: format!("could not read {}: {error}", path.display()),
                }),
            };
        }
    };

    let parsed: Config = match toml::from_str(&text) {
        Ok(config) => config,
        Err(error) => {
            return recover(path, &text, error.message().to_owned());
        }
    };

    if parsed.version > CURRENT_VERSION {
        // Written by a newer Hive. Rewriting it would silently drop fields we
        // cannot round-trip, so preserve it and start clean.
        let found = parsed.version;
        return match back_up(path, &text) {
            Ok(backup) => Loaded {
                config: Config::default(),
                notice: Some(Notice::FromFuture { backup, found }),
            },
            Err(reason) => Loaded {
                config: Config::default(),
                notice: Some(Notice::RecoveryFailed { reason }),
            },
        };
    }

    if parsed.version < CURRENT_VERSION {
        let from = parsed.version;
        let migrated = migrate(parsed);
        return Loaded {
            config: migrated,
            notice: Some(Notice::Migrated {
                from,
                to: CURRENT_VERSION,
            }),
        };
    }

    Loaded::clean(parsed)
}

fn recover(path: &Path, original: &str, reason: String) -> Loaded {
    match back_up(path, original) {
        Ok(backup) => Loaded {
            config: Config::default(),
            notice: Some(Notice::Recovered { backup, reason }),
        },
        Err(backup_error) => Loaded {
            config: Config::default(),
            notice: Some(Notice::RecoveryFailed {
                reason: format!("{reason}; backup also failed: {backup_error}"),
            }),
        },
    }
}

fn back_up(path: &Path, contents: &str) -> Result<PathBuf, String> {
    let backup = backup_path(path);
    std::fs::write(&backup, contents)
        .map(|()| backup.clone())
        .map_err(|error| format!("could not write {}: {error}", backup.display()))
}

/// `config.toml` -> `config.toml.bak`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

/// Bring an older config forward. No-op at v1; the arms land here as the schema
/// evolves.
fn migrate(mut config: Config) -> Config {
    // Each future migration is an arm here, applied in order:
    //   if config.version < 2 { ...; config.version = 2; }
    config.version = CURRENT_VERSION;
    config
}

/// Write `config` to `path` atomically.
///
/// Serializes to a sibling temp file, flushes it, then `rename`s over the
/// target. `rename` within a directory is atomic on Linux, so a crash or a
/// full disk mid-write leaves either the old file or the new one — never a
/// truncated config that would be discarded on next launch.
pub fn save(path: &Path, config: &Config) -> Result<(), SaveError> {
    use std::io::Write as _;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| SaveError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let text = toml::to_string_pretty(config)?;

    let temp = temp_path(path);
    {
        let mut file = std::fs::File::create(&temp).map_err(|source| SaveError::Write {
            path: temp.clone(),
            source,
        })?;
        file.write_all(text.as_bytes())
            .map_err(|source| SaveError::Write {
                path: temp.clone(),
                source,
            })?;
        // Flush to disk before the rename, so the rename cannot expose a file
        // whose contents have not landed yet.
        file.sync_all().map_err(|source| SaveError::Write {
            path: temp.clone(),
            source,
        })?;
    }

    std::fs::rename(&temp, path).map_err(|source| {
        // Leave no litter behind if the rename fails.
        let _ = std::fs::remove_file(&temp);
        SaveError::Write {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

// ---- Lenient enum deserializers -----------------------------------------
//
// A typo in one enum-valued key should not invalidate the whole file and reset
// every other setting. These fall back to the default value and leave a log
// line, while genuine type errors (a table where a string belongs) still fail
// the parse and take the backup-and-regenerate path.

fn lenient_accent<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Accent, D::Error> {
    let raw = String::deserialize(deserializer)?;
    Ok(Accent::from_id(&raw).unwrap_or_else(|| {
        tracing::warn!(value = %raw, "unknown accent in config; using default");
        defaults::ACCENT
    }))
}

fn lenient_view_mode<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ViewMode, D::Error> {
    let raw = String::deserialize(deserializer)?;
    Ok(ViewMode::from_id(&raw).unwrap_or_else(|| {
        tracing::warn!(value = %raw, "unknown view mode in config; using default");
        ViewMode::List
    }))
}

fn lenient_sort_key<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SortKey, D::Error> {
    let raw = String::deserialize(deserializer)?;
    Ok(SortKey::from_id(&raw).unwrap_or_else(|| {
        tracing::warn!(value = %raw, "unknown sort key in config; using default");
        SortKey::Name
    }))
}

fn lenient_sort_order<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SortOrder, D::Error> {
    let raw = String::deserialize(deserializer)?;
    match raw.as_str() {
        "ascending" => Ok(SortOrder::Ascending),
        "descending" => Ok(SortOrder::Descending),
        other => {
            tracing::warn!(value = %other, "unknown sort order in config; using default");
            Ok(SortOrder::Ascending)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn config_in(dir: &Path) -> PathBuf {
        dir.join("config.toml")
    }

    #[test]
    fn missing_file_is_first_launch_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(&config_in(dir.path()));
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn first_launch_defaults_match_the_spec() {
        let config = Config::default();
        assert_eq!(config.version, 1);
        assert_eq!(config.appearance.flavor, "mocha");
        assert_eq!(config.appearance.accent, Accent::Mauve);
        assert_eq!(config.view.mode, ViewMode::List);
        assert!(!config.appearance.follow_system);
        // Compositor owns rounding and shadow.
        assert!(!config.appearance.client_side_rounding);
        assert!(!config.appearance.client_side_shadow);
        // Symlinks are copied as links; vim keys off.
        assert!(!config.behavior.follow_symlinks_on_copy);
        assert!(!config.behavior.vim_keys);
        assert!(config.behavior.warn_clipboard_on_quit);
        // Thumbnail caps.
        assert_eq!(config.thumbnails.max_pixels, 256);
        assert_eq!(config.thumbnails.max_file_bytes, 32 * 1024 * 1024);
        assert_eq!(config.thumbnails.max_directory_entries, 2000);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());

        let mut config = Config::default();
        config.appearance.flavor = "latte".to_owned();
        config.appearance.accent = Accent::Teal;
        config.view.mode = ViewMode::Grid;
        config.view.show_hidden = true;
        config.view.sort_key = SortKey::Modified;
        config.view.sort_order = SortOrder::Descending;
        config.sidebar.pinned = vec![PathBuf::from("/home/diren/code")];

        save(&path, &config).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.config, config);
    }

    #[test]
    fn save_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/config.toml");
        save(&path, &Config::default()).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());
        save(&path, &Config::default()).unwrap();

        let leftovers: Vec<PathBuf> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file survived: {leftovers:?}");
    }

    #[test]
    fn save_overwrites_atomically_without_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());

        let mut first = Config::default();
        first.appearance.flavor = "frappe".to_owned();
        save(&path, &first).unwrap();

        let mut second = Config::default();
        second.appearance.flavor = "latte".to_owned();
        save(&path, &second).unwrap();

        assert_eq!(load(&path).config.appearance.flavor, "latte");
    }

    #[test]
    fn malformed_file_is_backed_up_and_defaults_regenerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());
        let garbage = "this is not valid = = toml [[[";
        std::fs::write(&path, garbage).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.config, Config::default(), "must still launch");

        let Some(Notice::Recovered { backup, .. }) = loaded.notice else {
            panic!("expected a Recovered notice, got {:?}", loaded.notice);
        };
        assert_eq!(backup, backup_path(&path));
        // The original content must be preserved verbatim in the backup.
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), garbage);
    }

    #[test]
    fn wrong_type_for_a_key_is_treated_as_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());
        std::fs::write(&path, "version = 1\n[appearance]\naccent = 42\n").unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.config, Config::default());
        assert!(matches!(loaded.notice, Some(Notice::Recovered { .. })));
    }

    #[test]
    fn unknown_enum_value_falls_back_without_discarding_the_file() {
        // A typo in one key must not reset every other setting.
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());
        std::fs::write(
            &path,
            "version = 1\n\
             [appearance]\n\
             flavor = \"mocha\"\n\
             accent = \"chartreuse\"\n\
             [view]\n\
             mode = \"grid\"\n\
             show_hidden = true\n",
        )
        .unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.notice, None, "not a recoverable-file situation");
        assert_eq!(loaded.config.appearance.accent, Accent::Mauve, "fell back");
        // Everything else survived.
        assert_eq!(loaded.config.view.mode, ViewMode::Grid);
        assert!(loaded.config.view.show_hidden);
    }

    #[test]
    fn partial_file_fills_in_defaults_for_absent_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());
        std::fs::write(&path, "version = 1\n[view]\nshow_hidden = true\n").unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.notice, None);
        assert!(loaded.config.view.show_hidden);
        assert_eq!(loaded.config.appearance.flavor, "mocha");
        assert_eq!(loaded.config.thumbnails.max_pixels, 256);
    }

    #[test]
    fn empty_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());
        std::fs::write(&path, "").unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn older_version_migrates_forward() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());
        std::fs::write(&path, "version = 0\n[view]\nshow_hidden = true\n").unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.config.version, CURRENT_VERSION);
        assert!(loaded.config.view.show_hidden, "settings survive migration");
        assert_eq!(
            loaded.notice,
            Some(Notice::Migrated {
                from: 0,
                to: CURRENT_VERSION
            })
        );
    }

    #[test]
    fn newer_version_is_preserved_rather_than_rewritten() {
        // A config from a future Hive may hold fields this build cannot
        // round-trip. Back it up instead of silently dropping them.
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());
        let future = "version = 99\n[appearance]\nflavor = \"latte\"\n";
        std::fs::write(&path, future).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.config, Config::default());
        let Some(Notice::FromFuture { backup, found }) = loaded.notice else {
            panic!("expected FromFuture, got {:?}", loaded.notice);
        };
        assert_eq!(found, 99);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), future);
    }

    #[test]
    fn a_directory_where_the_config_belongs_does_not_panic() {
        // Pathological but real: something created a directory at config.toml.
        let dir = tempfile::tempdir().unwrap();
        let path = config_in(dir.path());
        std::fs::create_dir(&path).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.config, Config::default(), "must still launch");
        assert!(matches!(loaded.notice, Some(Notice::RecoveryFailed { .. })));
    }

    #[test]
    fn save_reports_an_error_rather_than_panicking_on_a_bad_path() {
        // /proc is not writable; save must return Err, not unwind.
        let result = save(
            Path::new("/proc/hive-should-not-exist/config.toml"),
            &Config::default(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn view_mode_toggles() {
        assert_eq!(ViewMode::List.toggled(), ViewMode::Grid);
        assert_eq!(ViewMode::Grid.toggled(), ViewMode::List);
        assert_eq!(ViewMode::from_id("grid"), Some(ViewMode::Grid));
        assert_eq!(ViewMode::from_id("mosaic"), None);
    }

    #[test]
    fn serialized_form_is_human_editable() {
        // The file is meant to be hand-edited; check it looks like it.
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(text.contains("version = 1"), "{text}");
        assert!(text.contains("[appearance]"), "{text}");
        assert!(text.contains("flavor = \"mocha\""), "{text}");
        assert!(text.contains("accent = \"mauve\""), "{text}");
        assert!(text.contains("[thumbnails]"), "{text}");
    }
}
