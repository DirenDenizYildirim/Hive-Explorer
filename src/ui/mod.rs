//! Widgets. This layer touches GTK and holds no policy of its own.

pub mod breadcrumb;
pub mod debounce;
pub mod file_pane;
pub mod sidebar;
pub mod status_bar;
pub mod window;

pub use window::Window;
