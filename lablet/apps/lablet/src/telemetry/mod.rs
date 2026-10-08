//! The keys of a run's telemetry, and the schema URL its scope carries.
//!
//! [`key`] and [`SCHEMA_URL`] are `lablet_run`'s, where every span and
//! event of a run is declared, the root span and the wide event among them,
//! so a library caller may read a run's export by its keys and hold each
//! signal to the keys the registry requires and declares. The structs that
//! write the signals aren't part of the library, so the OpenTelemetry API
//! they're written through is no part of lablet's. Nothing plugs into a
//! `Lablet`'s telemetry.

pub use lablet_run::telemetry::generated::{SCHEMA_URL, key};
