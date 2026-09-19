use serde_json::json;

use super::*;

fn bash(id: &str) -> ToolUse {
    ToolUse {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new("bash").unwrap(),
        input: json!({ "command": "ls" }),
    }
}

fn tool_use(id: &str) -> ContentBlock {
    ContentBlock::ToolUse(bash(id))
}

fn output(call_id: &str, text: &str) -> ToolResult {
    ToolResult {
        call_id: ToolCallId::new(call_id).unwrap(),
        content: vec![ToolResultContent::Text(text.to_owned())],
        is_error: false,
    }
}

fn tool_result(call_id: &str, text: &str) -> ContentBlock {
    ContentBlock::ToolResult(output(call_id, text))
}

fn text(text: &str) -> ContentBlock {
    ContentBlock::Text(text.to_owned())
}

fn thinking() -> ContentBlock {
    ContentBlock::Thinking {
        text: "hm".to_owned(),
        signature: Some("sig".to_owned()),
    }
}

#[test]
fn a_message_with_distinct_tool_use_ids_is_valid() {
    let message = Message::new(
        Role::Assistant,
        vec![thinking(), text("on it"), tool_use("a"), tool_use("b")],
    )
    .unwrap();

    assert_eq!(message.role, Role::Assistant);
    assert_eq!(message.content.len(), 4);
}

#[test]
fn a_repeated_tool_use_id_is_refused() {
    let error = Message::new(
        Role::Assistant,
        vec![tool_use("a"), tool_use("b"), tool_use("a")],
    )
    .unwrap_err();

    assert_eq!(error, MessageError::DuplicateToolUse { id: "a".to_owned() });
}

#[test]
fn a_repeated_tool_result_id_is_refused() {
    let error = Message::new(
        Role::User,
        vec![tool_result("a", "one"), tool_result("a", "two")],
    )
    .unwrap_err();

    assert_eq!(
        error,
        MessageError::DuplicateToolResult { id: "a".to_owned() }
    );
}

#[test]
fn a_user_message_answers_each_of_several_calls_once() {
    assert!(
        Message::new(
            Role::User,
            vec![tool_result("a", "one"), tool_result("b", "two")]
        )
        .is_ok()
    );
}

#[test]
fn a_tool_use_block_in_a_user_message_is_refused() {
    assert_eq!(
        Message::new(Role::User, vec![tool_use("a")]),
        Err(MessageError::ToolUseFromUser { id: "a".to_owned() })
    );
}

#[test]
fn a_tool_result_block_in_an_assistant_message_is_refused() {
    assert_eq!(
        Message::new(Role::Assistant, vec![tool_result("a", "one")]),
        Err(MessageError::ToolResultFromAssistant { id: "a".to_owned() })
    );
}

#[test]
fn the_first_violation_in_block_order_is_the_one_reported() {
    let message = Message {
        role: Role::Assistant,
        content: vec![tool_use("a"), tool_result("x", "misplaced"), tool_use("a")],
    };

    assert_eq!(
        message.validate(),
        Err(MessageError::ToolResultFromAssistant { id: "x".to_owned() })
    );
}

#[test]
fn a_message_built_field_by_field_is_checked_by_validate() {
    let message = Message {
        role: Role::Assistant,
        content: vec![tool_use("a"), tool_use("a")],
    };

    assert!(message.validate().is_err());
}

#[test]
fn every_error_names_the_offending_id() {
    for error in [
        MessageError::DuplicateToolUse {
            id: "call_7".to_owned(),
        },
        MessageError::DuplicateToolResult {
            id: "call_7".to_owned(),
        },
        MessageError::ToolUseFromUser {
            id: "call_7".to_owned(),
        },
        MessageError::ToolResultFromAssistant {
            id: "call_7".to_owned(),
        },
    ] {
        assert!(error.to_string().contains("\"call_7\""), "{error}");
    }
}

