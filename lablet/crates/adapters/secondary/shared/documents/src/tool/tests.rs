use lablet_model::{self as model, ToolCallId, ToolName};
use serde_json::json;

use super::*;

fn docs_server() -> model::ToolSource {
    model::ToolSource::Mcp {
        server: "docs".to_owned(),
    }
}

#[test]
fn a_tool_spec_is_written_with_everything_the_model_was_offered() {
    let spec = model::ToolSpec {
        name: ToolName::new("search").unwrap(),
        description: "Search the docs.".to_owned(),
        input_schema: json!({ "type": "object" }),
        source: docs_server(),
        concurrency: model::ToolConcurrency::Shared,
    };

    assert_eq!(
        serde_json::to_string(&ToolSpec::from(spec)).unwrap(),
        concat!(
            r#"{"name":"search","description":"Search the docs.","#,
            r#""input_schema":{"type":"object"},"#,
            r#""source":{"mcp":{"server":"docs"}},"concurrency":"shared"}"#
        )
    );
}

#[test]
fn a_source_is_written_under_the_domain_s_spelling_with_the_server_of_an_mcp_tool() {
    let builtin = serde_json::to_value(ToolSource::from(model::ToolSource::Builtin)).unwrap();
    let mcp = serde_json::to_value(ToolSource::from(docs_server())).unwrap();

    assert_eq!(builtin, json!(model::ToolSource::Builtin.as_str()));
    assert_eq!(mcp, json!({ docs_server().as_str(): { "server": "docs" } }));
    assert_eq!(mcp, json!({ "mcp": { "server": "docs" } }));
}

#[test]
fn a_tool_s_concurrency_is_written_as_one_word() {
    for (concurrency, written) in [
        (model::ToolConcurrency::Exclusive, "exclusive"),
        (model::ToolConcurrency::Shared, "shared"),
    ] {
        assert_eq!(
            serde_json::to_value(ToolConcurrency::from(concurrency)).unwrap(),
            json!(written)
        );
    }
}

#[test]
fn a_call_no_tool_ran_for_is_written_as_the_one_word_telemetry_has_for_it() {
    for status in [
        model::ToolCallStatus::Unknown,
        model::ToolCallStatus::MalformedInput,
        model::ToolCallStatus::Rejected,
        model::ToolCallStatus::NotRun,
    ] {
        let written = serde_json::to_value(ToolCallStatus::from(status.clone())).unwrap();

        assert_eq!(written, json!(status.as_str()));
    }
}

/// The flattening the domain does is for telemetry. The document keeps the
/// two levels, so a reader can tell a built-in failure from an MCP one.
#[test]
fn a_call_a_tool_ran_for_is_written_with_where_the_tool_came_from_and_how_it_ended() {
    for ended in [
        model::ToolCallEnd::Ok,
        model::ToolCallEnd::ToolError,
        model::ToolCallEnd::Timeout,
        model::ToolCallEnd::Failed,
    ] {
        for (source, written) in [
            (model::ToolSource::Builtin, json!("builtin")),
            (docs_server(), json!({ "mcp": { "server": "docs" } })),
        ] {
            let status = model::ToolCallStatus::ran(source, ended);

            assert_eq!(
                serde_json::to_value(ToolCallStatus::from(status)).unwrap(),
                json!({ "ran": { "source": written, "ended": ended.as_str() } })
            );
        }
    }
}

#[test]
fn a_tool_call_outcome_is_written_without_a_name_an_input_or_an_error_flag() {
    let outcome = model::ToolCallOutcome {
        call_id: ToolCallId::new("call_1").unwrap(),
        status: model::ToolCallStatus::ran(docs_server(), model::ToolCallEnd::Timeout),
        started_ms: 2_000,
        latency_ms: 60_000,
        truncated_from_bytes: Some(19),
        content: vec![
            model::ToolResultContent::Text("timed ou".to_owned()),
            model::ToolResultContent::Text("[truncated: the first 8 of 19 bytes]".to_owned()),
        ],
    };

    assert_eq!(
        serde_json::to_string(&ToolCallOutcome::from(outcome)).unwrap(),
        concat!(
            r#"{"call_id":"call_1","#,
            r#""status":{"ran":{"source":{"mcp":{"server":"docs"}},"ended":"timeout"}},"#,
            r#""started_ms":2000,"latency_ms":60000,"truncated_from_bytes":19,"#,
            r#""content":[{"text":"timed ou"},{"text":"[truncated: the first 8 of 19 bytes]"}]}"#
        )
    );
}

#[test]
fn an_outcome_whose_output_was_not_cut_writes_the_size_before_the_cut_as_null() {
    let outcome = model::ToolCallOutcome {
        call_id: ToolCallId::new("call_1").unwrap(),
        status: model::ToolCallStatus::NotRun,
        started_ms: 0,
        latency_ms: 0,
        truncated_from_bytes: None,
        content: Vec::new(),
    };

    assert_eq!(
        serde_json::to_string(&ToolCallOutcome::from(outcome)).unwrap(),
        concat!(
            r#"{"call_id":"call_1","status":"not_run","started_ms":0,"latency_ms":0,"#,
            r#""truncated_from_bytes":null,"content":[]}"#
        )
    );
}
