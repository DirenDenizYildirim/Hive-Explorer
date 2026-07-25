//! The two clipboard payload formats a file manager has to speak.
//!
//! `text/uri-list` (RFC 2483) is what almost everything reads, but it carries no
//! cut-versus-copy distinction. `x-special/gnome-copied-files` does, and is what
//! Nautilus, Thunar, Dolphin and PCManFM all use. Offering only one of them
//! produces copy/paste that appears to work but only inside Hive — §10.1 hazard 3.

use std::path::{Path, PathBuf};

use crate::model::uri;

pub const GNOME_MIME: &str = "x-special/gnome-copied-files";
pub const URI_LIST_MIME: &str = "text/uri-list";

/// The MIME types Hive offers and accepts, best first.
pub const MIME_TYPES: [&str; 2] = [GNOME_MIME, URI_LIST_MIME];

/// Whether pasting should copy the files or move them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Copy,
    Cut,
}

impl Intent {
    const fn keyword(self) -> &'static str {
        match self {
            Intent::Copy => "copy",
            Intent::Cut => "cut",
        }
    }

    fn from_keyword(word: &str) -> Option<Self> {
        match word.trim() {
            "copy" => Some(Intent::Copy),
            "cut" => Some(Intent::Cut),
            _ => None,
        }
    }
}

/// A set of files on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileClip {
    pub intent: Intent,
    pub paths: Vec<PathBuf>,
}

impl FileClip {
    pub fn new(intent: Intent, paths: Vec<PathBuf>) -> Self {
        Self { intent, paths }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// Render the GNOME format: an intent keyword, then one URI per line.
pub fn to_gnome(clip: &FileClip) -> String {
    let mut out = String::from(clip.intent.keyword());
    for path in &clip.paths {
        out.push('\n');
        out.push_str(&uri::to_file_uri(path));
    }
    out
}

/// Render `text/uri-list`, which RFC 2483 terminates every line with CRLF.
pub fn to_uri_list(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        out.push_str(&uri::to_file_uri(path));
        out.push_str("\r\n");
    }
    out
}

/// Parse the GNOME format. Returns `None` when the first line is not an intent.
pub fn parse_gnome(text: &str) -> Option<FileClip> {
    let mut lines = text.split('\n');
    let intent = Intent::from_keyword(lines.next()?)?;
    let paths = lines.filter_map(parse_line).collect();
    Some(FileClip { intent, paths })
}

/// Parse `text/uri-list`, skipping `#` comment lines as the RFC requires.
pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.split('\n')
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Option<PathBuf> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    uri::from_file_uri(trimmed)
}

/// Interpret a payload, preferring the format that knows about cut.
///
/// A `text/uri-list` from another application says nothing about intent, so it
/// is read as a copy: treating an ambiguous paste as a move would delete the
/// other application's files.
pub fn parse(mime: &str, text: &str) -> Option<FileClip> {
    let clip = if mime == GNOME_MIME {
        parse_gnome(text)?
    } else {
        FileClip::new(Intent::Copy, parse_uri_list(text))
    };
    (!clip.is_empty()).then_some(clip)
}

