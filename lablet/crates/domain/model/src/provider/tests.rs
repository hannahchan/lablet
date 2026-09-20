use serde_json::json;

use super::*;

const fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Usage {
    Usage::from_inclusive(TokenCounts {
        input,
        output,
        reasoning: 0,
        cache_read,
        cache_write,
    })
}
use crate::{TokenCounts, ToolCallId, ToolInput, ToolName, ToolUse, Usage};

fn finish_reasons() -> [(FinishReason, &'static str); 6] {
    [
        (FinishReason::EndTurn, "end_turn"),
        (FinishReason::ToolUse, "tool_use"),
        (FinishReason::MaxTokens, "max_tokens"),
        (FinishReason::ContextWindow, "context_window"),
        (FinishReason::Refusal, "refusal"),
        (FinishReason::from("pause_turn".to_owned()), "pause_turn"),
    ]
}

#[test]
fn a_finish_reason_prints_what_it_serialises_as_and_reads_back_as_itself() {
    for (reason, spelling) in finish_reasons() {
        assert_eq!(reason.to_string(), spelling);
        assert_eq!(String::from(reason.clone()), spelling);
        assert_eq!(serde_json::to_value(&reason).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<FinishReason>(json!(spelling)).unwrap(),
            reason
        );
    }
}

#[test]
fn every_provider_spelling_of_a_known_reason_gives_that_reason() {
    for (spelling, reason) in [
        ("end_turn", FinishReason::EndTurn),
        ("stop", FinishReason::EndTurn),
        ("stop_sequence", FinishReason::EndTurn),
        ("tool_use", FinishReason::ToolUse),
        ("tool_calls", FinishReason::ToolUse),
        ("max_tokens", FinishReason::MaxTokens),
        ("length", FinishReason::MaxTokens),
        ("context_window", FinishReason::ContextWindow),
        ("model_context_window_exceeded", FinishReason::ContextWindow),
        ("refusal", FinishReason::Refusal),
        ("content_filter", FinishReason::Refusal),
    ] {
        assert_eq!(
            FinishReason::from(spelling.to_owned()),
            reason,
            "{spelling}"
        );
        assert_eq!(
            serde_json::from_value::<FinishReason>(json!(spelling)).unwrap(),
            reason,
            "{spelling}"
        );
    }
}

#[test]
fn only_a_string_that_spells_no_known_reason_is_kept_as_other() {
    for spelling in ["pause_turn", ""] {
        let reason = FinishReason::from(spelling.to_owned());

        assert!(
            matches!(&reason, FinishReason::Other(kept) if kept.as_str() == spelling),
            "{reason:?}"
        );
    }
    for (_, spelling) in finish_reasons().into_iter().take(5) {
        assert!(
            !matches!(
                FinishReason::from(spelling.to_owned()),
                FinishReason::Other(_)
            ),
            "{spelling}"
        );
    }
}

#[test]
fn a_provider_kind_prints_what_it_serialises_as() {
    for (kind, spelling) in [
        (ProviderKind::Anthropic, "anthropic"),
        (ProviderKind::Openai, "openai"),
        (ProviderKind::Fake, "fake"),
    ] {
        assert_eq!(kind.to_string(), spelling);
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<ProviderKind>(json!(spelling)).unwrap(),
            kind
        );
    }
}

#[test]
fn an_effort_prints_what_it_serialises_as_and_xhigh_is_one_word() {
    for (effort, spelling) in [
        (Effort::Low, "low"),
        (Effort::Medium, "medium"),
        (Effort::High, "high"),
        (Effort::XHigh, "xhigh"),
        (Effort::Max, "max"),
    ] {
        assert_eq!(effort.to_string(), spelling);
        assert_eq!(serde_json::to_value(effort).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<Effort>(json!(spelling)).unwrap(),
            effort
        );
    }
}

