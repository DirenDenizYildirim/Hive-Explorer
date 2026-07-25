//! Human-readable formatting for sizes and counts.
//!
//! Plain Rust, no GTK, so the rounding rules are unit-tested rather than eyeballed.

/// Format a byte count the way file managers do: binary units, at most one
/// decimal place, no decimal on whole units or on bytes.
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

    // One decimal below 10 (2.4 GiB), none above (241 GiB) — the extra digit
    // stops carrying information once the integer part is large.
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
    fn counts_are_pluralized() {
        assert_eq!(item_count(0), "0 items");
        assert_eq!(item_count(1), "1 item");
        assert_eq!(item_count(2), "2 items");
        assert_eq!(selection_count(0), "");
        assert_eq!(selection_count(1), "1 selected");
        assert_eq!(selection_count(9), "9 selected");
    }
}
