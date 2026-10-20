//! P1: thinking, redacted thinking and tool use mapped and replayed, and the
//! thinking, display, effort and temperature parameters as sent.

use std::num::NonZeroU32;

use lablet_model::{
    ContentBlock, Effort, Message, Thinking, ThinkingDisplay, ToolCallId, ToolInput, ToolName,
    ToolUse,
};
use lablet_run::ModelProvider as _;
use serde_json::{Value, json};
use wiremock::MockServer;

use crate::harness::{self, Tools};

const SIGNATURE: &str = "EqQBCgIYAhIMa2V5LWZvci10ZXN0GgwKCm1hZGUtdXAtc2ln";
const REDACTED: &str = "EmwKAhgBEgxyZWRhY3RlZC1kYXRhGgwKCm1hZGUtdXAtZGF0YQ==";

#[tokio::test]
async fn thinking_redacted_thinking_and_tool_use_are_mapped_and_replayed_byte_for_byte() {
    let server = MockServer::start().await;
    let mut first = harness::message(
        json!([
            { "type": "thinking", "thinking": "", "signature": SIGNATURE },
            { "type": "redacted_thinking", "data": REDACTED },
            { "type": "tool_use", "id": "toolu_01", "name": "read_file", "input": { "path": "notes.md" } }
        ]),
        "tool_use",
    );
    first["usage"] = json!({
        "input_tokens": 40,
        "output_tokens": 30,
        "cache_creation_input_tokens": 600,
        "cache_read_input_tokens": 2000
    });
    harness::answer_in_turn(
        &server,
        vec![harness::ok(first), harness::ok(harness::text("done"))],
    )
    .await;
    let provider = harness::provider(&server);

    let finished = harness::run(harness::looped(
        provider,
        vec![Tools::offering(&["read_file"])],
        0,
    ))
    .await;

    let turn = &finished.transcript.turns()[0];
    assert_eq!(
        turn.response(),
        [
            ContentBlock::Thinking {
                text: String::new(),
                signature: Some(SIGNATURE.to_owned()),
            },
            ContentBlock::RedactedThinking {
                data: REDACTED.to_owned(),
            },
            ContentBlock::ToolUse(ToolUse {
                id: ToolCallId::new("toolu_01").unwrap(),
                name: ToolName::new("read_file").unwrap(),
                input: ToolInput::Json(json!({ "path": "notes.md" })),
            }),
        ]
    );
    assert_eq!(turn.record().usage.cache_read_tokens, Some(2000));
    assert_eq!(turn.record().usage.cache_write_tokens, Some(600));
    assert_eq!(turn.record().usage.input_tokens, 2640);

    let received = harness::received(&server).await;
    assert_eq!(received.len(), 2);
    let replayed = String::from_utf8(received[1].body.clone()).unwrap();
    assert!(
        replayed.contains(&format!(
            r#"{{"role":"assistant","content":[{{"type":"thinking","thinking":"","signature":"{SIGNATURE}"}},{{"type":"redacted_thinking","data":"{REDACTED}"}},{{"type":"tool_use","id":"toolu_01","name":"read_file","input":{{"path":"notes.md"}}}}]}}"#
        )),
        "the blocks go back in order with their strings unchanged: {replayed}"
    );
}

/// The body one attempt with `thinking`, `display`, `effort` and
/// `temperature` sends.
async fn sent(
    thinking: Thinking,
    thinking_display: Option<ThinkingDisplay>,
    effort: Option<Effort>,
    temperature: Option<f64>,
) -> Value {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::provider(&server);
    let input = harness::prompt();
    let messages = [Message::User {
        tool_results: Vec::new(),
        input: &input,
    }];
    let mut request = harness::request(&messages, &[]);
    request.thinking = thinking;
    request.thinking_display = thinking_display;
    request.effort = effort;
    request.temperature = temperature;
    request.seed = Some(7);
    provider.complete(request).await.unwrap();
    harness::body(&harness::received(&server).await[0])
}

#[tokio::test]
async fn display_goes_inside_the_adaptive_and_the_enabled_objects() {
    let adaptive = sent(
        Thinking::Adaptive,
        Some(ThinkingDisplay::Summarized),
        None,
        None,
    )
    .await;
    assert_eq!(
        adaptive["thinking"],
        json!({ "type": "adaptive", "display": "summarized" })
    );

    let budget = sent(
        Thinking::Budget(NonZeroU32::new(2048).unwrap()),
        Some(ThinkingDisplay::Omitted),
        None,
        None,
    )
    .await;
    assert_eq!(
        budget["thinking"],
        json!({ "type": "enabled", "budget_tokens": 2048, "display": "omitted" })
    );

    let bare = sent(Thinking::Adaptive, None, None, None).await;
    assert_eq!(bare["thinking"], json!({ "type": "adaptive" }));
}

#[tokio::test]
async fn provider_default_sends_no_thinking_and_disabled_sends_disabled_with_no_display() {
    let default = sent(
        Thinking::ProviderDefault,
        Some(ThinkingDisplay::Summarized),
        None,
        None,
    )
    .await;
    assert_eq!(default.get("thinking"), None);

    let disabled = sent(
        Thinking::Disabled,
        Some(ThinkingDisplay::Summarized),
        None,
        None,
    )
    .await;
    assert_eq!(disabled["thinking"], json!({ "type": "disabled" }));
}

#[tokio::test]
async fn effort_and_a_stated_temperature_are_sent_and_a_seed_never_is() {
    let stated = sent(
        Thinking::ProviderDefault,
        None,
        Some(Effort::XHigh),
        Some(0.5),
    )
    .await;
    assert_eq!(stated["output_config"], json!({ "effort": "xhigh" }));
    assert_eq!(stated["temperature"], json!(0.5));
    assert_eq!(stated.get("seed"), None);

    let unstated = sent(Thinking::ProviderDefault, None, None, None).await;
    assert_eq!(unstated.get("output_config"), None);
    assert_eq!(unstated.get("temperature"), None);
}
