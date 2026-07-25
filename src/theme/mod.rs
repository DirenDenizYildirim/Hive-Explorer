//! Theming: palette data, built-in flavors, stylesheet generation.

pub mod catppuccin;
pub mod css;
pub mod palette;
pub mod provider;
pub mod registry;

pub use css::StyleOptions;
pub use palette::{Accent, Color, Palette};
pub use provider::ThemeProvider;
pub use registry::Registry;
