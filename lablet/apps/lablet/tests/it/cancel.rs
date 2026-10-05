//! A library caller's way to stop a run: a handle it gives the request and
//! fires, which the CLI's Ctrl-C and `SIGTERM` go through.

use std::path::Path;
use std::time::Duration;

use lablet::{CancelHandle, StopReason};
use serde_json::json;

use crate::harness::{ENDS, Lab, PROMPT, Traced, json_of, request};
use crate::key;

/// A call of `bash` that, the first time it runs, makes the file `started`
/// and then sleeps far longer than the run is given before the handle
/// fires, and that ends at once when the file is there, so a later run of
/// the same script doesn't sit the sleep out; and a response that ends the
/// run, which a run that went on would reach.
fn marks_sleeps_then_ends(started: &Path) -> String {
    format!(
        "
- response:
    content:
      - tool_use:
          id: call_1
          name: bash
          input: {{ json: {{ command: \"test -e '{0}' || {{ touch '{0}' && sleep 30; }}\" }} }}
    finish: tool_use
- response:
    content:
      - text: The command ran to its end.
    finish: end_turn
",
        started.display()
    )
}

#[tokio::test]
async fn a_run_given_a_fired_handle_stops_before_its_first_call_and_leaves_its_record() {
    let scratch = Lab::new("cancel-fired");
    let transcript = scratch.at("transcript.json");
    let config = scratch.config(ENDS, json!({ "run": { "transcript_path": transcript } }));
    let mut lablet = lablet::build(config).await.unwrap();
    let handle = CancelHandle::new();
    handle.cancel();

    let finished = lablet.run(request().cancellation(handle)).await;
    lablet.shutdown().await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Cancelled);
    assert_eq!(lablet::ErrorClass::of_run(outcome), None);
    assert_eq!(outcome.turns, 0);
    let document = json_of(&transcript);
    assert_eq!(document["turns"], json!([]));
    assert_eq!(document["task_prompt"], json!(PROMPT));
    let exported = scratch.exported();
    let traced = Traced::of(&exported, outcome.run_id.as_str());
    assert!(traced.chats().is_empty(), "no provider call was made");
    assert_eq!(
        traced.wide().attributes[key::LABLET_RUN_STOP_REASON],
        json!("cancelled")
    );
}

/// C8 through the library: the handle fires while a tool call runs, the
/// run stops once the call has been dropped, and no provider call follows
/// it. The runs are on the real clock, since `bash` starts a real process,
/// so a task fires the handle once the command says it's running, by the
/// file it makes before it sleeps; that it fired during the call is what
/// the spans say.
#[tokio::test]
async fn a_handle_fired_during_a_tool_call_stops_the_run_with_no_further_provider_call() {
    let scratch = Lab::new("cancel-during-call");
    let started = scratch.root().join("started");
    let config = scratch.config(
        &marks_sleeps_then_ends(&started),
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    let mut lablet = lablet::build(config).await.unwrap();
    let handle = CancelHandle::new();
    let fired = handle.clone();
    tokio::spawn(async move {
        while !started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        fired.cancel();
    });

    let finished = lablet.run(request().cancellation(handle.clone())).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Cancelled);
    assert_eq!((outcome.turns, outcome.tool_calls), (1, 1));
    assert!(handle.is_cancelled());
    let exported = scratch.exported();
    let traced = Traced::of(&exported, outcome.run_id.as_str());
    assert_eq!(traced.chats().len(), 1, "no provider call followed");
    let tools = traced.tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(
        tools[0].attributes[key::LABLET_TOOL_STATUS],
        json!("cancelled"),
        "the handle fired while the call ran"
    );

    // The handle was the request's: the next run has none, and completes.
    let next = lablet.run(request()).await;
    lablet.shutdown().await;
    assert_eq!(next.summary.outcome.stop_reason(), StopReason::Completed);
}

#[tokio::test]
async fn a_handle_that_is_never_fired_changes_nothing_of_a_run() {
    let scratch = Lab::new("cancel-never");
    let mut lablet = lablet::build(scratch.config(ENDS, json!({})))
        .await
        .unwrap();

    let finished = lablet
        .run(request().cancellation(CancelHandle::new()))
        .await;
    lablet.shutdown().await;

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
}
