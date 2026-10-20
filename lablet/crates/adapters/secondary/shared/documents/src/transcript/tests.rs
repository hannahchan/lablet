use lablet_model::{self as model, FinishReason, TokenCounts};
use serde_json::json;

use super::*;

fn reached_through(api: model::ProviderApi) -> ModelRef {
    ModelRef {
        api,
        name: "model-2026".to_owned(),
        replays_reasoning: true,
    }
}

#[test]
fn a_model_is_written_with_its_provider_before_how_it_was_reached() {
    assert_eq!(
        serde_json::to_string(&Model::from(reached_through(
            model::ProviderApi::ChatCompletions
        )))
        .unwrap(),
        concat!(
            r#"{"provider":"openai","api":"chat_completions","name":"model-2026","#,
            r#""replays_reasoning":true}"#
        )
    );
}

/// The provider a document names is the one the API decides, under the
/// spellings the domain has for both.
#[test]
fn every_api_is_written_as_the_domain_spells_it_beside_the_provider_whose_api_it_is() {
    for api in model::ProviderApi::ALL {
        let written = serde_json::to_value(Model::from(reached_through(api))).unwrap();

        assert_eq!(written["api"], json!(api.as_str()));
        assert_eq!(written["provider"], json!(api.provider().as_str()));
    }
}

fn record(finish: FinishReason) -> model::TurnRecord {
    model::TurnRecord {
        usage: model::Usage::from_inclusive(TokenCounts {
            input: 100,
            output: 20,
            reasoning: None,
            cache_read: Some(80),
            cache_write: None,
        }),
        finish,
        response_id: Some("msg_1".to_owned()),
        response_model: None,
        started_ms: 1_000,
        latency_ms: 800,
        attempts: 2,
    }
}

#[test]
fn a_turn_s_record_is_written_as_these_exact_bytes() {
    assert_eq!(
        serde_json::to_string(&TurnRecord::from(record(FinishReason::ToolUse))).unwrap(),
        concat!(
            r#"{"usage":{"input_tokens":100,"output_tokens":20,"reasoning_output_tokens":null,"#,
            r#""cache_read_tokens":80,"cache_write_tokens":null},"#,
            r#""finish":"tool_use","response_id":"msg_1","response_model":null,"#,
            r#""started_ms":1000,"latency_ms":800,"attempts":2}"#
        )
    );
}

/// A reason is written as the word that reads back as the same reason:
/// lablet's name for one it knows, and the provider's own string for one it
/// doesn't, which the adapter kept as the provider spelled it.
#[test]
fn a_finish_reason_is_written_as_lablet_s_name_for_it_or_the_provider_s_own_string() {
    let kept = FinishReason::from("pause_turn".to_owned());
    for (reason, written) in FinishReason::KNOWN
        .iter()
        .map(|reason| (reason.clone(), reason.as_str()))
        .chain([(kept, "pause_turn")])
    {
        let record = TurnRecord::from(record(reason.clone()));

        assert_eq!(
            serde_json::to_value(record).unwrap()["finish"],
            json!(written),
            "{reason:?}"
        );
    }
}

/// The constant is what a reader tells forms apart by, so a change to it is
/// a change to the contract and this test is where that is noticed.
#[test]
fn the_published_version_is_one() {
    assert_eq!(TRANSCRIPT_SCHEMA_VERSION, 1);
}
