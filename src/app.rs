//! Application bootstrap.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

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
    let window: Rc<RefCell<Option<Rc<Window>>>> = Rc::new(RefCell::new(None));

    {
        let state = Rc::clone(&state);
        app.connect_startup(move |_| {
            *state.borrow_mut() = Some(Rc::new(start_up()));
        });
    }

    {
        let state = Rc::clone(&state);
        let window = Rc::clone(&window);
        app.connect_activate(move |app| {
            let Some(state) = state.borrow().clone() else {
                return;
            };
            let window = ensure_window(app, &state, &window);
            if window.location().is_none() {
                window.navigate_home();
            }
            window.present();
        });
    }

    {
        let state = Rc::clone(&state);
        let window = Rc::clone(&window);
        app.connect_open(move |app, files, hint| {
            let Some(state) = state.borrow().clone() else {
                return;
            };
            let window = ensure_window(app, &state, &window);

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

    let config = Rc::new(RefCell::new(loaded.config));
    let registry = Rc::new(RefCell::new(registry));

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

fn ensure_window(
    app: &adw::Application,
    state: &Rc<AppState>,
    slot: &Rc<RefCell<Option<Rc<Window>>>>,
) -> Rc<Window> {
    if let Some(existing) = slot.borrow().clone() {
        return existing;
    }

    let window = Window::new(
        app,
        Rc::clone(&state.config),
        Rc::clone(&state.registry),
        state.theme.clone(),
    );

    if let Some(message) = state.startup_notice.borrow_mut().take() {
        window.show_banner(&message);
    }

    *slot.borrow_mut() = Some(Rc::clone(&window));
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
