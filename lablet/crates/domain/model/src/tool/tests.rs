use serde_json::json;

use super::*;
use crate::{OutputCut, ToolResultContent};

fn docs_server() -> ToolSource {
    ToolSource::Mcp {
        server: "docs".to_owned(),
    }
}

/// A call to a tool from `source`, which ended `ended`.
const fn ran(source: ToolSource, ended: ToolCallEnd) -> ToolCallStatus {
    ToolCallStatus::ran(source, ended)
}

// The literal spellings are the members of `lablet.tool.source` in the
// telemetry registry, which this crate can't depend on.
#[test]
fn as_str_is_the_telemetry_spelling_of_the_variant_without_the_server() {
    assert_eq!(ToolSource::Builtin.as_str(), "builtin");
    assert_eq!(docs_server().as_str(), "mcp");
}

#[test]
fn a_tool_source_prints_the_server_of_an_mcp_tool() {
    assert_eq!(ToolSource::Builtin.to_string(), "builtin");
    assert_eq!(docs_server().to_string(), "mcp:docs");
}

/// The bytes a run's tools digest is taken from, key order included: a
/// change to them changes the digest of every run.
#[test]
fn a_tool_spec_is_measured_as_these_exact_bytes() {
    let spec = ToolSpec {
        name: ToolName::new("search").unwrap(),
        description: "Search the docs.".to_owned(),
        input_schema: json!({ "type": "object" }),
        source: docs_server(),
        concurrency: ToolConcurrency::Shared,
    };
    let builtin = ToolSpec {
        name: ToolName::new("bash").unwrap(),
        source: ToolSource::Builtin,
        concurrency: ToolConcurrency::Exclusive,
        ..spec.clone()
    };

    assert_eq!(
        serde_json::to_string(&spec).unwrap(),
        concat!(
            r#"{"name":"search","description":"Search the docs.","#,
            r#""input_schema":{"type":"object"},"#,
            r#""source":{"mcp":{"server":"docs"}},"concurrency":"shared"}"#
        )
    );
    assert_eq!(
        serde_json::to_string(&builtin).unwrap(),
        concat!(
            r#"{"name":"bash","description":"Search the docs.","#,
            r#""input_schema":{"type":"object"},"#,
            r#""source":"builtin","concurrency":"exclusive"}"#
        )
    );
}

// Every spelling but `ok` and `not_run` is a value of `error.type` on the
// `execute_tool` span, which is a semantic-convention attribute with no
// registry enum to compare with. A call that was never run has no span.
const fn spelling(status: &ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Unknown => "unknown",
        ToolCallStatus::MalformedInput => "malformed_input",
        ToolCallStatus::Rejected => "rejected",
        ToolCallStatus::NotRun => "not_run",
        ToolCallStatus::Ran { ended, .. } => match ended {
            ToolCallEnd::Ok => "ok",
            ToolCallEnd::ToolError => "tool_error",
            ToolCallEnd::Timeout => "timeout",
            ToolCallEnd::Failed => "failed",
            ToolCallEnd::Cancelled => "cancelled",
        },
    }
}

/// Every status at both levels, each tool that ran a built-in one.
fn every_status() -> impl Iterator<Item = ToolCallStatus> {
    let ended = ToolCallEnd::ALL.map(|ended| ran(ToolSource::Builtin, ended));
    ToolCallStatus::NOTHING_RAN.into_iter().chain(ended)
}

#[test]
fn every_status_prints_as_its_error_type_spelling() {
    for status in every_status() {
        assert_eq!(status.as_str(), spelling(&status));
        assert_eq!(status.to_string(), spelling(&status));
    }
}

#[test]
fn every_status_but_ok_is_an_error_result_for_the_model() {
    for status in every_status() {
        assert_eq!(status.is_error(), spelling(&status) != "ok", "{status}");
    }
}

#[test]
fn a_call_is_invalid_when_no_tool_was_reached_and_never_when_one_was() {
    assert!(ToolCallStatus::Unknown.is_invalid());
    assert!(ToolCallStatus::MalformedInput.is_invalid());
    assert!(
        ToolCallStatus::Rejected.is_invalid(),
        "the loop answered it, so it reached no tool"
    );
    assert!(
        !ToolCallStatus::NotRun.is_invalid(),
        "the run's time had gone, which the model didn't get wrong"
    );
    for ended in ToolCallEnd::ALL {
        assert!(!ran(ToolSource::Builtin, ended).is_invalid(), "{ended}");
        assert!(!ran(docs_server(), ended).is_invalid(), "{ended} over MCP");
    }
}

#[test]
fn only_a_call_that_ran_has_a_source() {
    assert_eq!(ToolCallStatus::Unknown.source(), None);
    assert_eq!(ToolCallStatus::MalformedInput.source(), None);
    assert_eq!(ToolCallStatus::Rejected.source(), None);
    assert_eq!(ToolCallStatus::NotRun.source(), None);
    assert_eq!(
        ran(docs_server(), ToolCallEnd::Failed).source(),
        Some(&docs_server())
    );
}

/// A call the loop answered for its arguments, or rejected, still named a
/// tool the run has: only `unknown` says the name was the mistake. The name
/// of a call that was never run was looked up by nothing, so it's known to
/// name no tool, whatever it was.
#[test]
fn a_call_named_a_tool_the_run_offered_unless_its_name_was_unknown_or_never_looked_up() {
    for status in every_status() {
        assert_eq!(
            status.names_an_offered_tool(),
            !matches!(spelling(&status), "unknown" | "not_run"),
            "{status}"
        );
    }
}

