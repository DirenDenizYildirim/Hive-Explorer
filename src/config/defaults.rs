//! First-launch defaults, in one place.

use crate::theme::palette::Accent;

/// Startup flavor when `follow_system` is off, which is the default.
pub const FLAVOR: &str = "mocha";

/// Startup UI accent.
pub const ACCENT: Accent = Accent::Mauve;

/// Flavor used when following the system and it reports light.
pub const LIGHT_FLAVOR: &str = "latte";

/// Flavor used when following the system and it reports dark.
pub const DARK_FLAVOR: &str = "mocha";

/// Longest thumbnail edge, in pixels.
pub const THUMBNAIL_MAX_PIXELS: u32 = 256;

/// Skip thumbnailing source files larger than 32 MiB.
pub const THUMBNAIL_MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// Disable thumbnailing in directories with more entries than this.
pub const THUMBNAIL_MAX_DIRECTORY_ENTRIES: usize = 2000;

/// Coalescing window for view-derived UI and directory-monitor churn.
pub const DEBOUNCE_MS: u32 = 150;

/// Every animation in Hive, in milliseconds.
///
/// Inside the 120–180 ms budget, and one number rather than several so nothing
/// can drift into feeling slower than the thing next to it. Whether an
/// animation runs at all is `gtk-enable-animations`, which is honoured
/// separately: this is the duration, not the decision.
pub const TRANSITION_MS: u32 = 150;

/// Sidebar collapses to an overlay below this window width.
pub const SIDEBAR_BREAKPOINT_PX: i32 = 640;

/// Sidebar width when expanded.
pub const SIDEBAR_WIDTH_PX: i32 = 220;
