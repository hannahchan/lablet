use super::*;
use crate::{ContentBlock, Effort, ProviderKind, Thinking};

const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn name(value: &str) -> ToolName {
    ToolName::new(value).unwrap()
}

fn setup() -> RunSetup {
    RunSetup {
        model: ModelRef {
            provider: ProviderKind::Anthropic,
            name: "claude-sonnet-5".to_owned(),
        },
        endpoint: Some(Endpoint {
            host: "api.anthropic.com".to_owned(),
            port: 443,
        }),
        tools: vec![name("bash"), name("read_file")],
        completion: CompletionMode::Explicit,
        max_turns: 30,
        timeout: Duration::from_secs(600),
        request: RequestDefaults {
            max_tokens: 4096,
            temperature: None,
            thinking: Thinking::Adaptive,
            effort: Some(Effort::High),
            seed: Some(7),
        },
        prompt_system_bytes: 120,
        prompt_user_bytes: 40,
    }
}

fn completion(finish: FinishReason, input: u64, output: u64) -> Completion {
    Completion {
        content: vec![ContentBlock::Text("ok".to_owned())],
        usage: Usage::from_inclusive(input, output, 0, 0),
        finish,
        response_id: None,
        response_model: None,
    }
}

fn finish(tally: RunTally, stop: StopReason) -> RunSummary {
    tally.finish(
        RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        stop,
        ms(12_345),
        RunResult::default(),
        None,
        None,
    )
}

#[test]
fn a_run_that_did_nothing_has_a_summary_of_its_setup_and_zeros() {
    let summary = finish(RunTally::start(setup()), StopReason::Cancelled);

    assert_eq!(summary.model, setup().model);
    assert_eq!(summary.endpoint, setup().endpoint);
    assert_eq!(summary.tools, setup().tools);
    assert_eq!(summary.completion, CompletionMode::Explicit);
    assert_eq!(summary.max_turns, 30);
    assert_eq!(summary.timeout_ms, 600_000);
    assert_eq!(summary.request, setup().request);
    assert_eq!(summary.prompt_system_bytes, 120);
    assert_eq!(summary.prompt_user_bytes, 40);
    assert_eq!(summary.provider_calls(), 0);
    assert_eq!(summary.provider_retries, 0);
    assert_eq!(summary.provider_latency_total_ms, 0);
    assert_eq!(summary.provider_latency_max_ms, 0);
    assert!(summary.finish_reasons.is_empty());
    assert_eq!(summary.tool_calls_errors, 0);
    assert_eq!(summary.tool_calls_unknown, 0);
    assert_eq!(summary.tool_latency_total_ms, 0);
    assert_eq!(summary.tool_input_bytes, 0);
    assert_eq!(summary.tool_output_bytes, 0);
    assert!(summary.per_tool.is_empty());
    assert_eq!(summary.cost, None);
    assert_eq!(summary.outcome.turns, 0);
    assert_eq!(summary.outcome.usage, Usage::default());
    assert_eq!(summary.outcome.tool_calls, 0);
}

#[test]
fn finish_writes_the_outcome_it_is_given_and_the_duration_in_whole_milliseconds() {
    let summary = RunTally::start(setup()).finish(
        RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        StopReason::ProviderError,
        Duration::from_micros(12_345_999),
        RunResult {
            text: "partial".to_owned(),
            structured: None,
        },
        Some("provider: 401 unauthorized".to_owned()),
        Some(Cost::new(0.25)),
    );

    assert_eq!(
        summary.outcome.run_id.as_str(),
        "01K5F3Z8Q4X9T2M7B6W1R0VNEC"
    );
    assert_eq!(summary.outcome.stop_reason, StopReason::ProviderError);
    assert_eq!(summary.outcome.duration_ms, 12_345);
    assert_eq!(summary.outcome.result.text, "partial");
    assert_eq!(
        summary.outcome.error.as_deref(),
        Some("provider: 401 unauthorized")
    );
    assert_eq!(summary.cost, Some(Cost::new(0.25)));
}