#[test]
fn text_concatenates_the_text_blocks_and_skips_the_rest() {
    let message = Message {
        role: Role::Assistant,
        content: vec![
            thinking(),
            text("Done. "),
            tool_use("a"),
            ContentBlock::RedactedThinking {
                data: "opaque".to_owned(),
            },
            ContentBlock::Opaque {
                provider: ProviderKind::Openai,
                payload: json!({ "type": "refusal" }),
            },
            text("Two files changed."),
        ],
    };

    assert_eq!(message.text(), "Done. Two files changed.");
}

#[test]
fn text_is_empty_for_a_message_without_text_blocks() {
    let message = Message {
        role: Role::User,
        content: vec![tool_result("a", "not message text")],
    };

    assert_eq!(message.text(), "");
}

#[test]
fn tool_result_finds_the_block_that_answers_a_call() {
    let message = Message {
        role: Role::User,
        content: vec![
            text("note"),
            tool_result("a", "one"),
            tool_result("b", "two"),
        ],
    };

    assert_eq!(
        message.tool_result(&ToolCallId::new("b").unwrap()),
        Some(&output("b", "two"))
    );
    assert_eq!(message.tool_result(&ToolCallId::new("c").unwrap()), None);
}

#[test]
fn tool_uses_and_tool_results_yield_their_blocks_in_order_and_nothing_else() {
    let assistant = Message {
        role: Role::Assistant,
        content: vec![thinking(), tool_use("a"), text("and"), tool_use("b")],
    };
    let user = Message {
        role: Role::User,
        content: vec![
            tool_result("b", "two"),
            text("note"),
            tool_result("a", "one"),
        ],
    };

    assert_eq!(
        assistant.tool_uses().collect::<Vec<_>>(),
        [&bash("a"), &bash("b")]
    );
    assert_eq!(assistant.tool_results().count(), 0);
    assert_eq!(
        user.tool_results().collect::<Vec<_>>(),
        [&output("b", "two"), &output("a", "one")]
    );
    assert_eq!(user.tool_uses().count(), 0);
}

