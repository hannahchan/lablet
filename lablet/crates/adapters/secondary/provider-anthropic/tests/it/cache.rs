//! P6: the three cache breakpoints, none with the cache off, and the run-id
//! block under `cache_scope: run`.

use lablet_model::{
    ContentBlock, Message, ToolCallId, ToolInput, ToolName, ToolResult, ToolResultContent,
    ToolSpec, ToolUse,
};
use lablet_run::ModelProvider as _;
use lablet_test_support::SYSTEM;
use serde_json::{Value, json};
use wiremock::MockServer;

use crate::harness::{self, spec};

/// The body one attempt sends over `messages` and `tools`, with the cache
/// on or off and the run id under `cache_key`.
async fn sent(
    cache: bool,
    cache_key: Option<&str>,
    system: &str,
    messages: &[Message<'_>],
    tools: &[ToolSpec],
) -> String {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::with(
        harness::target(&server, cache),
        opentelemetry::trace::noop::NoopTextMapPropagator::new(),
        false,
    );
    let mut request = harness::request(messages, tools);
    request.system = system;
    request.cache_key = cache_key;
    provider.complete(request).await.unwrap();
    String::from_utf8(harness::received(&server).await[0].body.clone()).unwrap()
}

fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

#[tokio::test]
async fn with_the_cache_on_the_system_prompt_the_last_tool_and_the_latest_message_carry_markers() {
    let input = harness::prompt();
    let messages = [Message::User {
        tool_results: Vec::new(),
        input: &input,
    }];
    let tools = [spec("read_file"), spec("write_file")];

    let sent = sent(true, None, SYSTEM, &messages, &tools).await;

    let body: Value = serde_json::from_str(&sent).unwrap();
    assert_eq!(body["system"][0]["cache_control"], ephemeral());
    assert_eq!(body["tools"][0].get("cache_control"), None);
    assert_eq!(body["tools"][1]["cache_control"], ephemeral());
    assert_eq!(
        body["messages"][0]["content"][0]["cache_control"],
        ephemeral()
    );
    assert_eq!(sent.matches("cache_control").count(), 3);
    assert_eq!(
        body["tools"][0],
        json!({
            "name": "read_file",
            "description": "The read_file tool.",
            "input_schema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }
        }),
        "a tool is its name, description and schema, with no strict"
    );
}

#[tokio::test]
async fn the_latest_message_s_marker_sits_on_its_last_tool_result_when_results_are_all_it_holds() {
    let input = harness::prompt();
    let response = [ContentBlock::Text("Reading.".to_owned())];
    let call_id = ToolCallId::new("toolu_01").unwrap();
    let content = [ToolResultContent::Text("ok".to_owned())];
    let messages = [
        Message::User {
            tool_results: Vec::new(),
            input: &input,
        },
        Message::Assistant(&response),
        Message::User {
            tool_results: vec![ToolResult {
                call_id: &call_id,
                content: &content,
                is_error: false,
            }],
            input: &[],
        },
    ];

    let sent = sent(true, None, SYSTEM, &messages, &[]).await;

    let body: Value = serde_json::from_str(&sent).unwrap();
    assert_eq!(
        body["messages"][2]["content"][0],
        json!({
            "type": "tool_result",
            "tool_use_id": "toolu_01",
            "content": [{ "type": "text", "text": "ok" }],
            "cache_control": { "type": "ephemeral" }
        })
    );
    assert_eq!(body["messages"][0]["content"][0].get("cache_control"), None);
    assert_eq!(
        sent.matches("cache_control").count(),
        2,
        "the system prompt's and the message's"
    );
}

#[tokio::test]
async fn a_user_message_s_tool_results_go_ahead_of_its_text() {
    let input = harness::prompt();
    let response = [ContentBlock::Text("Reading.".to_owned())];
    let call_id = ToolCallId::new("toolu_01").unwrap();
    let content = [ToolResultContent::Text("ok".to_owned())];
    let messages = [
        Message::User {
            tool_results: Vec::new(),
            input: &input,
        },
        Message::Assistant(&response),
        Message::User {
            tool_results: vec![ToolResult {
                call_id: &call_id,
                content: &content,
                is_error: false,
            }],
            input: &input,
        },
    ];

    let sent = sent(false, None, SYSTEM, &messages, &[]).await;

    let body: Value = serde_json::from_str(&sent).unwrap();
    let blocks = body["messages"][2]["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "tool_result");
    assert_eq!(
        blocks[1]["type"], "text",
        "the API takes a turn's results ahead of any text and refuses the other order"
    );
}

#[tokio::test]
async fn a_tool_result_with_no_text_sends_no_content_and_unparsed_arguments_send_an_empty_object() {
    let input = harness::prompt();
    let response = [ContentBlock::ToolUse(ToolUse {
        id: ToolCallId::new("toolu_01").unwrap(),
        name: ToolName::new("read_file").unwrap(),
        input: ToolInput::Unparsed("{\"path\": \"READ".to_owned()),
    })];
    let call_id = ToolCallId::new("toolu_01").unwrap();
    let messages = [
        Message::User {
            tool_results: Vec::new(),
            input: &input,
        },
        Message::Assistant(&response),
        Message::User {
            tool_results: vec![ToolResult {
                call_id: &call_id,
                content: &[],
                is_error: true,
            }],
            input: &[],
        },
    ];

    let sent = sent(false, None, SYSTEM, &messages, &[]).await;

    let body: Value = serde_json::from_str(&sent).unwrap();
    assert_eq!(
        body["messages"][1]["content"][0],
        json!({ "type": "tool_use", "id": "toolu_01", "name": "read_file", "input": {} })
    );
    assert_eq!(
        body["messages"][2]["content"][0],
        json!({ "type": "tool_result", "tool_use_id": "toolu_01", "is_error": true })
    );
}

#[tokio::test]
async fn with_the_cache_off_no_marker_is_sent() {
    let input = harness::prompt();
    let messages = [Message::User {
        tool_results: Vec::new(),
        input: &input,
    }];
    let tools = [spec("read_file")];

    let sent = sent(false, None, SYSTEM, &messages, &tools).await;

    assert!(!sent.contains("cache_control"), "{sent}");
}

#[tokio::test]
async fn under_cache_scope_run_a_system_block_with_the_run_id_comes_first_and_carries_no_marker() {
    let input = harness::prompt();
    let messages = [Message::User {
        tool_results: Vec::new(),
        input: &input,
    }];

    let cached = sent(true, Some("01K8RUN"), SYSTEM, &messages, &[]).await;
    let body: Value = serde_json::from_str(&cached).unwrap();
    assert_eq!(
        body["system"],
        json!([
            { "type": "text", "text": "lablet run 01K8RUN" },
            { "type": "text", "text": SYSTEM, "cache_control": { "type": "ephemeral" } }
        ])
    );

    let uncached = sent(false, Some("01K8RUN"), SYSTEM, &messages, &[]).await;
    let body: Value = serde_json::from_str(&uncached).unwrap();
    assert_eq!(
        body["system"],
        json!([
            { "type": "text", "text": "lablet run 01K8RUN" },
            { "type": "text", "text": SYSTEM }
        ]),
        "the block is sent whether or not anything is cached"
    );
}

#[tokio::test]
async fn an_empty_system_prompt_sends_no_system_key() {
    let input = harness::prompt();
    let messages = [Message::User {
        tool_results: Vec::new(),
        input: &input,
    }];

    let unscoped = sent(true, None, "", &messages, &[]).await;
    let body: Value = serde_json::from_str(&unscoped).unwrap();
    assert_eq!(body.get("system"), None);

    let scoped = sent(true, Some("01K8RUN"), "", &messages, &[]).await;
    let body: Value = serde_json::from_str(&scoped).unwrap();
    assert_eq!(
        body["system"],
        json!([{ "type": "text", "text": "lablet run 01K8RUN" }]),
        "the run-id block alone, and no marker on it"
    );
}
