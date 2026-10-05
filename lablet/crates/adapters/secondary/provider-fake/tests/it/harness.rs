//! The loop, built around a scripted provider, on tokio's clock. A test
//! pauses that clock, so a script's latencies and the loop's waits pass at
//! once and are measured exactly.

use std::sync::Arc;
use std::time::Duration;

use lablet_model::FinishedRun;
use lablet_provider_fake::FakeProvider;
use lablet_run::RunService;
use lablet_run::telemetry::generated::{LabletChat, LabletRetry, key};
use lablet_test_support::{RunBuilder, context, prompts, scripted};
use opentelemetry::global::BoxedTracer;
use opentelemetry::trace::{Status, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};

/// How long one attempt may take in these runs, shorter than a built loop
/// gives it so that a script's latency can outlast it.
pub const PROVIDER_TIMEOUT: Duration = Duration::from_secs(1);

/// The spans the loop opened, read from the SDK's in-memory exporter.
pub struct Recorder {
    spans: InMemorySpanExporter,
    /// The provider the loop emitted through, held because an in-memory
    /// exporter forgets everything when its provider shuts down.
    _provider: SdkTracerProvider,
}

impl Recorder {
    fn new() -> (Self, BoxedTracer) {
        let spans = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(spans.clone())
            .build();
        let tracer = BoxedTracer::new(Box::new(provider.tracer("test")));
        (
            Self {
                spans,
                _provider: provider,
            },
            tracer,
        )
    }

    /// The chat spans, one for each attempt of a provider call, in the
    /// order the attempts ended.
    fn chats(&self) -> Vec<SpanData> {
        self.spans
            .get_finished_spans()
            .unwrap()
            .into_iter()
            .filter(|span| {
                span.attributes.iter().any(|attribute| {
                    attribute.key.as_str() == key::GEN_AI_OPERATION_NAME
                        && attribute.value.as_str() == LabletChat::GEN_AI_OPERATION_NAME
                })
            })
            .collect()
    }

    /// How many attempts of a provider call the loop began.
    pub fn attempts(&self) -> usize {
        self.chats().len()
    }

    /// Each failed attempt, in order: the failure's message, how long the
    /// attempt took, and the wait before the next, when there was one.
    pub fn failures(&self) -> Vec<(String, u64, Option<Duration>)> {
        self.chats()
            .into_iter()
            .filter_map(|chat| {
                let Status::Error { description } = &chat.status else {
                    return None;
                };
                let lasted = chat
                    .end_time
                    .duration_since(chat.start_time)
                    .unwrap_or_default();
                let retry = chat
                    .events
                    .iter()
                    .find(|event| event.name == LabletRetry::NAME)
                    .unwrap_or_else(|| panic!("a failed attempt's span says whether it's retried"));
                let backoff = retry
                    .attributes
                    .iter()
                    .find(|attribute| attribute.key.as_str() == key::LABLET_RETRY_BACKOFF_MS)
                    .map(|backoff| match backoff.value {
                        opentelemetry::Value::I64(ms) => {
                            Duration::from_millis(u64::try_from(ms).unwrap())
                        }
                        ref other => panic!("{other:?}"),
                    });
                Some((
                    description.to_string(),
                    u64::try_from(lasted.as_millis()).unwrap(),
                    backoff,
                ))
            })
            .collect()
    }
}

/// A loop with no tools, around a provider that plays a script.
pub struct Harness {
    pub provider: Arc<FakeProvider>,
    pub recorder: Recorder,
    service: RunService,
}

impl Harness {
    /// The loop around the YAML script `text`, which retries a failed
    /// attempt up to `max_retries` times.
    pub async fn playing(text: &str, max_retries: u32) -> Self {
        let provider = scripted(text);
        let (recorder, tracer) = Recorder::new();
        let service = RunBuilder::new(Arc::clone(&provider) as _)
            .tracer(tracer)
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
