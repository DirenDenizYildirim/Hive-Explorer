//! Thumbnail eligibility, cache addressing, and staleness.
//!
//! No GTK and no I/O: everything here is a decision the worker pool and the
//! bind callback both have to agree on, so it lives where `cargo test` can
//! reach it. §10.1 hazard 5 — an edited image showing its old thumbnail forever
//! — is settled here, by keying the cache on **(path, mtime, size)** rather
//! than on the path alone.

use std::path::{Path, PathBuf};

/// PNG text chunks the cached thumbnail carries so freshness can be checked.
///
/// The names follow the freedesktop thumbnail spec, which stores the same three
/// facts, so a cached file is self-describing rather than depending on the
/// directory layout to say what it came from.
pub const KEY_URI: &str = "tEXt::Thumb::URI";
pub const KEY_MTIME: &str = "tEXt::Thumb::MTime";
pub const KEY_SIZE: &str = "tEXt::Thumb::Size";

/// Whether a row is worth a thumbnail, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    Yes,
    /// Not an image. Video thumbnails are out of scope for v1.
    NotAnImage,
    /// Over the configured source-size cap.
    TooLarge,
}

/// Decide whether to thumbnail one entry.
///
/// The size cap is checked against the source file, not the decoded image: a
/// 40 MiB PNG is cheap to skip and expensive to decode, and the point of the
/// cap is to keep a directory of camera raws from stalling the pool.
pub fn eligibility(content_type: &str, bytes: u64, max_file_bytes: u64) -> Eligibility {
    if !is_image(content_type) {
        return Eligibility::NotAnImage;
    }
    if bytes > max_file_bytes {
        return Eligibility::TooLarge;
    }
    Eligibility::Yes
}

/// True for content types gdk-pixbuf might plausibly decode.
///
/// A type passing this test is not a promise: whether `image/svg+xml` loads at
/// all depends on which pixbuf loaders are installed. A failed decode is
/// remembered as "no thumbnail" and never retried, so guessing generously here
/// costs one attempt rather than a loop.
pub fn is_image(content_type: &str) -> bool {
    content_type.starts_with("image/")
}

/// Where the cached thumbnail for `source` lives under `root`.
///
/// One file per source path, in a fan-out directory so a cache of thousands
/// does not become one directory of thousands. Because the name depends only on
/// the path, re-thumbnailing an edited image *replaces* its entry instead of
/// adding a second one — the cache tracks the number of distinct images ever
/// seen, not the number of times they changed.
pub fn cache_path(root: &Path, source: &Path) -> PathBuf {
    let name = format!("{:032x}", fnv1a_128(path_bytes(source)));
    // Indexing is safe: the name is 32 hex digits by construction.
    let bucket = &name[..2];
    root.join(bucket).join(format!("{name}.png"))
}

/// Is a cached thumbnail still a picture of the file on disk?
///
/// Both recorded values must be present and match. A cache file written by
/// something else, or by an older Hive that recorded less, is treated as stale
/// and rewritten rather than trusted.
pub fn is_fresh(
    recorded_mtime: Option<&str>,
    recorded_size: Option<&str>,
    mtime: i64,
    size: u64,
) -> bool {
    let (Some(recorded_mtime), Some(recorded_size)) = (recorded_mtime, recorded_size) else {
        return false;
    };
    recorded_mtime.parse::<i64>() == Ok(mtime) && recorded_size.parse::<u64>() == Ok(size)
}

/// Target size for a thumbnail of a `width` × `height` image.
///
/// Never upscales: a 32×32 icon stays 32×32 rather than becoming a blurry
/// 256×256, which also keeps the cache file small. Aspect ratio is preserved,
/// and neither edge is ever allowed to round down to zero.
pub fn scaled(width: i32, height: i32, max_pixels: i32) -> (i32, i32) {
    let max = max_pixels.max(1);
    if width <= 0 || height <= 0 {
        return (max, max);
    }
    if width <= max && height <= max {
        return (width, height);
    }

    let ratio = f64::from(max) / f64::from(width.max(height));
    let scale = |edge: i32| ((f64::from(edge) * ratio).round() as i32).clamp(1, max);
    (scale(width), scale(height))
}

/// Raw bytes of a path, which need not be UTF-8.
///
/// Hashing the bytes rather than a lossy string means two folders whose names
/// differ only in invalid UTF-8 cannot collide onto one cache entry.
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

