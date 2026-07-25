//! Theme lookup: built-in flavors plus user-supplied themes.
//!
//! v1 ships only the four Catppuccin flavors. This module exists so that adding
//! a theme is a matter of dropping a file in a directory rather than editing and
//! recompiling Hive — the switcher lists whatever the registry holds, and the
//! stylesheet generator treats every entry identically.
//!
//! # `std::fs` carve-out
//!
//! Like `config` and `colors`, this module uses `std::fs` rather than `gio`. The
//! reason is the same: it must stay GTK/gio-free so the loader is unit-testable
//! without a display or a main context. It is read-only directory scanning of
//! Hive's own config area, never user-facing filesystem work.

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::catppuccin::{BUILT_IN, MOCHA};
use super::palette::Palette;

/// A theme file that could not be loaded. Surfaced as a banner, never fatal —
/// one bad theme file must not stop Hive from starting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThemeLoadError {
    #[error("could not read theme file {path}: {message}")]
    Read { path: PathBuf, message: String },
    #[error("theme file {path} is not valid: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("theme file {path} has an empty id")]
    EmptyId { path: PathBuf },
}

impl ThemeLoadError {
    pub fn path(&self) -> &Path {
        match self {
            ThemeLoadError::Read { path, .. }
            | ThemeLoadError::Parse { path, .. }
            | ThemeLoadError::EmptyId { path } => path,
        }
    }
}

/// Every theme available this session, in switcher order.
#[derive(Debug, Clone)]
pub struct Registry {
    palettes: Vec<Palette>,
    errors: Vec<ThemeLoadError>,
}

impl Registry {
    /// The four built-in flavors and nothing else.
    pub fn built_in() -> Self {
        Self {
            palettes: BUILT_IN.iter().map(|p| (*p).clone()).collect(),
            errors: Vec::new(),
        }
    }

    /// Built-ins plus any `*.toml` themes found in `user_dir`.
    ///
    /// A missing directory is not an error — it is the normal case, since Hive
    /// ships no user themes. Unreadable or malformed files are collected into
    /// [`Registry::errors`] and skipped.
    ///
    /// A user theme whose `id` matches a built-in **replaces** it, keeping the
    /// built-in's position in the list. That is deliberate: it lets someone
    /// adjust one flavor without losing the others or having a near-duplicate
    /// entry in the switcher.
    pub fn load(user_dir: &Path) -> Self {
        let mut registry = Self::built_in();

        let entries = match std::fs::read_dir(user_dir) {
            Ok(entries) => entries,
            // Missing directory is the expected default state.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return registry,
            Err(error) => {
                registry.errors.push(ThemeLoadError::Read {
                    path: user_dir.to_path_buf(),
                    message: error.to_string(),
                });
                return registry;
            }
        };

        // Sort for deterministic ordering — read_dir order is filesystem-defined.
        let mut files: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                files.push(path);
            }
        }
        files.sort();

        for path in files {
            match load_theme_file(&path) {
                Ok(palette) => registry.insert(palette),
                Err(error) => registry.errors.push(error),
            }
        }

        registry
    }

    fn insert(&mut self, palette: Palette) {
        if let Some(existing) = self.palettes.iter_mut().find(|p| p.id == palette.id) {
            *existing = palette;
        } else {
            self.palettes.push(palette);
        }
    }

    pub fn all(&self) -> &[Palette] {
        &self.palettes
    }

    pub fn errors(&self) -> &[ThemeLoadError] {
        &self.errors
    }

    pub fn get(&self, id: &str) -> Option<&Palette> {
        self.palettes.iter().find(|p| p.id == id)
    }

    /// Resolve `id`, falling back to Mocha when it names a theme that is not
    /// present — a config referencing a theme file that has since been deleted
    /// must not prevent startup.
    pub fn get_or_default(&self, id: &str) -> &Palette {
        self.get(id).unwrap_or_else(|| self.default_palette())
    }

    /// The startup default: Mocha if present, otherwise the first entry.
    pub fn default_palette(&self) -> &Palette {
        self.get("mocha")
            .or_else(|| self.palettes.first())
            .unwrap_or(&MOCHA)
    }

    /// First dark theme, used by follow-system mode when the configured dark
    /// flavor is missing.
    pub fn first_dark(&self) -> Option<&Palette> {
        self.palettes.iter().find(|p| p.dark)
    }

    /// First light theme, used by follow-system mode when the configured light
    /// flavor is missing.
    pub fn first_light(&self) -> Option<&Palette> {
        self.palettes.iter().find(|p| !p.dark)
    }

    /// Ids currently registered, for validation and for the switcher.
    pub fn ids(&self) -> HashSet<Cow<'static, str>> {
        self.palettes.iter().map(|p| p.id.clone()).collect()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::built_in()
    }
}

