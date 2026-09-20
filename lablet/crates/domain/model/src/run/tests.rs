use serde_json::json;

use super::*;
use crate::{
    ContentBlock, Effort, FinishReason, ProviderKind, StopClass, Thinking, ToolCallEnd, ToolCallId,
    ToolCallStatus, ToolResult, ToolResultContent, ToolSource, ToolStats, ToolUse,
};

fn nz(count: u32) -> NonZeroU32 {
    NonZeroU32::new(count).expect("the caps in these tests are all above zero")
}

/// A call to a tool the run offered, which ended `ended`.
fn ran(ended: ToolCallEnd) -> ToolCallStatus {
    ToolCallStatus::ran(ToolSource::Builtin, ended)
}

fn usd(usd: f64) -> Cost {
    Cost::new(usd).expect("the amounts in these tests are all real costs")
}

const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn name(value: &str) -> ToolName {
    ToolName::new(value).unwrap()
}

fn setup() -> RunSetup {
    RunSetup {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
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
        max_turns: nz(30),
        timeout: Duration::from_secs(600),
        request: RequestParams {
            max_tokens: 4096,
            temperature: None,
            thinking: Thinking::Adaptive,
            effort: Some(Effort::High),
            seed: Some(7),
        },
    }
}

fn start() -> Run {
    Run::start(
        setup(),
        "You fix tests.".to_owned(),
        "Fix the failing test.".to_owned(),
    )
}

/// A completion that says `text` and then calls each of `tools`, the n-th
/// under the id `call_n`.
fn response(text: &str, tools: &[&str], finish: FinishReason, input: u64) -> ProviderResponse {
    let mut content = vec![ContentBlock::Text(text.to_owned())];
    content.extend(tools.iter().enumerate().map(|(n, tool)| {
        ContentBlock::ToolUse(ToolUse {
            id: ToolCallId::new(format!("call_{n}")).unwrap(),
            name: name(tool),
            input: json!({ "n": n }),
        })
    }));
    ProviderResponse::new(
        content,
        Usage::from_inclusive(input, 20, 0, 0),
        finish,
        None,
        None,
    )
    .unwrap()
}

fn outcome(n: usize, status: ToolCallStatus, output: &str, latency: Duration) -> ToolCallOutcome {
    ToolCallOutcome::measured(
        ToolCallId::new(format!("call_{n}")).unwrap(),
        status,
        vec![ToolResultContent::Text(output.to_owned())],
        Some(8),
        ms(0),
        latency,
    )
}

/// Records a turn that calls `tools`, which end with `statuses`.
fn tool_turn(run: &mut Run, tools: &[&str], statuses: &[ToolCallStatus]) {
    run.responded(
        response("On it.", tools, FinishReason::ToolUse, 100),
        ms(0),
        ms(10),
    )
    .unwrap();
    let outcomes = statuses
        .iter()
        .enumerate()
        .map(|(n, status)| outcome(n, status.clone(), "out", ms(1)))
        .collect();
    run.tool_calls(outcomes).unwrap();
}

fn finish(run: Run, stop: StopReason) -> RunSummary {
    run.finish(stop, ms(12_345), None, None, None).summary
}