/// FNV-1a, widened to 128 bits.
///
/// A cache address, not a security boundary: the only consequence of a
/// collision is one wrong thumbnail, and at 128 bits that will not happen.
/// Chosen over a digest crate because it is ten lines and adds no dependency.
fn fnv1a_128(bytes: &[u8]) -> u128 {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const MAX_BYTES: u64 = 32 * 1024 * 1024;

    #[test]
    fn images_are_eligible_and_nothing_else_is() {
        assert_eq!(eligibility("image/png", 1024, MAX_BYTES), Eligibility::Yes);
        assert_eq!(eligibility("image/jpeg", 1024, MAX_BYTES), Eligibility::Yes);
        assert_eq!(
            eligibility("video/mp4", 1024, MAX_BYTES),
            Eligibility::NotAnImage,
            "video thumbnails are out of scope for v1"
        );
        assert_eq!(
            eligibility("inode/directory", 0, MAX_BYTES),
            Eligibility::NotAnImage
        );
        assert_eq!(eligibility("", 10, MAX_BYTES), Eligibility::NotAnImage);
    }

    #[test]
    fn the_source_size_cap_is_inclusive() {
        assert_eq!(
            eligibility("image/png", MAX_BYTES, MAX_BYTES),
            Eligibility::Yes,
            "exactly at the cap is still allowed"
        );
        assert_eq!(
            eligibility("image/png", MAX_BYTES + 1, MAX_BYTES),
            Eligibility::TooLarge
        );
    }

    #[test]
    fn a_cap_of_zero_disables_thumbnailing_rather_than_dividing_by_it() {
        assert_eq!(
            eligibility("image/png", 1, 0),
            Eligibility::TooLarge,
            "a zero cap must not be read as unlimited"
        );
    }

    #[test]
    fn cache_paths_are_stable_distinct_and_bucketed() {
        let root = Path::new("/cache/hive/thumbnails");
        let one = cache_path(root, Path::new("/home/diren/a.png"));
        let two = cache_path(root, Path::new("/home/diren/b.png"));

        assert_ne!(one, two);
        assert_eq!(one, cache_path(root, Path::new("/home/diren/a.png")));
        assert!(one.starts_with(root));
        assert_eq!(one.extension().unwrap(), "png");

        // The bucket directory is the first two characters of the file name.
        let name = one.file_stem().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 32);
        let bucket = one.parent().unwrap().file_name().unwrap().to_str().unwrap();
        assert_eq!(bucket, &name[..2]);
    }

    #[test]
    fn a_renamed_file_addresses_a_different_cache_entry() {
        let root = Path::new("/cache");
        assert_ne!(
            cache_path(root, Path::new("/x/photo.png")),
            cache_path(root, Path::new("/x/photo-2.png"))
        );
    }

    #[test]
    fn non_utf8_names_hash_without_being_flattened_together() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let root = Path::new("/cache");
        let one = PathBuf::from(OsStr::from_bytes(b"/tmp/bad\xff\xfe.png"));
        let two = PathBuf::from(OsStr::from_bytes(b"/tmp/bad\xfe\xff.png"));
        assert_ne!(cache_path(root, &one), cache_path(root, &two));
    }

    #[test]
    fn freshness_needs_both_recorded_values_to_match() {
        assert!(is_fresh(Some("1700"), Some("2048"), 1700, 2048));
        assert!(
            !is_fresh(Some("1699"), Some("2048"), 1700, 2048),
            "an edited image has a new mtime"
        );
        assert!(
            !is_fresh(Some("1700"), Some("2047"), 1700, 2048),
            "a same-second edit still changes the size"
        );
        assert!(!is_fresh(None, Some("2048"), 1700, 2048));
        assert!(!is_fresh(Some("1700"), None, 1700, 2048));
        assert!(!is_fresh(Some("not a number"), Some("2048"), 1700, 2048));
    }

    #[test]
    fn scaling_preserves_aspect_and_never_upscales() {
        assert_eq!(scaled(512, 256, 256), (256, 128));
        assert_eq!(scaled(256, 512, 256), (128, 256));
        assert_eq!(scaled(1000, 1000, 256), (256, 256));
        assert_eq!(scaled(64, 32, 256), (64, 32), "small images stay put");
        assert_eq!(scaled(256, 256, 256), (256, 256));
    }

    #[test]
    fn a_panoramic_image_keeps_at_least_one_pixel_of_height() {
        let (width, height) = scaled(20_000, 3, 256);
        assert_eq!(width, 256);
        assert_eq!(height, 1, "never zero, which no decoder accepts");
    }

    #[test]
    fn nonsense_dimensions_do_not_panic() {
        assert_eq!(scaled(0, 0, 256), (256, 256));
        assert_eq!(scaled(-4, 10, 256), (256, 256));
        assert_eq!(
            scaled(100, 100, 0),
            (1, 1),
            "a zero cap clamps, not divides"
        );
        assert_eq!(scaled(i32::MAX, i32::MAX, 256), (256, 256));
    }
}
