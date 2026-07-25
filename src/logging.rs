//! Logging to a rotating file under `$XDG_STATE_HOME/hive/logs/`.
//!
//! Daily rotation, five files kept. `--verbose` raises the level to debug.
//!
//! Logging must never be the reason Hive fails to start: if the log directory
//! cannot be created or opened, we fall back to stderr and carry on.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Keep the returned guard alive for the process lifetime — dropping it stops
/// the background writer and loses buffered lines.
#[must_use = "dropping the guard stops the log writer"]
pub struct LogGuard(#[allow(dead_code)] Option<WorkerGuard>);

/// Initialize logging. Returns a guard that must outlive the application.
pub fn init(log_dir: &Path, verbose: bool) -> LogGuard {
    let default_level = if verbose { "debug" } else { "info" };

    // RUST_LOG wins if set, so a user can debug one module without a rebuild.
    let filter = || {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(format!("hive={default_level},warn")))
    };

    // Two things can go wrong before a log file exists: the directory cannot be
    // created, or the appender cannot open a file in it. Both fall back to
    // stderr with the reason recorded, and neither stops startup.
    let outcome = std::fs::create_dir_all(log_dir)
        .map_err(|error| format!("could not create log directory: {error}"))
        .and_then(|()| {
            tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("hive")
                .filename_suffix("log")
                .max_log_files(5)
                .build(log_dir)
                .map_err(|error| format!("could not open rotating log file: {error}"))
        });

    match outcome {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::fmt()
                .with_env_filter(filter())
                .with_writer(writer)
                // No colour codes in a file.
                .with_ansi(false)
                .with_target(true)
                .init();
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                dir = %log_dir.display(),
                "hive starting"
            );
            LogGuard(Some(guard))
        }
        Err(reason) => {
            tracing_subscriber::fmt()
                .with_env_filter(filter())
                .with_writer(std::io::stderr)
                .init();
            tracing::warn!(
                dir = %log_dir.display(),
                %reason,
                "file logging unavailable; logging to stderr"
            );
            LogGuard(None)
        }
    }
}