#[test]
fn a_run_that_did_nothing_has_a_summary_of_its_setup_and_zeros() {
    let finished = start().finish(StopReason::Cancelled, ms(0), None, None, None);
    let summary = finished.summary;

    assert_eq!(summary.model, setup().model);
    assert_eq!(summary.endpoint, setup().endpoint);
    assert_eq!(summary.tools, setup().tools);
    assert_eq!(summary.completion, CompletionMode::Explicit);
    assert_eq!(summary.max_turns, nz(30));
    assert_eq!(summary.timeout_ms, 600_000);
    assert_eq!(summary.request, setup().request);
    assert_eq!(summary.prompt_system_bytes, 14);
    assert_eq!(summary.prompt_user_bytes, 21);
    assert_eq!(summary.provider_retries, 0);
    assert_eq!(summary.provider_latency_total_ms, 0);
    assert_eq!(summary.provider_latency_max_ms, 0);
    assert!(summary.finish_reasons.is_empty());
    assert_eq!(summary.tool_calls_errors, 0);
    assert_eq!(summary.tool_calls_unknown, 0);
    assert_eq!(summary.tool_latency_total_ms, 0);
    assert_eq!(summary.tool_input_bytes, 0);
    assert_eq!(summary.tool_output_bytes, 0);
    assert_eq!(summary.tool_calls_truncated, 0);
    assert!(summary.per_tool.is_empty());
    assert_eq!(summary.cost, None);
    assert_eq!(summary.outcome.turns, 0);
    assert_eq!(summary.outcome.usage, Usage::default());
    assert_eq!(summary.outcome.tool_calls, 0);
    assert_eq!(summary.outcome.result().text, "");
    assert_eq!(finished.transcript.system(), "You fix tests.");
    assert!(finished.transcript.turns().is_empty());
}

#[test]
fn the_prompt_sizes_are_byte_lengths_whether_or_not_a_turn_has_taken_the_prompt() {
    let mut run = Run::start(setup(), "caf\u{e9}".to_owned(), "na\u{ef}ve".to_owned());

    let waiting = finish(run.clone(), StopReason::Cancelled);
    run.responded(response("Hi.", &[], FinishReason::EndTurn, 1), ms(0), ms(1))
        .unwrap();
    let taken = finish(run, StopReason::Completed);

    for summary in [waiting, taken] {
        assert_eq!(summary.prompt_system_bytes, 5);
        assert_eq!(summary.prompt_user_bytes, 6);
    }
}

#[test]
fn the_conversation_so_far_can_be_read_while_the_run_goes_on() {
    let mut run = start();
    assert_eq!(run.transcript().system(), "You fix tests.");
    assert!(run.transcript().turns().is_empty());

    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Ok)]);
    assert_eq!(run.transcript().turns().len(), 1);
}

#[test]
fn the_prompt_is_the_input_of_the_first_turn_and_of_no_other() {
    let mut run = start();
    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Ok)]);
    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Ok)]);

    let turns = run.transcript.turns();
    assert_eq!(
        turns[0].input(),
        [UserContent::Text("Fix the failing test.".to_owned())]
    );
    assert!(turns[1].input().is_empty());
}

#[test]
fn the_messages_are_the_prompt_and_then_each_response_with_the_results_that_answer_it() {
    let mut run = start();
    let prompt = [UserContent::Text("Fix the failing test.".to_owned())];
    assert_eq!(
        run.messages(),
        [Message::User {
            tool_results: Vec::new(),
            input: &prompt,
        }]
    );

    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Timeout)]);

    let turn = &run.transcript.turns()[0];
    let outcome = &turn.tool_calls()[0];
    assert_eq!(
        run.messages(),
        [
            Message::User {
                tool_results: Vec::new(),
                input: &prompt,
            },
            Message::Assistant(turn.response()),
            Message::User {
                tool_results: vec![ToolResult {
                    call_id: &outcome.call_id,
                    content: &outcome.content,
                    is_error: true,
                }],
                input: &[],
            },
        ]
    );
}

#[test]
fn finish_writes_the_run_id_the_duration_in_whole_milliseconds_and_the_final_text() {
    let mut run = start();
    run.responded(
        response("Partial answer.", &[], FinishReason::EndTurn, 1),
        ms(0),
        ms(1),
    )
    .unwrap();
    run.failed_attempt(ms(1));

    let finished = run.finish(
        StopReason::ProviderError,
        Duration::from_micros(12_345_999),
        None,
        Some("provider: 401 unauthorized".to_owned()),
        Some(usd(0.25)),
    );

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.run_id.as_str(), "01K5F3Z8Q4X9T2M7B6W1R0VNEC");
    assert_eq!(outcome.stop_reason(), StopReason::ProviderError);
    assert_eq!(outcome.duration_ms, 12_345);
    assert_eq!(outcome.result().text, "Partial answer.");
    assert_eq!(outcome.result().text, finished.transcript.final_text());
    assert_eq!(outcome.error(), Some("provider: 401 unauthorized"));
    assert_eq!(finished.summary.cost, Some(usd(0.25)));
}