fn load_theme_file(path: &Path) -> Result<Palette, ThemeLoadError> {
    let text = std::fs::read_to_string(path).map_err(|error| ThemeLoadError::Read {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;

    let palette: Palette = toml::from_str(&text).map_err(|error| ThemeLoadError::Parse {
        path: path.to_path_buf(),
        message: error.message().to_owned(),
    })?;

    if palette.id.trim().is_empty() {
        return Err(ThemeLoadError::EmptyId {
            path: path.to_path_buf(),
        });
    }

    Ok(palette)
}

/// Serialize a palette to the user-theme TOML format. Used by the tests to keep
/// the documented format and the parser in lockstep.
pub fn to_toml(palette: &Palette) -> Result<String, toml::ser::Error> {
    toml::to_string_pretty(palette)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::theme::catppuccin::{LATTE, MOCHA};
    use crate::theme::palette::Accent;

    #[test]
    fn built_in_registry_has_four_flavors() {
        let registry = Registry::built_in();
        assert_eq!(registry.all().len(), 4);
        assert!(registry.get("mocha").is_some());
        assert!(registry.get("latte").is_some());
        assert!(registry.get("frappe").is_some());
        assert!(registry.get("macchiato").is_some());
        assert!(registry.errors().is_empty());
    }

    #[test]
    fn missing_user_dir_is_not_an_error() {
        let registry = Registry::load(Path::new("/nonexistent/hive/themes/xyzzy"));
        assert_eq!(registry.all().len(), 4);
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
    }

    #[test]
    fn a_built_in_roundtrips_through_the_user_theme_format() {
        // The documented file format must be exactly what the built-ins are, or
        // the README example would not actually load.
        let dir = tempfile::tempdir().unwrap();
        let text = to_toml(&MOCHA).unwrap();
        std::fs::write(
            dir.path().join("copy.toml"),
            text.replace("mocha", "mymocha"),
        )
        .unwrap();

        let registry = Registry::load(dir.path());
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
        let loaded = registry.get("mymocha").expect("user theme should load");
        assert_eq!(loaded.accent(Accent::Mauve), MOCHA.accent(Accent::Mauve));
        assert_eq!(loaded.neutrals.base, MOCHA.neutrals.base);
        assert_eq!(registry.all().len(), 5);
    }

    #[test]
    fn user_theme_can_override_a_built_in_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let mut custom = MOCHA.clone();
        custom.name = Cow::Owned("My Mocha".to_owned());
        custom.neutrals.base = crate::theme::palette::Color::rgb(0x000000);
        std::fs::write(dir.path().join("mocha.toml"), to_toml(&custom).unwrap()).unwrap();

        let registry = Registry::load(dir.path());
        assert_eq!(registry.all().len(), 4, "override must not add an entry");
        let mocha = registry.get("mocha").unwrap();
        assert_eq!(mocha.name, "My Mocha");
        assert_eq!(mocha.neutrals.base.to_hex(), "#000000");
        // Position preserved.
        assert_eq!(registry.all()[3].id, "mocha");
    }

    #[test]
    fn malformed_theme_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.toml"), "this is not = = toml").unwrap();
        std::fs::write(
            dir.path().join("good.toml"),
            to_toml(&LATTE).unwrap().replace("latte", "mylatte"),
        )
        .unwrap();

        let registry = Registry::load(dir.path());
        assert_eq!(registry.errors().len(), 1);
        assert!(registry.get("mylatte").is_some(), "good theme still loads");
        assert!(registry.get("mocha").is_some(), "built-ins still present");
    }

    #[test]
    fn theme_missing_a_color_is_a_parse_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("partial.toml"),
            "id = \"partial\"\nname = \"Partial\"\ndark = true\n",
        )
        .unwrap();

        let registry = Registry::load(dir.path());
        assert_eq!(registry.errors().len(), 1);
        assert!(matches!(registry.errors()[0], ThemeLoadError::Parse { .. }));
        assert_eq!(registry.all().len(), 4);
    }

    #[test]
    fn non_toml_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("README.md"), "# themes").unwrap();

        let registry = Registry::load(dir.path());
        assert!(registry.errors().is_empty());
        assert_eq!(registry.all().len(), 4);
    }

    #[test]
    fn unknown_id_falls_back_to_default_rather_than_failing() {
        let registry = Registry::built_in();
        assert_eq!(registry.get_or_default("deleted-theme").id, "mocha");
        assert_eq!(registry.default_palette().id, "mocha");
    }

    #[test]
    fn light_and_dark_lookups_work() {
        let registry = Registry::built_in();
        assert_eq!(registry.first_light().map(|p| p.id.as_ref()), Some("latte"));
        assert_eq!(registry.first_dark().map(|p| p.id.as_ref()), Some("frappe"));
    }

    #[test]
    fn user_themes_load_in_deterministic_order() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["zebra", "alpha", "middle"] {
            let mut p = MOCHA.clone();
            p.id = Cow::Owned(name.to_owned());
            p.name = Cow::Owned(name.to_owned());
            std::fs::write(
                dir.path().join(format!("{name}.toml")),
                to_toml(&p).unwrap(),
            )
            .unwrap();
        }
        let first = Registry::load(dir.path());
        let second = Registry::load(dir.path());
        let ids: Vec<&str> = first.all().iter().map(|p| p.id.as_ref()).collect();
        let ids2: Vec<&str> = second.all().iter().map(|p| p.id.as_ref()).collect();
        assert_eq!(ids, ids2);
        assert_eq!(&ids[4..], &["alpha", "middle", "zebra"]);
    }
}
