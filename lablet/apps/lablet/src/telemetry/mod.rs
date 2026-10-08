//! The keys of a run's telemetry, and the schema URL its scope carries.
//!
//! [`key`] and [`SCHEMA_URL`] are `lablet_run`'s, where every span and
//! event of a run is declared, the root span and the wide event among them,
//! so a host may read what a run emitted to the providers it handed in by
//! its keys, and hold each signal to the keys the registry requires and
//! declares. The structs that write the signals aren't part of the library:
//! what a host hands in is OpenTelemetry's own providers and propagator, in
//! an [`Otel`](crate::Otel), never a type of lablet's that writes a signal.

pub use lablet_run::telemetry::generated::{SCHEMA_URL, key};
