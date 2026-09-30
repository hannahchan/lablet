use serde_json::json;

use super::*;

/// What a provider that reports only its input and output gives.
const fn usage(input: u64, output: u64) -> Usage {
    Usage::from_inclusive(TokenCounts {
        input,
        output,
        reasoning: None,
        cache_read: None,
        cache_write: None,
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
fn a_finish_reason_prints_as_it_is_written_down_and_reads_back_as_itself() {
    for (reason, spelling) in finish_reasons() {
        assert_eq!(reason.as_str(), spelling);
        assert_eq!(reason.to_string(), spelling);
        assert_eq!(FinishReason::from(spelling.to_owned()), reason);
    }
}

/// Every spelling lablet reads as a known reason, with the reason it reads.
fn provider_spellings() -> [(&'static str, FinishReason); 11] {
    [
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
    ]
}

#[test]
fn every_provider_spelling_of_a_known_reason_gives_that_reason() {
    for (spelling, reason) in provider_spellings() {
        assert_eq!(
            FinishReason::from(spelling.to_owned()),
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
fn an_effort_prints_its_spelling_and_xhigh_is_one_word() {
    for (effort, spelling) in [
        (Effort::Low, "low"),
        (Effort::Medium, "medium"),
        (Effort::High, "high"),
        (Effort::XHigh, "xhigh"),
        (Effort::Max, "max"),
    ] {
        assert_eq!(effort.as_str(), spelling);
        assert_eq!(effort.to_string(), spelling);
    }
}

#[test]
fn thinking_defaults_to_the_providers_own_behaviour() {
    assert_eq!(Thinking::default(), Thinking::ProviderDefault);
}

#[test]
fn runs_share_a_cache_unless_a_run_is_given_one_of_its_own() {
    assert_eq!(CacheScope::default(), CacheScope::Shared);
}

#[test]
fn a_cache_scope_prints_its_spelling() {
    for (scope, spelling) in [(CacheScope::Shared, "shared"), (CacheScope::Run, "run")] {
        assert_eq!(scope.as_str(), spelling);
        assert_eq!(scope.to_string(), spelling);
    }
}

#[test]
fn a_provider_api_prints_its_spelling() {
    for (api, spelling) in [
        (ProviderApi::Messages, "messages"),
        (ProviderApi::Responses, "responses"),
        (ProviderApi::ChatCompletions, "chat_completions"),
        (ProviderApi::Script, "script"),
    ] {
        assert_eq!(api.as_str(), spelling);
        assert_eq!(api.to_string(), spelling);
    }
}

/// An API names its provider, so a model's record has no provider to get
/// wrong. OpenAI has two APIs, and a server that only speaks one of them is
/// reported under its family.
#[test]
fn an_api_is_its_provider_s_and_no_other_s() {
    for (api, provider) in [
        (ProviderApi::Messages, ProviderKind::Anthropic),
        (ProviderApi::Responses, ProviderKind::Openai),
        (ProviderApi::ChatCompletions, ProviderKind::Openai),
        (ProviderApi::Script, ProviderKind::Fake),
    ] {
        assert_eq!(api.provider(), provider, "{api}");
    }
}

fn bash(id: &str) -> ContentBlock {
    ContentBlock::ToolUse(ToolUse {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new("bash").unwrap(),
        input: ToolInput::Json(json!({})),
    })
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
        (ProviderErrorKind::Auth, "auth", false),
        (ProviderErrorKind::Fatal, "fatal", false),
        (ProviderErrorKind::Malformed, "malformed", true),
    ] {
        assert_eq!(kind.as_str(), spelling);
        assert_eq!(kind.to_string(), spelling);
        assert_eq!(kind.is_retryable(), retryable, "{spelling}");
    }
}

#[test]
fn a_repeated_tool_use_id_is_refused_and_the_error_names_it() {
    let repeated = ProviderResponse::new(
        vec![bash("a"), bash("b"), bash("a")],
        Usage::default(),
        FinishReason::ToolUse,
        None,
        None,
    );

    assert_eq!(
        repeated,
        Err(ResponseError::DuplicateToolUse { id: "a".to_owned() })
    );
    assert_eq!(
        repeated.unwrap_err().to_string(),
        r#"tool call id "a" is on more than one tool-use block of the response"#
    );
}

#[test]
fn a_completion_whose_tool_calls_have_distinct_ids_holds_its_content_in_order() {
    let content = vec![ContentBlock::Text("on it".to_owned()), bash("a"), bash("b")];

    let completion = ProviderResponse::new(
        content.clone(),
        usage(12, 3),
        FinishReason::ToolUse,
        Some("msg_1".to_owned()),
        Some("model-2026".to_owned()),
    )
    .unwrap();

    assert_eq!(completion.content(), content);
    assert_eq!(completion.usage, usage(12, 3));
    assert_eq!(completion.finish, FinishReason::ToolUse);
    assert_eq!(completion.response_id.as_deref(), Some("msg_1"));
    assert_eq!(completion.response_model.as_deref(), Some("model-2026"));
}

/// A provider's word for a finish reason: as often one of the spellings of a
/// known reason as any other text, which almost never spells one, so the law
/// below is held on every arm and not only on `Other`.
fn any_reason() -> impl proptest::strategy::Strategy<Value = String> {
    use proptest::strategy::Strategy;

    let spellings: Vec<&'static str> = provider_spellings()
        .iter()
        .map(|(spelling, _)| *spelling)
        .collect();
    proptest::prop_oneof![
        proptest::sample::select(spellings).prop_map(str::to_owned),
        ".{0,24}",
    ]
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

    /// Reading a provider's word for a finish reason twice says the same
    /// thing as reading it once, so a reason that round-trips through a
    /// document can't drift.
    #[test]
    fn normalising_a_finish_reason_is_idempotent(reason in any_reason()) {
        let once = FinishReason::from(reason);
        let twice = FinishReason::from(once.as_str().to_owned());

        proptest::prop_assert_eq!(&once, &twice);
    }
}
