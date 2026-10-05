//! Test support: the shared `ToolExecutor` conformance cases, pulled in as a
//! dev-dependency by each adapter that implements the port, the reader of the
//! OTLP/JSON lines lablet exports, and an OTLP receiver in the test's own
//! process, so a network scenario needs no collector.

use std::fmt::Display;

pub mod executor;
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
