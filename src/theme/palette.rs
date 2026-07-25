//! Theme data types.
//!
//! This module is deliberately free of GTK and gio so it can be unit-tested
//! without a display or a main context.
//!
//! # The 14-slot accent contract
//!
//! [`Accent`] is a fixed set of fourteen named slots. It is *not* Catppuccin
//! data — it is the contract every theme fills in. Folder colors (see the
//! `colors` module) are persisted by accent *name*, so a folder tagged "mauve"
//! resolves through whichever palette is currently active. That is what lets a
//! folder color survive a flavor switch, and it is also what will let it
//! survive a switch to a user-supplied theme that has nothing to do with
//! Catppuccin: such a theme simply maps its own colors onto these same slots.
//!
//! If accents were free-form strings, every stored folder color would break the
//! moment a theme that did not happen to define that name became active.

use std::borrow::Cow;
use std::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};

/// An 8-bit-per-channel opaque color.
///
/// Serialized as a CSS-style `#rrggbb` string so hand-written theme files stay
/// readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Build from a `0xRRGGBB` literal. Used by the built-in palettes so the
    /// constant tables read like the upstream Catppuccin definitions.
    pub const fn rgb(value: u32) -> Self {
        Self {
            r: ((value >> 16) & 0xff) as u8,
            g: ((value >> 8) & 0xff) as u8,
            b: (value & 0xff) as u8,
        }
    }

    /// `#rrggbb`.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// `rgba(r, g, b, a)` — for CSS that needs transparency over a base color.
    pub fn to_rgba(self, alpha: f32) -> String {
        let alpha = alpha.clamp(0.0, 1.0);
        format!("rgba({}, {}, {}, {alpha:.3})", self.r, self.g, self.b)
    }

    /// Parse `#rrggbb`, `#rgb`, or the same without the leading `#`.
    pub fn parse_hex(text: &str) -> Result<Self, ColorParseError> {
        let body = text.strip_prefix('#').unwrap_or(text);
        let digits: Vec<u8> = body
            .chars()
            .map(|c| {
                c.to_digit(16)
                    .map(|d| d as u8)
                    .ok_or_else(|| ColorParseError(text.to_owned()))
            })
            .collect::<Result<_, _>>()?;

        match digits.len() {
            3 => Ok(Self::new(digits[0] * 17, digits[1] * 17, digits[2] * 17)),
            6 => Ok(Self::new(
                digits[0] << 4 | digits[1],
                digits[2] << 4 | digits[3],
                digits[4] << 4 | digits[5],
            )),
            _ => Err(ColorParseError(text.to_owned())),
        }
    }

    /// Relative luminance per WCAG 2.x, used to pick readable foregrounds.
    pub fn relative_luminance(self) -> f32 {
        fn channel(value: u8) -> f32 {
            let v = f32::from(value) / 255.0;
            if v <= 0.039_285_7 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// Blend `self` toward `other` by `factor` (0.0 = self, 1.0 = other).
    pub fn mix(self, other: Self, factor: f32) -> Self {
        let f = factor.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| -> u8 {
            let a = f32::from(a);
            let b = f32::from(b);
            (a + (b - a) * f).round().clamp(0.0, 255.0) as u8
        };
        Self::new(
            lerp(self.r, other.r),
            lerp(self.g, other.g),
            lerp(self.b, other.b),
        )
    }
}

/// WCAG contrast ratio between two colors, from 1.0 (identical) to 21.0
/// (black on white). Order-independent.
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (lighter, darker) = {
        let (la, lb) = (a.relative_luminance(), b.relative_luminance());
        if la >= lb { (la, lb) } else { (lb, la) }
    };
    (lighter + 0.05) / (darker + 0.05)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid color {0:?}: expected #rrggbb or #rgb")]
pub struct ColorParseError(pub String);

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct HexVisitor;

        impl Visitor<'_> for HexVisitor {
            type Value = Color;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a hex color such as \"#cba6f7\"")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Color, E> {
                Color::parse_hex(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(HexVisitor)
    }
}

/// The fourteen accent slots every theme must fill.
///
/// Order matches the upstream Catppuccin ordering, which is also the order the
/// swatch grid renders in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    Rosewater,
    Flamingo,
    Pink,
    Mauve,
    Red,
    Maroon,
    Peach,
    Yellow,
    Green,
    Teal,
    Sky,
    Sapphire,
    Blue,
    Lavender,
}

impl Accent {
    pub const ALL: [Accent; 14] = [
        Accent::Rosewater,
        Accent::Flamingo,
        Accent::Pink,
        Accent::Mauve,
        Accent::Red,
        Accent::Maroon,
        Accent::Peach,
        Accent::Yellow,
        Accent::Green,
        Accent::Teal,
        Accent::Sky,
        Accent::Sapphire,
        Accent::Blue,
        Accent::Lavender,
    ];

