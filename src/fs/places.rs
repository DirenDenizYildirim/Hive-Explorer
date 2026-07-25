//! Resolution of the sidebar's Places section.
//!
//! The rule, in order, for each place:
//!
//! 1. Ask `glib::user_special_dir()`.
//! 2. If that returns `None`, fall back to the conventional `~/Name`.
//! 3. If that path does not exist on disk either, **omit the row entirely.**
//!
//! A sidebar entry that points nowhere is a bug, so there is no fourth branch
//! that renders a dead row, and Hive never offers to create the missing
//! directory.
//!
//! On a system without `xdg-user-dirs` and with only `~/Downloads` present, this
//! correctly yields Home, Downloads, and Trash.
//!
//! The resolution rule is written against injected lookups so it is unit-tested
//! headlessly, without depending on whatever the test machine happens to have in
//! its home directory.

use std::path::{Path, PathBuf};

/// The places Hive offers, in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Documents,
    Downloads,
    Pictures,
    Videos,
    Music,
    Trash,
}

impl PlaceKind {
    /// Sidebar order.
    pub const ALL: [PlaceKind; 7] = [
        PlaceKind::Home,
        PlaceKind::Documents,
        PlaceKind::Downloads,
        PlaceKind::Pictures,
        PlaceKind::Videos,
        PlaceKind::Music,
        PlaceKind::Trash,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            PlaceKind::Home => "Home",
            PlaceKind::Documents => "Documents",
            PlaceKind::Downloads => "Downloads",
            PlaceKind::Pictures => "Pictures",
            PlaceKind::Videos => "Videos",
            PlaceKind::Music => "Music",
            PlaceKind::Trash => "Trash",
        }
    }

    /// Adwaita symbolic icon. Monochrome, single stroke weight.
    pub const fn icon_name(self) -> &'static str {
        match self {
            PlaceKind::Home => "user-home-symbolic",
            PlaceKind::Documents => "folder-documents-symbolic",
            PlaceKind::Downloads => "folder-download-symbolic",
            PlaceKind::Pictures => "folder-pictures-symbolic",
            PlaceKind::Videos => "folder-videos-symbolic",
            PlaceKind::Music => "folder-music-symbolic",
            PlaceKind::Trash => "user-trash-symbolic",
        }
    }

    /// The conventional directory name under `$HOME`, used as the fallback when
    /// `user_special_dir()` has nothing to say.
    pub const fn conventional_name(self) -> Option<&'static str> {
        match self {
            PlaceKind::Home | PlaceKind::Trash => None,
            PlaceKind::Documents => Some("Documents"),
            PlaceKind::Downloads => Some("Downloads"),
            PlaceKind::Pictures => Some("Pictures"),
            PlaceKind::Videos => Some("Videos"),
            PlaceKind::Music => Some("Music"),
        }
    }

    /// Trash may be a URI rather than a path, depending on [`TrashAccess`].
    pub const fn is_virtual(self) -> bool {
        matches!(self, PlaceKind::Trash)
    }
}

/// How Trash can be reached on this system.
///
/// `trash://` is **not** part of GIO core — it is a gvfs backend. Without gvfs
/// installed, `gio info trash:///` answers "Operation not supported", so a Trash
/// row pointing at that URI is a dead row: it opens, fails to enumerate, and
/// shows an error banner every time.
///
/// Trashing files still works without gvfs, because `g_file_trash()` implements
/// the freedesktop spec directly. So the trashed files are there on disk — only
/// the URI scheme for browsing them is missing. Hive therefore falls back to
/// browsing `$XDG_DATA_HOME/Trash/files` as an ordinary directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashAccess {
    /// gvfs is present and `trash://` can be enumerated.
    Uri,
    /// No `trash://` backend; browse the spec directory directly.
    Directory(PathBuf),
    /// Neither is available — omit the row rather than render a dead one.
    Unavailable,
}

/// Decide how to reach Trash.
///
/// `data_home` is `$XDG_DATA_HOME`; `uri_supported` is what GIO's VFS reports
/// for the `trash` scheme. Split out from the gio call so the decision is
/// unit-tested.
pub fn detect_trash_access(
    data_home: &Path,
    uri_supported: bool,
    exists: impl Fn(&Path) -> bool,
) -> TrashAccess {
    if uri_supported {
        return TrashAccess::Uri;
    }

    let files = data_home.join("Trash").join("files");
    if exists(&files) {
        TrashAccess::Directory(files)
    } else {
        TrashAccess::Unavailable
    }
}

