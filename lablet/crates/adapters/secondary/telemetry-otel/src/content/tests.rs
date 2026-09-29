use lablet_model::{ProviderKind, ToolConcurrency, ToolName, ToolUse};
use serde_json::json;

use super::*;

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn text(text: &str) -> ContentBlock {
    ContentBlock::Text(text.to_owned())
}

fn call(id: &str, name: &str, input: ToolInput) -> ContentBlock {
    ContentBlock::ToolUse(ToolUse {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new(name).unwrap(),
        input,
    })
}

fn said(text: &str) -> Vec<ToolResultContent> {
    vec![ToolResultContent::Text(text.to_owned())]
}

fn id(call: &str) -> ToolCallId {
    ToolCallId::new(call).unwrap()
}

fn user(prompt: &str) -> Value {
    json!({ "role": "user", "parts": [{ "type": "text", "content": prompt }] })
}

#[test]
fn a_tool_definition_holds_what_the_model_was_told_of_the_tool() {
    let specs = [
        ToolSpec {
            name: ToolName::new("bash").unwrap(),
            description: "Runs a command.".to_owned(),
            input_schema: json!({ "type": "object", "properties": { "command": { "default": null } } }),
            source: ToolSource::Builtin,
            concurrency: ToolConcurrency::Exclusive,
        },
        ToolSpec {
            name: ToolName::new("mcp__docs__search").unwrap(),
            description: "Searches the docs.".to_owned(),
            input_schema: json!({ "type": "object" }),
            source: ToolSource::Mcp {
                server: "docs".to_owned(),
            },
            concurrency: ToolConcurrency::Shared,
        },
    ];

    assert_eq!(
        parsed(&tool_definitions(&specs)),
        json!([
            {
                "type": "function",
                "name": "bash",
                "description": "Runs a command.",
                "parameters": { "type": "object", "properties": { "command": { "default": null } } },
            },
            {
                "type": "extension",
                "name": "mcp__docs__search",
                "description": "Searches the docs.",
                "parameters": { "type": "object" },
            },
        ])
    );
    assert_eq!(tool_definitions(&[]), "[]");
}

#[test]
fn a_tool_result_is_an_object_that_says_whether_the_call_failed() {
    assert_eq!(
        tool_result(
            &[
                ToolResultContent::Text("one".to_owned()),
                ToolResultContent::Text("two".to_owned())
            ],
            false
        ),
        json!({
            "content": [{ "type": "text", "text": "one" }, { "type": "text", "text": "two" }],
            "isError": false,
        })
    );
    assert_eq!(
        tool_result(&[], true),
        json!({ "content": [], "isError": true })
    );
}

#[test]
fn the_first_call_is_sent_the_prompt_and_answers_with_its_response() {
    let mut conversation = Conversation::opening("You fix tests.", "Fix the failing test.");

    let exchange = conversation.responded(&[text("Done.")]);

    assert_eq!(
        parsed(&exchange.system),
        json!([{ "type": "text", "content": "You fix tests." }])
    );
    assert_eq!(
        parsed(&exchange.input),
        json!([user("Fix the failing test.")])
    );
    assert_eq!(
        parsed(&exchange.output.unwrap()),
        json!([{ "role": "assistant", "parts": [{ "type": "text", "content": "Done." }] }])
    );
}

#[test]
fn every_kind_of_block_is_a_part_that_holds_what_the_block_held() {
    let mut conversation = Conversation::opening("", "Go.");

    let exchange = conversation.responded(&[
        ContentBlock::Thinking {
            text: "The user wants the files.".to_owned(),
            signature: Some("c2lnbmVk".to_owned()),
        },
        ContentBlock::RedactedThinking {
            data: "ZW5jcnlwdGVk".to_owned(),
        },
        text("I'll list them."),
        call(
            "call_1",
            "bash",
            ToolInput::Json(json!({ "command": "ls", "cwd": null })),
        ),
        call(
            "call_2",
            "read_file",
            ToolInput::Unparsed("{\"path\": \"READ".to_owned()),
        ),
        ContentBlock::Opaque {
            provider: ProviderKind::Openai,
            payload: json!({ "type": "reasoning", "encrypted_content": "abc" }),
        },
    ]);

    assert_eq!(
        parsed(&exchange.output.unwrap()),
        json!([{
            "role": "assistant",
            "parts": [
                { "type": "reasoning", "content": "The user wants the files." },
                { "type": "redacted_reasoning", "data": "ZW5jcnlwdGVk" },
                { "type": "text", "content": "I'll list them." },
                {
                    "type": "tool_call",
                    "id": "call_1",
                    "name": "bash",
                    "arguments": { "command": "ls", "cwd": null },
                },
                {
                    "type": "tool_call",
                    "id": "call_2",
                    "name": "read_file",
                    "arguments": "{\"path\": \"READ",
                },
                {
                    "type": "opaque",
                    "provider": "openai",
                    "payload": { "type": "reasoning", "encrypted_content": "abc" },
                },
            ],
        }])
    );
}

