//! Test support: shared `ToolExecutor` and `RunObserver` conformance cases,
//! pulled in as a dev-dependency by each adapter that implements one of the ports.
//! Checks that span crates, which no single crate may make on its own, the
//! reader of the OTLP/JSON lines an observer exports, and an OTLP receiver
//! in the test's own process, so a network scenario needs no collector.

use std::fmt::Display;

pub mod executor;
pub mod observer;
pub mod otlp;
pub mod receiver;

/// What `result` holds, or the end of the case: whatever went wrong
/// `doing` it is a fault of the case's own, or of what it's handed.
pub(crate) fn must<T, E: Display>(result: Result<T, E>, doing: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{doing}: {error}"),
    }
}
