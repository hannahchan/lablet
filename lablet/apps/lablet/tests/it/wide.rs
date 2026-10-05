//! O1's sums and O4's scan through `build` and `run`: the wide event
//! against the spans beside it in the file the run wrote, and what a run
//! that captures no content leaves out of that file.
//!
//! The runs are on the runtime's own clock, since `bash` starts real
//! processes, and a paused clock would move on to a call's deadline while
//! one runs. The sums are exact all the same: every span is stamped at the
//! offsets the loop measured, so a span lasts the latency the wide event
//! adds up, and a loaded runner changes both alike.

use lablet::{FinishedRun, RunId, StopReason};
use lablet_conformance::otlp::{Exported, Span};
use serde_json::json;

use crate::harness::{Lab, PROMPT, SYSTEM, Traced, request};
use crate::key;
use crate::wide_checks::{
    assert_the_wide_event_counts_the_tokens_the_run_returned, assert_the_wide_event_is_declared,
    assert_the_wide_event_sums_its_steps,
};

const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";

/// A run that does something of everything a run's totals count, on the
/// built-in tools: attempts that fail, with and without usage to report, a
/// call whose output is cut, one that returns an error, one whose arguments
/// didn't parse, and one to a tool the run doesn't have.
const EVERYTHING: &str = r#"
- error:
    kind: retryable
    message: 529 overloaded
    usage: { input_tokens: 800, output_tokens: 3, cache_read_tokens: 600, cache_write_tokens: 50 }
    latency: 4ms
- response:
    content:
      - text: I'll note the fix, list the table and read the lexer.
      - tool_use: { id: call_1, name: write_file, input: { json: { path: notes/fix.md, content: Off by one in the lexer. } } }
      - tool_use: { id: call_2, name: bash, input: { json: { command: "sleep 0.02; for n in 1 2 3 4 5 6; do echo token-row-$n | tr a-z A-Z; done" } } }
      - tool_use: { id: call_3, name: read_file, input: { json: { path: src/lexer.rs } } }
    usage: { input_tokens: 1000, output_tokens: 50, cache_read_tokens: 200, cache_write_tokens: 90 }
    finish: tool_use
    latency: 6ms
- response:
    content:
      - tool_use: { id: call_4, name: no_such_tool, input: { json: {} } }
      - tool_use: { id: call_5, name: bash, input: { unparsed: '{"command": "cargo-lexer-check' } }
      - tool_use: { id: call_6, name: bash, input: { json: { command: "sleep 0.02; cat notes/fix.md" } } }
    usage: { input_tokens: 1100, output_tokens: 20, reasoning_output_tokens: 5 }
    finish: tool_use
    latency: 3ms
- error: { kind: retryable, message: 529 overloaded, latency: 2ms }
- error: { kind: retryable, message: 529 overloaded, usage: { input_tokens: 10 }, latency: 2ms }
- response:
    content:
      - text: The lexer was off by one.
    usage: { input_tokens: 1200, output_tokens: 30 }
    finish: end_turn
    latency: 5ms
"#;

/// What the run's prompts, its responses, its calls' arguments and its
/// tools' output hold, none of which is anywhere else in what it exports.
const SAID: [&str; 10] = [
    SYSTEM,
    PROMPT,
    "I'll note the fix, list the table and read the lexer.",
    "The lexer was off by one.",
    "Off by one in the lexer.",
    "token-row-",
    "TOKEN-ROW-1",
    "notes/fix.md",
    "src/lexer.rs",
    "cargo-lexer-check",
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

/// One run of the script, and its file as it was when `run` returned.
struct Wrote {
    finished: FinishedRun,
    exported: Exported,
    written: String,
}

async fn everything(test: &str, capture_content: bool) -> Wrote {
    let scratch = Lab::new(test);
    let config = scratch.config(
        EVERYTHING,
        json!({
            "run": { "retry_backoff_base": "1ms", "retry_backoff_max": "1ms" },
            "tools": {
                "builtin": scratch.builtin(&["bash", "read_file", "write_file"]),
                "max_output_bytes": 64,
                "output_cut": "head",
            },
            "telemetry": { "capture_content": capture_content },
        }),
    );
    let mut lablet = lablet::build(config).await.unwrap();

    let finished = lablet
        .run(request().run_id(RunId::new(RUN).unwrap()).unwrap())
        .await;
    let exported = scratch.exported();
    let written = std::fs::read_to_string(scratch.telemetry()).unwrap();
    lablet.shutdown().await;

    Wrote {
        finished,
        exported,
        written,
    }
}

// O1

#[tokio::test]
async fn the_numbers_of_the_wide_event_are_the_sums_of_the_spans_the_run_wrote_beside_it() {
    let wrote = everything("wide-sums", false).await;

    let summary = &wrote.finished.summary;
    assert_eq!(summary.outcome.stop_reason(), StopReason::Completed);
    // The error that answers arguments which didn't parse is longer than
    // the cap, so it's cut as the output of `bash` is.
    assert_eq!(
        (
            summary.outcome.turns,
            summary.provider.retries,
            summary.outcome.tool_calls,
            summary.tool_calls.errors,
            summary.tool_calls.unknown,
            summary.tool_calls.truncated,
        ),
        (3, 3, 6, 3, 1, 2),
        "the script did what the sums are to be checked on"
    );
    let traced = Traced::of(&wrote.exported, RUN);
    assert_the_wide_event_is_declared(traced.wide());
    assert_the_wide_event_sums_its_steps(&wrote.exported, RUN);
    assert_the_wide_event_counts_the_tokens_the_run_returned(&wrote.exported, RUN, summary);

    // Exactly, and not only within the case's 5 ms: the spans are stamped
    // at the loop's own offsets.
    let wide = &traced.wide().attributes;
    let lasted = |spans: Vec<&Span>| -> u64 { spans.iter().map(|span| span.duration_ms()).sum() };
    for (key, spans_lasted) in [
        (
            key::LABLET_PROVIDER_LATENCY_MS_TOTAL,
            lasted(traced.chats()),
        ),
        (
            key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL,
            lasted(traced.tools()),
        ),
        (key::LABLET_RUN_DURATION_MS, traced.root().duration_ms()),
    ] {
        assert_eq!(wide[key], json!(spans_lasted), "{key}");
    }
}

// O4

#[tokio::test]
async fn a_run_that_captures_no_content_writes_none_to_its_file() {
    let left_out = everything("o4-off", false).await;
    let kept = everything("o4-on", true).await;

    for said in SAID {
        assert!(
            kept.written.contains(said),
            "{said:?} isn't in what a run that captures content wrote, so its absence says nothing"
        );
        assert!(
            !left_out.written.contains(said),
            "{said:?} is in what the run wrote"
        );
    }
    let exported = &left_out.exported;
    assert!(
        exported
            .records_of("gen_ai.client.inference.operation.details")
            .is_empty()
    );
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
}