#[test]
fn each_completion_is_a_turn_with_its_usage_and_finish_reason() {
    let mut tally = RunTally::start(setup());
    tally.completion(&completion(FinishReason::ToolUse, 100, 20), ms(800));
    tally.completion(&completion(FinishReason::EndTurn, 180, 5), ms(300));

    assert_eq!(tally.usage(), Usage::from_inclusive(280, 25, 0, 0));
    assert_eq!(tally.progress(ms(1_500)).turns, 2);
    let summary = finish(tally, StopReason::Completed);
    assert_eq!(summary.outcome.turns, 2);
    assert_eq!(summary.provider_calls(), 2);
    assert_eq!(
        summary.finish_reasons,
        [FinishReason::ToolUse, FinishReason::EndTurn]
    );
    assert_eq!(summary.outcome.usage, Usage::from_inclusive(280, 25, 0, 0));
}

#[test]
fn provider_latency_is_summed_and_its_maximum_kept_over_every_attempt() {
    let mut tally = RunTally::start(setup());
    tally.failed_attempt(ms(900));
    tally.completion(&completion(FinishReason::EndTurn, 1, 1), ms(400));
    tally.failed_attempt(ms(200));

    let summary = finish(tally.clone(), StopReason::Completed);
    assert_eq!(summary.provider_latency_total_ms, 1_500);
    assert_eq!(summary.provider_latency_max_ms, 900);

    tally.completion(&completion(FinishReason::EndTurn, 1, 1), ms(1_200));
    let summary = finish(tally, StopReason::Completed);
    assert_eq!(summary.provider_latency_total_ms, 2_700);
    assert_eq!(summary.provider_latency_max_ms, 1_200);
}

#[test]
fn a_run_whose_only_provider_call_fails_took_no_turns_and_made_no_retries() {
    let mut tally = RunTally::start(setup());
    tally.failed_attempt(ms(250));

    assert_eq!(tally.progress(ms(250)).turns, 0);
    let summary = finish(tally, StopReason::ProviderError);
    assert_eq!(summary.outcome.turns, 0);
    assert_eq!(summary.provider_retries, 0);
    assert_eq!(summary.provider_latency_total_ms, 250);
    assert_eq!(summary.outcome.usage, Usage::default());
}

#[test]
fn a_retry_is_an_attempt_made_beyond_the_first_of_its_call() {
    let mut tally = RunTally::start(setup());
    tally.failed_attempt(ms(10));
    tally.retry();
    tally.failed_attempt(ms(10));
    tally.retry();
    tally.completion(&completion(FinishReason::EndTurn, 1, 1), ms(10));

    let summary = finish(tally, StopReason::Completed);
    assert_eq!(summary.provider_retries, 2);
    assert_eq!(summary.provider_calls(), 1);
}

#[test]
fn tool_calls_add_to_the_totals_and_to_the_share_of_their_tool() {
    let mut tally = RunTally::start(setup());
    tally.tool_call(&name("bash"), false, ms(30), 18, 2_000);
    tally.tool_call(&name("bash"), true, ms(5), 12, 48);
    tally.tool_call(&name("read_file"), false, ms(2), 40, 9_000);

    let summary = finish(tally, StopReason::Completed);
    assert_eq!(summary.outcome.tool_calls, 3);
    assert_eq!(summary.tool_calls_errors, 1);
    assert_eq!(summary.tool_calls_unknown, 0);
    assert_eq!(summary.tool_latency_total_ms, 37);
    assert_eq!(summary.tool_input_bytes, 70);
    assert_eq!(summary.tool_output_bytes, 11_048);
    assert_eq!(
        summary.per_tool,
        BTreeMap::from([
            (
                name("bash"),
                ToolStats {
                    calls: 2,
                    errors: 1,
                    latency_ms: 35,
                }
            ),
            (
                name("read_file"),
                ToolStats {
                    calls: 1,
                    errors: 0,
                    latency_ms: 2,
                }
            ),
        ])
    );
}

