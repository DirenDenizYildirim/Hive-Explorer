//! Sort comparators.
//!
//! Plain Rust with no GTK types, so `cargo test` exercises the ordering rules
//! without a display. The `gtk::CustomSorter` shim in `ui::file_pane` does
//! nothing but read fields off a `gio::FileInfo` into [`SortKeyData`] and call
//! [`compare`].

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// Which column the view is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
    Type,
}

impl SortKey {
    pub const ALL: [SortKey; 4] = [
        SortKey::Name,
        SortKey::Size,
        SortKey::Modified,
        SortKey::Type,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Modified => "modified",
            SortKey::Type => "type",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Size => "Size",
            SortKey::Modified => "Modified",
            SortKey::Type => "Type",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.id() == id)
    }
}

/// Ascending or descending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

impl SortOrder {
    pub const fn is_descending(self) -> bool {
        matches!(self, SortOrder::Descending)
    }

    pub const fn toggled(self) -> Self {
        match self {
            SortOrder::Ascending => SortOrder::Descending,
            SortOrder::Descending => SortOrder::Ascending,
        }
    }
}

/// The complete ordering configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SortSpec {
    pub key: SortKey,
    pub order: SortOrder,
    /// Directories group ahead of files regardless of the active key.
    pub folders_first: bool,
}

impl SortSpec {
    pub const fn new(key: SortKey, order: SortOrder, folders_first: bool) -> Self {
        Self {
            key,
            order,
            folders_first,
        }
    }
}

/// The fields an ordering decision needs, borrowed from whatever holds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKeyData<'a> {
    pub name: &'a str,
    pub is_dir: bool,
    /// Bytes. Directories report 0 and are ordered by name instead.
    pub size: i64,
    /// Unix seconds. Unknown timestamps should be 0.
    pub modified: i64,
    pub content_type: &'a str,
}

impl<'a> SortKeyData<'a> {
    pub fn new(name: &'a str, is_dir: bool) -> Self {
        Self {
            name,
            is_dir,
            size: 0,
            modified: 0,
            content_type: "",
        }
    }
}

/// Order two entries under `spec`.
///
/// The folders-first grouping is applied *outside* the ascending/descending
/// flip: reversing the sort must not sink directories below files, which is
/// what users mean by "folders first".
pub fn compare(a: &SortKeyData<'_>, b: &SortKeyData<'_>, spec: SortSpec) -> Ordering {
    if spec.folders_first && a.is_dir != b.is_dir {
        return if a.is_dir {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    let primary = match spec.key {
        SortKey::Name => Ordering::Equal,
        SortKey::Size => {
            // Sizing directories against files is meaningless — a directory's
            // own inode size says nothing about its contents, and computing the
            // real answer is an unbounded tree walk we refuse to do implicitly.
            // Group directories together and fall through to name.
            if a.is_dir && b.is_dir {
                Ordering::Equal
            } else {
                a.size.cmp(&b.size)
            }
        }
        SortKey::Modified => a.modified.cmp(&b.modified),
        SortKey::Type => natural_cmp(a.content_type, b.content_type),
    };

    let primary = if spec.order.is_descending() {
        primary.reverse()
    } else {
        primary
    };

    if primary != Ordering::Equal {
        return primary;
    }

    // Name is the tie-break for every key, and the only key for SortKey::Name.
    let by_name = natural_cmp(a.name, b.name);
    if spec.order.is_descending() {
        by_name.reverse()
    } else {
        by_name
    }
}

/// Human-friendly comparison: case-insensitive, with digit runs compared as
/// numbers so `file2` sorts before `file10`.
///
/// Ties broken by the raw bytes so the ordering is total and therefore stable —
/// a comparator that returns `Equal` for distinct names makes the sorted view
/// reshuffle on unrelated updates.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();

    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => break,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(lc), Some(rc)) => {
                if lc.is_ascii_digit() && rc.is_ascii_digit() {
                    let (l_digits, l_zeros) = take_number(&mut left);
                    let (r_digits, r_zeros) = take_number(&mut right);

                    // Compare by length first, then lexically: this compares
                    // arbitrarily long runs numerically without overflowing.
                    let by_number = l_digits
                        .len()
                        .cmp(&r_digits.len())
                        .then_with(|| l_digits.cmp(&r_digits));
                    if by_number != Ordering::Equal {
                        return by_number;
                    }
                    // Equal numerically: fewer leading zeros first, so `01`
                    // and `1` still order deterministically.
                    let by_zeros = l_zeros.cmp(&r_zeros);
                    if by_zeros != Ordering::Equal {
                        return by_zeros;
                    }
                } else {
                    left.next();
                    right.next();
                    let by_fold = fold(lc).cmp(&fold(rc));
                    if by_fold != Ordering::Equal {
                        return by_fold;
                    }
                }
            }
        }
    }

    // Case-folded equal: fall back to the raw form so `Foo` and `foo` have a
    // stable, total order rather than comparing equal.
    a.cmp(b)
}

