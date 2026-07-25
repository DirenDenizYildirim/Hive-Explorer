//! Widgets. This layer touches GTK and holds no policy of its own.

pub mod breadcrumb;
pub mod clipboard;
pub mod color_picker;
pub mod context_menu;
pub mod debounce;
pub mod dialogs;
pub mod dnd;
pub mod file_pane;
pub mod folders;
pub mod keys;
pub mod operations;
pub mod path_entry;
pub mod preferences;
pub mod progress;
pub mod sidebar;
pub mod status_bar;
pub mod window;

pub use window::Window;
