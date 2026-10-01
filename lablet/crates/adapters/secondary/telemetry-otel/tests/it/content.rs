//! O4, and the clauses of O10 that are of the flush and of the limit on a
//! value's length.

use lablet_telemetry_otel::ATTRIBUTE_MAX_BYTES;
use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::signals::{
    EVENT_GEN_AI_CLIENT_INFERENCE_OPERATION_DETAILS_KEYS as CONTENT_KEYS,
    EVENT_GEN_AI_CLIENT_INFERENCE_OPERATION_DETAILS_REQUIRED as CONTENT_REQUIRED,
};
use serde_json::{Value, json};
use std::fmt::Write as _;

use crate::harness::{
    BASH_SAYS, FAILS_CALLS_ENDS, PROMPT, READ_FILE_SAYS, SYSTEM, Traced, assert_declared, count,
    traced,
};

const CONTENT: &str = "gen_ai.client.inference.operation.details";

/// What the run's prompts, its responses, its calls' arguments and its
/// tools' output hold, none of which is anywhere else in what a run
/// exports.
const SAID: [&str; 8] = [
    SYSTEM,
    PROMPT,
    "I'll run the tests and read the parser.",
    "The parser test passes now.",
    "cargo test --quiet",
    "src/parser.rs",
    BASH_SAYS,
    READ_FILE_SAYS,
];

/// The content keys of the registry.
const HOLD_CONTENT: [&str; 6] = [
    key::GEN_AI_SYSTEM_INSTRUCTIONS,
    key::GEN_AI_INPUT_MESSAGES,
    key::GEN_AI_OUTPUT_MESSAGES,
    key::GEN_AI_TOOL_DEFINITIONS,
    key::GEN_AI_TOOL_CALL_ARGUMENTS,
    key::GEN_AI_TOOL_CALL_RESULT,
];

fn parsed(value: &Value) -> Value {
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}

// O4

