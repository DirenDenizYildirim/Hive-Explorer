//! Filesystem-facing layer.
//!
//! User-facing filesystem work goes through `gio`, so trash, mounts, URI
//! handling, and monitoring match the rest of the desktop. The modules here
//! that are pure policy — which places to show, which mounts belong in the
//! sidebar — are plain Rust and unit-tested without gio.

pub mod places;
pub mod volumes;