#[test]
fn the_outcome_keeps_of_what_the_loop_passes_only_what_the_stop_reason_allows() {
    let argument = json!({ "passed": true });
    let close = |stop: StopReason| {
        start()
            .finish(
                stop,
                ms(0),
                Some(argument.clone()),
                Some("boom".to_owned()),
                None,
            )
            .summary
            .outcome
    };

    let completed = close(StopReason::Completed);
    assert_eq!(completed.result().structured, Some(argument.clone()));
    assert_eq!(completed.error(), None);
    let stopped = close(StopReason::OutputTruncated);
    assert_eq!(stopped.stop_reason().class(), StopClass::Stopped);
    assert_eq!(stopped.result().structured, None);
    assert_eq!(stopped.error(), None);
    let failed = close(StopReason::RetriesExhausted);
    assert_eq!(failed.result().structured, None);
    assert_eq!(failed.error(), Some("boom"));
}

#[test]
fn a_run_stopped_at_the_context_window_by_its_finish_reason_says_so() {
    let outcome = finish(start(), StopReason::ContextExhausted).outcome;

    assert_eq!(
        outcome.error(),
        Some("the response was cut short at the model's context window")
    );
}

#[test]
fn each_completion_is_a_turn_with_its_usage_and_finish_reason() {
    let mut run = start();
    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Ok)]);
    run.responded(
        response("Done.", &[], FinishReason::EndTurn, 180),
        ms(0),
        ms(1),
    )
    .unwrap();

    assert_eq!(run.usage(), Usage::from_inclusive(280, 40, 0, 0));
    assert_eq!(run.progress(ms(1_500)).turns, 2);
    assert_eq!(run.transcript.turns().len(), 2);
    let finished = run.finish(StopReason::Completed, ms(0), None, None, None);
    assert_eq!(finished.summary.outcome.turns, 2);
    assert_eq!(
        finished.summary.finish_reasons,
        [FinishReason::ToolUse, FinishReason::EndTurn]
    );
    assert_eq!(
        finished.summary.outcome.usage,
        Usage::from_inclusive(280, 40, 0, 0)
    );
    assert_eq!(finished.transcript.turns().len(), 2);
}

#[test]
fn the_returned_turn_is_the_one_the_transcript_now_ends_with() {
    let mut run = start();

    let turn = run
        .responded(
            response("Hello.", &["bash"], FinishReason::ToolUse, 1),
            ms(40),
            ms(2),
        )
        .unwrap()
        .clone();

    assert_eq!(turn.record().started_ms, 40);
    assert_eq!(turn.record().latency_ms, 2);
    assert_eq!(turn.tool_uses().count(), 1);
    assert_eq!(run.transcript.turns(), [turn]);
}

#[test]
fn provider_latency_is_summed_and_its_maximum_kept_over_every_attempt() {
    let calling = response("On it.", &["bash"], FinishReason::ToolUse, 1);
    let mut run = start();
    run.failed_attempt(ms(900));
    run.responded(calling, ms(0), ms(400)).unwrap();
    run.tool_calls(vec![outcome(0, ran(ToolCallEnd::Ok), "out", ms(0))])
        .unwrap();
    run.failed_attempt(ms(200));

    let summary = finish(run.clone(), StopReason::ProviderError);
    assert_eq!(summary.provider_latency_total_ms, 1_500);
    assert_eq!(summary.provider_latency_max_ms, 900);

    let answer = response("Done.", &[], FinishReason::EndTurn, 1);
    run.responded(answer, ms(0), ms(1_200)).unwrap();
    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.provider_latency_total_ms, 2_700);
    assert_eq!(summary.provider_latency_max_ms, 1_200);
}

