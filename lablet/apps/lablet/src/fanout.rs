//! One observer in place of several, since the loop tells one.

use std::sync::Arc;

use lablet_model::ToolCallId;
use lablet_run::{RunEvent, RunObserver, TraceContext};

/// Tells each of its observers every event, in the order they were given.
pub(crate) struct FanOut {
    observers: Vec<Arc<dyn RunObserver>>,
}

impl FanOut {
    pub(crate) fn new(observers: Vec<Arc<dyn RunObserver>>) -> Self {
        Self { observers }
    }
}

#[async_trait::async_trait]
impl RunObserver for FanOut {
    async fn on(&self, event: RunEvent) {
        if let Some((last, others)) = self.observers.split_last() {
            for observer in others {
                observer.on(event.clone()).await;
            }
            last.on(event).await;
        }
    }

    /// The span of the first observer that opened one for the call: a call
    /// propagates one context, and the observers are in the order of whose
    /// it is to be.
    fn trace_context(&self, call_id: &ToolCallId) -> Option<TraceContext> {
        self.observers
            .iter()
            .find_map(|observer| observer.trace_context(call_id))
    }
}

#[cfg(test)]
mod tests;
