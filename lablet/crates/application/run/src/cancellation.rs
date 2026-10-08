//! The cancellation a loop is built with, which each run points at its own.

use std::sync::{Arc, Mutex, PoisonError};

use crate::Cancellation;

/// A run that nothing asks to stop: it ends for its own reasons.
#[derive(Debug, Clone, Copy, Default)]
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

/// The cancellation a loop is built with: the one of the run in progress,
/// which [`crate::Runner::run`] sets from the run's start, so one a run was
/// given never reaches the next.
pub struct RunCancellation {
    current: Mutex<Arc<dyn Cancellation>>,
}

impl Default for RunCancellation {
    fn default() -> Self {
        Self {
            current: Mutex::new(Arc::new(NeverCancelled)),
        }
    }
}

impl RunCancellation {
    /// Answers for `cancellation` until the next run sets its own, or for
    /// none when it's `None`.
    pub(crate) fn set(&self, cancellation: Option<Arc<dyn Cancellation>>) {
        let current = cancellation.unwrap_or_else(|| Arc::new(NeverCancelled));
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = current;
    }

    fn current(&self) -> Arc<dyn Cancellation> {
        Arc::clone(&self.current.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[async_trait::async_trait]
impl Cancellation for RunCancellation {
    fn is_cancelled(&self) -> bool {
        self.current().is_cancelled()
    }

    async fn cancelled(&self) {
        let current = self.current();
        current.cancelled().await;
    }
}

#[cfg(test)]
mod tests;
