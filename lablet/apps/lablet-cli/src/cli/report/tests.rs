use clap::Parser;
use lablet_config::{Config, Format};
use lablet_model::{FinishedRun, StopReason};
use lablet_run_request::RunRequest;
use lablet_test_support::{PROMPT, SYSTEM, Scratch};
use serde_json::json;

use super::{NOT_STARTED, exit_code, summary, summary_wanted, usage_exit_code};
use crate::cli::args::Cli;

/// A failed attempt that reported usage, a call of a tool the run doesn't
/// offer, and a response that ends the run.
const FAILS_CALLS_ENDS: &str = "
- error: { kind: retryable, message: 529 overloaded, usage: { input_tokens: 400 } }
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: ls } } }
    usage: { input_tokens: 100000, output_tokens: 12 }
    finish: tool_use
- response:
    content:
      - text: Nothing to fix.
    usage: { input_tokens: 100100, output_tokens: 20 }
    finish: end_turn
";

/// What a run of the library made of `script`, under the token budget
/// `budget`.
async fn finished(script: &str, budget: Option<u64>) -> FinishedRun {
    let scratch = Scratch::new("summary");
    let config = json!({
        "run": {
            "max_total_tokens": budget,
            "retry_backoff_base": "1ms",
            "retry_backoff_max": "1ms",
        },
        "model": {
            "provider": "fake",
            "script": scratch.write("script.yaml", script),
            "name": "scripted-1",
        },
        "prompt": { "system": SYSTEM },
        "telemetry": {
            "file": { "path": scratch.at("telemetry.otlp.jsonl") },
            "otlp": { "enabled": false },
        },
    });
    let config = Config::from_str(&config.to_string(), Format::Json).unwrap();
    let mut lablet = lablet_cli::compose::build(config).await.unwrap();
    let finished = lablet.run(RunRequest::new(PROMPT).unwrap()).await;
    lablet.shutdown().await;
    finished
}

fn lasting(mut run: FinishedRun, duration_ms: u64) -> FinishedRun {
    run.summary.outcome.duration_ms = duration_ms;
    run
}

#[test]
fn only_a_completed_run_exits_0_and_every_other_stop_reason_exits_2() {
    for reason in StopReason::ALL {
        let expected = if reason == StopReason::Completed {
            0
        } else {
            2
        };
        assert_eq!(exit_code(reason), expected, "{reason}");
    }
    assert_eq!(NOT_STARTED, 1);
}

#[test]
fn arguments_clap_refuses_exit_1_and_the_help_and_the_version_exit_0() {
    let code = |args: &[&str]| {
        let error = Cli::try_parse_from(["lablet"].iter().chain(args)).unwrap_err();
        usage_exit_code(&error)
    };
    assert_eq!(code(&["--help"]), 0);
    assert_eq!(code(&["run", "--help"]), 0);
    assert_eq!(code(&["--version"]), 0);
    assert_eq!(code(&[]), NOT_STARTED);
    assert_eq!(code(&["run"]), NOT_STARTED);
    assert_eq!(
        code(&["run", "--config", "c.yaml", "--turns", "3"]),
        NOT_STARTED
    );
}

#[test]
fn the_summary_line_is_left_out_when_quiet_and_when_the_telemetry_is_on_standard_error() {
    assert!(summary_wanted(false, false));
    assert!(!summary_wanted(true, false));
    assert!(!summary_wanted(false, true));
    assert!(!summary_wanted(true, true));
}

#[tokio::test]
async fn the_summary_names_the_stop_reason_the_turns_the_tokens_the_tool_calls_and_the_duration() {
    let run = lasting(finished(FAILS_CALLS_ENDS, None).await, 1_234);
    assert_eq!(
        summary(&run, None),
        "completed: 2 turns, 200,132 tokens, 1 tool call, 1.2s"
    );
}

#[tokio::test]
async fn a_count_of_one_is_singular() {
    let ends = "
- response:
    content:
      - text: Nothing to fix.
    usage: { input_tokens: 1 }
    finish: end_turn
";
    let run = lasting(finished(ends, None).await, 12);
    assert_eq!(
        summary(&run, None),
        "completed: 1 turn, 1 token, 0 tool calls, 12ms"
    );
}

#[tokio::test]
async fn a_run_that_reached_its_token_budget_gives_both_numbers_counting_the_failed_attempts() {
    let run = lasting(finished(FAILS_CALLS_ENDS, Some(100_000)).await, 61_500);
    assert_eq!(
        run.summary.outcome.stop_reason(),
        StopReason::MaxTotalTokens
    );
    assert_eq!(
        summary(&run, Some(100_000)),
        "max_total_tokens: 1 turn, token budget reached: 100,412 of 100,000 tokens, 1 tool call, 1m 1s"
    );
}

#[test]
fn a_duration_reads_in_the_unit_its_size_calls_for() {
    for (ms, shown) in [
        (0, "0ms"),
        (999, "999ms"),
        (1_000, "1.0s"),
        (1_099, "1.0s"),
        (1_100, "1.1s"),
        (59_999, "59.9s"),
        (60_000, "1m 0s"),
        (3_725_000, "62m 5s"),
    ] {
        assert_eq!(super::duration(ms), shown);
    }
}

#[test]
fn a_number_is_grouped_in_threes() {
    for (number, shown) in [
        (0, "0"),
        (999, "999"),
        (1_000, "1,000"),
        (100_412, "100,412"),
        (1_234_567, "1,234,567"),
        (u64::MAX, "18,446,744,073,709,551,615"),
    ] {
        assert_eq!(super::grouped(number), shown);
    }
}