#[test]
fn a_run_whose_only_provider_call_fails_took_no_turns_and_made_no_retries() {
    let mut run = start();
    run.failed_attempt(ms(250));

    assert_eq!(run.progress(ms(250)).turns, 0);
    let summary = finish(run, StopReason::ProviderError);
    assert_eq!(summary.outcome.turns, 0);
    assert_eq!(summary.provider_retries, 0);
    assert_eq!(summary.provider_latency_total_ms, 250);
    assert_eq!(summary.outcome.usage, Usage::default());
}

#[test]
fn a_turn_counts_the_attempts_of_its_call_and_the_next_call_starts_again() {
    let mut run = start();
    run.failed_attempt(ms(10));
    run.failed_attempt(ms(10));
    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Ok)]);
    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Ok)]);

    let attempts: Vec<u32> = run
        .transcript
        .turns()
        .iter()
        .map(|turn| turn.record().attempts)
        .collect();
    assert_eq!(attempts, [3, 1]);
    assert_eq!(finish(run, StopReason::Completed).provider_retries, 2);
}

#[test]
fn a_retry_is_an_attempt_made_beyond_the_first_of_its_call() {
    let mut run = start();
    run.failed_attempt(ms(10));
    run.responded(
        response("Done.", &[], FinishReason::EndTurn, 1),
        ms(0),
        ms(10),
    )
    .unwrap();
    for _ in 0..4 {
        run.failed_attempt(ms(10));
    }

    // One retry behind the turn, and three of the four failures of the last
    // call were followed by another attempt.
    let summary = finish(run, StopReason::RetriesExhausted);
    assert_eq!(summary.provider_retries, 4);
    assert_eq!(summary.outcome.turns, 1);
}

#[test]
fn tool_calls_add_to_the_totals_and_to_the_share_of_their_tool() {
    let mut run = start();
    run.responded(
        response(
            "On it.",
            &["bash", "bash", "read_file"],
            FinishReason::ToolUse,
            1,
        ),
        ms(0),
        ms(1),
    )
    .unwrap();
    run.tool_calls(vec![
        outcome(0, ran(ToolCallEnd::Ok), "12345678", ms(30)),
        outcome(1, ran(ToolCallEnd::ToolError), "exit 1", ms(5)),
        outcome(2, ran(ToolCallEnd::Ok), "0123456789", ms(2)),
    ])
    .unwrap();

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.outcome.tool_calls, 3);
    assert_eq!(summary.tool_calls_errors, 1);
    assert_eq!(summary.tool_calls_unknown, 0);
    assert_eq!(summary.tool_calls_truncated, 1);
    assert_eq!(summary.tool_latency_total_ms, 37);
    // Each input is `{"n":0}` with its own digit.
    assert_eq!(summary.tool_input_bytes, 3 * 7);
    // The third output was cut to 8 bytes and a 36-byte line.
    assert_eq!(summary.tool_output_bytes, 8 + 6 + 8 + 36);
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
    let mut run = start();
    tool_turn(
        &mut run,
        &["bash", "rm_rf", "invented_again"],
        &[
            ran(ToolCallEnd::Ok),
            ToolCallStatus::Unknown,
            ToolCallStatus::Unknown,
        ],
    );

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.outcome.tool_calls, 3);
    assert_eq!(summary.tool_calls_errors, 2);
    assert_eq!(summary.tool_calls_unknown, 2);
    assert_eq!(summary.tool_latency_total_ms, 3);
    assert_eq!(summary.per_tool.keys().collect::<Vec<_>>(), [&name("bash")]);
}

