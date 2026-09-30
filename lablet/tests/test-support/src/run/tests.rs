use std::sync::Mutex;
use std::time::Duration;

use lablet_model::StopReason;
use lablet_run::EventKind;

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

/// Keeps what the loop said of each failed attempt: how long it waited
/// before the next, when it tried again.
#[derive(Default)]
struct Waits(Mutex<Vec<Option<Duration>>>);

#[async_trait::async_trait]
impl RunObserver for Waits {
    async fn on(&self, event: RunEvent) {
        if let EventKind::ProviderCallFailed { retry, .. } = event.kind {
            self.0.lock().unwrap().push(retry);
        }
    }
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
    let waits = Arc::new(Waits::default());
    let mut service = RunBuilder::new(scripted(&fails))
        .observer(Arc::clone(&waits) as _)
        .build()
        .await;

    let finished = service.run(context(RUN), prompts()).await;

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::RetriesExhausted
    );
    assert_eq!(
        *waits.0.lock().unwrap(),
        [
            Some(Duration::from_millis(100)),
            Some(Duration::from_millis(200)),
            Some(Duration::from_millis(400)),
            None,
        ]
    );
}

#[test]
fn a_run_s_context_names_the_run_and_nothing_a_run_may_be_without() {
    let context = context(RUN);

    assert_eq!(context.run_id.as_str(), RUN);
    assert_eq!(context.labels, RunLabels::default());
    assert_eq!(context.started_unix_ms, STARTED_UNIX_MS);
    assert_eq!(context.config_digest, CONFIG_DIGEST);
    assert_eq!(context.agent_version, AGENT_VERSION);
    assert_eq!(context.transcript_path, None);
    assert_eq!(context.skills_count, 0);
    assert_eq!(context.mcp, None);
    assert!(!context.capture_content);
}
