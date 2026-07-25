//! Hive — a Catppuccin file manager for Hyprland.
//!
//! The crate is split so that the rules which decide what the user sees are
//! testable without a display:
//!
//! * [`model`], [`config`], and most of [`theme`] are plain Rust — no GTK, no
//!   gio, no main context — and carry the unit tests.
//! * [`ui`] and [`app`] are the GTK layer, and hold no policy of their own.
//!
//! The Wayland `app_id` and GTK application ID is [`paths::APP_ID`]
//! (`dev.diren.Hive`), which is what Hyprland `windowrulev2` rules target.

pub mod app;
pub mod cli;
pub mod config;
pub mod fs;
pub mod logging;
pub mod model;
pub mod paths;
pub mod theme;
pub mod ui;
