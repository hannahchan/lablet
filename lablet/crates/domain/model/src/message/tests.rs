use serde_json::json;

use super::*;

fn id(value: &str) -> ToolCallId {
    ToolCallId::new(value).unwrap()
}

fn bash(call_id: &str) -> ToolUse {
    ToolUse {
        id: id(call_id),
        name: ToolName::new("bash").unwrap(),
        input: ToolInput::Json(json!({ "command": "ls" })),
    }
}

fn text(text: &str) -> ToolResultContent {
    ToolResultContent::Text(text.to_owned())
}

fn texts(content: &[ToolResultContent]) -> Vec<&str> {
    content
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect()
}

#[test]
fn input_bytes_is_the_length_of_the_input_as_compact_json() {
    let call = ToolUse {
        input: ToolInput::Json(json!({ "command": "ls", "n": 2 })),
        ..bash("a")
    };

    assert_eq!(call.input_bytes(), 22);
    assert_eq!(
        ToolUse {
            input: ToolInput::Json(json!({})),
            ..bash("a")
        }
        .input_bytes(),
        2
    );
}

#[test]
fn the_size_of_content_is_the_sum_of_its_text_in_bytes_not_characters() {
    assert_eq!(
        ToolResultContent::bytes(&[text("exit 1"), text("caf\u{e9}")]),
        6 + 5
    );
    assert_eq!(ToolResultContent::bytes(&[text("")]), 0);
    assert_eq!(ToolResultContent::bytes(&[]), 0);
}

#[test]
fn omitted_content_is_worded_one_way() {
    assert_eq!(
        ToolResultContent::omitted("image", "image/png", 48_213),
        text("[image omitted: image/png, 48213 bytes]")
    );
}

#[test]
fn the_truncation_line_is_worded_one_way() {
    assert_eq!(
        ToolResultContent::truncated(100_000, 5_242_880),
        text("[truncated: the first 100000 of 5242880 bytes]")
    );
}

#[test]
fn content_within_the_cap_is_returned_unchanged() {
    let content = vec![text("0123456789")];

    for cap in [10, 11, u64::MAX] {
        assert_eq!(
            ToolResultContent::capped(content.clone(), cap),
            (content.clone(), None)
        );
    }
}

#[test]
fn content_one_byte_over_the_cap_is_cut_and_says_so_in_one_last_line() {
    let (capped, original) = ToolResultContent::capped(vec![text("0123456789")], 9);

    assert_eq!(original, Some(10));
    assert_eq!(
        texts(&capped),
        ["012345678", "[truncated: the first 9 of 10 bytes]"]
    );
}

#[test]
fn the_cut_falls_on_a_character_boundary_and_the_line_counts_what_was_kept() {
    // Each of these characters is two bytes, so a cap of 5 lands inside the third.
    let (capped, original) = ToolResultContent::capped(vec![text("\u{e9}\u{e9}\u{e9}\u{e9}")], 5);

    assert_eq!(original, Some(8));
    assert_eq!(
        texts(&capped),
        ["\u{e9}\u{e9}", "[truncated: the first 4 of 8 bytes]"]
    );
}

#[test]
fn the_cap_is_spent_across_the_pieces_in_order_and_what_is_left_over_is_dropped() {
    let content = vec![text("aaaa"), text("bbbbbbbb"), text("cccc")];

    let (capped, original) = ToolResultContent::capped(content, 10);

    assert_eq!(original, Some(16));
    assert_eq!(
        texts(&capped),
        ["aaaa", "bbbbbb", "[truncated: the first 10 of 16 bytes]"]
    );
}

#[test]
fn a_cap_of_zero_leaves_only_the_line_that_says_what_was_cut() {
    let (capped, original) = ToolResultContent::capped(vec![text("0123456789")], 0);

    assert_eq!(original, Some(10));
    assert_eq!(texts(&capped), ["[truncated: the first 0 of 10 bytes]"]);
}

#[test]
fn every_block_of_a_response_has_one_json_form() {
    let blocks = vec![
        ContentBlock::Text("hello".to_owned()),
        ContentBlock::Thinking {
            text: "hm".to_owned(),
            signature: Some("sig".to_owned()),
        },
        ContentBlock::RedactedThinking {
            data: "opaque".to_owned(),
        },
        ContentBlock::ToolUse(bash("a")),
        ContentBlock::Opaque {
            provider: ProviderKind::Anthropic,
            payload: json!({ "type": "server_tool_use" }),
        },
    ];
    let expected = json!([
        { "text": "hello" },
        { "thinking": { "text": "hm", "signature": "sig" } },
        { "redacted_thinking": { "data": "opaque" } },
        { "tool_use": { "id": "a", "name": "bash", "input": { "json": { "command": "ls" } } } },
        { "opaque": { "provider": "anthropic", "payload": { "type": "server_tool_use" } } },
    ]);

    assert_eq!(serde_json::to_value(&blocks).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<Vec<ContentBlock>>(expected).unwrap(),
        blocks
    );
}

