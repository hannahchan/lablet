//! P8: the fields and markers lablet sends, as the capture of the primary
//! reference compared them, held in CI. The comparison was made by hand at
//! the capture of 2026-10-25, Claude Code 2.1.281 through the Python SDK
//! 0.2.159, against what `replay.rs` wrote of lablet's side. The recorded
//! bodies stay out of the repository, so the conversation here is lablet's
//! own, made to the shape of the capture's ten-step run: a turn of thinking
//! with its signature, text and one call; a turn of two calls whose results
//! come back in one message; and a closing text.
//!
//! The constants are lablet's, not the reference's, whose values the parity
//! matrix's rows A5, A6, A8 and A9 name. Lablet sends five fields where the
//! reference adds `betas`, `thinking`, `context_management`, `fallbacks`,
//! `output_config`, `diagnostics` and `stream`; one system block where the
//! reference sends an identity line ahead of the prompt; and a marker on the
//! last tool where the reference marks that line and no tool, as
//! `decisions.md` records on 2026-10-25. `max_tokens` is the config's default
//! of 32,000, which the config's own tests hold; here it's what the adapter
//! sends unchanged.
//!
//! What other tests hold isn't repeated: each request repeating the previous
//! one byte for byte, markers aside, with each `tool_use` input in its sorted,
//! compact form, is `prefix.rs`'s; a tool as its name, description and schema,
//! and the markers with the cache off and under `cache_scope: run`, are
//! `cache.rs`'s (P6); the headers, with no `anthropic-beta`, and `is_error`
//! sent only when a call failed are `happy.rs`'s.

use lablet_model::RequestParams;
use serde_json::{Value, json};
use wiremock::MockServer;

use crate::harness::{self, MODEL, Tools};
use lablet_test_support::{PROMPT, SYSTEM};

/// The config's default `max_tokens`.
const MAX_TOKENS: u32 = 32_000;

/// The tools lablet offers, in the order it offers them.
const TOOLS: [&str; 3] = ["bash", "read_file", "write_file"];

/// Fields the reference sends, or a harness might, that lablet doesn't.
const NOT_SENT: [&str; 14] = [
    "betas",
    "context_management",
    "diagnostics",
    "disable_parallel_tool_use",
    "fallbacks",
    "metadata",
    "output_config",
    "stop_sequences",
    "stream",
    "temperature",
    "thinking",
    "tool_choice",
    "top_k",
    "top_p",
];

/// The requests of one run of the conversation, through the loop with the
/// cache on and the runs sharing what's cached.
async fn conversation() -> Vec<wiremock::Request> {
    let server = MockServer::start().await;
    harness::answer_in_turn(
        &server,
        vec![
            // Written as text, so each input's keys arrive in the model's
            // order, as the API sends them.
            harness::ok_as_written(
                r#"[
                    { "type": "thinking", "thinking": "I'll look first.", "signature": "sig-1" },
                    { "type": "text", "text": "Reading the notes." },
                    { "type": "tool_use", "id": "toolu_01", "name": "read_file", "input": { "path": "notes.md", "offset": 2, "limit": 5 } }
                ]"#,
                "tool_use",
            ),
            harness::ok_as_written(
                r#"[
                    { "type": "text", "text": "Writing, then checking." },
                    { "type": "tool_use", "id": "toolu_02", "name": "write_file", "input": { "path": "out.md", "content": "x" } },
                    { "type": "tool_use", "id": "toolu_03", "name": "read_file", "input": { "path": "out.md", "limit": 3 } }
                ]"#,
                "tool_use",
            ),
            harness::ok(harness::text("Done.")),
        ],
    )
    .await;
    let provider = harness::with(
        harness::target(&server, true),
        opentelemetry::trace::noop::NoopTextMapPropagator::new(),
        false,
    );

    harness::run(
        harness::looped(provider, vec![Tools::offering(&TOOLS)], 0).request(RequestParams {
            max_tokens: MAX_TOKENS,
            ..lablet_test_support::request()
        }),
    )
    .await;

    let received = harness::received(&server).await;
    assert_eq!(received.len(), 3, "one request for each response");
    received
}

fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

#[tokio::test]
async fn every_request_sends_lablet_s_five_fields_and_none_the_reference_adds() {
    let received = conversation().await;

    for (k, request) in received.iter().enumerate() {
        let body = harness::body(request);
        let fields: Vec<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            fields,
            ["max_tokens", "messages", "model", "system", "tools"],
            "request {k}"
        );
        for field in NOT_SENT {
            assert_eq!(body.get(field), None, "request {k} sends {field}");
        }
        assert_eq!(body["model"], MODEL);
        assert_eq!(body["max_tokens"], MAX_TOKENS);
        assert_eq!(
            body["system"],
            json!([{ "type": "text", "text": SYSTEM, "cache_control": ephemeral() }]),
            "one block, the prompt, with no identity line ahead of it"
        );
        let tools = body["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, TOOLS, "request {k}");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages.len(),
            2 * k + 1,
            "a turn adds two messages, not three"
        );
        for (i, message) in messages.iter().enumerate() {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            assert_eq!(
                message["role"], role,
                "request {k}, message {i}: no role system"
            );
        }
        let task = &messages[0]["content"];
        assert_eq!(
            task.as_array().unwrap().len(),
            1,
            "the task goes as a list of one text block"
        );
        assert_eq!(
            (task[0]["type"].as_str(), task[0]["text"].as_str()),
            (Some("text"), Some(PROMPT))
        );
    }
}

#[tokio::test]
async fn every_request_carries_three_markers_the_message_s_on_the_last_block_of_the_last_message() {
    let received = conversation().await;

    for (k, request) in received.iter().enumerate() {
        let sent = String::from_utf8(request.body.clone()).unwrap();
        assert_eq!(
            sent.matches("cache_control").count(),
            3,
            "request {k}: {sent}"
        );
        let body: Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(body["system"][0]["cache_control"], ephemeral());
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[2]["cache_control"], ephemeral(), "the last tool");
        assert_eq!(tools[0].get("cache_control"), None);
        assert_eq!(tools[1].get("cache_control"), None);
        let messages = body["messages"].as_array().unwrap();
        let marked: Vec<(usize, usize)> = messages
            .iter()
            .enumerate()
            .flat_map(|(i, message)| {
                message["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| block.get("cache_control").is_some())
                    .map(move |(j, _)| (i, j))
            })
            .collect();
        let last = messages.len() - 1;
        let last_block = messages[last]["content"].as_array().unwrap().len() - 1;
        assert_eq!(marked, [(last, last_block)], "request {k}");
        assert_eq!(
            messages[last]["content"][last_block]["cache_control"],
            ephemeral()
        );
    }

    let third: Value = harness::body(&received[2]);
    assert_eq!(
        third["messages"][4]["content"],
        json!([
            {
                "type": "tool_result",
                "tool_use_id": "toolu_02",
                "content": [{ "type": "text", "text": "ok" }]
            },
            {
                "type": "tool_result",
                "tool_use_id": "toolu_03",
                "content": [{ "type": "text", "text": "ok" }],
                "cache_control": { "type": "ephemeral" }
            }
        ]),
        "a turn's two results in one message, the marker on the second"
    );
}