/// The summary asks the outcome what became of the call, not the tool list
/// what the name looks like. The two agree in a run, because the executor
/// resolves the name against that same list; this is what keeps them from
/// being two answers to one question.
#[test]
fn whether_a_call_was_to_a_tool_the_run_has_is_read_from_the_outcome_alone() {
    let mut run = start();
    tool_turn(&mut run, &["bash"], &[ToolCallStatus::Unknown]);

    let summary = finish(run, StopReason::Completed);
    assert!(summary.tools.contains(&name("bash")));
    assert_eq!(summary.tool_calls_unknown, 1);
    assert!(summary.per_tool.is_empty());
}

#[test]
fn tool_calls_that_never_ran_are_in_no_total() {
    let mut run = start();
    run.responded(
        response("Done.", &["task_complete"], FinishReason::ToolUse, 1),
        ms(0),
        ms(1),
    )
    .unwrap();

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.outcome.tool_calls, 0);
    assert_eq!(summary.tool_calls_unknown, 0);
    assert_eq!(summary.tool_input_bytes, 0);
}

#[test]
fn consecutive_tool_errors_count_up_and_a_success_resets_them() {
    let mut run = start();
    let errors = |run: &Run| run.progress(ms(0)).consecutive_tool_errors;

    assert_eq!(errors(&run), 0);
    tool_turn(
        &mut run,
        &["bash", "no_such_tool"],
        &[ran(ToolCallEnd::Timeout), ToolCallStatus::Unknown],
    );
    assert_eq!(errors(&run), 2);
    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Ok)]);
    assert_eq!(errors(&run), 0);
    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Failed)]);
    assert_eq!(errors(&run), 1);
}

#[test]
fn progress_is_what_the_limits_are_held_against() {
    let mut run = start();
    assert_eq!(run.progress(ms(0)), Progress::default());

    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::ToolError)]);

    assert_eq!(
        run.progress(ms(830)),
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
    let mut run = start();
    run.failed_attempt(Duration::from_micros(1_600));
    run.responded(
        response("On it.", &["bash", "bash"], FinishReason::ToolUse, 1),
        ms(0),
        Duration::from_micros(1_999),
    )
    .unwrap();
    run.tool_calls(vec![
        outcome(0, ran(ToolCallEnd::Ok), "", Duration::from_micros(1_600)),
        outcome(1, ran(ToolCallEnd::Ok), "", Duration::from_micros(1_600)),
    ])
    .unwrap();

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.provider_latency_total_ms, 2);
    assert_eq!(summary.provider_latency_max_ms, 1);
    assert_eq!(summary.tool_latency_total_ms, 2);
    assert_eq!(summary.per_tool[&name("bash")].latency_ms, 2);
}

#[test]
fn sums_and_durations_saturate_rather_than_overflow() {
    let mut run = Run::start(
        RunSetup {
            timeout: Duration::MAX,
            ..setup()
        },
        String::new(),
        "Go.".to_owned(),
    );
    for _ in 0..2 {
        run.failed_attempt(Duration::MAX);
    }
    run.responded(
        response("On it.", &["bash", "bash"], FinishReason::ToolUse, 1),
        ms(0),
        Duration::MAX,
    )
    .unwrap();
    run.tool_calls(vec![
        outcome(0, ran(ToolCallEnd::Ok), "", Duration::MAX),
        outcome(1, ran(ToolCallEnd::Ok), "", Duration::MAX),
    ])
    .unwrap();

    let summary = finish(run, StopReason::Timeout);
    assert_eq!(summary.timeout_ms, u64::MAX);
    assert_eq!(summary.provider_latency_total_ms, u64::MAX);
    assert_eq!(summary.provider_latency_max_ms, u64::MAX);
    assert_eq!(summary.tool_latency_total_ms, u64::MAX);
    assert_eq!(summary.per_tool[&name("bash")].latency_ms, u64::MAX);
}

