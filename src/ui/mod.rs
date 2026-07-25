//! Widgets. This layer touches GTK and holds no policy of its own.

pub mod breadcrumb;
pub mod clipboard;
pub mod context_menu;
pub mod debounce;
pub mod dialogs;
pub mod file_pane;
pub mod keys;
pub mod operations;
pub mod path_entry;
pub mod progress;
pub mod sidebar;
pub mod status_bar;
pub mod window;

pub use window::Window;
