//! The single [`gtk::CssProvider`] through which every theme is applied.
//!
//! There is exactly one provider for the lifetime of the process. Switching
//! flavors reloads its data; it is never added twice, never removed, and no
//! widget is rebuilt. That is what makes a flavor switch flicker-free — GTK
//! recomputes styles in place and animates nothing structural.

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
    /// Registered *above* `STYLE_PROVIDER_PRIORITY_USER`, not at
    /// `STYLE_PROVIDER_PRIORITY_APPLICATION`. A system GTK theme symlinked into
    /// `~/.config/gtk-4.0/gtk.css` loads at USER (800), which outranks
    /// APPLICATION (600) — so at the lower priority a user running, say, a
    /// system-wide Catppuccin Mocha theme would silently override Hive's
    /// palette, and picking Latte in Hive would half-apply. The requirement is
    /// that the flavor chosen in Hive always wins and always works, which means
    /// outranking the user stylesheet for this application's widgets.
    pub fn install(display: &gdk::Display) -> Self {
        let provider = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(display, &provider, Self::PRIORITY);
        Self { provider }
    }

    /// One above `GTK_STYLE_PROVIDER_PRIORITY_USER`.
    pub const PRIORITY: u32 = gtk::STYLE_PROVIDER_PRIORITY_USER + 1;

    /// A provider not attached to any display.
    ///
    /// Only reachable when GTK has no display at all, where nothing will render
    /// regardless. It exists so that case degrades to "unstyled" instead of
    /// unwrapping a `None`.
    pub fn detached() -> Self {
        Self {
            provider: gtk::CssProvider::new(),
        }
    }

    /// Regenerate and apply the stylesheet for `palette`.
    ///
    /// `load_from_string` never fails loudly — GTK reports CSS problems through
    /// the provider's `parsing-error` signal, which [`Self::connect_diagnostics`]
    /// routes into the log. A malformed stylesheet therefore degrades to
    /// partially-applied styling rather than a crash.
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

    /// Keep libadwaita's light/dark machinery in step with the active palette,
    /// so stock widgets that branch on color scheme pick the right variant.
    pub fn sync_color_scheme(palette: &Palette) {
        let scheme = if palette.dark {
            adw::ColorScheme::ForceDark
        } else {
            adw::ColorScheme::ForceLight
        };
        adw::StyleManager::default().set_color_scheme(scheme);
    }
}