#[test]
fn thinking_defaults_to_the_providers_own_behaviour() {
    assert_eq!(Thinking::default(), Thinking::ProviderDefault);
}

#[test]
fn thinking_has_a_form_a_person_can_write() {
    for (thinking, form) in [
        (Thinking::ProviderDefault, json!("provider_default")),
        (Thinking::Adaptive, json!("adaptive")),
        (
            Thinking::Budget(NonZeroU32::new(2048).unwrap()),
            json!({ "budget": 2048 }),
        ),
        (Thinking::Disabled, json!("disabled")),
    ] {
        assert_eq!(serde_json::to_value(thinking).unwrap(), form);
        assert_eq!(serde_json::from_value::<Thinking>(form).unwrap(), thinking);
    }
}

#[test]
fn a_thinking_budget_of_zero_is_refused() {
    assert!(serde_json::from_value::<Thinking>(json!({ "budget": 0 })).is_err());
}

#[test]
fn request_defaults_have_one_json_form() {
    let request = RequestParams {
        max_tokens: 4096,
        temperature: Some(0.7),
        thinking: Thinking::Budget(NonZeroU32::new(1024).unwrap()),
        effort: Some(Effort::High),
        seed: Some(7),
    };
    let expected = json!({
        "max_tokens": 4096,
        "temperature": 0.7,
        "thinking": { "budget": 1024 },
        "effort": "high",
        "seed": 7,
    });

    assert_eq!(serde_json::to_value(&request).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<RequestParams>(expected).unwrap(),
        request
    );
}

fn bash(id: &str) -> ContentBlock {
    ContentBlock::ToolUse(ToolUse {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new("bash").unwrap(),
        input: ToolInput::Json(json!({})),
    })
}

#[test]
fn a_script_cannot_give_a_completion_a_role_or_a_misspelt_field() {
    for script in [
        json!({ "role": "user", "content": [], "finish": "end_turn" }),
        json!({ "message": { "role": "user", "content": [] }, "finish": "end_turn" }),
        json!({ "content": [], "finish": "end_turn", "usage": { "input_token": 12 } }),
        json!({ "content": [], "finish": "end_turn", "reponse_id": "msg_1" }),
    ] {
        assert!(
            serde_json::from_value::<ProviderResponse>(script.clone()).is_err(),
            "{script}"
        );
    }
}

#[test]
fn a_script_cannot_put_a_tool_result_in_a_completion() {
    let script = json!({
        "content": [{ "tool_result": { "call_id": "a", "content": [] } }],
        "finish": "end_turn",
    });

    assert!(serde_json::from_value::<ProviderResponse>(script).is_err());
}

#[test]
fn a_model_ref_and_an_endpoint_have_one_json_form() {
    let model = ModelRef {
        provider: ProviderKind::Anthropic,
        name: "claude-sonnet-5".to_owned(),
    };
    let endpoint = Endpoint {
        host: "api.anthropic.com".to_owned(),
        port: 443,
    };

    assert_eq!(
        serde_json::to_value(&model).unwrap(),
        json!({ "provider": "anthropic", "name": "claude-sonnet-5" })
    );
    assert_eq!(
        serde_json::to_value(&endpoint).unwrap(),
        json!({ "host": "api.anthropic.com", "port": 443 })
    );
}

/// The spellings are a failed chat span's `error.type`. That attribute is an
/// open set in the conventions, so no generated enum can pin them and this
/// does.
#[test]
fn every_provider_error_kind_is_spelled_as_the_chat_span_reports_it() {
    for (kind, spelling, retryable) in [
        (ProviderErrorKind::Retryable, "retryable", true),
        (
            ProviderErrorKind::ContextExhausted,
            "context_exhausted",
            false,
        ),
        (ProviderErrorKind::Fatal, "fatal", false),
        (ProviderErrorKind::Malformed, "malformed", true),
    ] {
        assert_eq!(kind.as_str(), spelling);
        assert_eq!(kind.to_string(), spelling);
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<ProviderErrorKind>(json!(spelling)).unwrap(),
            kind
        );
        assert_eq!(kind.is_retryable(), retryable, "{spelling}");
    }
}

