//! Hive — minimal pastel explorer.

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

    let _log_guard = logging::init(&paths::log_dir(), args.verbose);

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

    if let Err(error) = application.register(gio::Cancellable::NONE) {
        eprintln!("hive: could not register application: {error}");
        return glib::ExitCode::FAILURE;
    }

    match target {
        Some(path) => {
            let hint = if args.reveal { app::REVEAL_HINT } else { "" };
            application.open(&[gio::File::for_path(&path)], hint);
        }
        None => application.activate(),
    }

    if application.is_remote() {
        tracing::debug!("handed off to the running instance");
    }

    // Run even when remote. The dispatch above has already delivered the target,
    // and for a remote instance `run` returns straight away — but it is also
    // what tears the D-Bus registration down cleanly. Returning early instead
    // leaves the application finalized while still registered, which GIO warns
    // about on stderr. The extra `activate` it emits is harmless: the primary
    // instance just presents the window it already has.
    application.run_with_args::<&str>(&[program.to_string_lossy().as_ref()])
}
