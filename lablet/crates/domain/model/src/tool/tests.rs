use serde_json::json;

use super::*;

fn docs_server() -> ToolSource {
    ToolSource::Mcp {
        server: "docs".to_owned(),
    }
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
    };
    let expected = json!({
        "name": "search",
        "description": "Search the docs.",
        "input_schema": { "type": "object" },
        "source": { "mcp": { "server": "docs" } },
    });

    assert_eq!(serde_json::to_value(&spec).unwrap(), expected);
    assert_eq!(serde_json::from_value::<ToolSpec>(expected).unwrap(), spec);
}

// Every spelling but `ok` is a value of `error.type` on the `execute_tool`
// span, which is a semantic-convention attribute with no registry enum to
// compare with.
const STATUSES: [(ToolCallStatus, &str); 5] = [
    (ToolCallStatus::Ok, "ok"),
    (ToolCallStatus::ToolError, "tool_error"),
    (ToolCallStatus::Unknown, "unknown"),
    (ToolCallStatus::Timeout, "timeout"),
    (ToolCallStatus::Failed, "failed"),
];

#[test]
fn every_status_prints_and_serialises_as_its_error_type_spelling() {
    for (status, spelling) in STATUSES {
        assert_eq!(status.to_string(), spelling);
        assert_eq!(serde_json::to_value(status).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<ToolCallStatus>(json!(spelling)).unwrap(),
            status
        );
    }
}

#[test]
fn every_status_but_ok_is_an_error_result_for_the_model() {
    for (status, _) in STATUSES {
        assert_eq!(status.is_error(), status != ToolCallStatus::Ok, "{status}");
    }
}

fn call_1() -> ToolCallId {
    ToolCallId::new("call_1").unwrap()
}

fn text(text: &str) -> Vec<ToolResultContent> {
    vec![ToolResultContent::Text(text.to_owned())]
}

#[test]
fn an_outcome_holds_what_it_was_given_with_its_times_in_whole_milliseconds() {
    let outcome = ToolCallOutcome::measured(
        call_1(),
        Some(ToolSource::Builtin),
        ToolCallStatus::Ok,
        text("hello"),
        Some(100),
        Duration::from_micros(1_500_999),
        Duration::from_micros(42_999),
    );

    assert_eq!(
        outcome,
        ToolCallOutcome {
            call_id: call_1(),
            source: Some(ToolSource::Builtin),
            status: ToolCallStatus::Ok,
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
    for (status, _) in STATUSES {
        let outcome = ToolCallOutcome::measured(
            call_1(),
            None,
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
                is_error: status != ToolCallStatus::Ok,
            },
            "{status}"
        );
    }
}

#[test]
fn output_over_the_cap_is_cut_and_the_outcome_holds_the_size_sent_and_the_size_before() {
    let outcome = ToolCallOutcome::measured(
        call_1(),
        Some(docs_server()),
        ToolCallStatus::Ok,
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
    let outcome = ToolCallOutcome::measured(
        call_1(),
        None,
        ToolCallStatus::Ok,
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
    let outcome = ToolCallOutcome::measured(
        call_1(),
        None,
        ToolCallStatus::Ok,
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
    let outcome = ToolCallOutcome::measured(
        call_1(),
        Some(docs_server()),
        ToolCallStatus::Timeout,
        text("timed out after 60s"),
        Some(8),
        Duration::from_millis(2_000),
        Duration::from_millis(60_000),
    );
    let expected = json!({
        "call_id": "call_1",
        "source": { "mcp": { "server": "docs" } },
        "status": "timeout",
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
