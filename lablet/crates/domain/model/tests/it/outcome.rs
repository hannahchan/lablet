//! The outcome JSON is a public contract. The fixture is the contract's one
//! example, shared by every crate that prints or reads an outcome, and the
//! changelog gate watches it.

use lablet_model::{RunId, RunOutcome, RunResult, StopReason, Usage};
use serde_json::{Value, json};

// Compiled in, so the test reads the same file whatever directory it runs from.
const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../tests/fixtures/outcome.json"
));

fn outcome() -> RunOutcome {
    RunOutcome {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        stop_reason: StopReason::Completed,
        turns: 7,
        usage: Usage {
            input_tokens: 48_211,
            output_tokens: 1_840,
            cache_read_tokens: 39_104,
            cache_write_tokens: 6_144,
        },
        tool_calls: 5,
        duration_ms: 12_345,
        result: RunResult {
            text: "The failing test is fixed.".to_owned(),
            structured: Some(json!({ "passed": true, "files_changed": ["src/lib.rs"] })),
        },
        error: None,
    }
}

#[test]
fn the_fixture_deserialises_to_the_outcome_built_in_code() {
    assert_eq!(
        serde_json::from_str::<RunOutcome>(FIXTURE).unwrap(),
        outcome()
    );
}

#[test]
fn the_outcome_serialises_back_to_the_fixture() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();

    assert_eq!(serde_json::to_value(outcome()).unwrap(), fixture);
}

#[test]
fn the_fixture_has_exactly_the_keys_the_spec_lists() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let keys = |value: &Value| -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };

    assert_eq!(
        keys(&fixture),
        [
            "duration_ms",
            "error",
            "result",
            "run_id",
            "stop_reason",
            "tool_calls",
            "turns",
            "usage"
        ]
    );
    assert_eq!(
        keys(&fixture["usage"]),
        [
            "cache_read_tokens",
            "cache_write_tokens",
            "input_tokens",
            "output_tokens"
        ]
    );
    assert_eq!(keys(&fixture["result"]), ["structured", "text"]);
    assert!(fixture["duration_ms"].is_u64());
    assert!(fixture["error"].is_null());
}

#[test]
fn the_fixtures_cached_tokens_are_a_subset_of_its_input_tokens() {
    let usage = outcome().usage;

    assert!(usage.cache_read_tokens + usage.cache_write_tokens <= usage.input_tokens);
    assert_eq!(usage.total(), 48_211 + 1_840);
}
