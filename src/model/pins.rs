//! The pinned-folder list: adding, removing, and reordering.
//!
//! The list is an ordered set — order is the user's, and a folder appears once.
//! Every operation here is a plain function over a `Vec<PathBuf>` so that the
//! sidebar's drag-and-drop, which is easy to get subtly wrong, is testable
//! without a display.

use std::path::{Path, PathBuf};

/// Drop duplicates and non-absolute entries, keeping the first of each.
///
/// Applied to whatever the config file happened to contain, so a hand-edited
/// list cannot produce two rows for one folder.
pub fn normalize(list: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(list.len());
    for path in list {
        if path.is_absolute() && !out.iter().any(|existing| existing == path) {
            out.push(path.clone());
        }
    }
    out
}

/// True when `path` is already pinned.
pub fn contains(list: &[PathBuf], path: &Path) -> bool {
    list.iter().any(|existing| existing == path)
}

/// Append `paths` that are not pinned yet. Returns how many were added.
pub fn add(list: &mut Vec<PathBuf>, paths: &[PathBuf]) -> usize {
    insert(list, paths, list.len())
}

/// Insert `paths` at `index`, moving any that were already pinned.
///
/// `index` is read against the list **as the user sees it**: it names the row
/// the block lands in front of, and `list.len()` means the end. That is the
/// only interpretation that matches a drop indicator drawn between two rows —
/// lifting the dragged rows out first would otherwise shift the target up
/// underneath them, and the drop would land one place too far down for each
/// row being moved.
///
/// Returns how many rows the list gained. Moving an existing pin is a reorder,
/// not an addition, so it counts zero — which is what lets the caller say
/// "Pinned 2 folders" without lying about the third that merely moved.
pub fn insert(list: &mut Vec<PathBuf>, paths: &[PathBuf], index: usize) -> usize {
    let incoming = normalize(paths);
    if incoming.is_empty() {
        return 0;
    }

    // How many entries before the insertion point are about to be lifted out.
    // Without this the removal shifts everything left and the block lands one
    // position too far down for each one.
    let lifted_before = list
        .iter()
        .take(index.min(list.len()))
        .filter(|existing| contains(&incoming, existing))
        .count();

    let added = incoming.iter().filter(|path| !contains(list, path)).count();

    list.retain(|existing| !contains(&incoming, existing));

    let at = index
        .min(list.len() + lifted_before)
        .saturating_sub(lifted_before);
    let at = at.min(list.len());
    for (offset, path) in incoming.into_iter().enumerate() {
        list.insert(at + offset, path);
    }

    added
}

