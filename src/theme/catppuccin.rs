//! The four built-in Catppuccin flavors, as compile-time constants.
//!
//! These are the only themes Hive ships. They are plain [`Palette`] values with
//! no privileged status — the stylesheet generator cannot tell them apart from a
//! user-supplied theme, which is what keeps the theming system genuinely
//! extensible rather than Catppuccin-shaped.
//!
//! Values are the upstream Catppuccin palette
//! (<https://github.com/catppuccin/catppuccin>).

use std::borrow::Cow;

use super::palette::{Accents, Color, Neutrals, Palette};

pub const LATTE: Palette = Palette {
    id: Cow::Borrowed("latte"),
    name: Cow::Borrowed("Latte"),
    dark: false,
    accents: Accents {
        rosewater: Color::rgb(0xdc8a78),
        flamingo: Color::rgb(0xdd7878),
        pink: Color::rgb(0xea76cb),
        mauve: Color::rgb(0x8839ef),
        red: Color::rgb(0xd20f39),
        maroon: Color::rgb(0xe64553),
        peach: Color::rgb(0xfe640b),
        yellow: Color::rgb(0xdf8e1d),
        green: Color::rgb(0x40a02b),
        teal: Color::rgb(0x179299),
        sky: Color::rgb(0x04a5e5),
        sapphire: Color::rgb(0x209fb5),
        blue: Color::rgb(0x1e66f5),
        lavender: Color::rgb(0x7287fd),
    },
    neutrals: Neutrals {
        crust: Color::rgb(0xdce0e8),
        mantle: Color::rgb(0xe6e9ef),
        base: Color::rgb(0xeff1f5),
        surface0: Color::rgb(0xccd0da),
        surface1: Color::rgb(0xbcc0cc),
        surface2: Color::rgb(0xacb0be),
        overlay0: Color::rgb(0x9ca0b0),
        overlay1: Color::rgb(0x8c8fa1),
        overlay2: Color::rgb(0x7c7f93),
        subtext0: Color::rgb(0x6c6f85),
        subtext1: Color::rgb(0x5c5f77),
        text: Color::rgb(0x4c4f69),
    },
};

pub const FRAPPE: Palette = Palette {
    id: Cow::Borrowed("frappe"),
    name: Cow::Borrowed("Frappé"),
    dark: true,
    accents: Accents {
        rosewater: Color::rgb(0xf2d5cf),
        flamingo: Color::rgb(0xeebebe),
        pink: Color::rgb(0xf4b8e4),
        mauve: Color::rgb(0xca9ee6),
        red: Color::rgb(0xe78284),
        maroon: Color::rgb(0xea999c),
        peach: Color::rgb(0xef9f76),
        yellow: Color::rgb(0xe5c890),
        green: Color::rgb(0xa6d189),
        teal: Color::rgb(0x81c8be),
        sky: Color::rgb(0x99d1db),
        sapphire: Color::rgb(0x85c1dc),
        blue: Color::rgb(0x8caaee),
        lavender: Color::rgb(0xbabbf1),
    },
    neutrals: Neutrals {
        crust: Color::rgb(0x232634),
        mantle: Color::rgb(0x292c3c),
        base: Color::rgb(0x303446),
        surface0: Color::rgb(0x414559),
        surface1: Color::rgb(0x51576d),
        surface2: Color::rgb(0x626880),
        overlay0: Color::rgb(0x737994),
        overlay1: Color::rgb(0x838ba7),
        overlay2: Color::rgb(0x949cbb),
        subtext0: Color::rgb(0xa5adce),
        subtext1: Color::rgb(0xb5bfe2),
        text: Color::rgb(0xc6d0f5),
    },
};

pub const MACCHIATO: Palette = Palette {
    id: Cow::Borrowed("macchiato"),
    name: Cow::Borrowed("Macchiato"),
    dark: true,
    accents: Accents {
        rosewater: Color::rgb(0xf4dbd6),
        flamingo: Color::rgb(0xf0c6c6),
        pink: Color::rgb(0xf5bde6),
        mauve: Color::rgb(0xc6a0f6),
        red: Color::rgb(0xed8796),
        maroon: Color::rgb(0xee99a0),
        peach: Color::rgb(0xf5a97f),
        yellow: Color::rgb(0xeed49f),
        green: Color::rgb(0xa6da95),
        teal: Color::rgb(0x8bd5ca),
        sky: Color::rgb(0x91d7e3),
        sapphire: Color::rgb(0x7dc4e4),
        blue: Color::rgb(0x8aadf4),
        lavender: Color::rgb(0xb7bdf8),
    },
    neutrals: Neutrals {
        crust: Color::rgb(0x181926),
        mantle: Color::rgb(0x1e2030),
        base: Color::rgb(0x24273a),
        surface0: Color::rgb(0x363a4f),
        surface1: Color::rgb(0x494d64),
        surface2: Color::rgb(0x5b6078),
        overlay0: Color::rgb(0x6e738d),
        overlay1: Color::rgb(0x8087a2),
        overlay2: Color::rgb(0x939ab7),
        subtext0: Color::rgb(0xa5adcb),
        subtext1: Color::rgb(0xb8c0e0),
        text: Color::rgb(0xcad3f5),
    },
};

