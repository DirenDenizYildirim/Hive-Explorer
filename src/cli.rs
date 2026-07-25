//! Command-line parsing.
//!
//! Hand-rolled rather than pulled from a crate: the surface is four forms, and
//! keeping it here means the parse is unit-tested without a display and without
//! GApplication's own option machinery getting in the way.
//!
//! ```text
//! hive                  open the home directory
//! hive PATH             open PATH; if PATH is a file, open its parent and preselect it
//! hive --select PATH    always reveal: open PATH's parent and preselect PATH
//! hive --verbose        raise the log level
//! ```
//!
//! `--select` is the stable form other tools use for "reveal in file manager",
//! so its behavior must not drift.

use std::ffi::OsString;
use std::path::PathBuf;

pub const HELP: &str = "\
Hive — a Catppuccin file manager for Hyprland

USAGE:
    hive [OPTIONS] [PATH]

ARGS:
    <PATH>              Directory to open. If PATH is a file, opens its parent
                        directory and preselects the file.

OPTIONS:
    -s, --select <PATH> Reveal PATH: open its parent directory and preselect it.
                        Works for directories too. This is the flag to use from
                        other applications for \"reveal in file manager\".
    -v, --verbose       Raise the log level to debug.
    -h, --help          Print this help.
    -V, --version       Print version information.
";

/// What the user asked for on the command line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    /// A path to open, exactly as given — not yet resolved against the working
    /// directory. [`crate::app`] canonicalizes it in the local process.
    pub target: Option<PathBuf>,
    /// True when the target came from `--select`, meaning "reveal" even if the
    /// target is itself a directory.
    pub reveal: bool,
    pub verbose: bool,
    pub help: bool,
    pub version: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    #[error("unknown option '{0}'\n\nTry 'hive --help'.")]
    UnknownOption(String),
    #[error("option '{0}' needs a path\n\nTry 'hive --help'.")]
    MissingValue(String),
    #[error("unexpected extra argument '{0}'\n\nTry 'hive --help'.")]
    TooManyArguments(String),
}

/// Parse arguments, excluding the program name.
pub fn parse<I, S>(args: I) -> Result<Args, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut parsed = Args::default();
    let mut iter = args.into_iter().map(Into::into).peekable();
    // Everything after a bare `--` is a path, even if it looks like a flag.
    let mut options_done = false;

    while let Some(raw) = iter.next() {
        let text = raw.to_string_lossy().into_owned();

        if !options_done && text == "--" {
            options_done = true;
            continue;
        }

        let is_option = !options_done && text.starts_with('-') && text != "-";

        if is_option {
            match text.as_str() {
                "-h" | "--help" => parsed.help = true,
                "-V" | "--version" => parsed.version = true,
                "-v" | "--verbose" => parsed.verbose = true,
                "-s" | "--select" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| CliError::MissingValue(text.clone()))?;
                    set_target(&mut parsed, PathBuf::from(value), true)?;
                }
                other => {
                    // `--select=PATH` form.
                    if let Some(value) = other.strip_prefix("--select=") {
                        if value.is_empty() {
                            return Err(CliError::MissingValue("--select".to_owned()));
                        }
                        set_target(&mut parsed, PathBuf::from(value), true)?;
                    } else {
                        return Err(CliError::UnknownOption(other.to_owned()));
                    }
                }
            }
        } else {
            set_target(&mut parsed, PathBuf::from(raw), false)?;
        }
    }

    Ok(parsed)
}

