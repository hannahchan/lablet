//! The composition root's telemetry, typed: the root span and the wide
//! event, as structs generated from the registry, and the domain's closed
//! sets spelt as the registry spells them.
//!
//! `generated` is the module `cargo xtask weaver generate` writes from the
//! registry folder `lablet/telemetry/registry/apps/lablet/`. Of it, only
//! [`key`] and [`SCHEMA_URL`] are public, so a library caller may read the
//! root span and the wide event by their keys and hold each to the keys the
//! registry requires and declares; the keys of the loop's spans and records
//! are `lablet_run`'s. The structs that write the signals are the crate's
//! own, so the OpenTelemetry API they're written through is no part of
//! lablet's. Nothing plugs into a `Lablet`'s telemetry.

#[expect(
    dead_code,
    clippy::struct_field_names,
    clippy::unused_self,
    reason = "each signal's struct is generated whole, as a public module's is, with its fields named by their keys; and the root span is filled through the reference its context holds, which the struct's `record` can't take"
)]
pub(crate) mod generated;
pub(crate) mod spellings;

pub use generated::{SCHEMA_URL, key};
