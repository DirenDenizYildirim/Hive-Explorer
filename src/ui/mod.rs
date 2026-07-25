//! Widgets. Everything in here touches GTK; the rules they enforce live in
//! `crate::model`, `crate::config`, and `crate::theme`.

pub mod breadcrumb;
pub mod debounce;
pub mod file_pane;
pub mod sidebar;
pub mod status_bar;
pub mod window;

pub use window::Window;
