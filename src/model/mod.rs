//! Pure logic shared by the UI: ordering, filtering, path handling, formatting.

pub mod completion;
pub mod filter;
pub mod format;
pub mod history;
pub mod path;
pub mod sort;

pub use filter::{FilterInput, FilterSpec};
pub use history::History;
pub use sort::{SortKey, SortKeyData, SortOrder, SortSpec};