#[test]
fn a_repeated_tool_use_id_is_refused_in_code_and_in_a_script() {
    let repeated = ProviderResponse::new(
        vec![bash("a"), bash("b"), bash("a")],
        Usage::default(),
        FinishReason::ToolUse,
        None,
        None,
    );
    let script = json!({
        "content": [
            { "tool_use": { "id": "a", "name": "bash", "input": { "json": {} } } },
            { "tool_use": { "id": "a", "name": "bash", "input": { "json": {} } } },
        ],
        "finish": "tool_use",
    });

    assert_eq!(
        repeated,
        Err(ResponseError::DuplicateToolUse { id: "a".to_owned() })
    );
    let error = serde_json::from_value::<ProviderResponse>(script).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(r#"tool call id "a" is on more than one tool-use block"#),
        "{error}"
    );
}

#[test]
fn a_completion_reads_from_the_form_a_script_would_hold() {
    let script = json!({
        "content": [{ "text": "done" }],
        "usage": { "input_tokens": 12, "output_tokens": 3 },
        "finish": "stop",
    });
    let completion = ProviderResponse::new(
        vec![ContentBlock::Text("done".to_owned())],
        usage(12, 3, 0, 0),
        FinishReason::EndTurn,
        None,
        None,
    )
    .unwrap();

    assert_eq!(
        serde_json::from_value::<ProviderResponse>(script).unwrap(),
        completion
    );
    assert_eq!(
        serde_json::to_value(&completion).unwrap(),
        json!({
            "content": [{ "text": "done" }],
            "usage": {
                "input_tokens": 12,
                "output_tokens": 3,
                "reasoning_output_tokens": 0,
                "cache_read_tokens": 0,
                "cache_write_tokens": 0,
            },
            "finish": "end_turn",
            "response_id": null,
            "response_model": null,
        })
    );
}

#[test]
fn a_completion_whose_tool_calls_have_distinct_ids_holds_its_content_in_order() {
    let content = vec![ContentBlock::Text("on it".to_owned()), bash("a"), bash("b")];

    let completion = ProviderResponse::new(
        content.clone(),
        usage(12, 3, 0, 0),
        FinishReason::ToolUse,
        Some("msg_1".to_owned()),
        Some("model-2026".to_owned()),
    )
    .unwrap();

    assert_eq!(completion.content(), content);
    assert_eq!(completion.usage, usage(12, 3, 0, 0));
    assert_eq!(completion.finish, FinishReason::ToolUse);
    assert_eq!(completion.response_id.as_deref(), Some("msg_1"));
    assert_eq!(completion.response_model.as_deref(), Some("model-2026"));
}

#[test]
fn a_completion_without_usage_used_no_tokens() {
    let completion = serde_json::from_value::<ProviderResponse>(json!({
        "content": [],
        "finish": "end_turn",
        "response_id": "msg_1",
        "response_model": "model-2026",
    }))
    .unwrap();

    assert_eq!(completion.usage, Usage::default());
    assert_eq!(completion.response_id.as_deref(), Some("msg_1"));
    assert_eq!(completion.response_model.as_deref(), Some("model-2026"));
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

    /// Reading a provider's word for a finish reason twice says the same
    /// thing as reading it once, so a reason that round-trips through a
    /// document can't drift.
    #[test]
    fn normalising_a_finish_reason_is_idempotent(reason in ".{0,24}") {
        let once = FinishReason::from(reason.clone());
        let twice = FinishReason::from(once.as_str().to_owned());

        proptest::prop_assert_eq!(&once, &twice);
        proptest::prop_assert_eq!(
            serde_json::from_value::<FinishReason>(json!(reason)).unwrap(),
            once
        );
    }
}
