//! First-launch defaults, in one place.
//!
//! The flavor is explicit rather than derived from the system appearance
//! portal, so first run never depends on a portal reply that may not come.

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

/// Coalescing window for view-derived UI (status counts, selection totals) and
/// for directory-monitor churn.
pub const DEBOUNCE_MS: u32 = 150;

/// Sidebar collapses to an overlay below this window width.
pub const SIDEBAR_BREAKPOINT_PX: i32 = 640;

/// Sidebar width when expanded.
pub const SIDEBAR_WIDTH_PX: i32 = 220;
