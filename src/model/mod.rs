//! Pure logic shared by the UI: ordering, filtering, path handling, formatting.

pub mod clipboard;
pub mod completion;
pub mod filter;
pub mod format;
pub mod history;
pub mod naming;
pub mod path;
pub mod pins;
pub mod preflight;
pub mod sort;
pub mod trashinfo;
pub mod undo;
pub mod uri;

pub use filter::{FilterInput, FilterSpec};
pub use history::History;
pub use sort::{SortKey, SortKeyData, SortOrder, SortSpec};