#[test]
fn a_later_call_is_sent_the_results_in_call_order_whichever_call_finished_first() {
    let mut conversation = Conversation::opening("", "Go.");
    let first = [
        call("call_1", "bash", ToolInput::Json(json!({}))),
        call("call_2", "read_file", ToolInput::Json(json!({}))),
    ];
    conversation.responded(&first);
    conversation.answered(&id("call_2"), &said("the file"), false);
    conversation.answered(&id("call_1"), &said("no such command"), true);

    let exchange = conversation.responded(&[text("Done.")]);

    let input = parsed(&exchange.input);
    assert_eq!(input[0], user("Go."));
    assert_eq!(input[1]["role"], "assistant");
    assert_eq!(
        input[2],
        json!({
            "role": "tool",
            "parts": [
                {
                    "type": "tool_call_response",
                    "id": "call_1",
                    "response": {
                        "content": [{ "type": "text", "text": "no such command" }],
                        "isError": true,
                    },
                },
                {
                    "type": "tool_call_response",
                    "id": "call_2",
                    "response": {
                        "content": [{ "type": "text", "text": "the file" }],
                        "isError": false,
                    },
                },
            ],
        })
    );
    assert_eq!(input.as_array().unwrap().len(), 3);
}

#[test]
fn an_attempt_that_failed_was_sent_what_the_attempt_after_it_is_sent() {
    let mut conversation = Conversation::opening("", "Go.");
    conversation.responded(&[call("call_1", "bash", ToolInput::Json(json!({})))]);
    conversation.answered(&id("call_1"), &said("ok"), false);

    let failed = conversation.unanswered();
    let answered = conversation.responded(&[text("Done.")]);

    assert_eq!(failed.output, None);
    assert_eq!(failed.input, answered.input);
    assert_eq!(failed.system, answered.system);
    assert_eq!(
        parsed(&failed.input).as_array().unwrap().len(),
        3,
        "the results are one message, and it's there once"
    );
}

#[test]
fn a_call_that_was_never_answered_leaves_no_part_and_a_turn_of_them_no_message() {
    let mut conversation = Conversation::opening("", "Go.");
    conversation.responded(&[
        call("call_1", "bash", ToolInput::Json(json!({}))),
        call("call_2", "bash", ToolInput::Json(json!({}))),
    ]);
    conversation.answered(&id("call_1"), &said("ok"), false);

    let partly = conversation.responded(&[call("call_3", "bash", ToolInput::Json(json!({})))]);
    let none = conversation.unanswered();

    let partly = parsed(&partly.input);
    assert_eq!(partly[2]["parts"].as_array().unwrap().len(), 1);
    assert_eq!(partly[2]["parts"][0]["id"], "call_1");
    let none = parsed(&none.input);
    assert_eq!(
        none.as_array().unwrap().len(),
        4,
        "the prompt, two responses, and the results of the first"
    );
    assert_eq!(none[3]["role"], "assistant");
}

#[test]
fn a_result_for_a_call_the_last_response_did_not_make_joins_no_message() {
    let mut conversation = Conversation::opening("", "Go.");
    conversation.responded(&[call("call_1", "bash", ToolInput::Json(json!({})))]);
    conversation.answered(&id("call_9"), &said("stray"), false);
    conversation.answered(&id("call_1"), &said("ok"), false);

    let second = conversation.responded(&[call("call_9", "bash", ToolInput::Json(json!({})))]);
    let third = conversation.unanswered();

    assert_eq!(parsed(&second.input)[2]["parts"][0]["id"], "call_1");
    assert_eq!(
        parsed(&third.input).as_array().unwrap().len(),
        4,
        "the stray result didn't wait for a later call of the same id"
    );
}
