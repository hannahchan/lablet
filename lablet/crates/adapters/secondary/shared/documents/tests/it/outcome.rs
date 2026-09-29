//! The outcome JSON is a public contract. The fixture is the contract's one
//! example, and the changelog gate watches it.

use lablet_documents::{OUTCOME_SCHEMA_VERSION, OutcomeDocument};
use lablet_model::{
    OutcomeParts, RunId, RunLabels, RunOutcome, StopReason, TaskResult, TokenCounts, Usage,
};
use serde_json::{Value, json};

use crate::as_checked_in;

// Compiled in, so the test reads the same file whatever directory it runs from.
const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../../tests/fixtures/outcome.json"
));

/// The outcome the fixture describes, built the way the loop closes a run.
fn outcome() -> RunOutcome {
    RunOutcome::closing(OutcomeParts {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        labels: RunLabels {
            task: Some("fix-failing-test".to_owned()),
            experiment: None,
            trial: Some("3".to_owned()),
        },
        stop_reason: StopReason::Completed,
        turns: 7,
        usage: Usage::from_inclusive(TokenCounts {
            input: 48_211,
            output: 1_840,
            reasoning: Some(1_216),
            cache_read: Some(39_104),
            cache_write: None,
        }),
        tool_calls: 5,
        duration_ms: 12_345,
        result: TaskResult {
            text: "The failing test is fixed.".to_owned(),
            structured: Some(json!({ "files_changed": ["src/lib.rs"], "passed": true })),
        },
        error: None,
    })
}

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

#[test]
fn the_fixture_reads_as_the_outcome_it_describes() {
    let document = serde_json::from_str::<OutcomeDocument>(FIXTURE).unwrap();

    assert_eq!(RunOutcome::try_from(document), Ok(outcome()));
}

#[test]
fn the_outcome_is_written_back_as_the_fixture_byte_for_byte() {
    assert_eq!(as_checked_in(&OutcomeDocument::from(outcome())), FIXTURE);
}

#[test]
fn the_fixture_has_exactly_the_keys_the_spec_lists() {
    let fixture = fixture();
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
            "labels",
            "result",
            "run_id",
            "schema_version",
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
            "output_tokens",
            "reasoning_output_tokens"
        ]
    );
    assert_eq!(keys(&fixture["labels"]), ["experiment", "task", "trial"]);
    assert_eq!(keys(&fixture["result"]), ["structured", "text"]);
    assert!(fixture["duration_ms"].is_u64());
    assert!(fixture["error"].is_null());
}

#[test]
fn the_fixture_states_the_version_this_crate_writes() {
    assert_eq!(fixture()["schema_version"], json!(OUTCOME_SCHEMA_VERSION));
}

/// A count the provider didn't report and a label the request didn't name
/// are in the document as `null`, so a reader finds every key in every
/// outcome and never takes a gap for a zero.
#[test]
fn the_fixture_writes_what_is_missing_as_null() {
    let fixture = fixture();

    assert!(fixture["usage"]["cache_write_tokens"].is_null());
    assert!(fixture["labels"]["experiment"].is_null());
    assert_eq!(outcome().usage.cache_write_tokens, None);
    assert_eq!(outcome().labels.experiment, None);
}

#[test]
fn the_fixtures_cached_tokens_are_a_subset_of_its_input_tokens() {
    let usage = outcome().usage;

    assert_eq!(usage.uncached_input_tokens(), 48_211 - 39_104);
    assert_eq!(usage.total(), 48_211 + 1_840);
}