/// Paths whose parent is `directory`.
///
/// Cut-then-paste into the same directory is a no-op, not a duplicate and not a
/// deletion — §10.1 hazard 8.
pub fn is_same_directory_move(clip: &FileClip, directory: &Path) -> bool {
    clip.intent == Intent::Cut
        && !clip.paths.is_empty()
        && clip
            .paths
            .iter()
            .all(|path| path.parent() == Some(directory))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    fn clip(intent: Intent, paths: &[&str]) -> FileClip {
        FileClip::new(intent, paths.iter().map(PathBuf::from).collect())
    }

    #[test]
    fn gnome_format_leads_with_the_intent() {
        let text = to_gnome(&clip(Intent::Copy, &["/tmp/a", "/tmp/b"]));
        assert_eq!(text, "copy\nfile:///tmp/a\nfile:///tmp/b");

        let text = to_gnome(&clip(Intent::Cut, &["/tmp/a"]));
        assert_eq!(text, "cut\nfile:///tmp/a");
    }

    #[test]
    fn uri_list_uses_crlf_line_endings() {
        let paths = vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")];
        assert_eq!(to_uri_list(&paths), "file:///tmp/a\r\nfile:///tmp/b\r\n");
    }

    #[test]
    fn gnome_payloads_round_trip() {
        for intent in [Intent::Copy, Intent::Cut] {
            let original = clip(intent, &["/tmp/a b", "/tmp/c#d"]);
            let parsed = parse_gnome(&to_gnome(&original)).unwrap();
            assert_eq!(parsed, original);
        }
    }

    #[test]
    fn uri_list_payloads_round_trip() {
        let paths = vec![PathBuf::from("/tmp/a b"), PathBuf::from("/tmp/100%")];
        assert_eq!(parse_uri_list(&to_uri_list(&paths)), paths);
    }

    #[test]
    fn a_nautilus_payload_is_understood() {
        let text = "cut\nfile:///home/diren/Downloads/a.txt\nfile:///home/diren/b.txt";
        let parsed = parse(GNOME_MIME, text).unwrap();
        assert_eq!(parsed.intent, Intent::Cut);
        assert_eq!(
            parsed.paths,
            vec![
                PathBuf::from("/home/diren/Downloads/a.txt"),
                PathBuf::from("/home/diren/b.txt")
            ]
        );
    }

    #[test]
    fn a_trailing_newline_does_not_produce_an_empty_entry() {
        let parsed = parse_gnome("copy\nfile:///tmp/a\n").unwrap();
        assert_eq!(parsed.paths, vec![PathBuf::from("/tmp/a")]);
    }

    #[test]
    fn crlf_terminated_gnome_payloads_are_tolerated() {
        let parsed = parse_gnome("copy\r\nfile:///tmp/a\r\n").unwrap();
        assert_eq!(parsed.intent, Intent::Copy);
        assert_eq!(parsed.paths, vec![PathBuf::from("/tmp/a")]);
    }

    #[test]
    fn uri_list_comments_are_skipped() {
        let text = "# a comment\r\nfile:///tmp/a\r\n#another\r\nfile:///tmp/b\r\n";
        assert_eq!(
            parse_uri_list(text),
            vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]
        );
    }

    #[test]
    fn a_uri_list_from_elsewhere_is_read_as_a_copy_never_a_move() {
        let parsed = parse(URI_LIST_MIME, "file:///tmp/a\r\n").unwrap();
        assert_eq!(parsed.intent, Intent::Copy);
    }

    #[test]
    fn remote_uris_are_dropped_rather_than_turned_into_bad_paths() {
        let text = "copy\nsmb://server/share/a\nfile:///tmp/b\ntrash:///c";
        let parsed = parse_gnome(text).unwrap();
        assert_eq!(parsed.paths, vec![PathBuf::from("/tmp/b")]);
    }

    #[test]
    fn a_payload_with_no_usable_paths_is_not_a_clip() {
        assert_eq!(parse(GNOME_MIME, "copy\nsmb://x/y"), None);
        assert_eq!(parse(URI_LIST_MIME, "# only a comment\r\n"), None);
        assert_eq!(parse(GNOME_MIME, "garbage"), None);
        assert_eq!(parse(GNOME_MIME, ""), None);
    }

    #[test]
    fn names_with_newlines_survive_both_formats() {
        let path = PathBuf::from("/tmp/two\nlines");
        let original = FileClip::new(Intent::Cut, vec![path.clone()]);

        let parsed = parse_gnome(&to_gnome(&original)).unwrap();
        assert_eq!(parsed.paths, vec![path.clone()], "newline split the entry");

        assert_eq!(
            parse_uri_list(&to_uri_list(std::slice::from_ref(&path))),
            vec![path]
        );
    }

    #[test]
    fn invalid_utf8_names_survive_both_formats() {
        let raw = OsString::from_vec(b"/tmp/bad\xffname".to_vec());
        let path = PathBuf::from(raw);
        let original = FileClip::new(Intent::Copy, vec![path.clone()]);

        assert_eq!(
            parse_gnome(&to_gnome(&original)).unwrap().paths,
            vec![path.clone()]
        );
        assert_eq!(
            parse_uri_list(&to_uri_list(std::slice::from_ref(&path))),
            vec![path]
        );
    }

    #[test]
    fn a_cut_back_into_its_own_directory_is_recognised() {
        let directory = Path::new("/tmp/work");
        let cut = clip(Intent::Cut, &["/tmp/work/a", "/tmp/work/b"]);
        assert!(is_same_directory_move(&cut, directory));

        let copy = clip(Intent::Copy, &["/tmp/work/a"]);
        assert!(
            !is_same_directory_move(&copy, directory),
            "a copy duplicates"
        );

        let elsewhere = clip(Intent::Cut, &["/tmp/work/a", "/tmp/other/b"]);
        assert!(!is_same_directory_move(&elsewhere, directory));

        let empty = FileClip::new(Intent::Cut, Vec::new());
        assert!(!is_same_directory_move(&empty, directory));
    }

    #[test]
    fn a_name_that_is_only_invalid_bytes_still_parses() {
        let name = OsStr::from_bytes(b"\xfe\xff");
        let path = Path::new("/tmp").join(name);
        let text = to_gnome(&FileClip::new(Intent::Copy, vec![path.clone()]));
        assert_eq!(parse_gnome(&text).unwrap().paths, vec![path]);
    }
}
