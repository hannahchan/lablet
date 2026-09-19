use serde_json::json;

use super::*;
use crate::{ContentBlock, Role};

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
fn a_finish_reason_prints_what_it_serialises_as() {
    for (reason, spelling) in [
        (FinishReason::EndTurn, "end_turn"),
        (FinishReason::ToolUse, "tool_use"),
        (FinishReason::MaxTokens, "max_tokens"),
        (
            FinishReason::Other("content_filter".to_owned()),
            "content_filter",
        ),
    ] {
        assert_eq!(reason.to_string(), spelling);
        assert_eq!(serde_json::to_value(&reason).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<FinishReason>(json!(spelling)).unwrap(),
            reason
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
    let thinking = Thinking::default();

    assert_eq!(thinking.mode, ThinkingMode::Default);
    assert_eq!(thinking.budget, None);
    assert_eq!(thinking.effort, None);
}

#[test]
fn request_defaults_have_one_json_form() {
    let request = RequestDefaults {
        max_tokens: 4096,
        temperature: Some(0.5),
        thinking: Thinking {
            mode: ThinkingMode::Enabled,
            budget: Some(1024),
            effort: Some(Effort::High),
        },
        seed: Some(7),
    };
    let expected = json!({
        "max_tokens": 4096,
        "temperature": 0.5,
        "thinking": { "mode": "enabled", "budget": 1024, "effort": "high" },
        "seed": 7,
    });

    assert_eq!(serde_json::to_value(&request).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<RequestDefaults>(expected).unwrap(),
        request
    );
    assert_eq!(
        serde_json::to_value(ThinkingMode::Disabled).unwrap(),
        json!("disabled")
    );
}

#[test]
fn a_cost_is_a_bare_number_of_dollars() {
    let cost = Cost::new(0.0125);

    assert!((cost.usd() - 0.0125).abs() < f64::EPSILON);
    assert_eq!(serde_json::to_value(cost).unwrap(), json!(0.0125));
    assert_eq!(serde_json::from_value::<Cost>(json!(0.0125)).unwrap(), cost);
    assert!(Cost::new(1.0) < Cost::new(2.0));
}

#[test]
fn a_completion_reads_from_the_form_a_script_would_hold() {
    let script = json!({
        "message": { "role": "assistant", "content": [{ "text": "done" }] },
        "usage": { "input_tokens": 12, "output_tokens": 3 },
        "finish": "end_turn",
    });
    let completion = Completion {
        message: Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Text("done".to_owned())],
        },
        usage: usage(12, 3, 0, 0),
        finish: FinishReason::EndTurn,
        response_id: None,
        response_model: None,
    };

    assert_eq!(
        serde_json::from_value::<Completion>(script).unwrap(),
        completion
    );
    assert_eq!(
        serde_json::to_value(&completion).unwrap(),
        json!({
            "message": { "role": "assistant", "content": [{ "text": "done" }] },
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
