# Hive — development tasks.
#
# Note: `just` is not required to build or verify Hive. Everything here is a
# thin wrapper over plain cargo, so `cargo build` / `cargo test` work identically
# without it.

default:
    @just --list

# Run Hive against the Wayland session.
run *ARGS:
    GDK_BACKEND=wayland cargo run -- {{ARGS}}

# Run with debug logging on stderr as well as the log file.
debug *ARGS:
    GDK_BACKEND=wayland RUST_LOG=hive=debug cargo run -- --verbose {{ARGS}}

# Unit tests. No display required.
test:
    cargo test

# Clippy, warnings denied.
lint:
    cargo clippy --all-targets -- -D warnings

# Check formatting without rewriting.
fmt:
    cargo fmt --check

# Rewrite formatting in place.
fmt-fix:
    cargo fmt

# Everything CI would run.
check: fmt lint test
    cargo build

# Build and install the Arch package.
install:
    makepkg -si --noconfirm

# Release binary only, no packaging.
build:
    cargo build --release --locked

# Generate the torture directory used for manual stability testing.
torture DIR="/tmp/hive-torture":
    ./scripts/make-torture-dir.py {{DIR}}

# Tail today's log.
logs:
    tail -f "${XDG_STATE_HOME:-$HOME/.local/state}/hive/logs/"hive.log.*

clean:
    cargo clean