    /// Stable serialization key. This string is what ends up in the folder-color
    /// store, so it must never change.
    pub const fn id(self) -> &'static str {
        match self {
            Accent::Rosewater => "rosewater",
            Accent::Flamingo => "flamingo",
            Accent::Pink => "pink",
            Accent::Mauve => "mauve",
            Accent::Red => "red",
            Accent::Maroon => "maroon",
            Accent::Peach => "peach",
            Accent::Yellow => "yellow",
            Accent::Green => "green",
            Accent::Teal => "teal",
            Accent::Sky => "sky",
            Accent::Sapphire => "sapphire",
            Accent::Blue => "blue",
            Accent::Lavender => "lavender",
        }
    }

    /// Title-cased label for menus and tooltips.
    pub const fn display_name(self) -> &'static str {
        match self {
            Accent::Rosewater => "Rosewater",
            Accent::Flamingo => "Flamingo",
            Accent::Pink => "Pink",
            Accent::Mauve => "Mauve",
            Accent::Red => "Red",
            Accent::Maroon => "Maroon",
            Accent::Peach => "Peach",
            Accent::Yellow => "Yellow",
            Accent::Green => "Green",
            Accent::Teal => "Teal",
            Accent::Sky => "Sky",
            Accent::Sapphire => "Sapphire",
            Accent::Blue => "Blue",
            Accent::Lavender => "Lavender",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|accent| accent.id() == id)
    }
}

/// Values for the fourteen accent slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accents {
    pub rosewater: Color,
    pub flamingo: Color,
    pub pink: Color,
    pub mauve: Color,
    pub red: Color,
    pub maroon: Color,
    pub peach: Color,
    pub yellow: Color,
    pub green: Color,
    pub teal: Color,
    pub sky: Color,
    pub sapphire: Color,
    pub blue: Color,
    pub lavender: Color,
}

impl Accents {
    pub const fn get(&self, accent: Accent) -> Color {
        match accent {
            Accent::Rosewater => self.rosewater,
            Accent::Flamingo => self.flamingo,
            Accent::Pink => self.pink,
            Accent::Mauve => self.mauve,
            Accent::Red => self.red,
            Accent::Maroon => self.maroon,
            Accent::Peach => self.peach,
            Accent::Yellow => self.yellow,
            Accent::Green => self.green,
            Accent::Teal => self.teal,
            Accent::Sky => self.sky,
            Accent::Sapphire => self.sapphire,
            Accent::Blue => self.blue,
            Accent::Lavender => self.lavender,
        }
    }
}

/// The twelve-step neutral ramp, darkest-surface to brightest-text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neutrals {
    pub crust: Color,
    pub mantle: Color,
    pub base: Color,
    pub surface0: Color,
    pub surface1: Color,
    pub surface2: Color,
    pub overlay0: Color,
    pub overlay1: Color,
    pub overlay2: Color,
    pub subtext0: Color,
    pub subtext1: Color,
    pub text: Color,
}

/// A complete theme: identity, polarity, fourteen accents, twelve neutrals.
///
/// `Cow<'static, str>` lets the built-in flavors be genuine `const` values while
/// user-supplied themes deserialize into owned strings through the same type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    /// Stable identifier, e.g. `"mocha"`. Persisted in the config file.
    pub id: Cow<'static, str>,
    /// Human-readable name shown in the flavor switcher, e.g. `"Mocha"`.
    pub name: Cow<'static, str>,
    /// Whether this is a dark theme. Drives `AdwStyleManager` color scheme and
    /// the direction of elevation shading.
    pub dark: bool,
    pub accents: Accents,
    pub neutrals: Neutrals,
}

impl Palette {
    pub fn accent(&self, accent: Accent) -> Color {
        self.accents.get(accent)
    }

