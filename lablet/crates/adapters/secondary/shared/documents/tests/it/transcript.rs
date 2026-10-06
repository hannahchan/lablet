//! The transcript JSON is what a grader or a composer reads. The fixture is
//! its one example, and the changelog gate watches it. Nothing reads a
//! transcript back, so the example is built here and compared with the
//! file, where the outcome's is read from it.

use lablet_documents::{TRANSCRIPT_SCHEMA_VERSION, TranscriptDocument};
use serde_json::{Value, json};

use crate::as_checked_in;
use crate::run::{TASK, context, model, of_two_turns, tools, without_a_turn};

// Compiled in, so the test reads the same file whatever directory it runs from.
const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../../tests/fixtures/transcript.json"
));

fn document() -> TranscriptDocument {
    TranscriptDocument::new(context(), model(), tools(), TASK.to_owned(), of_two_turns())
}

fn written(document: &TranscriptDocument) -> Value {
    serde_json::to_value(document).unwrap()
}

fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

/// O8, the document's half, and the guard against drift: a field the
/// document gains, loses or renames, at any depth, changes these bytes.
#[test]
fn the_document_of_a_run_is_the_fixture_byte_for_byte() {
    assert_eq!(as_checked_in(&document()), FIXTURE);
}

#[test]
fn the_fixture_states_the_version_this_crate_writes() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();

    assert_eq!(fixture["schema_version"], json!(TRANSCRIPT_SCHEMA_VERSION));
}

/// What a context holds beyond what names the run is the wide event's to
/// report: the path, the skills, the MCP servers and whether content is
/// captured are in no key of the document.
#[test]
fn the_document_has_the_version_what_names_the_run_its_tools_and_its_conversation() {
    assert_eq!(
        keys(&written(&document())),
        [
            "config_digest",
            "labels",
            "lablet_version",
            "model",
            "run_id",
            "schema_version",
            "started_unix_ms",
            "system",
            "task_prompt",
            "tools",
            "turns",
        ]
    );
}

#[test]
fn what_names_the_run_is_what_its_context_states() {
    let context = context();

    let document = written(&document());

    assert_eq!(document["run_id"], json!(context.run_id.as_str()));
    assert_eq!(
        document["labels"],
        json!({ "task": "fix-failing-test", "experiment": null, "trial": "3" })
    );
    assert_eq!(
        document["config_digest"],
        json!(context.config_digest.as_str())
    );
    assert_eq!(document["lablet_version"], json!(context.agent_version));
    assert_eq!(document["started_unix_ms"], json!(1_790_000_000_123_u64));
    assert_eq!(
        document["model"],
        json!({
            "provider": "anthropic",
            "api": "messages",
            "name": "model-2026",
            "replays_reasoning": true,
        })
    );
}

#[test]
fn the_tools_are_the_specs_the_run_offered_in_the_order_it_offered_them() {
    let document = written(&document());

    let names: Vec<&Value> = document["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|spec| &spec["name"])
        .collect();

    assert_eq!(names, [&json!("bash"), &json!("search")]);
    assert_eq!(
        document["tools"][1],
        json!({
            "name": "search",
            "description": "Searches the docs.",
            "input_schema": { "type": "object" },
            "source": { "mcp": { "server": "docs" } },
            "concurrency": "shared",
        })
    );
}

#[test]
fn every_turn_has_its_input_its_response_its_record_and_its_tool_call_outcomes() {
    let document = written(&document());
    let turns = document["turns"].as_array().unwrap();

    assert_eq!(turns.len(), 2);
    for turn in turns {
        assert_eq!(keys(turn), ["input", "record", "response", "tool_calls"]);
    }
    assert_eq!(
        turns[0]["input"],
        json!([{ "text": "Fix the failing test." }]),
        "the first turn's input is the prompt"
    );
    assert_eq!(turns[1]["input"], json!([]));
    assert_eq!(turns[0]["response"].as_array().unwrap().len(), 4);
    assert_eq!(turns[1]["response"], json!([{ "text": "Fixed." }]));
    assert_eq!(turns[0]["record"]["attempts"], json!(2));
    assert_eq!(turns[1]["record"]["finish"], json!("end_turn"));
    assert_eq!(turns[1]["tool_calls"], json!([]));
}

#[test]
fn a_turn_s_outcomes_answer_its_calls_in_call_order() {
    let document = written(&document());
    let turn = &document["turns"][0];

    let called: Vec<&Value> = turn["response"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|block| block.get("tool_use"))
        .map(|call| &call["id"])
        .collect();
    let answered: Vec<&Value> = turn["tool_calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| &outcome["call_id"])
        .collect();

    assert_eq!(called, [&json!("call_a"), &json!("call_b")]);
    assert_eq!(answered, called);
    assert_eq!(
        turn["tool_calls"][0],
        json!({
            "call_id": "call_a",
            "status": { "ran": { "source": "builtin", "ended": "tool_error" } },
            "started_ms": 1_200,
            "latency_ms": 30,
            "truncated_from_bytes": 39,
            "content": [
                { "text": "test result: FAILED. 1 p" },
                { "text": "[truncated: the first 24 of 39 bytes]" },
            ],
        })
    );
    assert_eq!(turn["tool_calls"][1]["status"], json!("malformed_input"));
}

/// A run that received no response has no turns, so its transcript holds
/// the system prompt and the task prompt, under everything that names the
/// run: it still says what was asked.
#[test]
fn a_run_without_a_turn_publishes_its_prompts_and_no_turns() {
    let document = written(&TranscriptDocument::new(
        context(),
        model(),
        tools(),
        "Say what was asked.".to_owned(),
        without_a_turn(),
    ));

    assert_eq!(document["system"], json!("You fix tests."));
    assert_eq!(document["task_prompt"], json!("Say what was asked."));
    assert_eq!(document["turns"], json!([]));
    assert_eq!(document["run_id"], json!("01K5F3Z8Q4X9T2M7B6W1R0VNEC"));
    assert_eq!(document["schema_version"], json!(1));
}

#[test]
fn the_version_is_written_first_so_a_reader_knows_the_form_before_the_content() {
    let written = serde_json::to_string(&document()).unwrap();

    assert!(
        written.starts_with(r#"{"schema_version":1,"run_id":"#),
        "{written}"
    );
}
