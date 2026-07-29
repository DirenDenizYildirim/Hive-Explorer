//! Application bootstrap.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

use crate::colors;
use crate::config::{self, Config};
use crate::paths;
use crate::theme::{Registry, ThemeProvider};
use crate::ui::Window;

/// Passed as the `open` hint to mean "reveal this, do not enter it".
pub const REVEAL_HINT: &str = "select";

/// Everything built once at startup and shared by every window.
struct AppState {
    config: Rc<RefCell<Config>>,
    registry: Rc<RefCell<Registry>>,
    colors: Rc<RefCell<colors::Store>>,
    theme: ThemeProvider,
    /// Deferred until a window exists to show it in.
    startup_notice: RefCell<Option<String>>,
}

/// Build the application. Call [`run`] to start it.
pub fn build() -> adw::Application {
    let app = adw::Application::builder()
        .application_id(paths::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    let state: Rc<RefCell<Option<Rc<AppState>>>> = Rc::new(RefCell::new(None));

    {
        let state = Rc::clone(&state);
        app.connect_startup(move |_| {
            *state.borrow_mut() = Some(Rc::new(start_up()));
        });
    }

    {
        let state = Rc::clone(&state);
        app.connect_activate(move |app| {
            // Every launch reaches Hive as `open`, including the one carrying
            // nothing — see `main`. `run` then emits an activate of its own on
            // top of it, so activate presents what is already there rather than
            // opening a window: otherwise one launch would produce two.
            if let Some(window) = app.active_window() {
                window.present();
                return;
            }

            let Some(state) = state.borrow().clone() else {
                return;
            };
            let window = new_window(app, &state);
            window.navigate_home();
            window.present();
        });
    }

    {
        let state = Rc::clone(&state);
        app.connect_open(move |app, files, hint| {
            let Some(state) = state.borrow().clone() else {
                return;
            };

            // A launch while Hive is already running opens another window, the
            // way every other file manager does. Two windows is also the only
            // way to drag a file from one folder to another and watch both.
            let window = new_window(app, &state);

            // The hint carries the intent across the D-Bus handoff to an
            // already-running instance, which plain argv cannot do.
            let reveal = hint == REVEAL_HINT;
            match files.first() {
                Some(file) => navigate_to_target(&window, file, reveal),
                None => window.navigate_home(),
            }
            window.present();
        });
    }

    app
}

/// Load config and themes, and install the stylesheet provider.
fn start_up() -> AppState {
    adw::init().unwrap_or_else(|error| {
        tracing::error!(%error, "libadwaita failed to initialize");
    });

    register_resources();

    let config_path = paths::config_file();
    let loaded = config::load(&config_path);
    let mut notice = describe_notice(loaded.notice.as_ref(), &config_path);

    let registry = Registry::load(&paths::themes_dir());
    for error in registry.errors() {
        tracing::warn!(%error, "ignoring theme file");
    }
    if !registry.errors().is_empty() && notice.is_none() {
        notice = Some(format!(
            "{} theme file(s) could not be loaded and were skipped",
            registry.errors().len()
        ));
    }

    let colors_path = paths::folder_colors_file();
    let folder_colors = colors::load(&colors_path);
    if let Some(message) = describe_color_notice(folder_colors.notice.as_ref(), &colors_path)
        && notice.is_none()
    {
        notice = Some(message);
    }

    let config = Rc::new(RefCell::new(loaded.config));
    let registry = Rc::new(RefCell::new(registry));
    let colors = Rc::new(RefCell::new(folder_colors.store));

    let theme = match gtk::gdk::Display::default() {
        Some(display) => {
            gtk::IconTheme::for_display(&display).add_resource_path("/dev/diren/Hive/icons");
            let provider = ThemeProvider::install(&display);
            provider.connect_diagnostics();

            let borrowed = config.borrow();
            let registry = registry.borrow();
            let palette = registry.get_or_default(&borrowed.appearance.flavor);
            let options = crate::theme::StyleOptions {
                accent: borrowed.appearance.accent,
                client_side_rounding: borrowed.appearance.client_side_rounding,
                client_side_shadow: borrowed.appearance.client_side_shadow,
                animations: gtk::Settings::for_display(&display).is_gtk_enable_animations(),
            };
            ThemeProvider::sync_color_scheme(palette);
            provider.apply(palette, &options);
            provider
        }
        None => {
            tracing::error!("no display available; continuing unstyled");
            ThemeProvider::detached()
        }
    };

    AppState {
        config,
        registry,
        colors,
        theme,
        startup_notice: RefCell::new(notice),
    }
}

/// Register the icon bundle compiled in by `build.rs`.
fn register_resources() {
    if let Err(error) = gio::resources_register_include!("hive.gresource") {
        tracing::warn!(%error, "bundled icons unavailable; falling back to the icon theme");
    }
}

/// A new window, sharing everything the application built once at startup.
///
/// Nothing here keeps the window: the application owns it until it is closed,
/// and closing the last one is what ends the process.
fn new_window(app: &adw::Application, state: &Rc<AppState>) -> Rc<Window> {
    let window = Window::new(
        app,
        Rc::clone(&state.config),
        Rc::clone(&state.registry),
        Rc::clone(&state.colors),
        state.theme.clone(),
    );

    // Whatever went wrong at startup is reported once, in the first window that
    // opens, rather than in every window for the rest of the session.
    if let Some(message) = state.startup_notice.borrow_mut().take() {
        window.show_banner(&message);
    }

    window
}

/// Navigate to a path from the command line.
///
/// A directory is opened. A file — or anything at all under `--select` — is
/// revealed instead: its parent opens and the entry itself is preselected.
fn navigate_to_target(window: &Rc<Window>, file: &gio::File, reveal: bool) {
    let Some(path) = file.path() else {
        window.navigate_to(file);
        return;
    };

    if path.is_dir() && !reveal {
        window.navigate_to_path(&path);
        return;
    }

    if !path.exists() {
        tracing::warn!(path = %path.display(), "target does not exist; showing home");
        window.navigate_home();
        window.show_banner(&format!("{} is not available", path.display()));
        return;
    }

    window.reveal(&path);
}

fn describe_notice(notice: Option<&config::Notice>, path: &Path) -> Option<String> {
    match notice? {
        config::Notice::Recovered { backup, reason } => {
            tracing::warn!(%reason, backup = %backup.display(), "config was malformed");
            Some(format!(
                "Your settings file was unreadable and has been reset. The original is at {}",
                backup.display()
            ))
        }
        config::Notice::RecoveryFailed { reason } => {
            tracing::warn!(%reason, path = %path.display(), "config unusable");
            Some("Your settings could not be read; running with defaults".to_owned())
        }
        config::Notice::FromFuture { backup, found } => {
            tracing::warn!(found, backup = %backup.display(), "config from a newer version");
            Some(format!(
                "Your settings were written by a newer version of Hive and have been set aside at {}",
                backup.display()
            ))
        }
        config::Notice::Migrated { from, to } => {
            tracing::info!(from, to, "config migrated");
            None
        }
    }
}

/// Folder colors are decorative, so a bad file is worth a line in the log and
/// at most a banner — never a refusal to launch.
fn describe_color_notice(notice: Option<&colors::Notice>, path: &Path) -> Option<String> {
    match notice? {
        colors::Notice::Recovered { backup, reason } => {
            tracing::warn!(%reason, backup = %backup.display(), "folder colors were malformed");
            Some(format!(
                "Your folder colors were unreadable and have been reset. The original is at {}",
                backup.display()
            ))
        }
        colors::Notice::RecoveryFailed { reason } => {
            tracing::warn!(%reason, path = %path.display(), "folder colors unusable");
            Some("Your folder colors could not be read; none are shown".to_owned())
        }
        colors::Notice::FromFuture { backup, found } => {
            tracing::warn!(found, backup = %backup.display(), "folder colors from a newer version");
            Some(format!(
                "Your folder colors were written by a newer version of Hive and have been set aside at {}",
                backup.display()
            ))
        }
    }
}

/// Resolve a command-line path in *this* process.
///
/// `hive .` must resolve against the invoking shell's working directory. If the
/// already-running instance resolved it, `.` would mean whatever directory that
/// process started in, and the feature would silently open the wrong folder.
pub fn canonicalize_local(input: &Path) -> PathBuf {
    let home = paths::home_dir();
    let expanded = crate::model::path::expand_tilde(input, &home);

    let cwd = std::env::current_dir().unwrap_or_else(|error| {
        tracing::warn!(%error, "no working directory; resolving against home");
        home.clone()
    });

    let resolved = crate::model::path::resolve_against(&cwd, &expanded);

    crate::model::path::canonicalize_existing(&resolved).unwrap_or(resolved)
}
