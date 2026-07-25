//! Pure logic shared by the UI: ordering, filtering, path handling, formatting.
//!
//! Nothing in this module touches GTK, gio, or a main context. That is what lets
//! `cargo test` cover the rules that decide what the user sees — the sort
//! comparators, the hidden-file predicate, and the containment checks that stand
//! between a copy operation and a recursive data-eater — without a display.

pub mod filter;
pub mod format;
pub mod path;
pub mod sort;

pub use filter::{FilterInput, FilterSpec};
pub use sort::{SortKey, SortKeyData, SortOrder, SortSpec};
