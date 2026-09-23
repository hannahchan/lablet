use serde_json::json;

use super::*;
use crate::ToolResultContent;

fn docs_server() -> ToolSource {
    ToolSource::Mcp {
        server: "docs".to_owned(),
    }
}

/// A call to a built-in tool, which ended `ended`.
const fn ran(ended: ToolCallEnd) -> ToolCallStatus {
    ToolCallStatus::ran(ToolSource::Builtin, ended)
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

#[test]
fn a_tool_source_serialises_under_the_spelling_it_prints() {
    assert_eq!(
        serde_json::to_value(ToolSource::Builtin).unwrap(),
        json!("builtin")
    );
    assert_eq!(
        serde_json::to_value(docs_server()).unwrap(),
        json!({ "mcp": { "server": "docs" } })
    );
    assert_eq!(
        serde_json::from_value::<ToolSource>(json!({ "mcp": { "server": "docs" } })).unwrap(),
        docs_server()
    );
    assert_eq!(
        serde_json::from_value::<ToolSource>(json!("builtin")).unwrap(),
        ToolSource::Builtin
    );
}

#[test]
fn a_tool_spec_has_one_json_form() {
    let spec = ToolSpec {
        name: ToolName::new("search").unwrap(),
        description: "Search the docs.".to_owned(),
        input_schema: json!({ "type": "object" }),
        source: docs_server(),
        concurrency: ToolConcurrency::Shared,
    };
    let expected = json!({
        "name": "search",
        "description": "Search the docs.",
        "input_schema": { "type": "object" },
        "source": { "mcp": { "server": "docs" } },
        "concurrency": "shared",
    });

    assert_eq!(serde_json::to_value(&spec).unwrap(), expected);
    assert_eq!(serde_json::from_value::<ToolSpec>(expected).unwrap(), spec);
}

// Every spelling but `ok` is a value of `error.type` on the `execute_tool`
// span, which is a semantic-convention attribute with no registry enum to
// compare with.
const STATUSES: [(ToolCallStatus, &str); 5] = [
    (ran(ToolCallEnd::Ok), "ok"),
    (ran(ToolCallEnd::ToolError), "tool_error"),
    (ToolCallStatus::Unknown, "unknown"),
    (ran(ToolCallEnd::Timeout), "timeout"),
    (ran(ToolCallEnd::Failed), "failed"),
];

#[test]
fn every_status_prints_as_its_error_type_spelling() {
    for (status, spelling) in STATUSES {
        assert_eq!(status.as_str(), spelling);
        assert_eq!(status.to_string(), spelling);
    }
}

#[test]
fn every_status_but_ok_is_an_error_result_for_the_model() {
    for (status, spelling) in STATUSES {
        assert_eq!(status.is_error(), spelling != "ok", "{status}");
    }
}

#[test]
fn only_a_call_that_ran_has_a_source() {
    assert_eq!(ToolCallStatus::Unknown.source(), None);
    assert_eq!(
        ToolCallStatus::ran(docs_server(), ToolCallEnd::Failed).source(),
        Some(&docs_server())
    );
}

/// The flattening that `as_str` does is for telemetry; the document keeps the
/// two levels, so a reader can tell a built-in failure from an MCP one.
#[test]
fn a_status_serialises_as_the_two_levels_it_holds() {
    assert_eq!(
        serde_json::to_value(ToolCallStatus::Unknown).unwrap(),
        json!("unknown")
    );
    assert_eq!(
        serde_json::from_value::<ToolCallStatus>(json!("unknown")).unwrap(),
        ToolCallStatus::Unknown
    );

    let timed_out = ToolCallStatus::ran(docs_server(), ToolCallEnd::Timeout);
    let expected = json!({
        "ran": { "source": { "mcp": { "server": "docs" } }, "ended": "timeout" },
    });

    assert_eq!(serde_json::to_value(&timed_out).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<ToolCallStatus>(expected).unwrap(),
        timed_out
    );
}

#[test]
fn a_status_that_ran_without_saying_where_the_tool_came_from_is_not_a_status() {
    let no_source = json!({ "ran": { "ended": "ok" } });
    let unknown_field = json!({ "ran": { "source": "builtin", "ended": "ok", "why": "?" } });

    assert!(serde_json::from_value::<ToolCallStatus>(no_source).is_err());
    assert!(serde_json::from_value::<ToolCallStatus>(unknown_field).is_err());
}

fn call_1() -> ToolCallId {
    ToolCallId::new("call_1").unwrap()
}

/// One content block, for the cap tests below.
fn block(text: &str) -> ToolResultContent {
    ToolResultContent::Text(text.to_owned())
}

fn block_texts(content: &[ToolResultContent]) -> Vec<&str> {
    content
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect()
}

#[test]
fn a_tool_nobody_classified_runs_its_calls_alone() {
    assert_eq!(ToolConcurrency::default(), ToolConcurrency::Exclusive);
    assert_eq!(
        serde_json::to_value(ToolConcurrency::Exclusive).unwrap(),
        json!("exclusive")
    );
}

/// An outcome as the run records one: measured as an answer, then given the
/// id of the call it answers.
fn measured(
    call_id: ToolCallId,
    status: ToolCallStatus,
    content: Vec<ToolResultContent>,
    max_output_bytes: Option<u64>,
    started: Duration,
    latency: Duration,
) -> ToolCallOutcome {
    Answer::measured(status, content, max_output_bytes, started, latency).answering(call_id)
}

fn text(text: &str) -> Vec<ToolResultContent> {
    vec![ToolResultContent::Text(text.to_owned())]
}

#[test]
fn an_outcome_holds_what_it_was_given_with_its_times_in_whole_milliseconds() {
    let outcome = measured(
        call_1(),
        ran(ToolCallEnd::Ok),
        text("hello"),
        Some(100),
        Duration::from_micros(1_500_999),
        Duration::from_micros(42_999),
    );

    assert_eq!(
        outcome,
        ToolCallOutcome {
            call_id: call_1(),
            status: ran(ToolCallEnd::Ok),
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
    for (status, spelling) in STATUSES {
        let outcome = measured(
            call_1(),
            status,
            text("no"),
            None,
            Duration::ZERO,
            Duration::ZERO,
        );

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
        ran(ToolCallEnd::Ok),
        text("0123456789"),
        Some(4),
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

#[test]
fn output_that_just_fits_the_cap_is_not_cut() {
    let outcome = measured(
        call_1(),
        ran(ToolCallEnd::Ok),
        text("0123456789"),
        Some(10),
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
        ran(ToolCallEnd::Ok),
        text("0123456789"),
        None,
        Duration::ZERO,
        Duration::ZERO,
    );

    assert_eq!(outcome.content, text("0123456789"));
    assert_eq!(outcome.output_bytes(), 10);
    assert_eq!(outcome.truncated_from_bytes, None);
}

#[test]
fn a_tool_call_outcome_has_one_json_form_without_a_name_an_input_or_an_error_flag() {
    let outcome = measured(
        call_1(),
        ToolCallStatus::ran(docs_server(), ToolCallEnd::Timeout),
        text("timed out after 60s"),
        Some(8),
        Duration::from_millis(2_000),
        Duration::from_millis(60_000),
    );
    let expected = json!({
        "call_id": "call_1",
        "status": {
            "ran": { "source": { "mcp": { "server": "docs" } }, "ended": "timeout" },
        },
        "started_ms": 2_000,
        "latency_ms": 60_000,
        "truncated_from_bytes": 19,
        "content": [
            { "text": "timed ou" },
            { "text": "[truncated: the first 8 of 19 bytes]" },
        ],
    });

    assert_eq!(serde_json::to_value(&outcome).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<ToolCallOutcome>(expected).unwrap(),
        outcome
    );
}

#[test]
fn the_truncation_line_is_worded_one_way() {
    assert_eq!(
        ToolResultContent::truncated(100_000, 5_242_880),
        block("[truncated: the first 100000 of 5242880 bytes]")
    );
}

#[test]
fn content_within_the_cap_is_returned_unchanged() {
    let content = vec![block("0123456789")];

    for cap in [10, 11, u64::MAX] {
        assert_eq!(
            ToolResultContent::capped(content.clone(), cap),
            (content.clone(), None)
        );
    }
}

#[test]
fn content_one_byte_over_the_cap_is_cut_and_says_so_in_one_last_line() {
    let (capped, original) = ToolResultContent::capped(vec![block("0123456789")], 9);

    assert_eq!(original, Some(10));
    assert_eq!(
        block_texts(&capped),
        ["012345678", "[truncated: the first 9 of 10 bytes]"]
    );
}

#[test]
fn the_cut_falls_on_a_character_boundary_and_the_line_counts_what_was_kept() {
    // Each of these characters is two bytes, so a cap of 5 lands inside the third.
    let (capped, original) = ToolResultContent::capped(vec![block("\u{e9}\u{e9}\u{e9}\u{e9}")], 5);

    assert_eq!(original, Some(8));
    assert_eq!(
        block_texts(&capped),
        ["\u{e9}\u{e9}", "[truncated: the first 4 of 8 bytes]"]
    );
}

#[test]
fn the_cap_is_spent_across_the_pieces_in_order_and_what_is_left_over_is_dropped() {
    let content = vec![block("aaaa"), block("bbbbbbbb"), block("cccc")];

    let (capped, original) = ToolResultContent::capped(content, 10);

    assert_eq!(original, Some(16));
    assert_eq!(
        block_texts(&capped),
        ["aaaa", "bbbbbb", "[truncated: the first 10 of 16 bytes]"]
    );
}

#[test]
fn a_cap_of_zero_leaves_only_the_line_that_says_what_was_cut() {
    let (capped, original) = ToolResultContent::capped(vec![block("0123456789")], 0);

    assert_eq!(original, Some(10));
    assert_eq!(
        block_texts(&capped),
        ["[truncated: the first 0 of 10 bytes]"]
    );
}
