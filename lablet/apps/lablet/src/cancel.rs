//! Asking a run to stop, from outside it.

use std::sync::{Arc, Mutex, PoisonError};

use lablet_run::Cancellation;
use tokio::sync::watch;

use crate::clock::NeverCancelled;

/// Asks a run to stop. A caller gives one to a run with
/// [`crate::RunRequest::cancellation`], keeps a clone, and fires it with
/// [`CancelHandle::cancel`], from a signal handler or a task of its own.
///
/// The run stops the provider call or the tool calls in flight, and still
/// ends with an outcome, a transcript and a wide event, which say the run
/// was cancelled. Once fired, a handle stays fired: a run given it after
/// that stops before its first provider call.
///
/// Clones are one handle, and two handles are equal when they're clones.
#[derive(Debug, Clone)]
pub struct CancelHandle {
    fired: Arc<watch::Sender<bool>>,
}

impl CancelHandle {
    /// A handle that hasn't been fired.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fired: Arc::new(watch::Sender::new(false)),
        }
    }

    /// Asks every run this handle was given to stop.
    pub fn cancel(&self) {
        self.fired.send_replace(true);
    }

    /// Whether the handle has been fired.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.fired.borrow()
    }

    /// Resolves once the handle is fired, and at once if it has been.
    pub async fn cancelled(&self) {
        let mut fired = self.fired.subscribe();
        // The sender is this handle's own, so it outlives the wait, and the
        // wait ends only when the handle is fired.
        let _ = fired.wait_for(|fired| *fired).await;
    }
}

impl Default for CancelHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for CancelHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.fired, &other.fired)
    }
}

impl Eq for CancelHandle {}

#[async_trait::async_trait]
impl Cancellation for CancelHandle {
    fn is_cancelled(&self) -> bool {
        Self::is_cancelled(self)
    }

    async fn cancelled(&self) {
        Self::cancelled(self).await;
    }
}

/// The cancellation a `Lablet`'s loop is built with: the handle of the run
/// in progress, which each run sets from its request.
pub(crate) struct RunCancellation {
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
    /// Answers for `handle` until the next run sets its own, or for no
    /// handle when `handle` is `None`.
    pub(crate) fn set(&self, handle: Option<CancelHandle>) {
        let current: Arc<dyn Cancellation> = match handle {
            Some(handle) => Arc::new(handle),
            None => Arc::new(NeverCancelled),
        };
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
