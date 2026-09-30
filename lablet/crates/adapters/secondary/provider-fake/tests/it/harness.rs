//! The loop, built around a scripted provider, on tokio's clock. A test
//! pauses that clock, so a script's latencies and the loop's waits pass at
//! once and are measured exactly.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lablet_model::FinishedRun;
use lablet_provider_fake::FakeProvider;
use lablet_run::{EventKind, RunEvent, RunObserver, RunService};
use lablet_test_support::{RunBuilder, context, prompts, scripted};

/// How long one attempt may take in these runs, shorter than a built loop
/// gives it so that a script's latency can outlast it.
pub const PROVIDER_TIMEOUT: Duration = Duration::from_secs(1);

/// Keeps every event, in the order the loop emitted them.
#[derive(Default)]
pub struct Recorder {
    events: Mutex<Vec<EventKind>>,
}

#[async_trait::async_trait]
impl RunObserver for Recorder {
    async fn on(&self, event: RunEvent) {
        self.events.lock().unwrap().push(event.kind);
    }
}

impl Recorder {
    /// How many attempts of a provider call the loop began.
    pub fn attempts(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, EventKind::ProviderCallStarted { .. }))
            .count()
    }

    /// Each failed attempt, in order: the failure's message, how long the
    /// attempt took, and the wait before the next, when there was one.
    pub fn failures(&self) -> Vec<(String, u64, Option<Duration>)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                EventKind::ProviderCallFailed {
                    error,
                    latency_ms,
                    retry,
                    ..
                } => Some((error.message().to_owned(), *latency_ms, *retry)),
                _ => None,
            })
            .collect()
    }
}

/// A loop with no tools, around a provider that plays a script.
pub struct Harness {
    pub provider: Arc<FakeProvider>,
    pub recorder: Arc<Recorder>,
    service: RunService,
}

impl Harness {
    /// The loop around the YAML script `text`, which retries a failed
    /// attempt up to `max_retries` times.
    pub async fn playing(text: &str, max_retries: u32) -> Self {
        let provider = scripted(text);
        let recorder = Arc::new(Recorder::default());
        let service = RunBuilder::new(Arc::clone(&provider) as _)
            .observer(Arc::clone(&recorder) as _)
            .max_retries(max_retries)
            .provider_timeout(PROVIDER_TIMEOUT)
            .build()
            .await;
        Self {
            provider,
            recorder,
            service,
        }
    }

    /// One run, under the run id `run_id`.
    pub async fn run(&mut self, run_id: &str) -> FinishedRun {
        self.service.run(context(run_id), prompts()).await
    }
}