/// Consume a digit run, returning its significant digits and how many leading
/// zeros preceded them.
fn take_number(iter: &mut std::iter::Peekable<std::str::Chars<'_>>) -> (String, usize) {
    let mut zeros = 0usize;
    while iter.peek().is_some_and(|c| *c == '0') {
        iter.next();
        zeros += 1;
    }

    let mut digits = String::new();
    while let Some(c) = iter.peek().copied() {
        if c.is_ascii_digit() {
            digits.push(c);
            iter.next();
        } else {
            break;
        }
    }

    (digits, zeros)
}

/// Lowercase for comparison. Uses simple case folding, which is correct for the
/// filename cases that matter and avoids allocating per character.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn dir(name: &str) -> SortKeyData<'_> {
        SortKeyData::new(name, true)
    }

    fn file(name: &str) -> SortKeyData<'_> {
        SortKeyData::new(name, false)
    }

    fn sorted<'a>(mut items: Vec<SortKeyData<'a>>, spec: SortSpec) -> Vec<&'a str> {
        items.sort_by(|a, b| compare(a, b, spec));
        items.into_iter().map(|i| i.name).collect()
    }

    #[test]
    fn natural_order_treats_digit_runs_as_numbers() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("file10", "file2"), Ordering::Greater);
        assert_eq!(natural_cmp("a1b2", "a1b10"), Ordering::Less);
        assert_eq!(natural_cmp("img12.png", "img9.png"), Ordering::Greater);
    }

    #[test]
    fn very_long_digit_runs_do_not_overflow() {
        // A 40-digit number would overflow u64/u128 parsing; length-then-lexical
        // comparison handles it.
        let a = format!("f{}1", "9".repeat(40));
        let b = format!("f{}2", "9".repeat(40));
        assert_eq!(natural_cmp(&a, &b), Ordering::Less);

        let short = "f9".to_owned();
        let long = format!("f{}", "1".repeat(30));
        assert_eq!(natural_cmp(&short, &long), Ordering::Less);
    }

    #[test]
    fn name_order_is_case_insensitive_but_total() {
        assert_eq!(natural_cmp("apple", "Banana"), Ordering::Less);
        assert_eq!(natural_cmp("Apple", "banana"), Ordering::Less);
        // Same folded form must still be a strict order, never Equal.
        assert_ne!(natural_cmp("Foo", "foo"), Ordering::Equal);
        assert_eq!(natural_cmp("foo", "foo"), Ordering::Equal);
    }

    #[test]
    fn leading_zeros_order_deterministically() {
        assert_ne!(natural_cmp("01", "1"), Ordering::Equal);
        assert_eq!(natural_cmp("1", "01"), Ordering::Less);
    }

    #[test]
    fn comparator_is_antisymmetric() {
        let spec = SortSpec::new(SortKey::Name, SortOrder::Ascending, true);
        let samples = [
            dir("src"),
            file("a.txt"),
            file("B.txt"),
            file("file2"),
            file("file10"),
            dir("Docs"),
        ];
        for a in &samples {
            for b in &samples {
                assert_eq!(
                    compare(a, b, spec).reverse(),
                    compare(b, a, spec),
                    "not antisymmetric for {:?} vs {:?}",
                    a.name,
                    b.name
                );
            }
        }
    }

    #[test]
    fn folders_come_first_in_both_directions() {
        let items = vec![file("b.txt"), dir("zed"), file("a.txt"), dir("alpha")];

        let asc = SortSpec::new(SortKey::Name, SortOrder::Ascending, true);
        assert_eq!(
            sorted(items.clone(), asc),
            ["alpha", "zed", "a.txt", "b.txt"]
        );

        // Reversing must reverse within each group, not sink folders below files.
        let desc = SortSpec::new(SortKey::Name, SortOrder::Descending, true);
        assert_eq!(
            sorted(items.clone(), desc),
            ["zed", "alpha", "b.txt", "a.txt"]
        );

        let mixed = SortSpec::new(SortKey::Name, SortOrder::Ascending, false);
        assert_eq!(sorted(items, mixed), ["a.txt", "alpha", "b.txt", "zed"]);
    }

    #[test]
    fn size_sorts_numerically_and_groups_directories() {
        let mut big = file("big");
        big.size = 1_000_000;
        let mut small = file("small");
        small.size = 12;
        let mut empty = file("empty");
        empty.size = 0;
        let folder = dir("folder");

        let spec = SortSpec::new(SortKey::Size, SortOrder::Ascending, true);
        let items = vec![big, small, empty, folder];
        assert_eq!(sorted(items, spec), ["folder", "empty", "small", "big"]);
    }

    #[test]
    fn directories_never_compare_by_their_own_size() {
        // Two directories differ only in reported inode size; they must order by
        // name, not by that meaningless number.
        let mut a = dir("zebra");
        a.size = 4096;
        let mut b = dir("alpha");
        b.size = 20_480;
        let spec = SortSpec::new(SortKey::Size, SortOrder::Ascending, true);
        assert_eq!(sorted(vec![a, b], spec), ["alpha", "zebra"]);
    }

    #[test]
    fn modified_sorts_by_timestamp() {
        let mut old = file("old");
        old.modified = 1_000;
        let mut new = file("new");
        new.modified = 2_000;
        let mut unknown = file("unknown");
        unknown.modified = 0;

        let spec = SortSpec::new(SortKey::Modified, SortOrder::Descending, false);
        assert_eq!(
            sorted(vec![old, new, unknown], spec),
            ["new", "old", "unknown"]
        );
    }

    #[test]
    fn type_sorts_by_content_type_then_name() {
        let mut a = file("b.png");
        a.content_type = "image/png";
        let mut b = file("a.png");
        b.content_type = "image/png";
        let mut c = file("z.txt");
        c.content_type = "text/plain";

        let spec = SortSpec::new(SortKey::Type, SortOrder::Ascending, false);
        assert_eq!(sorted(vec![c, a, b], spec), ["a.png", "b.png", "z.txt"]);
    }

    #[test]
    fn equal_primary_keys_fall_back_to_name() {
        let mut a = file("zebra");
        a.size = 100;
        let mut b = file("alpha");
        b.size = 100;
        let spec = SortSpec::new(SortKey::Size, SortOrder::Ascending, false);
        assert_eq!(sorted(vec![a, b], spec), ["alpha", "zebra"]);
    }

    #[test]
    fn names_with_newlines_and_odd_bytes_still_order() {
        // Invalid UTF-8 arrives lossily converted; the comparator must not care.
        let weird = ["a\nb", "a b", "a\tb", "a\u{fffd}b", ""];
        let items: Vec<SortKeyData<'_>> = weird.iter().map(|n| file(n)).collect();
        let spec = SortSpec::new(SortKey::Name, SortOrder::Ascending, false);
        let out = sorted(items, spec);
        assert_eq!(out.len(), weird.len());
    }

    #[test]
    fn sort_key_ids_roundtrip() {
        for key in SortKey::ALL {
            assert_eq!(SortKey::from_id(key.id()), Some(key));
        }
        assert_eq!(SortKey::from_id("nonsense"), None);
    }

    #[test]
    fn order_toggles() {
        assert_eq!(SortOrder::Ascending.toggled(), SortOrder::Descending);
        assert_eq!(SortOrder::Descending.toggled(), SortOrder::Ascending);
        assert!(!SortOrder::Ascending.is_descending());
    }
}