fn set_target(parsed: &mut Args, path: PathBuf, reveal: bool) -> Result<(), CliError> {
    if parsed.target.is_some() {
        return Err(CliError::TooManyArguments(
            path.to_string_lossy().into_owned(),
        ));
    }
    parsed.target = Some(path);
    // `--select` wins over a bare path if both somehow appear; it is the
    // explicit form.
    parsed.reveal = reveal;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn parse_ok(args: &[&str]) -> Args {
        parse(args.iter().copied()).expect("should parse")
    }

    #[test]
    fn no_arguments_means_open_home() {
        let args = parse_ok(&[]);
        assert_eq!(args.target, None);
        assert!(!args.reveal);
        assert!(!args.verbose);
    }

    #[test]
    fn a_bare_path_is_the_target() {
        let args = parse_ok(&["/home/diren/Downloads"]);
        assert_eq!(args.target, Some(PathBuf::from("/home/diren/Downloads")));
        assert!(!args.reveal, "a bare path is not an explicit reveal");
    }

    #[test]
    fn relative_paths_are_left_unresolved_for_the_local_process() {
        // Resolution happens in app::, against the invoking shell's cwd. The
        // parser must not touch it.
        let args = parse_ok(&["."]);
        assert_eq!(args.target, Some(PathBuf::from(".")));
        let args = parse_ok(&["../sibling"]);
        assert_eq!(args.target, Some(PathBuf::from("../sibling")));
    }

    #[test]
    fn select_sets_the_reveal_flag() {
        for form in [
            vec!["--select", "/home/diren/notes.txt"],
            vec!["-s", "/home/diren/notes.txt"],
            vec!["--select=/home/diren/notes.txt"],
        ] {
            let args = parse(form.iter().copied()).expect("should parse");
            assert_eq!(args.target, Some(PathBuf::from("/home/diren/notes.txt")));
            assert!(args.reveal, "{form:?}");
        }
    }

    #[test]
    fn verbose_is_recognized_in_both_forms() {
        assert!(parse_ok(&["--verbose"]).verbose);
        assert!(parse_ok(&["-v"]).verbose);
        // And alongside a path, in either order.
        let args = parse_ok(&["-v", "/tmp"]);
        assert!(args.verbose);
        assert_eq!(args.target, Some(PathBuf::from("/tmp")));

        let args = parse_ok(&["/tmp", "-v"]);
        assert!(args.verbose);
        assert_eq!(args.target, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn help_and_version_are_recognized() {
        assert!(parse_ok(&["--help"]).help);
        assert!(parse_ok(&["-h"]).help);
        assert!(parse_ok(&["--version"]).version);
        assert!(parse_ok(&["-V"]).version);
    }

    #[test]
    fn double_dash_stops_option_parsing() {
        // A file legitimately named "--verbose" must be openable.
        let args = parse_ok(&["--", "--verbose"]);
        assert_eq!(args.target, Some(PathBuf::from("--verbose")));
        assert!(!args.verbose);
    }

    #[test]
    fn a_lone_dash_is_treated_as_a_path_not_an_option() {
        let args = parse_ok(&["-"]);
        assert_eq!(args.target, Some(PathBuf::from("-")));
    }

    #[test]
    fn unknown_options_are_rejected_with_a_message() {
        let error = parse(["--frobnicate"]).unwrap_err();
        assert_eq!(error, CliError::UnknownOption("--frobnicate".to_owned()));
        assert!(error.to_string().contains("--help"));
    }

    #[test]
    fn select_without_a_value_is_an_error() {
        assert_eq!(
            parse(["--select"]).unwrap_err(),
            CliError::MissingValue("--select".to_owned())
        );
        assert_eq!(
            parse(["--select="]).unwrap_err(),
            CliError::MissingValue("--select".to_owned())
        );
    }

    #[test]
    fn two_paths_are_an_error_rather_than_a_silent_pick() {
        let error = parse(["/tmp", "/var"]).unwrap_err();
        assert_eq!(error, CliError::TooManyArguments("/var".to_owned()));
    }

    #[test]
    fn paths_with_odd_bytes_survive_parsing() {
        use std::os::unix::ffi::OsStringExt;
        let weird = OsString::from_vec(b"/tmp/bad\xffname".to_vec());
        let args = parse([weird.clone()]).expect("should parse");
        assert_eq!(args.target, Some(PathBuf::from(weird)));
    }

    #[test]
    fn paths_containing_newlines_survive_parsing() {
        let args = parse_ok(&["/tmp/two\nlines"]);
        assert_eq!(args.target, Some(PathBuf::from("/tmp/two\nlines")));
    }

    #[test]
    fn select_takes_its_value_even_if_it_looks_like_a_flag() {
        let args = parse_ok(&["--select", "--weird-filename"]);
        assert_eq!(args.target, Some(PathBuf::from("--weird-filename")));
        assert!(args.reveal);
    }
}
