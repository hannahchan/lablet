//! C8, as far as it's of telemetry: a run cancelled with a call in flight
//! ends every span it started, and its wide event is made and sums them.

use std::time::Duration;

use lablet_conformance::observer::{
    assert_the_wide_event_is_declared, assert_the_wide_event_sums_its_steps,
};
use lablet_conformance::otlp::Status;
use lablet_model::StopReason;
use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::signals::{
    SPAN_LABLET_CHAT_KEYS, SPAN_LABLET_CHAT_REQUIRED, SPAN_LABLET_EXECUTE_TOOL_KEYS,
    SPAN_LABLET_EXECUTE_TOOL_REQUIRED,
};

use crate::harness::{RUN, STARTED_UNIX_MS, assert_declared, count, traced};

/// A response that calls `bash`, which takes a second, and then
/// `read_file`, which runs after it, and a response that's never bought.
const CALLS_BASH_THEN_READS: &str = r"
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: cargo test --quiet } } }
      - tool_use: { id: call_2, name: read_file, input: { json: { path: src/parser.rs } } }
    usage: { input_tokens: 1000, output_tokens: 50 }
    finish: tool_use
    latency: 250ms
- response: { content: [{ text: Never reached. }], finish: end_turn }
";

/// An attempt that fails, and a retry that takes ten minutes to answer.
const FAILS_THEN_TAKES_ITS_TIME: &str = r"
- error: { kind: retryable, message: 529 overloaded, latency: 40ms }
- response: { content: [{ text: Never reached. }], finish: end_turn, latency: 10m }
";

/// When a span started and how long it lasted, in milliseconds into the
/// run.
fn timing(span: &lablet_conformance::otlp::Span) -> (u64, u64) {
    (
        span.start_unix_nano / 1_000_000 - STARTED_UNIX_MS,
        span.duration_ms(),
    )
}

#[tokio::test(start_paused = true)]
async fn a_run_cancelled_during_a_tool_call_ends_the_call_s_span_and_has_its_wide_event() {
    let traced = traced("c8-tool-call", CALLS_BASH_THEN_READS, |settings| {
        settings.cancelled_after = Some(Duration::from_millis(750));
    })
    .await;

    assert_eq!(
        traced.finished.summary.outcome.stop_reason(),
        StopReason::Cancelled
    );
    let tools = traced.tools();
    assert_eq!(
        tools.len(),
        1,
        "the call in flight has a span, and the call after it never started"
    );
    let bash = tools[0];
    assert_eq!(bash.name, "execute_tool bash");
    assert_eq!(bash.attributes[key::LABLET_TOOL_STATUS], "cancelled");
    assert_eq!(bash.attributes[key::ERROR_TYPE], "cancelled");
    assert_eq!(bash.status, Status::Error(String::new()));
    assert_eq!(
        timing(bash),
        (250, 500),
        "the call was stopped when the run was cancelled, half-way through"
    );
    assert_declared(
        "the tool span",
        &bash.attributes,
        SPAN_LABLET_EXECUTE_TOOL_REQUIRED,
        SPAN_LABLET_EXECUTE_TOOL_KEYS,
    );
    assert_eq!(traced.chats().len(), 1);
    let root = traced.root();
    assert_eq!(root.attributes[key::LABLET_RUN_STOP_REASON], "cancelled");
    assert_eq!(
        timing(root),
        (0, 750),
        "the run ended when it was cancelled"
    );

    let wide = traced.wide();
    assert_the_wide_event_is_declared(wide);
    assert_the_wide_event_sums_its_steps(&traced.exported, RUN);
    for (key, counted) in [
        (key::LABLET_TOOL_CALLS_TOTAL, 1),
        (key::LABLET_TOOL_CALLS_ERRORS, 1),
        (key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL, 500),
        ("lablet.tool.calls.bash", 1),
        ("lablet.tool.errors.bash", 1),
    ] {
        assert_eq!(count(&wide.attributes, key), counted, "{key}");
    }
    assert_eq!(
        wide.attributes.get("lablet.tool.calls.read_file"),
        None,
        "a call that never started counts nowhere"
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_cancelled_during_a_provider_attempt_ends_the_attempt_s_span_and_has_its_wide_event()
{
    let traced = traced(
        "c8-provider-attempt",
        FAILS_THEN_TAKES_ITS_TIME,
        |settings| {
            settings.cancelled_after = Some(Duration::from_secs(1));
        },
    )
    .await;

    assert_eq!(
        traced.finished.summary.outcome.stop_reason(),
        StopReason::Cancelled
    );
    let chats = traced.chats();
    assert_eq!(chats.len(), 2, "every attempt that started has a span");
    let dropped = chats[1];
    assert_eq!(count(&dropped.attributes, key::LABLET_ATTEMPT), 2);
    assert_eq!(dropped.attributes[key::ERROR_TYPE], "cancelled");
    assert_eq!(
        dropped.status,
        Status::Error("the run was cancelled while the attempt was in flight".to_owned())
    );
    assert_eq!(
        timing(dropped),
        (140, 860),
        "the attempt began after the backoff and was dropped when the run was cancelled"
    );
    assert_eq!(dropped.attributes.get(key::GEN_AI_USAGE_INPUT_TOKENS), None);
    assert!(dropped.events.is_empty(), "no retry was decided");
    assert_declared(
        "the chat span",
        &dropped.attributes,
        SPAN_LABLET_CHAT_REQUIRED,
        SPAN_LABLET_CHAT_KEYS,
    );
    assert_eq!(
        traced
            .exported
            .records_of("gen_ai.client.operation.exception")
            .len(),
        1,
        "only the attempt that failed raised an exception"
    );
    assert_eq!(timing(traced.root()), (0, 1_000));

    let wide = traced.wide();
    assert_the_wide_event_is_declared(wide);
    assert_the_wide_event_sums_its_steps(&traced.exported, RUN);
    for (key, counted) in [
        (key::LABLET_RUN_TURNS, 0),
        (key::LABLET_PROVIDER_RETRIES, 1),
        (key::LABLET_PROVIDER_LATENCY_MS_TOTAL, 900),
        (key::LABLET_PROVIDER_LATENCY_MS_MAX, 860),
        (key::LABLET_RUN_DURATION_MS, 1_000),
    ] {
        assert_eq!(count(&wide.attributes, key), counted, "{key}");
    }
}
