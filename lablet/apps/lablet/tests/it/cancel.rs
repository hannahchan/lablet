//! A library caller's way to stop a run: a handle it gives the request and
//! fires, which the CLI's Ctrl-C and `SIGTERM` go through.

use std::sync::Arc;

use lablet::{CancelHandle, EventKind, RunEvent, RunObserver, StopReason};
use serde_json::json;

use crate::harness::{ENDS, Lab, PROMPT, Traced, json_of, observed, request};

const CALLS_THEN_ENDS: &str = "
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo one test fails } } }
    finish: tool_use
- response:
    content:
      - text: One test fails.
    finish: end_turn
";

/// Fires a handle when a tool call starts, as a signal that came while a
/// command ran would.
struct FiresOnToolCall(CancelHandle);

#[async_trait::async_trait]
impl RunObserver for FiresOnToolCall {
    async fn on(&self, event: RunEvent) {
        if matches!(event.kind, EventKind::ToolCallStarted { .. }) {
            self.0.cancel();
        }
    }
}

#[tokio::test]
async fn a_run_given_a_fired_handle_stops_before_its_first_call_and_leaves_its_record() {
    let scratch = Lab::new("cancel-fired");
    let transcript = scratch.at("transcript.json");
    let config = scratch.config(ENDS, json!({ "run": { "transcript_path": transcript } }));
    let (mut lablet, recorder) = observed(config).await;
    let handle = CancelHandle::new();
    handle.cancel();

    let finished = lablet.run(request().cancellation(handle)).await;
    lablet.shutdown().await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Cancelled);
    assert_eq!(lablet::ErrorClass::of_run(outcome), None);
    assert_eq!(outcome.turns, 0);
    assert_eq!(recorder.attempts(), 0);
    let document = json_of(&transcript);
    assert_eq!(document["turns"], json!([]));
    assert_eq!(document["task_prompt"], json!(PROMPT));
    let exported = scratch.exported();
    assert_eq!(
        Traced::of(&exported, outcome.run_id.as_str())
            .wide()
            .attributes["lablet.run.stop_reason"],
        json!("cancelled")
    );
}

/// C8 through the library: the handle fires while a tool call runs, the
/// run stops once the call has returned, and no provider call follows it.
#[tokio::test]
async fn a_handle_fired_during_a_tool_call_stops_the_run_with_no_further_provider_call() {
    let scratch = Lab::new("cancel-during-call");
    let config = scratch.config(
        CALLS_THEN_ENDS,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    let handle = CancelHandle::new();
    let recorder = Arc::new(crate::harness::Recorder::default());
    let mut lablet = lablet::build_observed(
        config,
        vec![
            Arc::new(FiresOnToolCall(handle.clone())) as _,
            Arc::clone(&recorder) as _,
        ],
    )
    .await
    .unwrap();

    let finished = lablet.run(request().cancellation(handle.clone())).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Cancelled);
    assert_eq!((outcome.turns, outcome.tool_calls), (1, 1));
    assert_eq!(recorder.attempts(), 1);
    assert!(handle.is_cancelled());

    // The handle was the request's: the next run has none, and completes.
    let next = lablet.run(request()).await;
    lablet.shutdown().await;
    assert_eq!(next.summary.outcome.stop_reason(), StopReason::Completed);
}

#[tokio::test]
async fn a_handle_that_is_never_fired_changes_nothing_of_a_run() {
    let scratch = Lab::new("cancel-never");
    let (mut lablet, _) = observed(scratch.config(ENDS, json!({}))).await;

    let finished = lablet
        .run(request().cancellation(CancelHandle::new()))
        .await;
    lablet.shutdown().await;

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
}
