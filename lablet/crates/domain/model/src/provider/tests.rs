use serde_json::json;

use super::*;
use crate::{ToolCallId, ToolName, ToolUse};

const fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Usage {
    Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
    }
}

#[test]
fn total_is_input_plus_output_because_input_already_holds_the_cached_tokens() {
    assert_eq!(usage(1000, 200, 700, 100).total(), 1200);
}

#[test]
fn uncached_input_is_input_less_both_cache_fields() {
    assert_eq!(usage(1000, 200, 700, 100).uncached_input_tokens(), 200);
    assert_eq!(usage(1000, 200, 0, 0).uncached_input_tokens(), 1000);
}

#[test]
fn uncached_input_stops_at_zero_when_a_provider_reports_more_cache_than_input() {
    assert_eq!(usage(10, 0, 8, 8).uncached_input_tokens(), 0);
}

#[test]
fn usage_adds_field_by_field() {
    let sum = usage(1, 20, 300, 4000) + usage(5, 60, 700, 8000);

    assert_eq!(sum, usage(6, 80, 1000, 12000));
}

#[test]
fn add_assign_accumulates_like_add() {
    let mut running = usage(1, 2, 3, 4);
    running += usage(10, 20, 30, 40);
    running += usage(100, 200, 300, 400);

    assert_eq!(running, usage(111, 222, 333, 444));
}

#[test]
fn sums_saturate_rather_than_overflow() {
    let full = usage(u64::MAX, u64::MAX, u64::MAX, u64::MAX);

    assert_eq!(full + usage(1, 1, 1, 1), full);
    assert_eq!(full.total(), u64::MAX);
}

#[test]
fn the_default_usage_is_zero() {
    assert_eq!(Usage::default(), usage(0, 0, 0, 0));
}

#[test]
fn usage_serialises_all_four_fields_and_reads_a_missing_one_as_zero() {
    assert_eq!(
        serde_json::to_value(usage(1, 2, 3, 4)).unwrap(),
        json!({
            "input_tokens": 1,
            "output_tokens": 2,
            "cache_read_tokens": 3,
            "cache_write_tokens": 4,
        })
    );
    assert_eq!(
        serde_json::from_value::<Usage>(json!({ "input_tokens": 9, "output_tokens": 1 })).unwrap(),
        usage(9, 1, 0, 0)
    );
}

#[test]
fn from_inclusive_takes_the_input_count_as_it_is() {
    assert_eq!(
        Usage::from_inclusive(1000, 200, 700, 100),
        usage(1000, 200, 700, 100)
    );
}

#[test]
fn from_uncached_adds_both_cache_counts_into_the_input() {
    assert_eq!(
        Usage::from_uncached(200, 50, 700, 100),
        usage(1000, 50, 700, 100)
    );
    assert_eq!(
        Usage::from_uncached(200, 50, 700, 100).uncached_input_tokens(),
        200
    );
    assert_eq!(
        Usage::from_uncached(u64::MAX, 0, 1, 1).input_tokens,
        u64::MAX
    );
}

#[test]
fn a_misspelt_usage_field_is_an_error_not_a_silent_zero() {
    let error = serde_json::from_value::<Usage>(json!({ "input_token": 12, "output_tokens": 3 }))
        .unwrap_err();

    assert!(error.to_string().contains("input_token"), "{error}");
}

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

fn usd(usd: f64) -> Cost {
    Cost::new(usd).expect("the amounts in these tests are all real costs")
}

#[test]
fn a_cost_is_a_bare_number_of_dollars() {
    let cost = usd(0.0125);

    assert!((cost.usd() - 0.0125).abs() < f64::EPSILON);
    assert_eq!(serde_json::to_value(cost).unwrap(), json!(0.0125));
    assert_eq!(serde_json::from_value::<Cost>(json!(0.0125)).unwrap(), cost);
    assert!(usd(1.0) < usd(2.0));
    assert_eq!(usd(0.0), Cost::new(0.0).unwrap());
}

/// An amount JSON can't write, or one that means nothing, is refused on both
/// paths: serde would otherwise put a `null` where a cost belongs, and `null`
/// is how a run with no pricing configured writes the same field.
#[test]
fn an_amount_that_is_not_a_finite_number_of_dollars_is_no_cost() {
    for amount in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01] {
        assert!(Cost::new(amount).is_err(), "{amount}");
    }
    // JSON has no infinity or NaN, so a negative is the only one a document
    // can hold.
    assert!(serde_json::from_value::<Cost>(json!(-0.01)).is_err());
    assert_eq!(
        Cost::new(-1.5).unwrap_err().to_string(),
        "-1.5 isn't a finite number of US dollars of at least 0"
    );
}

fn bash(id: &str) -> ContentBlock {
    ContentBlock::ToolUse(ToolUse {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new("bash").unwrap(),
        input: json!({}),
    })
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
            { "tool_use": { "id": "a", "name": "bash", "input": {} } },
            { "tool_use": { "id": "a", "name": "bash", "input": {} } },
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
