//! Human-readable formatting for sizes and counts.

/// Binary units, at most one decimal place.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    const STEP: f64 = 1024.0;

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= STEP && unit + 1 < UNITS.len() {
        value /= STEP;
        unit += 1;
    }

    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{} {}", value.round() as u64, UNITS[unit])
    }
}

/// `"1 item"` / `"12 items"`.
pub fn item_count(count: usize) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

/// `"3 selected"`, or empty when nothing is selected.
pub fn selection_count(count: usize) -> String {
    if count == 0 {
        String::new()
    } else {
        format!("{count} selected")
    }
}

/// A Unix mode as `rwxr-xr-x`, for the Properties dialog.
///
/// Only the permission bits; the file type is shown as its own row rather than
/// as a leading character, since the dialog already says what the thing is.
/// The special bits are rendered where `ls` puts them — an `s` or `t` in place
/// of the execute bit, capitalised when the execute bit is not actually set,
/// because "setuid but not executable" is worth being able to see.
pub fn permissions(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    let triples = [
        (mode >> 6, mode & 0o4000, 's'),
        (mode >> 3, mode & 0o2000, 's'),
        (mode, mode & 0o1000, 't'),
    ];

    for (bits, special, marker) in triples {
        out.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        out.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        out.push(match (bits & 0o1 != 0, special != 0) {
            (true, false) => 'x',
            (true, true) => marker,
            (false, true) => marker.to_ascii_uppercase(),
            (false, false) => '-',
        });
    }

    out
}

/// The same mode as four octal digits, which is what `chmod` wants.
pub fn permissions_octal(mode: u32) -> String {
    format!("{:04o}", mode & 0o7777)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn bytes_below_a_kibibyte_stay_plain() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1), "1 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1023), "1023 B");
    }

    #[test]
    fn units_step_at_1024() {
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(human_bytes(1024 * 1024 * 1024), "1.0 GiB");
    }

    #[test]
    fn large_values_drop_the_decimal() {
        assert_eq!(human_bytes(15 * 1024), "15 KiB");
        assert_eq!(human_bytes(241 * 1024 * 1024 * 1024), "241 GiB");
    }

    #[test]
    fn multi_gigabyte_and_max_values_do_not_overflow_or_panic() {
        let out = human_bytes(u64::MAX);
        assert!(out.ends_with("PiB"), "{out}");
        assert!(human_bytes(u64::MAX / 2).ends_with("PiB"));
    }

    #[test]
    fn permission_bits_read_like_ls() {
        assert_eq!(permissions(0o755), "rwxr-xr-x");
        assert_eq!(permissions(0o644), "rw-r--r--");
        assert_eq!(permissions(0o600), "rw-------");
        assert_eq!(permissions(0o777), "rwxrwxrwx");
        assert_eq!(permissions(0o000), "---------");
    }

    #[test]
    fn the_file_type_bits_are_ignored() {
        // 0o100644 is a regular file; 0o40755 is a directory.
        assert_eq!(permissions(0o100_644), "rw-r--r--");
        assert_eq!(permissions(0o040_755), "rwxr-xr-x");
        assert_eq!(permissions_octal(0o100_644), "0644");
    }

    #[test]
    fn special_bits_show_where_ls_puts_them() {
        assert_eq!(permissions(0o4755), "rwsr-xr-x", "setuid");
        assert_eq!(permissions(0o2755), "rwxr-sr-x", "setgid");
        assert_eq!(permissions(0o1777), "rwxrwxrwt", "sticky, as on /tmp");
        assert_eq!(
            permissions(0o4644),
            "rwSr--r--",
            "setuid without execute is capitalised"
        );
        assert_eq!(permissions(0o1666), "rw-rw-rwT");
    }

    #[test]
    fn octal_keeps_four_digits_and_drops_the_type() {
        assert_eq!(permissions_octal(0o755), "0755");
        assert_eq!(permissions_octal(0o7), "0007");
        assert_eq!(permissions_octal(0o4755), "4755");
        assert_eq!(permissions_octal(0), "0000");
        assert_eq!(permissions_octal(u32::MAX), "7777");
    }

    #[test]
    fn counts_are_pluralized() {
        assert_eq!(item_count(0), "0 items");
        assert_eq!(item_count(1), "1 item");
        assert_eq!(item_count(2), "2 items");
        assert_eq!(selection_count(0), "");
        assert_eq!(selection_count(1), "1 selected");
        assert_eq!(selection_count(9), "9 selected");
    }
}
