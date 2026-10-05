//! The export module's tests through what the composition root uses of it:
//! a run's spans and records are emitted through the tracer and the logger
//! a `Telemetry` hands out, as the loop emits them, exported to a file or to
//! the in-process receiver, and read back. They name only what `export.rs`
//! re-exports, and `testing.rs`, so they hold the surface the composition
//! root has.

mod file;
mod harness;
mod limits;
mod network;
mod resource;
