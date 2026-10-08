//! Time and cancellation, as ports, so a test can drive both.

use std::time::{Duration, Instant, SystemTime};

/// The loop's only source of time.
///
/// Every offset and latency a run records is read from here, and so is the
/// time the run has left, which each call's deadline is taken from, so a test
/// drives timeouts and backoff without waiting. Adapters enforce the
/// deadline they're handed in real time; this is the run's clock, not
/// theirs.
#[async_trait::async_trait]
pub trait Clock: Send + Sync {
    /// The instant now.
    fn now(&self) -> Instant;

    /// The time now on the wall clock, which a run reads once, as it
    /// starts, and times every span and record by, with the offsets
    /// [`Clock::now`] measures.
    ///
    /// Never before the Unix epoch: an implementation reads a clock that's
    /// earlier as the epoch, since an exporter puts any time before it at
    /// 0, which would leave every span of the run no length.
    fn wall(&self) -> SystemTime;

    /// Waits for `duration`.
    async fn sleep(&self, duration: Duration);
}

/// Whether the run has been asked to stop.
///
/// The loop asks before each provider call, after each tool phase and
/// before each tool call starts. It asks ahead of the stop policy, which is
/// why `cancelled` isn't one of the reasons the policy decides. It races
/// each provider call attempt, each wait before a retry and each tool call
/// against [`Cancellation::cancelled`], and drops whichever is in flight
/// when the run should stop, so a run stops as soon as its futures are
/// dropped and never waits for a call to return.
///
/// Once a run should stop it stays that way: an implementation never
/// answers `false` after it has answered `true`, and `cancelled` resolves
/// at once from then on.
#[async_trait::async_trait]
pub trait Cancellation: Send + Sync {
    /// Whether the run should stop now.
    fn is_cancelled(&self) -> bool;

    /// Resolves once the run should stop, and at once if it should already.
    async fn cancelled(&self);
}
