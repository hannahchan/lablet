//! The run's clock, as a library has it.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lablet_run::Clock;

/// The runtime's clock, which is also the one the adapters hold their
/// deadlines on.
pub(crate) struct TokioClock;

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