#[test]
fn a_tool_result_is_not_a_block_a_response_can_hold() {
    let block = json!({ "tool_result": { "call_id": "a", "content": [{ "text": "ok" }] } });

    assert!(serde_json::from_value::<ContentBlock>(block).is_err());
}

#[test]
fn a_misspelt_field_of_a_tool_use_is_an_error() {
    let call =
        json!({ "tool_use": { "id": "a", "name": "bash", "input": { "json": {} }, "args": {} } });

    assert!(serde_json::from_value::<ContentBlock>(call).is_err());
}

#[test]
fn a_tool_use_with_an_invalid_name_does_not_deserialise() {
    let block = json!({ "tool_use": { "id": "a", "name": "not a name", "input": { "json": {} } } });

    assert!(serde_json::from_value::<ContentBlock>(block).is_err());
}

#[test]
fn content_a_tool_returned_is_text_and_nothing_else() {
    assert_eq!(
        serde_json::to_value(text("exit 1")).unwrap(),
        json!({ "text": "exit 1" })
    );
    assert!(serde_json::from_value::<ToolResultContent>(json!({ "json": { "code": 1 } })).is_err());
}

#[test]
fn thinking_without_a_signature_is_null_not_an_empty_string() {
    let block = ContentBlock::Thinking {
        text: "hm".to_owned(),
        signature: None,
    };
    let expected = json!({ "thinking": { "text": "hm", "signature": null } });

    assert_eq!(serde_json::to_value(&block).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<ContentBlock>(json!({ "thinking": { "text": "hm" } })).unwrap(),
        block
    );
}

#[test]
fn each_message_serialises_under_its_role_to_these_exact_bytes() {
    let response = [ContentBlock::ToolUse(bash("a"))];
    let call_id = id("a");
    let content = [text("ok")];
    let input = [UserContent::Text("List the files.".to_owned())];
    let user = Message::User {
        tool_results: vec![ToolResult {
            call_id: &call_id,
            content: &content,
            is_error: false,
        }],
        input: &input,
    };

    assert_eq!(
        serde_json::to_string(&user).unwrap(),
        concat!(
            r#"{"user":{"tool_results":[{"call_id":"a","content":[{"text":"ok"}],"is_error":false}],"#,
            r#""input":[{"text":"List the files."}]}}"#
        )
    );
    assert_eq!(
        serde_json::to_string(&Message::Assistant(&response)).unwrap(),
        r#"{"assistant":[{"tool_use":{"id":"a","name":"bash","input":{"json":{"command":"ls"}}}}]}"#
    );
}

#[test]
fn user_content_is_text_and_nothing_a_model_or_a_tool_produces() {
    let prompt = UserContent::Text("List the files.".to_owned());

    assert_eq!(
        serde_json::to_value(&prompt).unwrap(),
        json!({ "text": "List the files." })
    );
    assert_eq!(
        serde_json::from_value::<UserContent>(json!({ "text": "List the files." })).unwrap(),
        prompt
    );
    for block in [
        json!({ "tool_use": { "id": "a", "name": "bash", "input": { "json": {} } } }),
        json!({ "tool_result": { "call_id": "a", "content": [] } }),
        json!({ "thinking": { "text": "hm" } }),
    ] {
        assert!(serde_json::from_value::<UserContent>(block).is_err());
    }
}

#[test]
fn the_size_of_user_content_is_the_sum_of_its_text_in_bytes() {
    let content = [
        UserContent::Text("caf\u{e9}".to_owned()),
        UserContent::Text("ok".to_owned()),
    ];

    assert_eq!(UserContent::bytes(&content), 5 + 2);
    assert_eq!(UserContent::bytes(&[]), 0);
}

#[test]
fn tool_uses_yields_the_calls_in_order_and_nothing_else() {
    let content = [
        ContentBlock::Text("first".to_owned()),
        ContentBlock::ToolUse(bash("a")),
        ContentBlock::Text("and".to_owned()),
        ContentBlock::ToolUse(bash("b")),
    ];

    assert_eq!(
        tool_uses(&content).collect::<Vec<_>>(),
        [&bash("a"), &bash("b")]
    );
    assert_eq!(tool_uses(&content[..1]).count(), 0);
}

/// A model that can't serialise against an awkward schema is shown its own
/// text back rather than having the whole response refused, so the call is in
/// the transcript and counts in the tool statistics like any other.
#[test]
fn arguments_that_are_not_json_are_kept_as_the_model_wrote_them() {
    let call = ToolUse {
        input: ToolInput::Unparsed(r#"{"command": "ls"#.to_owned()),
        ..bash("a")
    };

    assert_eq!(call.input_bytes(), 15);
    assert_eq!(
        serde_json::to_value(&call.input).unwrap(),
        json!({ "unparsed": r#"{"command": "ls"# })
    );
    assert_eq!(
        serde_json::to_value(ToolInput::Json(json!({ "a": 1 }))).unwrap(),
        json!({ "json": { "a": 1 } })
    );
    for form in [
        json!({ "unparsed": r#"{"command": "ls"# }),
        json!({ "json": { "a": 1 } }),
    ] {
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<ToolInput>(form.clone()).unwrap())
                .unwrap(),
            form
        );
    }
}