/// A resolved, navigable sidebar entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub kind: PlaceKind,
    /// `None` for virtual locations such as Trash.
    pub path: Option<PathBuf>,
}

impl Place {
    pub fn label(&self) -> &'static str {
        self.kind.label()
    }

    pub fn icon_name(&self) -> &'static str {
        self.kind.icon_name()
    }

    /// The URI to navigate to.
    pub fn uri(&self) -> String {
        match (&self.path, self.kind) {
            // A Trash place with a path is the no-gvfs fallback: an ordinary
            // directory, addressed as one.
            (Some(path), _) => format!("file://{}", path.display()),
            (None, PlaceKind::Trash) => "trash:///".to_owned(),
            (None, _) => String::new(),
        }
    }
}

/// Resolve the Places list.
///
/// * `home` — the user's home directory.
/// * `special` — `glib::user_special_dir()`, or a stand-in in tests.
/// * `exists` — an existence predicate, so tests do not depend on the real
///   filesystem.
pub fn resolve(
    home: &Path,
    special: impl Fn(PlaceKind) -> Option<PathBuf>,
    exists: impl Fn(&Path) -> bool,
    trash: &TrashAccess,
) -> Vec<Place> {
    let mut places = Vec::with_capacity(PlaceKind::ALL.len());

    for kind in PlaceKind::ALL {
        match kind {
            // Home is the one place that is always offered: it is where Hive
            // falls back to when anything else goes wrong, so a session with no
            // Home row would have no safe harbor.
            PlaceKind::Home => places.push(Place {
                kind,
                path: Some(home.to_path_buf()),
            }),

            PlaceKind::Trash => match trash {
                TrashAccess::Uri => places.push(Place { kind, path: None }),
                TrashAccess::Directory(path) => places.push(Place {
                    kind,
                    path: Some(path.clone()),
                }),
                // Same rule as every other place: never render a row that
                // points nowhere.
                TrashAccess::Unavailable => {}
            },

            _ => {
                let resolved =
                    special(kind).or_else(|| kind.conventional_name().map(|name| home.join(name)));

                // Omit rather than render a row that points nowhere.
                if let Some(path) = resolved
                    && exists(&path)
                {
                    places.push(Place {
                        kind,
                        path: Some(path),
                    });
                }
            }
        }
    }

    places
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn kinds(places: &[Place]) -> Vec<PlaceKind> {
        places.iter().map(|p| p.kind).collect()
    }

    #[test]
    fn this_machine_yields_home_downloads_trash() {
        // The documented state of the target machine: no xdg-user-dirs, so
        // user_special_dir() answers None for everything, and only ~/Downloads
        // exists on disk.
        let home = Path::new("/home/diren");
        let present: HashSet<PathBuf> = [home.join("Downloads")].into_iter().collect();

        let places = resolve(home, |_| None, |p| present.contains(p), &TrashAccess::Uri);

        assert_eq!(
            kinds(&places),
            vec![PlaceKind::Home, PlaceKind::Downloads, PlaceKind::Trash]
        );
    }

    #[test]
    fn never_renders_a_place_that_points_nowhere() {
        let home = Path::new("/home/diren");
        let places = resolve(home, |_| None, |_| false, &TrashAccess::Uri);

        // Only Home (always offered) and Trash (virtual) survive.
        assert_eq!(kinds(&places), vec![PlaceKind::Home, PlaceKind::Trash]);
        for place in &places {
            if let Some(path) = &place.path {
                assert!(!path.as_os_str().is_empty());
            }
        }
    }

    #[test]
    fn special_dir_wins_over_the_conventional_fallback() {
        let home = Path::new("/home/diren");
        let custom = PathBuf::from("/mnt/bulk/Pictures");
        let present: HashSet<PathBuf> = [custom.clone()].into_iter().collect();

        let places = resolve(
            home,
            |kind| (kind == PlaceKind::Pictures).then(|| custom.clone()),
            |p| present.contains(p),
            &TrashAccess::Uri,
        );

        let pictures = places
            .iter()
            .find(|p| p.kind == PlaceKind::Pictures)
            .expect("Pictures should resolve via user_special_dir");
        assert_eq!(pictures.path.as_deref(), Some(custom.as_path()));
    }

    #[test]
    fn special_dir_pointing_at_a_missing_path_is_still_omitted() {
        // xdg-user-dirs can name a directory the user has since deleted.
        let home = Path::new("/home/diren");
        let places = resolve(
            home,
            |kind| (kind == PlaceKind::Music).then(|| PathBuf::from("/gone/Music")),
            |_| false,
            &TrashAccess::Uri,
        );
        assert!(!kinds(&places).contains(&PlaceKind::Music));
    }

    #[test]
    fn conventional_fallback_is_used_when_it_exists() {
        let home = Path::new("/home/diren");
        let present: HashSet<PathBuf> = [home.join("Documents"), home.join("Music")]
            .into_iter()
            .collect();

        let places = resolve(home, |_| None, |p| present.contains(p), &TrashAccess::Uri);

        assert_eq!(
            kinds(&places),
            vec![
                PlaceKind::Home,
                PlaceKind::Documents,
                PlaceKind::Music,
                PlaceKind::Trash
            ]
        );
        let docs = places
            .iter()
            .find(|p| p.kind == PlaceKind::Documents)
            .unwrap();
        assert_eq!(docs.path, Some(home.join("Documents")));
    }

    #[test]
    fn a_fully_populated_home_yields_every_place_in_order() {
        let home = Path::new("/home/diren");
        let places = resolve(home, |_| None, |_| true, &TrashAccess::Uri);
        assert_eq!(kinds(&places), PlaceKind::ALL.to_vec());
    }

    #[test]
    fn home_and_trash_are_always_present() {
        let home = Path::new("/home/diren");
        for exists in [true, false] {
            let places = resolve(home, |_| None, |_| exists, &TrashAccess::Uri);
            assert!(places.iter().any(|p| p.kind == PlaceKind::Home));
            assert!(places.iter().any(|p| p.kind == PlaceKind::Trash));
        }
    }

    #[test]
    fn trash_navigates_to_a_uri_not_a_path() {
        let place = Place {
            kind: PlaceKind::Trash,
            path: None,
        };
        assert_eq!(place.uri(), "trash:///");
    }

    #[test]
    fn trash_uses_the_uri_when_gvfs_provides_it() {
        let data_home = Path::new("/home/diren/.local/share");
        assert_eq!(
            detect_trash_access(data_home, true, |_| false),
            TrashAccess::Uri
        );
    }

    #[test]
    fn trash_falls_back_to_the_spec_directory_without_gvfs() {
        // This is the target machine: no gvfs, so `trash://` is unsupported,
        // but ~/.local/share/Trash/files exists because g_file_trash() writes
        // there directly. Browsing the directory is the only way to see it.
        let data_home = Path::new("/home/diren/.local/share");
        let files = data_home.join("Trash").join("files");
        let present: HashSet<PathBuf> = [files.clone()].into_iter().collect();

        let access = detect_trash_access(data_home, false, |p| present.contains(p));
        assert_eq!(access, TrashAccess::Directory(files.clone()));

        let places = resolve(Path::new("/home/diren"), |_| None, |_| false, &access);
        let trash = places
            .iter()
            .find(|p| p.kind == PlaceKind::Trash)
            .expect("Trash should still be offered");
        assert_eq!(trash.path.as_deref(), Some(files.as_path()));
        assert_eq!(trash.uri(), "file:///home/diren/.local/share/Trash/files");
    }

    #[test]
    fn trash_row_is_omitted_when_it_is_reachable_by_neither_route() {
        // No gvfs and nothing has ever been trashed: the directory does not
        // exist. A row here would open, fail to enumerate, and show an error
        // banner every single time — the same dead-row bug as a missing Place.
        let data_home = Path::new("/home/diren/.local/share");
        let access = detect_trash_access(data_home, false, |_| false);
        assert_eq!(access, TrashAccess::Unavailable);

        let places = resolve(Path::new("/home/diren"), |_| None, |_| false, &access);
        assert_eq!(kinds(&places), vec![PlaceKind::Home]);
    }

    #[test]
    fn ordinary_places_produce_file_uris() {
        let place = Place {
            kind: PlaceKind::Downloads,
            path: Some(PathBuf::from("/home/diren/Downloads")),
        };
        assert_eq!(place.uri(), "file:///home/diren/Downloads");
    }

    #[test]
    fn every_place_has_a_symbolic_icon_and_label() {
        for kind in PlaceKind::ALL {
            assert!(kind.icon_name().ends_with("-symbolic"), "{kind:?}");
            assert!(!kind.label().is_empty());
        }
    }
}
