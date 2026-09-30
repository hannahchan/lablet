//! The loop's time and cancellation, as the tests of adapters drive them.

use std::time::{Duration, Instant};

use lablet_run::{Cancellation, Clock};

/// The loop's clock on tokio's time. In a test that pauses tokio's clock, a
/// script's latencies and the loop's waits pass at once, and the loop
/// measures each as exactly what was asked.
pub struct TokioClock;

#[async_trait::async_trait]
impl Clock for TokioClock {
    fn now(&self) -> Instant {
        tokio::time::Instant::now().into_std()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// A run nobody asks to stop.
pub struct NeverCancelled;

#[async_trait::async_trait]
impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }

    async fn cancelled(&self) {
        std::future::pending::<()>().await;
    }
}

/// A run that's asked to stop once a span of tokio's time has passed since
/// this was made. On a paused clock the time passes only while something
/// waits, so the run is cancelled part-way through whatever it was waiting
/// for then: a script's latency, a backoff, or a tool call.
pub struct CancelledAfter {
    at: tokio::time::Instant,
}

impl CancelledAfter {
    /// Cancels the run `after` from now.
    #[must_use]
    pub fn new(after: Duration) -> Self {
        Self {
            at: tokio::time::Instant::now() + after,
        }
    }
}

#[async_trait::async_trait]
impl Cancellation for CancelledAfter {
    fn is_cancelled(&self) -> bool {
        tokio::time::Instant::now() >= self.at
    }

    async fn cancelled(&self) {
        tokio::time::sleep_until(self.at).await;
    }
}

#[cfg(test)]
mod tests;
