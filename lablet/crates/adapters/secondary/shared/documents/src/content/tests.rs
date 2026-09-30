use lablet_model::{self as model, ToolCallId, ToolName};
use serde_json::{Value, json};

use super::*;

fn bash(call_id: &str) -> model::ToolUse {
    model::ToolUse {
        id: ToolCallId::new(call_id).unwrap(),
        name: ToolName::new("bash").unwrap(),
        input: model::ToolInput::Json(json!({ "command": "ls" })),
    }
}

/// One block of every kind, as the domain holds it, beside the form it's
/// written in.
fn every_kind_of_block() -> Vec<(model::ContentBlock, Value)> {
    vec![
        (
            model::ContentBlock::Text("hello".to_owned()),
            json!({ "text": "hello" }),
        ),
        (
            model::ContentBlock::Thinking {
                text: "hm".to_owned(),
                signature: Some("sig".to_owned()),
            },
            json!({ "thinking": { "text": "hm", "signature": "sig" } }),
        ),
        (
            model::ContentBlock::Thinking {
                text: String::new(),
                signature: None,
            },
            json!({ "thinking": { "text": "", "signature": null } }),
        ),
        (
            model::ContentBlock::RedactedThinking {
                data: "encrypted".to_owned(),
            },
            json!({ "redacted_thinking": { "data": "encrypted" } }),
        ),
        (
            model::ContentBlock::ToolUse(bash("call_a")),
            json!({ "tool_use": {
                "id": "call_a",
                "name": "bash",
                "input": { "json": { "command": "ls" } },
            } }),
        ),
        (
            model::ContentBlock::ToolUse(model::ToolUse {
                input: model::ToolInput::Unparsed(r#"{"command": "ls"#.to_owned()),
                ..bash("call_b")
            }),
            json!({ "tool_use": {
                "id": "call_b",
                "name": "bash",
                "input": { "unparsed": r#"{"command": "ls"# },
            } }),
        ),
        (
            model::ContentBlock::Opaque {
                provider: model::ProviderKind::Anthropic,
                payload: json!({ "type": "server_tool_use" }),
            },
            json!({ "opaque": {
                "provider": "anthropic",
                "payload": { "type": "server_tool_use" },
            } }),
        ),
    ]
}

#[test]
fn every_kind_of_block_is_written_in_its_one_form() {
    for (block, form) in every_kind_of_block() {
        let written = serde_json::to_value(ContentBlock::from(block)).unwrap();

        assert_eq!(written, form);
    }
}

/// What a script states is what the run is handed: nothing is lost or
/// changed on the way in, whatever kind the block is.
#[test]
fn every_kind_of_block_reads_from_its_form_as_the_block_the_domain_holds() {
    for (block, form) in every_kind_of_block() {
        let read = serde_json::from_value::<ContentBlock>(form).unwrap();

        assert_eq!(model::ContentBlock::from(read), block);
    }
}

#[test]
fn a_tool_use_is_written_with_its_id_before_its_name_before_its_input() {
    assert_eq!(
        serde_json::to_string(&ToolUse::from(bash("call_a"))).unwrap(),
        r#"{"id":"call_a","name":"bash","input":{"json":{"command":"ls"}}}"#
    );
}

#[test]
fn thinking_may_be_written_by_hand_without_a_signature() {
    let read =
        serde_json::from_value::<ContentBlock>(json!({ "thinking": { "text": "hm" } })).unwrap();

    assert_eq!(
        read,
        ContentBlock::Thinking {
            text: "hm".to_owned(),
            signature: None,
        }
    );
}

/// A result is an outcome of a turn, so a script can't put one in a
/// response, under any name.
#[test]
fn a_tool_result_is_not_a_block_a_response_can_hold() {
    let block = json!({ "tool_result": { "call_id": "a", "content": [{ "text": "ok" }] } });

    let refused = serde_json::from_value::<ContentBlock>(block).unwrap_err();

    assert!(
        refused
            .to_string()
            .contains("unknown variant `tool_result`"),
        "{refused}"
    );
}

#[test]
fn a_key_a_block_does_not_know_is_refused_whatever_kind_the_block_is() {
    for (block, unknown) in [
        (
            json!({ "thinking": { "text": "hm", "signatrue": "sig" } }),
            "signatrue",
        ),
        (
            json!({ "redacted_thinking": { "data": "encrypted", "text": "hm" } }),
            "text",
        ),
        (
            json!({ "tool_use": { "id": "a", "name": "bash", "input": { "json": {} }, "args": {} } }),
            "args",
        ),
        (
            json!({ "opaque": { "provider": "fake", "payload": {}, "kind": "x" } }),
            "kind",
        ),
    ] {
        let refused = serde_json::from_value::<ContentBlock>(block).unwrap_err();

        assert!(
            refused
                .to_string()
                .contains(&format!("unknown field `{unknown}`")),
            "{refused}"
        );
    }
}

#[test]
fn a_tool_use_whose_id_or_name_breaks_the_domain_s_rule_is_refused_with_the_rule() {
    for (id, name, broken) in [
        (
            "a",
            "not a name",
            r#"tool name "not a name" has a character other than"#,
        ),
        ("", "bash", "tool call id is empty"),
    ] {
        let block = json!({ "tool_use": { "id": id, "name": name, "input": { "json": {} } } });

        let refused = serde_json::from_value::<ContentBlock>(block).unwrap_err();

        assert!(refused.to_string().contains(broken), "{refused}");
    }
}

#[test]
fn arguments_are_the_value_that_parsed_or_the_text_that_did_not_and_nothing_else() {
    let refused = serde_json::from_value::<ToolInput>(json!({ "yaml": "a: 1" })).unwrap_err();

    assert!(
        refused.to_string().contains("unknown variant `yaml`"),
        "{refused}"
    );
}

/// An opaque block is replayed to the provider it names, so the name a
/// document holds has to be the one the domain means by it.
#[test]
fn a_provider_kind_is_spelled_as_the_domain_spells_it_and_reads_back_as_itself() {
    for kind in model::ProviderKind::ALL {
        let written = serde_json::to_value(ProviderKind::from(kind)).unwrap();

        assert_eq!(written, json!(kind.as_str()));
        assert_eq!(
            model::ProviderKind::from(serde_json::from_value::<ProviderKind>(written).unwrap()),
            kind
        );
    }
}

#[test]
fn what_a_user_supplied_and_what_a_tool_returned_are_each_written_as_their_text() {
    let prompt = UserContent::from(model::UserContent::Text("List the files.".to_owned()));
    let output = ToolResultContent::from(model::ToolResultContent::Text("exit 1".to_owned()));

    assert_eq!(
        serde_json::to_string(&prompt).unwrap(),
        r#"{"text":"List the files."}"#
    );
    assert_eq!(
        serde_json::to_string(&output).unwrap(),
        r#"{"text":"exit 1"}"#
    );
}
