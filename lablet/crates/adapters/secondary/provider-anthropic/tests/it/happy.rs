//! A response mapped field by field, and what one request carries.

use lablet_model::{
    ContentBlock, Endpoint, FinishReason, ProviderKind, StopReason, TokenCounts, Usage,
};
use lablet_run::ModelProvider as _;
use lablet_test_support::{PROMPT, SYSTEM};
use serde_json::json;
use wiremock::MockServer;

use crate::harness::{self, ANSWERED_BY, KEY, MODEL, Tools};

#[tokio::test]
async fn a_text_response_is_mapped_with_its_id_model_usage_and_finish() {
    let server = MockServer::start().await;
    let mut body = harness::text("Hello");
    body["usage"] = json!({
        "input_tokens": 12,
        "output_tokens": 5,
        "cache_creation_input_tokens": 300,
        "cache_read_input_tokens": 1200,
        "output_tokens_details": { "thinking_tokens": 2 }
    });
    harness::answer(&server, harness::ok(body)).await;
    let provider = harness::provider(&server);

    let response = harness::complete(&provider).await.unwrap();

    assert_eq!(response.content(), [ContentBlock::Text("Hello".to_owned())]);
    assert_eq!(response.finish, FinishReason::EndTurn);
    assert_eq!(response.response_id.as_deref(), Some("msg_01"));
    assert_eq!(response.response_model.as_deref(), Some(ANSWERED_BY));
    assert_eq!(
        response.usage,
        Usage::from_uncached(TokenCounts {
            input: 12,
            output: 5,
            reasoning: Some(2),
            cache_read: Some(1200),
            cache_write: Some(300),
        })
    );
    assert_eq!(
        response.usage.input_tokens, 1_512,
        "the API's input count is the uncached remainder, so both cache counts are added in"
    );
}

#[tokio::test]
async fn a_count_the_body_doesn_t_carry_isn_t_reported() {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::provider(&server);

    let response = harness::complete(&provider).await.unwrap();

    assert_eq!(
        response.usage,
        Usage {
            input_tokens: 12,
            output_tokens: 5,
            reasoning_output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        }
    );
}

#[tokio::test]
async fn one_attempt_is_one_post_with_the_api_s_headers_and_nothing_the_loop_didn_t_state() {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::provider(&server);

    harness::complete(&provider).await.unwrap();

    let received = harness::received(&server).await;
    assert_eq!(received.len(), 1);
    let request = &received[0];
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(request.url.path(), "/v1/messages");
    assert_eq!(harness::header(request, "x-api-key"), Some(KEY));
    assert_eq!(
        harness::header(request, "anthropic-version"),
        Some("2023-06-01")
    );
    assert_eq!(
        harness::header(request, "content-type"),
        Some("application/json")
    );
    assert_eq!(harness::header(request, "accept"), Some("application/json"));
    assert_eq!(
        harness::header(request, "user-agent"),
        Some(concat!("lablet/", env!("CARGO_PKG_VERSION")))
    );
    assert_eq!(harness::header(request, "anthropic-beta"), None);
    assert_eq!(harness::header(request, "authorization"), None);
    assert_eq!(
        harness::body(request),
        json!({
            "model": MODEL,
            "max_tokens": 1024,
            "system": [{ "type": "text", "text": SYSTEM }],
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": PROMPT }] }]
        }),
        "no tools, thinking, output_config, temperature, tool_choice, metadata, stream or \
         fallbacks key"
    );
}

#[tokio::test]
async fn the_endpoint_is_the_base_url_s_host_and_port_and_reaches_the_run_s_summary() {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::provider(&server);
    let endpoint = Endpoint {
        host: "127.0.0.1".to_owned(),
        port: server.address().port(),
    };
    assert_eq!(provider.endpoint(), Some(endpoint.clone()));

    let finished = harness::run(harness::looped(provider, Vec::new(), 0)).await;

    assert_eq!(finished.summary.endpoint, Some(endpoint));
    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
    assert_eq!(finished.transcript.final_text(), "Hello");
    assert_eq!(finished.summary.model.name, MODEL);
    assert_eq!(
        finished.summary.model.api.provider(),
        ProviderKind::Anthropic
    );
}

#[tokio::test]
async fn each_stop_reason_maps_to_its_finish_reason() {
    let table = [
        ("end_turn", "end_turn"),
        ("stop_sequence", "end_turn"),
        ("tool_use", "tool_use"),
        ("max_tokens", "max_tokens"),
        ("model_context_window_exceeded", "context_window"),
        ("refusal", "refusal"),
        ("pause_turn", "pause_turn"),
    ];
    for (stop_reason, finish) in table {
        let server = MockServer::start().await;
        let mut body = harness::message(json!([]), stop_reason);
        if stop_reason == "refusal" {
            body["stop_details"] = json!({ "type": "refusal", "category": "cyber" });
        }
        harness::answer(&server, harness::ok(body)).await;
        let provider = harness::provider(&server);

        let response = harness::complete(&provider).await.unwrap();

        assert_eq!(
            response.finish.as_str(),
            finish,
            "stop_reason {stop_reason}"
        );
        assert_eq!(
            matches!(response.finish, FinishReason::Other(_)),
            stop_reason == "pause_turn"
        );
    }
}

#[tokio::test]
async fn a_block_of_a_kind_lablet_has_no_name_for_is_kept_whole_and_sent_back_as_it_came() {
    let server = MockServer::start().await;
    let unknown = json!({
        "type": "server_tool_use",
        "id": "srvtoolu_01",
        "name": "web_search",
        "input": { "query": "lablet" }
    });
    harness::answer_in_turn(
        &server,
        vec![
            harness::ok(harness::message(
                json!([
                    unknown,
                    { "type": "tool_use", "id": "toolu_01", "name": "read_file", "input": { "path": "a.md" } }
                ]),
                "tool_use",
            )),
            harness::ok(harness::text("done")),
        ],
    )
    .await;
    let provider = harness::provider(&server);

    let finished = harness::run(harness::looped(
        provider,
        vec![Tools::offering(&["read_file"])],
        0,
    ))
    .await;

    assert_eq!(
        finished.transcript.turns()[0].response()[0],
        ContentBlock::Opaque {
            provider: ProviderKind::Anthropic,
            payload: unknown.clone(),
        }
    );
    let received = harness::received(&server).await;
    assert_eq!(received.len(), 2);
    assert_eq!(
        harness::body(&received[1])["messages"][1]["content"][0],
        unknown
    );
}

#[tokio::test]
async fn a_tool_result_carries_is_error_only_when_the_call_failed() {
    let server = MockServer::start().await;
    harness::answer_in_turn(
        &server,
        vec![
            harness::ok(harness::message(
                json!([
                    { "type": "tool_use", "id": "toolu_01", "name": "read_file", "input": { "path": "a.md" } },
                    { "type": "tool_use", "id": "toolu_02", "name": "missing", "input": { "path": "b.md" } }
                ]),
                "tool_use",
            )),
            harness::ok(harness::text("done")),
        ],
    )
    .await;
    let provider = harness::provider(&server);

    harness::run(harness::looped(
        provider,
        vec![Tools::offering(&["read_file"])],
        0,
    ))
    .await;

    let received = harness::received(&server).await;
    assert_eq!(
        harness::body(&received[1])["messages"][2],
        json!({
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "toolu_01",
                    "content": [{ "type": "text", "text": "ok" }]
                },
                {
                    "type": "tool_result",
                    "tool_use_id": "toolu_02",
                    "content": [{ "type": "text", "text": "no tool named missing is offered by this run" }],
                    "is_error": true
                }
            ]
        })
    );
}
