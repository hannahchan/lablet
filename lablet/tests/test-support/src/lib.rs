//! Test support: what the tests of lablet's crates share, so that each is
//! written once. A scratch directory of a test's own, the loop's clock and
//! cancellation on tokio's time, and the loop itself, built around a
//! scripted provider. Behind the `conformance` feature, the shared
//! `ToolExecutor` cases, the OTLP/JSON reader, a host's in-memory providers
//! and the in-process OTLP receiver.
//!
//! A crate this one depends on can't take it into its own unit tests: they
//! would compile that crate a second time, whose types don't match the
//! first's. That's why `lablet-run` keeps its fakes in its `src/tests/`.

use std::fmt::Display;
use std::net::{Ipv4Addr, SocketAddr};

mod clock;
#[cfg(feature = "conformance")]
pub mod conformance;
mod run;
mod scratch;

pub use clock::{CancelledAfter, NeverCancelled, TokioClock};
pub use run::{
    AGENT_VERSION, CONFIG_DIGEST, MODEL, PROMPT, RunBuilder, SCRIPT, STARTED_UNIX_MS, SYSTEM,
    context, prompts, request, scripted,
};
pub use scratch::Scratch;

/// An address on the loopback interface that nothing listens on, so a
/// connection to it is refused. It's a port below the range the system
/// hands out, one no service is assigned, which no listener a test binds
/// at 0 can be given and no unprivileged process can bind: a port bound
/// and let go of was taken by another test's receiver before the client
/// connected. Outside the `conformance` feature, so an adapter's test of a
/// refused connection or a redirect pulls in no server.
#[must_use]
pub fn refused_address() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 4))
}

/// What `result` holds, or the end of the test: whatever went wrong `doing`
/// it is a fault of the test's setting, not of what it tests.
fn must<T, E: Display>(result: Result<T, E>, doing: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{doing}: {error}"),
    }
}