/// The loop answers an invalid call without a tool, and it still took the
/// call up and told the observer. Only `not_run` says it never did.
#[test]
fn something_was_started_for_every_call_but_one_that_was_never_run() {
    for status in every_status() {
        assert_eq!(
            status.was_started(),
            spelling(&status) != "not_run",
            "{status}"
        );
    }
}

fn call_1() -> ToolCallId {
    ToolCallId::new("call_1").unwrap()
}

#[test]
fn a_tool_nobody_classified_runs_its_calls_alone() {
    assert_eq!(ToolConcurrency::default(), ToolConcurrency::Exclusive);
}

/// An outcome as the run records one: measured as an answer, then given the
/// id of the call it answers.
fn measured(
    call_id: ToolCallId,
    status: ToolCallStatus,
    output: &str,
    cap: Option<OutputCap>,
    started: Duration,
    latency: Duration,
) -> ToolCallOutcome {
    Answer::measured(status, KeptOutput::whole(output), cap, started, latency).answering(call_id)
}

/// A cap that keeps the start.
fn head(max_bytes: u64) -> OutputCap {
    OutputCap::new(max_bytes, OutputCut::Head).unwrap()
}

fn text(text: &str) -> Vec<ToolResultContent> {
    vec![ToolResultContent::Text(text.to_owned())]
}

#[test]
fn an_outcome_holds_what_it_was_given_with_its_times_in_whole_milliseconds() {
    let outcome = measured(
        call_1(),
        ran(ToolSource::Builtin, ToolCallEnd::Ok),
        "hello",
        Some(head(100)),
        Duration::from_micros(1_500_999),
        Duration::from_micros(42_999),
    );

    assert_eq!(
        outcome,
        ToolCallOutcome {
            call_id: call_1(),
            status: ran(ToolSource::Builtin, ToolCallEnd::Ok),
            started_ms: 1_500,
            latency_ms: 42,
            truncated_from_bytes: None,
            content: text("hello"),
        }
    );
    assert_eq!(outcome.output_bytes(), 5);
}

#[test]
fn the_result_the_model_is_sent_is_an_error_exactly_when_the_status_is_not_ok() {
    for status in every_status() {
        let spelling = spelling(&status);
        let outcome = measured(call_1(), status, "no", None, Duration::ZERO, Duration::ZERO);

        assert_eq!(
            outcome.result(),
            ToolResult {
                call_id: &call_1(),
                content: &text("no"),
                is_error: spelling != "ok",
            },
            "{spelling}"
        );
    }
}

#[test]
fn output_over_the_cap_is_cut_and_the_outcome_holds_the_size_sent_and_the_size_before() {
    let outcome = measured(
        call_1(),
        ran(ToolSource::Builtin, ToolCallEnd::Ok),
        "0123456789",
        Some(head(4)),
        Duration::ZERO,
        Duration::ZERO,
    );

    assert_eq!(
        outcome.content,
        [
            ToolResultContent::Text("0123".to_owned()),
            ToolResultContent::Text("[truncated: the first 4 of 10 bytes]".to_owned()),
        ]
    );
    assert_eq!(outcome.truncated_from_bytes, Some(10));
    assert_eq!(outcome.output_bytes(), 4 + 36);
}

/// The size the model was sent counts the line and both ends, so it can
/// exceed the cap, which bounds the tool's own text.
#[test]
fn an_answer_holds_what_the_cap_s_cut_sends_of_what_an_executor_kept() {
    let cap = OutputCap::new(10, OutputCut::HeadTail).unwrap();
    let mut output = KeptOutput::new(Some(cap.keeps()));
    output.push("0123456789abcdef");

    let answer = Answer::measured(
        ran(ToolSource::Builtin, ToolCallEnd::ToolError),
        output,
        Some(cap),
        Duration::ZERO,
        Duration::ZERO,
    );

    assert_eq!(
        answer.content(),
        [
            ToolResultContent::Text("01234".to_owned()),
            ToolResultContent::Text("[truncated: 6 of 16 bytes left out]".to_owned()),
            ToolResultContent::Text("bcdef".to_owned()),
        ]
    );
    assert_eq!(answer.truncated_from_bytes(), Some(16));
    assert_eq!(answer.output_bytes(), 5 + 35 + 5);
    assert_eq!(
        answer.status(),
        &ran(ToolSource::Builtin, ToolCallEnd::ToolError),
        "a cut changes nothing else about the answer"
    );
}

#[test]
fn output_that_just_fits_the_cap_is_not_cut() {
    let outcome = measured(
        call_1(),
        ran(ToolSource::Builtin, ToolCallEnd::Ok),
        "0123456789",
        Some(head(10)),
        Duration::ZERO,
        Duration::ZERO,
    );

    assert_eq!(outcome.content, text("0123456789"));
    assert_eq!(outcome.truncated_from_bytes, None);
}

#[test]
fn without_a_cap_no_output_is_cut() {
    let outcome = measured(
        call_1(),
        ran(ToolSource::Builtin, ToolCallEnd::Ok),
        "0123456789",
        None,
        Duration::ZERO,
        Duration::ZERO,
    );

    assert_eq!(outcome.content, text("0123456789"));
    assert_eq!(outcome.output_bytes(), 10);
    assert_eq!(outcome.truncated_from_bytes, None);
}
