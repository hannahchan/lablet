use std::time::Duration;

use lablet_model::{StopReason, ToolCallStatus, Usage};

use crate::harness::{Harness, PROVIDER_TIMEOUT};

const FIRST_RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
const SECOND_RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";

/// A failed attempt, a response that calls a tool, and a response that
/// ends the run, each with what it used and how long it took.
const FAILS_CALLS_ENDS: &str = r"
- error:
    kind: retryable
    message: 529 overloaded
    usage: { input_tokens: 800 }
    retry_after: 2s
    latency: 40ms
- response:
    content:
      - text: I'll run the tests.
      - tool_use: { id: call_1, name: bash, input: { json: { command: cargo test } } }
    usage: { input_tokens: 1000, output_tokens: 50, cache_read_tokens: 0 }
    finish: tool_use
    latency: 250ms
- response:
    content:
      - text: This run offers no tool, so the test stays as it is.
    usage: { input_tokens: 1100, output_tokens: 20 }
    finish: end_turn
    latency: 120ms
";

#[tokio::test(start_paused = true)]
async fn a_script_is_played_through_a_run_and_the_run_counts_what_it_stated() {
    let mut harness = Harness::playing(FAILS_CALLS_ENDS, 3).await;

    let finished = harness.run(FIRST_RUN).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.error(), None);
    assert_eq!(outcome.turns, 2);
    assert_eq!(
        outcome.result().text,
        "This run offers no tool, so the test stays as it is."
    );
    assert_eq!(
        outcome.usage,
        Usage {
            input_tokens: 2_100,
            output_tokens: 70,
            reasoning_output_tokens: None,
            cache_read_tokens: Some(0),
            cache_write_tokens: None,
        },
        "a count is reported when a response of the script stated it, and only then"
    );
    assert_eq!(
        finished.summary.failed_usage,
        Some(Usage {
            input_tokens: 800,
            ..Usage::default()
        })
    );
    assert_eq!(finished.summary.provider.retries, 1);
    assert_eq!(harness.recorder.attempts(), 3);
    assert_eq!(
        harness.recorder.failures(),
        [(
            "529 overloaded".to_owned(),
            40,
            Some(Duration::from_secs(2))
        )],
        "the loop waited as long as the script's server asked"
    );

    let turns = finished.transcript.turns();
    assert_eq!(turns[0].record().attempts, 2);
    assert_eq!(turns[0].record().latency_ms, 250);
    assert_eq!(turns[0].record().started_ms, 2_040);
    assert_eq!(turns[0].tool_calls()[0].status, ToolCallStatus::Unknown);
    assert_eq!(turns[1].record().attempts, 1);
    assert_eq!(turns[1].record().latency_ms, 120);
    assert_eq!(finished.summary.provider.latency.total_ms(), 410);
}

#[tokio::test(start_paused = true)]
async fn a_run_that_outlasts_its_script_ends_as_a_provider_error_that_says_what_to_do() {
    let mut harness = Harness::playing(
        r"
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: ls } } }
    usage: { input_tokens: 1000, output_tokens: 50 }
    finish: tool_use
",
        3,
    )
    .await;

    let finished = harness.run(FIRST_RUN).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::ProviderError);
    assert_eq!(
        outcome.error(),
        Some(
            "script \"scripts/run.yaml\" ran out: entry 1 was its last, and the run made \
             another attempt of a provider call. Add an entry to answer it, or have the last \
             response end the run."
        )
    );
    assert_eq!(outcome.turns, 1);
    assert_eq!(
        harness.recorder.attempts(),
        2,
        "the attempt the script couldn't answer is the run's last, though it had retries left"
    );
    assert_eq!(finished.summary.provider.retries, 0);
    assert_eq!(
        harness.recorder.failures(),
        [(outcome.error().unwrap().to_owned(), 0, None)]
    );
}

#[tokio::test(start_paused = true)]
async fn an_attempt_the_provider_timeout_cuts_short_has_stopped_and_the_next_entry_answers_the_retry()
 {
    let mut harness = Harness::playing(
        r"
- response: { content: [{ text: too late }], finish: end_turn, latency: 5s }
- response: { content: [{ text: in time }], finish: end_turn, latency: 300ms }
",
        3,
    )
    .await;

    let finished = harness.run(FIRST_RUN).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.result().text, "in time");
    assert_eq!(
        harness.recorder.failures(),
        [(
            "entry 1 of script \"scripts/run.yaml\" answers after 5s, and the attempt's \
             deadline was 1s"
                .to_owned(),
            u64::try_from(PROVIDER_TIMEOUT.as_millis()).unwrap(),
            Some(Duration::from_millis(100))
        )],
        "the attempt took its deadline and no longer"
    );
    let record = finished.transcript.turns()[0].record();
    assert_eq!(record.attempts, 2);
    assert_eq!(record.started_ms, 1_100);
    assert_eq!(record.latency_ms, 300);
}

#[tokio::test(start_paused = true)]
async fn a_failure_no_attempt_could_answer_ends_the_run_with_the_scripts_own_message() {
    let mut harness = Harness::playing(
        r"
- error: { kind: auth, message: the key was rejected }
- response: { content: [{ text: never played }], finish: end_turn }
",
        3,
    )
    .await;

    let finished = harness.run(FIRST_RUN).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::ProviderError);
    assert_eq!(outcome.error(), Some("the key was rejected"));
    assert_eq!(harness.recorder.attempts(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_script_rewound_between_two_runs_gives_both_the_same_run() {
    let mut harness = Harness::playing(FAILS_CALLS_ENDS, 3).await;

    let first = harness.run(FIRST_RUN).await;
    harness.provider.rewind();
    let second = harness.run(SECOND_RUN).await;

    let (first, second) = (first.summary.outcome, second.summary.outcome);
    assert_eq!(first.stop_reason(), StopReason::Completed);
    assert_eq!(second.stop_reason(), first.stop_reason());
    assert_eq!(second.turns, first.turns);
    assert_eq!(second.usage, first.usage);
    assert_eq!(second.tool_calls, first.tool_calls);
    assert_eq!(second.result(), first.result());
    assert_eq!(second.duration_ms, first.duration_ms);
    assert_ne!(second.run_id, first.run_id);
}

#[tokio::test(start_paused = true)]
async fn a_script_nobody_rewound_is_one_the_second_run_finds_played() {
    let mut harness = Harness::playing(FAILS_CALLS_ENDS, 3).await;

    let first = harness.run(FIRST_RUN).await;
    let second = harness.run(SECOND_RUN).await;

    assert_eq!(first.summary.outcome.stop_reason(), StopReason::Completed);
    assert_eq!(
        second.summary.outcome.stop_reason(),
        StopReason::ProviderError
    );
    assert_eq!(second.summary.outcome.turns, 0);
}
