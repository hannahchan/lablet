//! Adapter: the run's clock on tokio's time, one clock for both roots, so a
//! library run and a command line run are timed alike.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lablet_run::Clock;

/// The runtime's clock, which is also the one the adapters hold their
/// deadlines on. In a test that pauses tokio's clock, a script's latencies
/// and the loop's waits pass at once, and the loop measures each as exactly
/// what was asked.
pub struct TokioClock;

#[async_trait::async_trait]
impl Clock for TokioClock {
    fn now(&self) -> Instant {
        tokio::time::Instant::now().into_std()
    }

    fn wall(&self) -> SystemTime {
        at_or_after_epoch(SystemTime::now())
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// `time`, or the epoch for a time before it, as the documents give it.
fn at_or_after_epoch(time: SystemTime) -> SystemTime {
    time.max(UNIX_EPOCH)
}

#[cfg(test)]
mod tests;