/// Remove `path`. Returns whether it was there.
pub fn remove(list: &mut Vec<PathBuf>, path: &Path) -> bool {
    let before = list.len();
    list.retain(|existing| existing != path);
    list.len() != before
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn adding_appends_and_never_duplicates() {
        let mut list = paths(&["/a", "/b"]);

        assert_eq!(add(&mut list, &paths(&["/c"])), 1);
        assert_eq!(list, paths(&["/a", "/b", "/c"]));

        assert_eq!(add(&mut list, &paths(&["/b"])), 0, "already pinned");
        assert_eq!(
            list,
            paths(&["/a", "/c", "/b"]),
            "re-adding moves to the end"
        );
    }

    #[test]
    fn adding_several_at_once_keeps_their_order() {
        let mut list = paths(&["/a"]);
        assert_eq!(add(&mut list, &paths(&["/b", "/c", "/d"])), 3);
        assert_eq!(list, paths(&["/a", "/b", "/c", "/d"]));
    }

    #[test]
    fn a_relative_path_is_never_pinned() {
        let mut list = Vec::new();
        assert_eq!(add(&mut list, &paths(&["relative/thing"])), 0);
        assert!(list.is_empty());
    }

    #[test]
    fn removing_reports_whether_it_was_there() {
        let mut list = paths(&["/a", "/b"]);
        assert!(remove(&mut list, Path::new("/a")));
        assert_eq!(list, paths(&["/b"]));
        assert!(!remove(&mut list, Path::new("/a")));
    }

    #[test]
    fn inserting_a_new_folder_lands_at_the_index() {
        let mut list = paths(&["/a", "/b", "/c"]);
        assert_eq!(insert(&mut list, &paths(&["/new"]), 0), 1);
        assert_eq!(list, paths(&["/new", "/a", "/b", "/c"]));

        assert_eq!(insert(&mut list, &paths(&["/end"]), 99), 1);
        assert_eq!(list, paths(&["/new", "/a", "/b", "/c", "/end"]));
    }

    /// Dropping a row onto a position *below* itself must land where the user
    /// aimed. Lifting it out first shifts the target up by one, so the insert
    /// point is compensated; getting this wrong is the classic off-by-one that
    /// makes a reorder feel like it ignores the drop.
    #[test]
    fn dragging_a_pin_downwards_lands_where_it_was_dropped() {
        // Dropped on the top half of the row at index 2 — in front of "/c".
        let mut list = paths(&["/a", "/b", "/c", "/d"]);
        assert_eq!(
            insert(&mut list, &paths(&["/a"]), 2),
            0,
            "a move, not an add"
        );
        assert_eq!(list, paths(&["/b", "/a", "/c", "/d"]));

        // Dropped on the bottom half of that same row — after "/c".
        let mut list = paths(&["/a", "/b", "/c", "/d"]);
        assert_eq!(insert(&mut list, &paths(&["/a"]), 3), 0);
        assert_eq!(list, paths(&["/b", "/c", "/a", "/d"]));
    }

    /// Two rows moving together must stay adjacent and in order, and must not
    /// each drag the landing point one row further along.
    #[test]
    fn dragging_two_pins_downwards_keeps_them_together() {
        let mut list = paths(&["/a", "/b", "/c", "/d", "/e"]);
        assert_eq!(insert(&mut list, &paths(&["/a", "/b"]), 4), 0);
        assert_eq!(list, paths(&["/c", "/d", "/a", "/b", "/e"]));
    }

    #[test]
    fn dragging_a_pin_upwards_lands_where_it_was_dropped() {
        let mut list = paths(&["/a", "/b", "/c", "/d"]);
        assert_eq!(insert(&mut list, &paths(&["/d"]), 1), 0);
        assert_eq!(list, paths(&["/a", "/d", "/b", "/c"]));
    }

    #[test]
    fn dropping_a_pin_onto_itself_changes_nothing() {
        let mut list = paths(&["/a", "/b", "/c"]);
        assert_eq!(insert(&mut list, &paths(&["/b"]), 1), 0);
        assert_eq!(list, paths(&["/a", "/b", "/c"]));
    }

    #[test]
    fn dragging_to_the_end_moves_rather_than_duplicating() {
        let mut list = paths(&["/a", "/b", "/c"]);
        assert_eq!(insert(&mut list, &paths(&["/a"]), 3), 0);
        assert_eq!(list, paths(&["/b", "/c", "/a"]));
    }

    #[test]
    fn a_mixed_drop_moves_the_pinned_and_adds_the_rest_as_one_block() {
        let mut list = paths(&["/a", "/b", "/c"]);
        assert_eq!(insert(&mut list, &paths(&["/c", "/new"]), 1), 1);
        assert_eq!(list, paths(&["/a", "/c", "/new", "/b"]));
    }

    #[test]
    fn normalizing_drops_duplicates_and_relative_entries() {
        let list = paths(&["/a", "/a", "relative", "/b"]);
        assert_eq!(normalize(&list), paths(&["/a", "/b"]));
    }

    #[test]
    fn inserting_nothing_is_not_a_reorder() {
        let mut list = paths(&["/a", "/b"]);
        assert_eq!(insert(&mut list, &[], 0), 0);
        assert_eq!(list, paths(&["/a", "/b"]));
    }
}
