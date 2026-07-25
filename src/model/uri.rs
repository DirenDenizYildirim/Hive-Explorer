//! Percent-encoding for `file://` URIs and `.trashinfo` paths.
//!
//! Hand-rolled rather than delegated to glib because both the clipboard formats
//! and the freedesktop trash spec are byte-oriented: a filename is an arbitrary
//! byte string, not necessarily UTF-8, and a round trip through `String` would
//! lose it. Encoding is deliberately conservative — everything outside the
//! unreserved set is escaped — because over-escaping always decodes correctly
//! and under-escaping does not.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

/// Percent-encode a path, keeping `/` as a separator.
pub fn encode_path(path: &Path) -> String {
    let mut out = String::new();
    for &byte in path.as_os_str().as_bytes() {
        if is_unreserved(byte) || byte == b'/' {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[usize::from(byte >> 4)] as char);
            out.push(HEX[usize::from(byte & 0x0f)] as char);
        }
    }
    out
}

/// Decode percent escapes back into a path.
///
/// A malformed escape is kept verbatim rather than dropped: a literal `%` in a
/// filename is legal, and losing bytes here would point an operation at the
/// wrong file.
pub fn decode_path(encoded: &str) -> PathBuf {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }

    PathBuf::from(OsString::from_vec(out))
}

/// `/home/diren/a b` -> `file:///home/diren/a%20b`.
pub fn to_file_uri(path: &Path) -> String {
    format!("file://{}", encode_path(path))
}

/// Parse a `file://` URI back to an absolute path.
///
/// Accepts the empty and `localhost` authorities, which both appear in the
/// wild. Anything else is a remote location Hive cannot operate on locally.
pub fn from_file_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.trim().strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);

    if !rest.starts_with('/') {
        return None;
    }

    let path = decode_path(rest);
    path.is_absolute().then_some(path)
}

/// The final component of a percent-encoded path, still encoded.
pub fn encoded_file_name(path: &Path) -> Option<String> {
    path.file_name().map(|name| encode_path(Path::new(name)))
}

/// Build a path from a directory and a raw byte-string name.
pub fn join_bytes(directory: &Path, name: &[u8]) -> PathBuf {
    directory.join(OsStr::from_bytes(name))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn round_trip(path: &Path) {
        let uri = to_file_uri(path);
        assert_eq!(
            from_file_uri(&uri).as_deref(),
            Some(path),
            "round trip failed for {uri}"
        );
    }

    #[test]
    fn plain_paths_are_untouched() {
        assert_eq!(
            encode_path(Path::new("/home/diren/notes.txt")),
            "/home/diren/notes.txt"
        );
        assert_eq!(to_file_uri(Path::new("/tmp/a")), "file:///tmp/a");
    }

    #[test]
    fn spaces_and_reserved_characters_are_escaped() {
        assert_eq!(encode_path(Path::new("/tmp/a b")), "/tmp/a%20b");
        assert_eq!(encode_path(Path::new("/tmp/100%")), "/tmp/100%25");
        assert_eq!(encode_path(Path::new("/tmp/a#b")), "/tmp/a%23b");
        assert_eq!(encode_path(Path::new("/tmp/a?b")), "/tmp/a%3Fb");
    }

    #[test]
    fn newlines_survive_the_uri_list_format() {
        // A newline in a filename is legal and would otherwise be read as a
        // separator by every consumer of text/uri-list.
        let path = Path::new("/tmp/two\nlines");
        assert_eq!(encode_path(path), "/tmp/two%0Alines");
        round_trip(path);
    }

    #[test]
    fn invalid_utf8_names_round_trip_byte_for_byte() {
        let raw = OsStr::from_bytes(b"/tmp/bad\xff\xfename");
        let path = Path::new(raw);
        assert_eq!(encode_path(path), "/tmp/bad%FF%FEname");
        round_trip(path);
    }

    #[test]
    fn decoding_is_case_insensitive_in_hex() {
        assert_eq!(decode_path("/tmp/a%20b"), PathBuf::from("/tmp/a b"));
        assert_eq!(decode_path("/tmp/a%2fb"), PathBuf::from("/tmp/a/b"));
    }

    #[test]
    fn a_lone_percent_is_kept_rather_than_dropped() {
        assert_eq!(decode_path("/tmp/50%"), PathBuf::from("/tmp/50%"));
        assert_eq!(decode_path("/tmp/%zz"), PathBuf::from("/tmp/%zz"));
        assert_eq!(decode_path("/tmp/%2"), PathBuf::from("/tmp/%2"));
    }

    #[test]
    fn localhost_authority_is_accepted() {
        assert_eq!(
            from_file_uri("file://localhost/tmp/a"),
            Some(PathBuf::from("/tmp/a"))
        );
    }

    #[test]
    fn non_local_uris_are_rejected() {
        assert_eq!(from_file_uri("trash:///foo"), None);
        assert_eq!(from_file_uri("smb://server/share"), None);
        assert_eq!(from_file_uri("https://example.com/a"), None);
        assert_eq!(from_file_uri("file://server/share"), None);
        assert_eq!(from_file_uri(""), None);
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        assert_eq!(
            from_file_uri("  file:///tmp/a\r\n"),
            Some(PathBuf::from("/tmp/a"))
        );
    }

    #[test]
    fn every_byte_value_round_trips() {
        for byte in 1u8..=255 {
            if byte == b'/' {
                continue;
            }
            let mut name = b"/tmp/x".to_vec();
            name.push(byte);
            let path = PathBuf::from(OsString::from_vec(name));
            round_trip(&path);
        }
    }

    #[test]
    fn encoded_file_name_escapes_the_last_component_only() {
        assert_eq!(
            encoded_file_name(Path::new("/tmp/sub dir/a b")).as_deref(),
            Some("a%20b")
        );
        assert_eq!(encoded_file_name(Path::new("/")), None);
    }
}
