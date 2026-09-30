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

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests;