pub const MOCHA: Palette = Palette {
    id: Cow::Borrowed("mocha"),
    name: Cow::Borrowed("Mocha"),
    dark: true,
    accents: Accents {
        rosewater: Color::rgb(0xf5e0dc),
        flamingo: Color::rgb(0xf2cdcd),
        pink: Color::rgb(0xf5c2e7),
        mauve: Color::rgb(0xcba6f7),
        red: Color::rgb(0xf38ba8),
        maroon: Color::rgb(0xeba0ac),
        peach: Color::rgb(0xfab387),
        yellow: Color::rgb(0xf9e2af),
        green: Color::rgb(0xa6e3a1),
        teal: Color::rgb(0x94e2d5),
        sky: Color::rgb(0x89dceb),
        sapphire: Color::rgb(0x74c7ec),
        blue: Color::rgb(0x89b4fa),
        lavender: Color::rgb(0xb4befe),
    },
    neutrals: Neutrals {
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
    },
};

/// All built-in flavors, in the order the switcher lists them (light to dark).
pub const BUILT_IN: [&Palette; 4] = [&LATTE, &FRAPPE, &MACCHIATO, &MOCHA];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::theme::palette::Accent;

    #[test]
    fn built_in_ids_are_unique() {
        let mut ids: Vec<&str> = BUILT_IN.iter().map(|p| p.id.as_ref()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn only_latte_is_light() {
        let light: Vec<&str> = BUILT_IN
            .iter()
            .filter(|p| !p.dark)
            .map(|p| p.id.as_ref())
            .collect();
        assert_eq!(light, ["latte"]);
    }

    #[test]
    fn every_flavor_defines_every_accent_distinctly() {
        // Guards against a copy-paste slip leaving two slots with the same value.
        for palette in BUILT_IN {
            let mut seen: Vec<String> = Accent::ALL
                .into_iter()
                .map(|a| palette.accent(a).to_hex())
                .collect();
            let count = seen.len();
            seen.sort();
            seen.dedup();
            assert_eq!(
                seen.len(),
                count,
                "duplicate accent value in {}",
                palette.id
            );
        }
    }

    #[test]
    fn elevation_steps_below_base_get_progressively_deeper() {
        // crust -> mantle -> base is the "below the content surface" run, and it
        // brightens in *both* polarities: in Latte, crust is a shade darker than
        // base rather than lighter. The header bar and sidebar sit on mantle, so
        // an inversion here would make them read as raised instead of recessed.
        for palette in BUILT_IN {
            let n = &palette.neutrals;
            let steps = [n.crust, n.mantle, n.base];
            for pair in steps.windows(2) {
                assert!(
                    pair[1].relative_luminance() > pair[0].relative_luminance(),
                    "{} elevation run is not ascending",
                    palette.id
                );
            }
        }
    }

    #[test]
    fn the_content_ramp_runs_monotonically_from_base_to_text() {
        // base -> surface -> overlay -> subtext -> text moves away from the
        // background toward the foreground. In a dark flavor that means getting
        // lighter; in Latte, darker. Either way it must be monotonic, or
        // elevation shading inverts partway and surfaces stop reading as
        // layered.
        for palette in BUILT_IN {
            let n = &palette.neutrals;
            let ramp = [
                n.base, n.surface0, n.surface1, n.surface2, n.overlay0, n.overlay1, n.overlay2,
                n.subtext0, n.subtext1, n.text,
            ];
            for pair in ramp.windows(2) {
                let (from, to) = (pair[0].relative_luminance(), pair[1].relative_luminance());
                if palette.dark {
                    assert!(to > from, "{} content ramp not ascending", palette.id);
                } else {
                    assert!(to < from, "{} content ramp not descending", palette.id);
                }
            }
        }
    }

    #[test]
    fn every_accent_gets_a_readable_foreground() {
        // Selection fills and suggested buttons draw text on an accent. The
        // chosen foreground must be the better of the ramp's two extremes for
        // all 14 accents in all 4 flavors — this is the assertion that fails if
        // on_color goes back to branching on theme polarity.
        use crate::theme::palette::contrast_ratio;

        for palette in BUILT_IN {
            for accent in Accent::ALL {
                let bg = palette.accent(accent);
                let fg = palette.on_color(bg);
                let chosen = contrast_ratio(fg, bg);
                let alternative = if fg == palette.neutrals.crust {
                    contrast_ratio(palette.neutrals.text, bg)
                } else {
                    contrast_ratio(palette.neutrals.crust, bg)
                };
                assert!(
                    chosen >= alternative,
                    "{} {} picked the lower-contrast foreground ({chosen:.2} < {alternative:.2})",
                    palette.id,
                    accent.id()
                );
            }
        }
    }

    #[test]
    fn mocha_matches_upstream_spot_values() {
        assert_eq!(MOCHA.neutrals.base.to_hex(), "#1e1e2e");
        assert_eq!(MOCHA.accent(Accent::Mauve).to_hex(), "#cba6f7");
        assert_eq!(LATTE.neutrals.base.to_hex(), "#eff1f5");
        assert_eq!(LATTE.accent(Accent::Mauve).to_hex(), "#8839ef");
    }
}
