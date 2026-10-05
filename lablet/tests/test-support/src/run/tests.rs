use std::time::Duration;

use lablet_model::StopReason;
use lablet_run::telemetry::generated::key;
use opentelemetry::global::BoxedTracer;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

use super::*;

const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";

/// A response that ends the run, and nothing before it.
const ENDS: &str = "
- response:
    content:
      - text: Nothing to fix.
    usage: { input_tokens: 100, output_tokens: 10 }
    finish: end_turn
";

/// What the loop said of each failed attempt, on its span's `lablet.retry`
/// event: how long it waited before the next, when it tried again.
fn waits(spans: &InMemorySpanExporter) -> Vec<Option<i64>> {
    spans
        .get_finished_spans()
        .unwrap()
        .iter()
        .flat_map(|span| span.events.iter())
        .map(|retry| {
            retry
                .attributes
                .iter()
                .find(|attribute| attribute.key.as_str() == key::LABLET_RETRY_BACKOFF_MS)
                .map(|backoff| match &backoff.value {
                    opentelemetry::Value::I64(ms) => *ms,
                    other => panic!("{other:?}"),
                })
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_loop_built_with_nothing_said_runs_with_the_policies_its_builder_names() {
    let mut service = RunBuilder::new(scripted(ENDS)).build().await;

    let finished = service.run(context(RUN), prompts()).await;

    let summary = &finished.summary;
    assert_eq!(summary.outcome.stop_reason(), StopReason::Completed);
    assert_eq!(summary.model.name, MODEL);
    assert!(summary.tools.is_empty(), "{:?}", summary.tools);
    assert_eq!(summary.completion, CompletionMode::Natural);
    assert_eq!(summary.max_turns, None);
    assert_eq!(summary.timeout_ms, 3_600_000);
    assert_eq!(summary.request, request());
    assert_eq!(summary.rates, None);
    assert_eq!(finished.transcript.system(), SYSTEM);
}

#[tokio::test(start_paused = true)]
async fn a_loop_built_with_nothing_said_tries_a_call_three_times_more_after_waits_that_double() {
    let fails = "- error: { kind: retryable, message: 529 overloaded }\n".repeat(4);
    let spans = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(spans.clone())
        .build();
    let mut service = RunBuilder::new(scripted(&fails))
        .tracer(BoxedTracer::new(Box::new(provider.tracer("test"))))
        .build()
        .await;

    let finished = service.run(context(RUN), prompts()).await;

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::RetriesExhausted
    );
    assert_eq!(waits(&spans), [Some(100), Some(200), Some(400), None]);
}

#[tokio::test(start_paused = true)]
async fn a_loop_built_with_a_cancellation_is_stopped_by_it() {
    let slow = "- response: { content: [{ text: Done. }], finish: end_turn, latency: 10m }\n";
    let mut service = RunBuilder::new(scripted(slow))
        .cancellation(Arc::new(crate::CancelledAfter::new(Duration::from_secs(1))))
        .build()
        .await;

    let finished = service.run(context(RUN), prompts()).await;

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Cancelled
    );
    assert_eq!(finished.summary.outcome.duration_ms, 1_000);
}

#[test]
fn a_run_s_context_names_the_run_and_nothing_a_run_may_be_without() {
    let context = context(RUN);

    assert_eq!(context.run_id.as_str(), RUN);
    assert_eq!(context.labels, RunLabels::default());
    assert_eq!(context.started_unix_ms, STARTED_UNIX_MS);
    assert_eq!(context.config_digest.as_str(), CONFIG_DIGEST);
    assert_eq!(context.agent_version, AGENT_VERSION);
    assert_eq!(context.transcript_path, None);
    assert_eq!(context.skills_count, 0);
    assert_eq!(context.mcp, None);
    assert!(!context.capture_content);
}
