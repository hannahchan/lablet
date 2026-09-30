//! Runs of the loop itself, around a scripted provider and the built-in
//! tools, on the clock as it runs.

use std::sync::Arc;
use std::time::Duration;

use lablet_model::{FinishedRun, Secrets, StopReason, ToolCallOutcome, ToolResultContent};
use lablet_test_support::{RunBuilder, context, prompts, scripted};
use lablet_tools_builtin::{BuiltinTools, Settings, Withheld};

use crate::harness::{Root, link};

const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";

const SECRET: &str = "what the model is not to read";

/// A variable cargo sets for every test, as a key is set for lablet, and
/// withheld as lablet's key is.
const NOT_FOR_A_COMMAND: &str = "CARGO_MANIFEST_DIR";

/// One run of the YAML script `script`, whose tools are `tools`.
async fn run(script: &str, tools: BuiltinTools) -> FinishedRun {
    let mut service = RunBuilder::new(scripted(script))
        .tools(vec![Arc::new(tools) as _])
        .build()
        .await;
    service.run(context(RUN), prompts()).await
}

/// What the model was sent of a call: its status, and its text.
fn sent(outcome: &ToolCallOutcome) -> (&'static str, String) {
    let text = outcome
        .content
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect();
    (outcome.status.as_str(), text)
}

#[tokio::test]
async fn a_command_past_the_timeout_is_an_error_result_of_kind_timeout_and_the_run_goes_on() {
    let scratch = Root::new("run-timeout");
    scratch.holds("notes.md", "on it goes");
    let tools = BuiltinTools::new(Settings {
        timeout: Duration::from_secs(1),
        ..scratch.settings()
    })
    .unwrap();

    let finished = run(
        r"
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: sleep 60 } } }
    finish: tool_use
- response:
    content:
      - tool_use: { id: call_2, name: read_file, input: { json: { path: notes.md } } }
    finish: tool_use
- response:
    content:
      - text: The command takes too long.
    finish: end_turn
",
        tools,
    )
    .await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.result().text, "The command takes too long.");
    assert_eq!((outcome.turns, outcome.tool_calls), (3, 2));
    assert_eq!(finished.summary.tool_calls.errors, 1);
    let turns = finished.transcript.turns();
    let timed_out = &turns[0].tool_calls()[0];
    assert_eq!(timed_out.call_id.as_str(), "call_1");
    assert!(timed_out.status.is_error());
    assert_eq!(
        sent(timed_out),
        (
            "timeout",
            "bash was stopped after 1s, the longest the call could take".to_owned()
        )
    );
    assert!(
        (1_000..10_000).contains(&timed_out.latency_ms),
        "the call took {} ms",
        timed_out.latency_ms
    );
    // The call after the timeout starts no process, so no real-time limit
    // bounds work that has to finish.
    assert_eq!(
        sent(&turns[1].tool_calls()[0]),
        ("ok", "on it goes".to_owned())
    );
}

#[tokio::test]
async fn a_run_is_refused_what_is_outside_the_root_and_kept_from_lablet_s_environment() {
    let scratch = Root::new("run-refusals");
    std::fs::write(scratch.outside("secret.txt"), SECRET).unwrap();
    link(
        &scratch.outside("secret.txt"),
        &scratch.root().join("notes.txt"),
    );
    let held = std::env::var(NOT_FOR_A_COMMAND).expect("cargo sets it for a test");
    let tools = BuiltinTools::new(Settings {
        withheld: Withheld {
            variables: [NOT_FOR_A_COMMAND.to_owned()].into(),
            values: Secrets::new([held.clone()]),
        },
        ..scratch.settings()
    })
    .unwrap();

    let finished = run(
        r"
- response:
    content:
      - tool_use: { id: call_1, name: read_file, input: { json: { path: ../secret.txt } } }
      - tool_use: { id: call_2, name: read_file, input: { json: { path: notes.txt } } }
      - tool_use: { id: call_3, name: bash, input: { json: { command: env } } }
      - tool_use: { id: call_4, name: bash, input: { json: { command: exit 3 } } }
    finish: tool_use
- response:
    content:
      - text: Done.
    finish: end_turn
",
        tools,
    )
    .await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!((outcome.turns, outcome.tool_calls), (2, 4));
    assert_eq!(finished.summary.tool_calls.errors, 2);
    let calls = finished.transcript.turns()[0].tool_calls();
    let ids: Vec<&str> = calls.iter().map(|call| call.call_id.as_str()).collect();
    assert_eq!(ids, ["call_1", "call_2", "call_3", "call_4"]);
    assert_eq!(
        sent(&calls[0]),
        (
            "tool_error",
            "../secret.txt wasn't read: it leads outside the run's root directory".to_owned()
        )
    );
    assert_eq!(
        sent(&calls[1]),
        (
            "tool_error",
            "notes.txt wasn't read: it leads outside the run's root directory".to_owned()
        )
    );
    let (status, environment) = sent(&calls[2]);
    assert_eq!(status, "ok");
    assert!(environment.contains("PATH="), "{environment}");
    assert!(
        !environment.contains(NOT_FOR_A_COMMAND) && !environment.contains(&held),
        "{environment}"
    );
    assert_eq!(sent(&calls[3]), ("ok", "exit code: 3".to_owned()));
    assert!(!calls[3].status.is_error());
}
