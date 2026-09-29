//! The run's clock and its cancellation, as a library has them.

use std::time::{Duration, Instant};

use lablet_run::{Cancellation, Clock};

/// The runtime's clock, which is also the one the adapters hold their
/// deadlines on.
pub(crate) struct TokioClock;

#[async_trait::async_trait]
impl Clock for TokioClock {
    fn now(&self) -> Instant {
        tokio::time::Instant::now().into_std()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// A run of the library ends for its own reasons: nothing here asks one to
/// stop.
pub(crate) struct NeverCancelled;

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}
