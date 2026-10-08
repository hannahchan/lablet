//! The canaries: tests of the OpenTelemetry crates themselves, each of
//! which pins a gap in the pinned release that lablet fills with code of
//! its own. A canary passes while the gap is there and fails on the release
//! that closes it.
//!
//! Each that pins the crates' reading of the process environment runs in a
//! child process, since `cargo xtask test` strips that environment and a
//! test can't set it for its own process.
//!
//! The commit that upgrades the crates bumps their pins alone and lets the
//! canaries fail. For each one that fails, that commit, or one right after
//! it, deletes lablet's code and lets the crate decide, or records in
//! `product/decisions.md` why lablet keeps it.

mod context;
mod exporter;
mod propagation;