    /// A foreground legible on top of `background`.
    ///
    /// Picks whichever end of the neutral ramp contrasts more, rather than
    /// branching on the theme's polarity. Polarity is not enough: in Latte both
    /// `crust` and `base` are light, so a polarity-based choice puts pale text
    /// on Latte's yellow accent. `crust` and `text` are the ramp's extremes and
    /// always straddle the midpoint in opposite directions, in every flavor and
    /// in any user theme that fills the slots sensibly.
    pub fn on_color(&self, background: Color) -> Color {
        let candidates = [self.neutrals.crust, self.neutrals.text];
        candidates
            .into_iter()
            .max_by(|a, b| {
                contrast_ratio(*a, background).total_cmp(&contrast_ratio(*b, background))
            })
            // `candidates` is a non-empty array literal, so this is unreachable.
            .unwrap_or(self.neutrals.text)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn rgb_literal_splits_channels() {
        let color = Color::rgb(0xcba6f7);
        assert_eq!(color, Color::new(0xcb, 0xa6, 0xf7));
    }

    #[test]
    fn hex_roundtrips() {
        let color = Color::rgb(0x1e1e2e);
        assert_eq!(color.to_hex(), "#1e1e2e");
        assert_eq!(Color::parse_hex("#1e1e2e").unwrap(), color);
        assert_eq!(Color::parse_hex("1e1e2e").unwrap(), color);
        assert_eq!(Color::parse_hex("#1E1E2E").unwrap(), color);
    }

    #[test]
    fn short_hex_expands() {
        assert_eq!(Color::parse_hex("#fff").unwrap(), Color::new(255, 255, 255));
        assert_eq!(Color::parse_hex("#08f").unwrap(), Color::new(0, 0x88, 0xff));
    }

    #[test]
    fn bad_hex_is_an_error_not_a_panic() {
        assert!(Color::parse_hex("#zzz").is_err());
        assert!(Color::parse_hex("#12345").is_err());
        assert!(Color::parse_hex("").is_err());
        assert!(Color::parse_hex("#").is_err());
    }

    #[test]
    fn accent_ids_are_unique_and_roundtrip() {
        let mut ids: Vec<&str> = Accent::ALL.iter().map(|a| a.id()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "accent ids must be unique");

        for accent in Accent::ALL {
            assert_eq!(Accent::from_id(accent.id()), Some(accent));
        }
        assert_eq!(Accent::from_id("chartreuse"), None);
    }

    #[test]
    fn accents_get_covers_every_slot() {
        // Every slot must return the value stored in its own field; a copy-paste
        // slip in `get` would silently mis-tint folder colors.
        let accents = Accents {
            rosewater: Color::rgb(0x000001),
            flamingo: Color::rgb(0x000002),
            pink: Color::rgb(0x000003),
            mauve: Color::rgb(0x000004),
            red: Color::rgb(0x000005),
            maroon: Color::rgb(0x000006),
            peach: Color::rgb(0x000007),
            yellow: Color::rgb(0x000008),
            green: Color::rgb(0x000009),
            teal: Color::rgb(0x00000a),
            sky: Color::rgb(0x00000b),
            sapphire: Color::rgb(0x00000c),
            blue: Color::rgb(0x00000d),
            lavender: Color::rgb(0x00000e),
        };
        let seen: Vec<u8> = Accent::ALL.into_iter().map(|a| accents.get(a).b).collect();
        assert_eq!(seen, (1..=14).collect::<Vec<u8>>());
    }

    #[test]
    fn color_serde_uses_hex_strings() {
        let toml = toml::to_string(&Neutrals {
            crust: Color::rgb(0x11111b),
            mantle: Color::rgb(0x181825),
            base: Color::rgb(0x1e1e2e),
            surface0: Color::rgb(0x313244),
            surface1: Color::rgb(0x45475a),
            surface2: Color::rgb(0x585b70),
            overlay0: Color::rgb(0x6c7086),
            overlay1: Color::rgb(0x7f849c),
            overlay2: Color::rgb(0x9399b2),
            subtext0: Color::rgb(0xa6adc8),
            subtext1: Color::rgb(0xbac2de),
            text: Color::rgb(0xcdd6f4),
        })
        .unwrap();
        assert!(toml.contains("crust = \"#11111b\""), "{toml}");

        let back: Neutrals = toml::from_str(&toml).unwrap();
        assert_eq!(back.base, Color::rgb(0x1e1e2e));
    }

    #[test]
    fn luminance_orders_light_above_dark() {
        assert!(
            Color::rgb(0xffffff).relative_luminance() > Color::rgb(0x000000).relative_luminance()
        );
        assert!(
            Color::rgb(0xf9e2af).relative_luminance() > Color::rgb(0x1e1e2e).relative_luminance()
        );
    }

    #[test]
    fn mix_interpolates_between_endpoints() {
        let a = Color::new(0, 0, 0);
        let b = Color::new(255, 255, 255);
        assert_eq!(a.mix(b, 0.0), a);
        assert_eq!(a.mix(b, 1.0), b);
        assert_eq!(a.mix(b, 0.5), Color::new(128, 128, 128));
        // Out-of-range factors clamp rather than wrapping or panicking.
        assert_eq!(a.mix(b, -3.0), a);
        assert_eq!(a.mix(b, 9.0), b);
    }
}
