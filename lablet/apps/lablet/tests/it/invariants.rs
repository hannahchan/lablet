//! What the conformance cases held of every observer, held now on what a
//! collector sees: exactly one wide event for each run, in its root span's
//! context, whatever the stop reason and however many runs a `Lablet` makes;
//! the file and the receiver holding the same run; and a destination that
//! can't be written or can't be reached leaving the outcome what a good
//! destination gives.

use lablet::{FinishedRun, OutcomeDocument, RunId, StopReason};
use lablet_conformance::receiver::{Mode, Receiver};
use serde_json::{Value, json};

use crate::harness::{Diagnostics, Lab, Traced, request};
use crate::key;
use crate::wide_checks::{
    assert_hold_the_same_run, assert_the_wide_event_counts_the_tokens_the_run_returned,
    assert_the_wide_event_is_declared, assert_the_wide_event_sums_its_steps, the_wide_event,
};

/// A rejected key, and a response the run never asks for.
const REJECTS_THE_KEY: &str = "
- error: { kind: auth, message: 401 invalid x-api-key }
- response: { content: [{ text: Never said. }], finish: end_turn }
";

/// A failed attempt, a response that calls a tool, and a response that ends
/// the run, so a run has every kind of span to sum.
const FAILS_CALLS_ENDS: &str = "
- error: { kind: retryable, message: 529 overloaded, usage: { input_tokens: 40 } }
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo one test fails } } }
    usage: { input_tokens: 100, output_tokens: 10 }
    finish: tool_use
- response:
    content:
      - text: One test fails.
    usage: { input_tokens: 150, output_tokens: 8 }
    finish: end_turn
";

/// The outcome as its document, without what names the run and what times
/// it: what two runs of one config and one script share, whatever their
/// destinations did.
fn shared(finished: &FinishedRun) -> Value {
    let mut document =
        serde_json::to_value(OutcomeDocument::from(finished.summary.outcome.clone())).unwrap();
    let keys = document.as_object_mut().unwrap();
    assert!(keys.remove("run_id").is_some());
    assert!(keys.remove("duration_ms").is_some());
    document
}

/// One run of `script` under `run_id`, with `more` stated over the config,
/// to a file the lab names.
async fn run(lab: &Lab, script: &str, run_id: &str, more: Value) -> FinishedRun {
    let mut lablet = lablet::build(lab.config(script, more)).await.unwrap();
    let finished = lablet
        .run(request().run_id(RunId::new(run_id).unwrap()).unwrap())
        .await;
    lablet.shutdown().await;
    finished
}

#[tokio::test]
async fn a_completed_run_and_a_failed_run_each_have_exactly_one_wide_event_in_their_roots_context()
{
    let lab = Lab::new("invariants-one-wide-event");
    let more = json!({
        "run": { "retry_backoff_base": "1ms", "retry_backoff_max": "1ms" },
        "tools": { "builtin": lab.builtin(&["bash"]) },
    });
    // Two runs of one `Lablet`, each of which hears the script from its
    // first entry and completes, and a run of a second `Lablet` on the same
    // file, which the provider's error ends.
    let mut completing = lablet::build(lab.config(FAILS_CALLS_ENDS, more.clone()))
        .await
        .unwrap();
    let completed = completing
        .run(request().run_id(RunId::new("completed").unwrap()).unwrap())
        .await;
    let again = completing
        .run(
            request()
                .run_id(RunId::new("completed-again").unwrap())
                .unwrap(),
        )
        .await;
    completing.shutdown().await;
    let mut fails = lablet::build(lab.config(REJECTS_THE_KEY, more))
        .await
        .unwrap();
    let failed = fails
        .run(request().run_id(RunId::new("failed").unwrap()).unwrap())
        .await;
    fails.shutdown().await;

    for finished in [&completed, &again] {
        assert_eq!(
            finished.summary.outcome.stop_reason(),
            StopReason::Completed
        );
    }
    assert_eq!(
        failed.summary.outcome.stop_reason(),
        StopReason::ProviderError
    );
    let exported = lab.exported();
    assert_eq!(exported.records_of(key::WIDE_EVENT).len(), 3);
    for (run_id, finished) in [
        ("completed", &completed),
        ("completed-again", &again),
        ("failed", &failed),
    ] {
        let traced = Traced::of(&exported, run_id);
        let (root, wide) = (traced.root(), the_wide_event(&exported, run_id));
        assert_eq!(
            (&wide.trace_id, &wide.span_id),
            (&root.trace_id, &root.span_id)
        );
        assert_the_wide_event_is_declared(wide);
        assert_the_wide_event_sums_its_steps(&exported, run_id);
        assert_the_wide_event_counts_the_tokens_the_run_returned(
            &exported,
            run_id,
            &finished.summary,
        );
    }
}

#[tokio::test]
async fn the_file_and_the_receiver_hold_the_same_run() {
    let receiver = Receiver::start(Mode::Answers).await;
    let lab = Lab::new("invariants-same-run");
    let more = json!({
        "run": { "retry_backoff_base": "1ms", "retry_backoff_max": "1ms" },
        "tools": { "builtin": lab.builtin(&["bash"]) },
        "telemetry": { "otlp": { "endpoint": receiver.grpc_endpoint() } },
    });

    run(&lab, FAILS_CALLS_ENDS, "both", more).await;

    assert_hold_the_same_run(&lab.exported(), &receiver.exported().unwrap(), "both");
}

#[tokio::test]
async fn a_file_that_cannot_be_written_leaves_the_outcome_what_a_good_destination_gives() {
    let good = Lab::new("invariants-good-file");
    let bad = Lab::new("invariants-unwritable-file");
    let nowhere = bad.at("no-such-directory/telemetry.otlp.jsonl");

    let expected = run(&good, FAILS_CALLS_ENDS, "good", json!({})).await;
    let unwritable = run(
        &bad,
        FAILS_CALLS_ENDS,
        "unwritable",
        json!({ "telemetry": { "file": { "path": nowhere } } }),
    )
    .await;

    assert_eq!(shared(&unwritable), shared(&expected));
    assert!(!nowhere.exists());
    assert_eq!(
        unwritable.summary.outcome.stop_reason(),
        StopReason::Completed
    );
}

#[tokio::test]
async fn an_endpoint_nothing_listens_on_leaves_the_outcome_what_a_good_destination_gives() {
    let closed = Receiver::closed();
    let good = Lab::new("invariants-good-endpoint");
    let unreachable = Lab::new("invariants-closed-endpoint");

    let expected = run(&good, REJECTS_THE_KEY, "good", json!({})).await;
    let diagnostics = Diagnostics::capture();
    let refused = run(
        &unreachable,
        REJECTS_THE_KEY,
        "unreachable",
        json!({ "telemetry": { "otlp": { "endpoint": format!("http://{closed}") } } }),
    )
    .await;

    let lines = diagnostics.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("the run's telemetry wasn't exported whole")),
        "the run's flush said the network destination failed: {lines:?}"
    );

    assert_eq!(shared(&refused), shared(&expected));
    assert_eq!(
        refused.summary.outcome.stop_reason(),
        StopReason::ProviderError
    );
    assert_eq!(
        unreachable.exported().records_of(key::WIDE_EVENT).len(),
        1,
        "the file is whole whatever the port did"
    );
}
