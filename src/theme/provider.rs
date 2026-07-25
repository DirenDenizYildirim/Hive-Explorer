//! The single [`gtk::CssProvider`] through which every theme is applied.

use gtk::gdk;

use super::css::{self, StyleOptions};
use super::palette::Palette;

/// Owns the process-wide stylesheet provider.
#[derive(Debug, Clone)]
pub struct ThemeProvider {
    provider: gtk::CssProvider,
}

impl ThemeProvider {
    /// Install the provider on `display`.
    ///
    /// Registered above `PRIORITY_USER`, not at `PRIORITY_APPLICATION`. A system
    /// GTK theme symlinked into ~/.config/gtk-4.0/gtk.css loads at USER (800),
    /// which outranks APPLICATION (600) — at the lower priority it silently
    /// overrides Hive's palette and the flavor picked in Hive only half-applies.
    pub fn install(display: &gdk::Display) -> Self {
        let provider = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(display, &provider, Self::PRIORITY);
        Self { provider }
    }

    /// One above `GTK_STYLE_PROVIDER_PRIORITY_USER`.
    pub const PRIORITY: u32 = gtk::STYLE_PROVIDER_PRIORITY_USER + 1;

    /// A provider not attached to any display.
    pub fn detached() -> Self {
        Self {
            provider: gtk::CssProvider::new(),
        }
    }

    /// Regenerate and apply the stylesheet for `palette`.
    pub fn apply(&self, palette: &Palette, options: &StyleOptions) {
        let stylesheet = css::generate(palette, options);
        tracing::debug!(
            theme = %palette.id,
            accent = options.accent.id(),
            bytes = stylesheet.len(),
            "applying stylesheet"
        );
        self.provider.load_from_string(&stylesheet);
    }

    /// Log CSS parsing errors instead of letting them vanish.
    pub fn connect_diagnostics(&self) {
        self.provider.connect_parsing_error(|_, section, error| {
            tracing::warn!(
                location = %section.to_str(),
                error = %error,
                "generated stylesheet failed to parse"
            );
        });
    }

    /// Keep libadwaita's light/dark machinery in step with the active palette.
    pub fn sync_color_scheme(palette: &Palette) {
        let scheme = if palette.dark {
            adw::ColorScheme::ForceDark
        } else {
            adw::ColorScheme::ForceLight
        };
        adw::StyleManager::default().set_color_scheme(scheme);
    }
}
