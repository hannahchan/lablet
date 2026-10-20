//! E1: each request repeats the previous one's `system`, `tools` and
//! messages byte for byte, `cache_control` aside, with the thinking blocks
//! in order and a `tool_use` input in its sorted, compact form.

use serde_json::{Value, json};
use wiremock::MockServer;

use crate::harness::{self, Tools};

/// `value` with every `cache_control` key taken out, since the moving
/// marker differs between adjacent requests and is no edit.
fn without_markers(mut value: Value) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(object) => {
                object.remove("cache_control");
                object.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut value);
    value
}

#[tokio::test]
async fn each_request_repeats_the_previous_one_s_system_tools_and_messages_byte_for_byte() {
    let server = MockServer::start().await;
    harness::answer_in_turn(
        &server,
        vec![
            // Written as text, since `json!` sorts keys and the pin is that
            // the adapter sorts what arrived unsorted.
            harness::ok_as_written(
                r#"[
                    { "type": "thinking", "thinking": "I'll read it.", "signature": "sig-1" },
                    { "type": "tool_use", "id": "toolu_01", "name": "read_file", "input": { "path": "b.md", "offset": 2 } },
                    { "type": "text", "text": "Reading." }
                ]"#,
                "tool_use",
            ),
            harness::ok_as_written(
                r#"[
                    { "type": "thinking", "thinking": "Two more.", "signature": "sig-2" },
                    { "type": "tool_use", "id": "toolu_02", "name": "read_file", "input": { "path": "c.md" } },
                    { "type": "thinking", "thinking": "", "signature": "sig-3" },
                    { "type": "tool_use", "id": "toolu_03", "name": "write_file", "input": { "path": "out.md", "content": "x" } },
                    { "type": "text", "text": "Writing." }
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

    harness::run(harness::looped(
        provider,
        vec![Tools::offering(&["read_file", "write_file"])],
        0,
    ))
    .await;

    let received = harness::received(&server).await;
    assert_eq!(received.len(), 3);
    let bodies: Vec<Value> = received
        .iter()
        .map(|request| without_markers(harness::body(request)))
        .collect();
    for (earlier, later) in bodies.iter().zip(&bodies[1..]) {
        let text = |value: &Value| serde_json::to_string(value).unwrap();
        assert_eq!(text(&earlier["system"]), text(&later["system"]));
        assert_eq!(text(&earlier["tools"]), text(&later["tools"]));
        let earlier_messages = earlier["messages"].as_array().unwrap();
        let later_messages = later["messages"].as_array().unwrap();
        assert_eq!(
            text(&json!(later_messages[..earlier_messages.len()])),
            text(&json!(earlier_messages))
        );
    }
    let last = &bodies[2]["messages"];
    assert_eq!(
        last[1]["content"][0],
        json!({ "type": "thinking", "thinking": "I'll read it.", "signature": "sig-1" })
    );
    assert_eq!(
        last[3]["content"][0],
        json!({ "type": "thinking", "thinking": "Two more.", "signature": "sig-2" })
    );
    assert_eq!(
        last[3]["content"][2],
        json!({ "type": "thinking", "thinking": "", "signature": "sig-3" })
    );
    let third = String::from_utf8(received[2].body.clone()).unwrap();
    assert!(
        third.contains(r#""input":{"offset":2,"path":"b.md"}"#),
        "the input is replayed with sorted keys and compact whitespace: {third}"
    );
    assert!(
        third.contains(r#""input":{"content":"x","path":"out.md"}"#),
        "{third}"
    );
}

#[tokio::test]
async fn a_retry_within_a_turn_sends_the_same_bytes_twice() {
    let server = MockServer::start().await;
    harness::answer_in_turn(
        &server,
        vec![
            harness::envelope(529, "overloaded_error", "Overloaded"),
            harness::ok(harness::text("Hello")),
        ],
    )
    .await;
    let provider = harness::with(
        harness::target(&server, true),
        opentelemetry::trace::noop::NoopTextMapPropagator::new(),
        false,
    );

    harness::run(harness::looped(provider, Vec::new(), 1)).await;

    let received = harness::received(&server).await;
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].body, received[1].body);
}