#[tokio::test(start_paused = true)]
async fn a_run_that_captures_no_content_exports_none() {
    let traced = traced("o4-off", FAILS_CALLS_ENDS, |_| {}).await;

    for said in SAID {
        assert!(
            !traced.written.contains(said),
            "{said:?} is in what the run exported"
        );
    }
    assert!(traced.exported.records_of(CONTENT).is_empty());
    let events: Vec<_> = traced
        .exported
        .records
        .iter()
        .map(|record| record.event_name.as_str())
        .collect();
    assert_eq!(events, ["gen_ai.client.operation.exception", "lablet.run"]);
    let exported = &traced.exported;
    let attributes = exported
        .spans
        .iter()
        .map(|span| &span.attributes)
        .chain(exported.records.iter().map(|record| &record.attributes));
    for attributes in attributes {
        for key in HOLD_CONTENT {
            assert_eq!(attributes.get(key), None, "{key}");
        }
    }
    assert_eq!(
        count(&traced.tools()[0].attributes, key::LABLET_TOOL_OUTPUT_BYTES),
        BASH_SAYS.len() as u64,
        "the size of what wasn't captured is there all the same"
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_that_captures_content_exports_it_in_log_records_and_on_no_span() {
    let traced = traced("o4-on", FAILS_CALLS_ENDS, |settings| {
        settings.capture_content = true;
    })
    .await;

    for said in SAID {
        assert!(
            traced.written.contains(said),
            "{said:?} isn't in what the run exported"
        );
    }
    for span in &traced.exported.spans {
        for key in HOLD_CONTENT {
            assert_eq!(span.attributes.get(key), None, "{key} of {}", span.name);
        }
        let written = serde_json::to_string(&span.attributes).unwrap();
        for said in SAID {
            assert!(!written.contains(said), "{said:?} is on {}", span.name);
        }
    }
    let records = traced.exported.records_of(CONTENT);
    assert_eq!(records.len(), 1 + 4 + 3);
    for record in records {
        assert_declared(
            "a record of content",
            &record.attributes,
            CONTENT_REQUIRED,
            CONTENT_KEYS,
        );
    }
}

/// The span `record` is in the context of.
fn span_of<'a>(
    traced: &'a Traced,
    record: &lablet_conformance::otlp::LogRecord,
) -> &'a lablet_conformance::otlp::Span {
    traced
        .exported
        .spans
        .iter()
        .find(|span| span.span_id == record.span_id)
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn each_record_of_content_is_in_the_context_of_the_span_the_content_belongs_to() {
    let traced = traced("content-context", FAILS_CALLS_ENDS, |settings| {
        settings.capture_content = true;
    })
    .await;

    let belongs: Vec<_> = traced
        .exported
        .records_of(CONTENT)
        .into_iter()
        .map(|record| {
            let span = span_of(&traced, record);
            assert_eq!(record.trace_id, span.trace_id);
            assert_eq!(
                record.attributes[key::GEN_AI_OPERATION_NAME],
                span.attributes[key::GEN_AI_OPERATION_NAME]
            );
            assert_eq!(
                record.attributes.get(key::LABLET_TURN),
                span.attributes.get(key::LABLET_TURN)
            );
            let held: Vec<_> = HOLD_CONTENT
                .into_iter()
                .filter(|key| record.attributes.contains_key(*key))
                .collect();
            (span.name.as_str(), held)
        })
        .collect();

    let chat = "chat scripted-1";
    let sent = vec![key::GEN_AI_SYSTEM_INSTRUCTIONS, key::GEN_AI_INPUT_MESSAGES];
    let exchanged = vec![
        key::GEN_AI_SYSTEM_INSTRUCTIONS,
        key::GEN_AI_INPUT_MESSAGES,
        key::GEN_AI_OUTPUT_MESSAGES,
    ];
    let called = vec![
        key::GEN_AI_TOOL_CALL_ARGUMENTS,
        key::GEN_AI_TOOL_CALL_RESULT,
    ];
    assert_eq!(
        belongs,
        [
            ("invoke_agent lablet", vec![key::GEN_AI_TOOL_DEFINITIONS]),
            (chat, sent),
            (chat, exchanged.clone()),
            ("execute_tool bash", called.clone()),
            ("execute_tool read_file", called.clone()),
            (chat, exchanged.clone()),
            ("execute_tool no_such_tool", called),
            (chat, exchanged),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn the_records_of_a_run_hold_the_conversation_as_the_model_was_sent_it() {
    let traced = traced("conversation", FAILS_CALLS_ENDS, |settings| {
        settings.capture_content = true;
    })
    .await;

    let records = traced.exported.records_of(CONTENT);
    let offered = parsed(&records[0].attributes[key::GEN_AI_TOOL_DEFINITIONS]);
    assert_eq!(
        offered,
        json!([
            { "type": "function", "name": "bash", "description": "Runs a command.", "parameters": { "type": "object" } },
            { "type": "function", "name": "read_file", "description": "Reads a file.", "parameters": { "type": "object" } },
            { "type": "function", "name": "write", "description": "Writes bytes.", "parameters": { "type": "object" } },
        ])
    );

    let last = records.last().unwrap();
    assert_eq!(
        parsed(&last.attributes[key::GEN_AI_SYSTEM_INSTRUCTIONS]),
        json!([{ "type": "text", "content": SYSTEM }])
    );
    let text = |text: &str| json!({ "type": "text", "content": text });
    let result = |text: &str, failed: bool| json!({ "content": [{ "type": "text", "text": text }], "isError": failed });
    assert_eq!(
        parsed(&last.attributes[key::GEN_AI_INPUT_MESSAGES]),
        json!([
            { "role": "user", "parts": [text(PROMPT)] },
            { "role": "assistant", "parts": [
                text("I'll run the tests and read the parser."),
                { "type": "tool_call", "id": "call_1", "name": "bash", "arguments": { "command": "cargo test --quiet" } },
                { "type": "tool_call", "id": "call_2", "name": "read_file", "arguments": { "path": "src/parser.rs" } },
            ] },
            { "role": "tool", "parts": [
                { "type": "tool_call_response", "id": "call_1", "response": result(BASH_SAYS, false) },
                { "type": "tool_call_response", "id": "call_2", "response": result(READ_FILE_SAYS, false) },
            ] },
            { "role": "assistant", "parts": [
                { "type": "tool_call", "id": "call_3", "name": "no_such_tool", "arguments": {} },
            ] },
            { "role": "tool", "parts": [
                { "type": "tool_call_response", "id": "call_3", "response": result("no tool named no_such_tool is offered by this run", true) },
            ] },
        ])
    );
    assert_eq!(
        parsed(&last.attributes[key::GEN_AI_OUTPUT_MESSAGES]),
        json!([{ "role": "assistant", "parts": [text("The parser test passes now.")] }])
    );

    let of_bash = records[3];
    assert_eq!(of_bash.attributes[key::GEN_AI_TOOL_NAME], "bash");
    assert_eq!(of_bash.attributes[key::GEN_AI_TOOL_CALL_ID], "call_1");
    assert_eq!(
        parsed(&of_bash.attributes[key::GEN_AI_TOOL_CALL_ARGUMENTS]),
        json!({ "command": "cargo test --quiet" })
    );
    assert_eq!(
        parsed(&of_bash.attributes[key::GEN_AI_TOOL_CALL_RESULT]),
        result(BASH_SAYS, false)
    );
}

// O10

/// A script of `turns` turns that each call `write` for a few bytes and a
/// last one that ends the run; the call of the turn before the last asks
/// for `long` bytes.
pub fn writing(turns: usize, long: usize) -> String {
    let mut script = String::new();
    for turn in 1..=turns {
        let bytes = if turn == turns { long } else { 4 };
        write!(
            script,
            "
- response:
    content:
      - tool_use: {{ id: call_{turn}, name: write, input: {{ json: {{ bytes: {bytes} }} }} }}
    usage: {{ input_tokens: 10, output_tokens: 5 }}
    finish: tool_use
    latency: 10ms
"
        )
        .unwrap();
    }
    script.push_str(
        "
- response:
    content: [{ text: Done. }]
    usage: { input_tokens: 10, output_tokens: 5 }
    finish: end_turn
",
    );
    script
}

#[tokio::test(start_paused = true)]
async fn the_wide_event_is_the_last_line_of_a_run_whose_content_outgrew_a_batch() {
    let turns = 300;
    let long = ATTRIBUTE_MAX_BYTES + 4_096;
    let traced = traced("o10", &writing(turns, long), |settings| {
        settings.capture_content = true;
    })
    .await;

    let exported = &traced.exported;
    let content = exported.records_of(CONTENT);
    assert_eq!(content.len(), 1 + (turns + 1) + turns);
    let lines_of_content: std::collections::BTreeSet<_> =
        content.iter().map(|record| record.line).collect();
    assert!(
        lines_of_content.len() > 1,
        "the records are more than one export holds: {lines_of_content:?}"
    );

    let wide = traced.wide();
    assert_eq!(wide.line, exported.lines, "the wide event is the last line");
    let of_the_last_line: Vec<_> = exported
        .records
        .iter()
        .filter(|record| record.line == wide.line)
        .collect();
    assert_eq!(of_the_last_line.len(), 1, "and it's the line's only record");
    assert!(exported.spans.iter().all(|span| span.line < wide.line));
    assert_eq!(
        wide.attributes[key::LABLET_TELEMETRY_DROPPED_RECORDS],
        json!(0)
    );
    assert_eq!(
        exported.spans.len(),
        1 + (turns + 1) + turns,
        "nothing of the run is missing"
    );

    let lengths = |key: &str| -> Vec<usize> {
        content
            .iter()
            .filter_map(|record| record.attributes.get(key))
            .map(|value| value.as_str().unwrap().len())
            .collect()
    };
    let results = lengths(key::GEN_AI_TOOL_CALL_RESULT);
    assert_eq!(results.iter().max(), Some(&ATTRIBUTE_MAX_BYTES));
    assert_eq!(
        results
            .iter()
            .filter(|length| **length == ATTRIBUTE_MAX_BYTES)
            .count(),
        1,
        "the one result that was longer is cut, and no other"
    );
    let sent = lengths(key::GEN_AI_INPUT_MESSAGES);
    assert_eq!(
        sent.last(),
        Some(&ATTRIBUTE_MAX_BYTES),
        "the call after it was sent it, and the record of that is cut too"
    );
    assert!(
        sent[..sent.len() - 1]
            .iter()
            .all(|length| *length < ATTRIBUTE_MAX_BYTES)
    );
    assert_eq!(
        count(
            &traced.tools()[turns - 1].attributes,
            key::LABLET_TOOL_OUTPUT_BYTES
        ),
        long as u64,
        "the span says how long the output was, whatever its record holds of it"
    );
}
