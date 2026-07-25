//! Theming: palette data, the four built-in Catppuccin flavors, the runtime
//! stylesheet generator, and the single CssProvider that applies it.
//!
//! Everything except [`provider`] is GTK-free and unit-tested headlessly.

pub mod catppuccin;
pub mod css;
pub mod palette;
pub mod provider;
pub mod registry;

pub use css::StyleOptions;
pub use palette::{Accent, Color, Palette};
pub use provider::ThemeProvider;
pub use registry::Registry;
