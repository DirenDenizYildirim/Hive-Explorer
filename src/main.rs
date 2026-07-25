//! Hive — a Catppuccin file manager for Hyprland.
//!
//! A thin entry point: parse the command line, resolve any path against *this*
//! process's working directory, then hand off to [`hive::app`].

use std::ffi::OsString;

use adw::prelude::*;
use hive::{app, cli, logging, paths};

fn main() -> glib::ExitCode {
    let raw: Vec<OsString> = std::env::args_os().collect();
    let program = raw
        .first()
        .cloned()
        .unwrap_or_else(|| OsString::from("hive"));

    let args = match cli::parse(raw.into_iter().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("hive: {error}");
            return glib::ExitCode::FAILURE;
        }
    };

    if args.help {
        print!("{}", cli::HELP);
        return glib::ExitCode::SUCCESS;
    }

    if args.version {
        println!("hive {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }

    // Hold the guard for the whole process: dropping it stops the log writer.
    let _log_guard = logging::init(&paths::log_dir(), args.verbose);

    // Resolve the target here, in the invoking process, before anything is
    // dispatched to an already-running instance. See app::canonicalize_local.
    let target = args.target.as_deref().map(|input| {
        let resolved = app::canonicalize_local(input);
        tracing::debug!(
            input = %input.display(),
            resolved = %resolved.display(),
            reveal = args.reveal,
            "resolved command-line target"
        );
        resolved
    });

    let application = app::build();

    // Register before dispatching so `open`/`activate` reach the running
    // instance over D-Bus when there is one. Going through GApplication's own
    // argv parsing instead would force paths through `&str`, which would mangle
    // filenames that are not valid UTF-8.
    if let Err(error) = application.register(gio::Cancellable::NONE) {
        eprintln!("hive: could not register application: {error}");
        return glib::ExitCode::FAILURE;
    }

    match target {
        Some(path) => {
            let hint = if args.reveal { "select" } else { "" };
            application.open(&[gio::File::for_path(&path)], hint);
        }
        None => application.activate(),
    }

    if application.is_remote() {
        // A window already exists in the primary instance and has been told
        // where to go. Nothing further to do in this process.
        tracing::debug!("handed off to the running instance");
        return glib::ExitCode::SUCCESS;
    }

    application.run_with_args::<&str>(&[program.to_string_lossy().as_ref()])
}
