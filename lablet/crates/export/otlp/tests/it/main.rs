//! Integration tests of `lablet-otlp`, through its public surface only: a
//! run's spans and records are emitted through the tracer and the logger a
//! `Telemetry` hands out, as the loop emits them, exported to a file or to
//! the in-process receiver, and read back.

#[cfg(test)]
mod file;
#[cfg(test)]
mod harness;
#[cfg(test)]
mod limits;
#[cfg(test)]
mod network;
#[cfg(test)]
mod resource;