#[test]
fn input_bytes_is_the_length_of_the_input_as_compact_json() {
    let call = ToolUse {
        input: json!({ "command": "ls", "n": 2 }),
        ..bash("a")
    };

    assert_eq!(call.input.to_string(), r#"{"command":"ls","n":2}"#);
    assert_eq!(call.input_bytes(), 22);
    assert_eq!(
        ToolUse {
            input: json!({}),
            ..bash("a")
        }
        .input_bytes(),
        2
    );
}

#[test]
fn content_bytes_sums_the_text_of_a_result_in_bytes_not_characters() {
    let result = ToolResult {
        content: vec![
            ToolResultContent::Text("exit 1".to_owned()),
            ToolResultContent::Text("caf\u{e9}".to_owned()),
        ],
        ..output("a", "")
    };

    assert_eq!(result.content_bytes(), 6 + 5);
    assert_eq!(output("a", "").content_bytes(), 0);
}

#[test]
fn omitted_content_is_worded_one_way() {
    assert_eq!(
        ToolResultContent::omitted("image", "image/png", 48_213),
        ToolResultContent::Text("[image omitted: image/png, 48213 bytes]".to_owned())
    );
}

#[test]
fn tool_result_does_not_mistake_a_tool_use_for_its_answer() {
    let message = Message {
        role: Role::Assistant,
        content: vec![tool_use("a")],
    };

    assert_eq!(message.tool_result(&ToolCallId::new("a").unwrap()), None);
}

#[test]
fn a_message_has_one_json_form() {
    let message = Message {
        role: Role::Assistant,
        content: vec![
            text("hello"),
            thinking(),
            ContentBlock::RedactedThinking {
                data: "opaque".to_owned(),
            },
            tool_use("a"),
            ContentBlock::Opaque {
                provider: ProviderKind::Anthropic,
                payload: json!({ "type": "server_tool_use" }),
            },
        ],
    };
    let expected = json!({
        "role": "assistant",
        "content": [
            { "text": "hello" },
            { "thinking": { "text": "hm", "signature": "sig" } },
            { "redacted_thinking": { "data": "opaque" } },
            { "tool_use": { "id": "a", "name": "bash", "input": { "command": "ls" } } },
            { "opaque": { "provider": "anthropic", "payload": { "type": "server_tool_use" } } },
        ],
    });

    assert_eq!(serde_json::to_value(&message).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<Message>(expected).unwrap(),
        message
    );
}

#[test]
fn a_tool_result_has_one_json_form() {
    let message = Message {
        role: Role::User,
        content: vec![ContentBlock::ToolResult(ToolResult {
            call_id: ToolCallId::new("a").unwrap(),
            content: vec![
                ToolResultContent::Text("exit 1".to_owned()),
                ToolResultContent::Text("no such file".to_owned()),
            ],
            is_error: true,
        })],
    };
    let expected = json!({
        "role": "user",
        "content": [{
            "tool_result": {
                "call_id": "a",
                "content": [{ "text": "exit 1" }, { "text": "no such file" }],
                "is_error": true,
            },
        }],
    });

    assert_eq!(serde_json::to_value(&message).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<Message>(expected).unwrap(),
        message
    );
}

#[test]
fn the_tool_blocks_serialise_to_these_exact_bytes() {
    assert_eq!(
        serde_json::to_string(&tool_use("a")).unwrap(),
        r#"{"tool_use":{"id":"a","name":"bash","input":{"command":"ls"}}}"#
    );
    assert_eq!(
        serde_json::to_string(&tool_result("a", "ok")).unwrap(),
        r#"{"tool_result":{"call_id":"a","content":[{"text":"ok"}],"is_error":false}}"#
    );
}

#[test]
fn a_tool_result_without_is_error_reads_as_a_success() {
    let block = json!({ "tool_result": { "call_id": "a", "content": [{ "text": "ok" }] } });

    assert_eq!(
        serde_json::from_value::<ContentBlock>(block).unwrap(),
        tool_result("a", "ok")
    );
}

#[test]
fn a_misspelt_field_of_a_tool_block_is_an_error() {
    let result = json!({ "tool_result": { "call_id": "a", "content": [], "is_eror": true } });
    let call = json!({ "tool_use": { "id": "a", "name": "bash", "input": {}, "args": {} } });

    assert!(serde_json::from_value::<ContentBlock>(result).is_err());
    assert!(serde_json::from_value::<ContentBlock>(call).is_err());
}

#[test]
fn a_structured_tool_result_is_not_a_form_the_model_reads() {
    let block = json!({
        "tool_result": { "call_id": "a", "content": [{ "json": { "code": 1 } }] },
    });

    assert!(serde_json::from_value::<ContentBlock>(block).is_err());
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
fn reading_a_message_validates_it() {
    let from_user = json!({
        "role": "user",
        "content": [{ "tool_use": { "id": "a", "name": "bash", "input": {} } }],
    });
    let repeated = json!({
        "role": "assistant",
        "content": [
            { "tool_use": { "id": "a", "name": "bash", "input": {} } },
            { "tool_use": { "id": "a", "name": "bash", "input": {} } },
        ],
    });

    let error = serde_json::from_value::<Message>(from_user).unwrap_err();
    assert!(
        error.to_string().contains("is in a user message"),
        "{error}"
    );
    let error = serde_json::from_value::<Message>(repeated).unwrap_err();
    assert!(
        error.to_string().contains("more than one tool-use block"),
        "{error}"
    );
}

#[test]
fn a_message_with_a_field_it_does_not_have_is_refused() {
    let message = json!({ "role": "user", "content": [], "name": "sam" });

    assert!(serde_json::from_value::<Message>(message).is_err());
}

#[test]
fn a_tool_use_with_an_invalid_name_does_not_deserialise() {
    let block = json!({ "tool_use": { "id": "a", "name": "not a name", "input": {} } });

    assert!(serde_json::from_value::<ContentBlock>(block).is_err());
}
