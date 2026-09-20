//! Time and cancellation, as ports, so a test can drive both.

use std::time::{Duration, Instant};

/// The loop's only source of time.
///
/// Every offset and latency a run records is read from here, so a test drives
/// timeouts and backoff without waiting. Adapters enforce their own per-call
/// deadlines in real time; this is the run's clock, not theirs.
#[async_trait::async_trait]
pub trait Clock: Send + Sync {
    /// The instant now.
    fn now(&self) -> Instant;

    /// Waits for `duration`.
    async fn sleep(&self, duration: Duration);
}

/// Whether the run has been asked to stop.
///
/// The loop polls this before each provider call and after each tool phase,
/// ahead of asking the stop policy, which is why `cancelled` isn't one of the
/// reasons the policy decides.
pub trait Cancellation: Send + Sync {
    /// Whether the run should stop now.
    fn is_cancelled(&self) -> bool;
}
