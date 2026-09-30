//! Test support: what the tests of lablet's crates share, so that each is
//! written once. A scratch directory of a test's own, the loop's clock and
//! cancellation on tokio's time, and the loop itself, built around a
//! scripted provider.
//!
//! A crate this one depends on can't take it into its own unit tests: they
//! would compile that crate a second time, whose types don't match the
//! first's. That's why `lablet-run` keeps its fakes in its `src/tests/`.

use std::fmt::Display;

mod clock;
mod run;
mod scratch;

pub use clock::{NeverCancelled, TokioClock};
pub use run::{
    AGENT_VERSION, CONFIG_DIGEST, MODEL, PROMPT, RunBuilder, SCRIPT, STARTED_UNIX_MS, SYSTEM,
    Unobserved, context, prompts, request, scripted,
};
pub use scratch::Scratch;

/// What `result` holds, or the end of the test: whatever went wrong `doing`
/// it is a fault of the test's setting, not of what it tests.
fn must<T, E: Display>(result: Result<T, E>, doing: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{doing}: {error}"),
    }
}
