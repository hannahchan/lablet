//! Test support: shared `ToolExecutor` and `RunObserver` conformance cases,
//! pulled in as a dev-dependency by each adapter that implements one of the ports.
//! Checks that span crates, which no single crate may make on its own, and
//! the reader of the OTLP/JSON lines an observer exports.

pub mod observer;
pub mod otlp;