#[test]
fn the_summarys_totals_are_those_of_the_transcript_it_comes_with() {
    let mut run = start();
    tool_turn(
        &mut run,
        &["bash", "read_file"],
        &[ran(ToolCallEnd::Ok), ran(ToolCallEnd::ToolError)],
    );
    tool_turn(&mut run, &["bash"], &[ran(ToolCallEnd::Ok)]);

    let finished = run.finish(StopReason::MaxTurns, ms(50), None, None, None);

    let turns = finished.transcript.turns();
    let outcomes = || turns.iter().flat_map(Turn::tool_calls);
    let summary = finished.summary;
    assert_eq!(summary.outcome.turns, 2);
    assert_eq!(turns.len(), 2);
    assert_eq!(summary.outcome.tool_calls, 3);
    assert_eq!(outcomes().count(), 3);
    assert_eq!(
        summary.outcome.usage.input_tokens,
        turns
            .iter()
            .map(|turn| turn.record().usage.input_tokens)
            .sum::<u64>()
    );
    assert_eq!(
        summary.provider_latency_total_ms,
        turns
            .iter()
            .map(|turn| turn.record().latency_ms)
            .sum::<u64>()
    );
    assert_eq!(
        summary.tool_latency_total_ms,
        outcomes().map(|outcome| outcome.latency_ms).sum::<u64>()
    );
    assert_eq!(
        summary.tool_output_bytes,
        outcomes().map(ToolCallOutcome::output_bytes).sum::<u64>()
    );
    assert_eq!(
        summary.tool_input_bytes,
        turns
            .iter()
            .flat_map(Turn::tool_uses)
            .map(ToolUse::input_bytes)
            .sum::<u64>()
    );
}

#[test]
fn a_completion_is_refused_while_the_last_turns_tool_calls_are_unanswered() {
    let mut run = start();
    let calling = || response("On it.", &["bash"], FinishReason::ToolUse, 1);
    run.responded(calling(), ms(0), ms(1)).unwrap();

    assert_eq!(
        run.responded(calling(), ms(0), ms(1)).unwrap_err(),
        TranscriptError::UnansweredCalls {
            turn: 1,
            calls: vec!["call_0".to_owned()],
        }
    );
    assert_eq!(run.transcript.turns().len(), 1);
}

#[test]
fn a_refused_completion_leaves_the_prompt_and_the_attempts_for_the_turn_that_is_accepted() {
    let mut run = Run::start(setup(), "You fix tests.".to_owned(), "Go.".to_owned());
    run.input.clear();
    run.failed_attempt(ms(1));

    let refused = run.responded(response("Hi.", &[], FinishReason::EndTurn, 1), ms(0), ms(1));
    assert_eq!(
        refused.unwrap_err(),
        TranscriptError::NothingFromTheUser { turn: 1 }
    );

    run.input = vec![UserContent::Text("Go.".to_owned())];
    let turn = run
        .responded(response("Hi.", &[], FinishReason::EndTurn, 1), ms(0), ms(1))
        .unwrap();
    assert_eq!(turn.record().attempts, 2);
}

#[test]
fn a_completion_after_a_turn_that_called_no_tools_is_refused() {
    let mut run = start();
    let answer = || response("Done.", &[], FinishReason::EndTurn, 1);
    run.responded(answer(), ms(0), ms(1)).unwrap();

    assert_eq!(
        run.responded(answer(), ms(0), ms(1)).unwrap_err(),
        TranscriptError::NothingFromTheUser { turn: 2 }
    );
    assert_eq!(run.transcript.turns().len(), 1);
}

#[test]
fn outcomes_that_are_not_those_of_the_last_turns_calls_are_refused() {
    let mut run = start();
    run.responded(
        response("On it.", &["bash"], FinishReason::ToolUse, 1),
        ms(0),
        ms(1),
    )
    .unwrap();

    assert_eq!(
        run.tool_calls(vec![outcome(7, ran(ToolCallEnd::Ok), "out", ms(1))]),
        Err(TranscriptError::OutcomesDontAnswerCalls {
            calls: vec!["call_0".to_owned()],
            outcomes: vec!["call_7".to_owned()],
        })
    );
    assert!(run.transcript.turns()[0].tool_calls().is_empty());
}
