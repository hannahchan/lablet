//! The export module's tests through what the CLI root uses of it:
//! a run's spans and records are emitted through the tracer and the logger
//! a `Telemetry` hands out, as the loop emits them, exported to a file or to
//! the in-process receiver, and read back. They name only what `export.rs`
//! re-exports, `testing.rs`, and the seam the environment is read through,
//! so they hold the surface the CLI root has.

mod canaries;
mod file;
mod harness;
mod limits;
mod network;
mod resource;