#[test]
fn a_call_to_a_name_the_run_did_not_offer_counts_in_the_totals_and_gets_no_per_tool_entry() {
    let mut tally = RunTally::start(setup());
    tally.tool_call(&name("bash"), false, ms(30), 18, 2_000);
    tally.tool_call(&name("rm_rf"), true, ms(1), 7, 25);
    tally.tool_call(&name("invented_again"), true, ms(1), 2, 25);

    let summary = finish(tally, StopReason::Completed);
    assert_eq!(summary.outcome.tool_calls, 3);
    assert_eq!(summary.tool_calls_errors, 2);
    assert_eq!(summary.tool_calls_unknown, 2);
    assert_eq!(summary.tool_latency_total_ms, 32);
    assert_eq!(summary.tool_input_bytes, 27);
    assert_eq!(summary.tool_output_bytes, 2_050);
    assert_eq!(summary.per_tool.keys().collect::<Vec<_>>(), [&name("bash")]);
}

#[test]
fn consecutive_tool_errors_count_up_and_a_success_resets_them() {
    let mut tally = RunTally::start(setup());
    let errors = |tally: &RunTally| tally.progress(ms(0)).consecutive_tool_errors;

    assert_eq!(errors(&tally), 0);
    tally.tool_call(&name("bash"), true, ms(1), 0, 0);
    tally.tool_call(&name("no_such_tool"), true, ms(1), 0, 0);
    assert_eq!(errors(&tally), 2);
    tally.tool_call(&name("bash"), false, ms(1), 0, 0);
    assert_eq!(errors(&tally), 0);
    tally.tool_call(&name("bash"), true, ms(1), 0, 0);
    assert_eq!(errors(&tally), 1);
}

#[test]
fn progress_is_what_the_limits_are_held_against() {
    let mut tally = RunTally::start(setup());
    assert_eq!(tally.progress(ms(0)), Progress::default());

    tally.completion(&completion(FinishReason::ToolUse, 100, 20), ms(800));
    tally.tool_call(&name("bash"), true, ms(30), 18, 2_000);

    assert_eq!(
        tally.progress(ms(830)),
        Progress {
            turns: 1,
            elapsed: ms(830),
            usage: Usage::from_inclusive(100, 20, 0, 0),
            consecutive_tool_errors: 1,
        }
    );
}

#[test]
fn a_latency_is_truncated_as_it_is_recorded_so_totals_are_sums_of_whole_milliseconds() {
    let mut tally = RunTally::start(setup());
    tally.completion(
        &completion(FinishReason::ToolUse, 1, 1),
        Duration::from_micros(1_999),
    );
    tally.failed_attempt(Duration::from_micros(1_600));
    tally.tool_call(&name("bash"), false, Duration::from_micros(1_600), 0, 0);
    tally.tool_call(&name("bash"), false, Duration::from_micros(1_600), 0, 0);

    let summary = finish(tally, StopReason::Completed);
    assert_eq!(summary.provider_latency_total_ms, 2);
    assert_eq!(summary.provider_latency_max_ms, 1);
    assert_eq!(summary.tool_latency_total_ms, 2);
    assert_eq!(summary.per_tool[&name("bash")].latency_ms, 2);
}

#[test]
fn sums_and_durations_saturate_rather_than_overflow() {
    let mut tally = RunTally::start(RunSetup {
        timeout: Duration::MAX,
        ..setup()
    });
    for _ in 0..2 {
        tally.failed_attempt(Duration::MAX);
        tally.tool_call(&name("bash"), true, Duration::MAX, u64::MAX, u64::MAX);
    }

    let summary = finish(tally, StopReason::Timeout);
    assert_eq!(summary.timeout_ms, u64::MAX);
    assert_eq!(summary.provider_latency_total_ms, u64::MAX);
    assert_eq!(summary.provider_latency_max_ms, u64::MAX);
    assert_eq!(summary.tool_latency_total_ms, u64::MAX);
    assert_eq!(summary.tool_input_bytes, u64::MAX);
    assert_eq!(summary.tool_output_bytes, u64::MAX);
    assert_eq!(summary.per_tool[&name("bash")].latency_ms, u64::MAX);
}
