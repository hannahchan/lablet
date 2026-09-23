use std::cell::RefCell;
use std::pin::Pin;
use std::task::{Context, Poll};

use serde_json::json;

use super::*;
use crate::tests::block_on;
use crate::{
    ContentBlock, Effort, FinishReason, Prompts, ProviderKind, Rates, StopClass, Thinking,
    TokenCounts, ToolCallEnd, ToolCallId, ToolCallOutcome, ToolCallStatus, ToolResult,
    ToolResultContent, ToolSource, ToolStats,
};

fn nz(count: u32) -> NonZeroU32 {
    NonZeroU32::new(count).expect("the caps in these tests are all above zero")
}

/// A call to a built-in tool the run offered, which ended `ended`.
fn ran(ended: ToolCallEnd) -> ToolCallStatus {
    ToolCallStatus::ran(ToolSource::Builtin, ended)
}

/// The same, for a tool served over MCP.
fn ran_over_mcp(ended: ToolCallEnd) -> ToolCallStatus {
    ToolCallStatus::ran(
        ToolSource::Mcp {
            server: "docs".to_owned(),
        },
        ended,
    )
}

fn rates() -> Rates {
    Rates::new(3.0, 15.0, 0.3, 3.75).expect("ordinary published rates")
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
        Prompts {
            system: "You fix tests.".to_owned(),
            task: "Fix the failing test.".to_owned(),
        },
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
            input: ToolInput::Json(json!({ "n": n })),
        })
    }));
    ProviderResponse::new(
        content,
        Usage::from_inclusive(TokenCounts {
            input,
            output: 20,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        }),
        finish,
        None,
        None,
    )
    .unwrap()
}

fn answer(status: ToolCallStatus, output: &str, latency: Duration) -> Answer {
    Answer::measured(
        status,
        vec![ToolResultContent::Text(output.to_owned())],
        Some(8),
        ms(0),
        latency,
    )
}

fn pending(responded: Responded) -> Pending {
    match responded {
        Responded::Pending(pending) => pending,
        Responded::Final(_) => panic!("the response called a tool, so the run is pending"),
    }
}

fn final_run(responded: Responded) -> Final {
    match responded {
        Responded::Final(done) => done,
        Responded::Pending(_) => panic!("the response called no tool, so the run is final"),
    }
}

/// Records a response that calls `tools`, with a 10ms attempt.
fn calling(run: Run, tools: &[&str]) -> Pending {
    pending(run.responded(
        response("On it.", tools, FinishReason::ToolUse, 100),
        ms(0),
        ms(10),
    ))
}

/// Records a response that calls no tool.
fn done(run: Run, input: u64, latency: Duration) -> Final {
    final_run(run.responded(
        response("Done.", &[], FinishReason::EndTurn, input),
        ms(0),
        latency,
    ))
}

/// The n in a call's id, `call_n`.
fn index(call: &ToolUse) -> usize {
    call.id.as_str()["call_".len()..].parse().unwrap()
}

fn exclusive(_: &ToolUse) -> ToolConcurrency {
    ToolConcurrency::Exclusive
}

fn one_at_a_time() -> Schedule<'static> {
    Schedule {
        max_concurrent: nz(1),
        concurrency: &exclusive,
    }
}

/// Answers the n-th call with the n-th of `answers`.
fn answered(pending: Pending, answers: &[Answer]) -> Run {
    block_on(pending.answer(one_at_a_time(), |call| {
        std::future::ready(answers[index(&call)].clone())
    }))
}

/// Records a turn that calls `tools`, which end with `statuses`.
fn tool_turn(run: Run, tools: &[&str], statuses: &[ToolCallStatus]) -> Run {
    let answers: Vec<Answer> = statuses
        .iter()
        .map(|status| answer(status.clone(), "out", ms(1)))
        .collect();
    answered(calling(run, tools), &answers)
}

fn finish(run: Run, stop: StopReason) -> RunSummary {
    run.finish(stop, ms(12_345), None, None, None, None).summary
}

#[test]
fn a_run_that_did_nothing_has_a_summary_of_its_setup_and_zeros() {
    let finished = start().finish(StopReason::Cancelled, ms(0), None, None, None, None);
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
    let run = Run::start(
        setup(),
        Prompts {
            system: "caf\u{e9}".to_owned(),
            task: "na\u{ef}ve".to_owned(),
        },
    );

    let waiting = finish(run.clone(), StopReason::Cancelled);
    let taken =
        final_run(run.responded(response("Hi.", &[], FinishReason::EndTurn, 1), ms(0), ms(1)))
            .finish(StopReason::Completed, ms(0), None, None, None, None)
            .summary;

    for summary in [waiting, taken] {
        assert_eq!(summary.prompt_system_bytes, 5);
        assert_eq!(summary.prompt_user_bytes, 6);
    }
}

#[test]
fn the_conversation_so_far_can_be_read_while_the_run_goes_on() {
    let run = start();
    assert_eq!(run.transcript().system(), "You fix tests.");
    assert!(run.transcript().turns().is_empty());

    let run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::Ok)]);
    assert_eq!(run.transcript().turns().len(), 1);
}

#[test]
fn the_prompt_is_the_input_of_the_first_turn_and_of_no_other() {
    let run = tool_turn(start(), &["bash"], &[ran(ToolCallEnd::Ok)]);
    let run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::Ok)]);

    let turns = run.transcript.turns();
    assert_eq!(
        turns[0].input(),
        [UserContent::Text("Fix the failing test.".to_owned())]
    );
    assert!(turns[1].input().is_empty());
}

#[test]
fn the_messages_are_the_prompt_and_then_each_response_with_the_results_that_answer_it() {
    let run = start();
    let prompt = [UserContent::Text("Fix the failing test.".to_owned())];
    assert_eq!(
        run.messages(),
        [Message::User {
            tool_results: Vec::new(),
            input: &prompt,
        }]
    );

    let run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::Timeout)]);

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
    let run = final_run(start().responded(
        response("Partial answer.", &[], FinishReason::EndTurn, 1),
        ms(0),
        ms(1),
    ));

    let finished = run.finish(
        StopReason::ProviderError,
        Duration::from_micros(12_345_999),
        None,
        Some("provider: 401 unauthorized".to_owned()),
        Some(rates()),
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
    assert_eq!(finished.summary.rates, Some(rates()));
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
    let run = tool_turn(start(), &["bash"], &[ran(ToolCallEnd::Ok)]);
    assert_eq!(run.progress(ms(1_500)).turns, 1);
    let run = done(run, 180, ms(1));

    assert_eq!(
        run.usage(),
        Usage::from_inclusive(TokenCounts {
            input: 280,
            output: 40,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0
        })
    );
    let finished = run.finish(StopReason::Completed, ms(0), None, None, None, None);
    assert_eq!(finished.summary.outcome.turns, 2);
    assert_eq!(
        finished.summary.finish_reasons,
        [FinishReason::ToolUse, FinishReason::EndTurn]
    );
    assert_eq!(
        finished.summary.outcome.usage,
        Usage::from_inclusive(TokenCounts {
            input: 280,
            output: 40,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0
        })
    );
    assert_eq!(finished.transcript.turns().len(), 2);
}

#[test]
fn the_recorded_turn_is_the_one_the_transcript_ends_with_when_the_run_finishes() {
    let calling = pending(start().responded(
        response("Hello.", &["bash"], FinishReason::ToolUse, 1),
        ms(40),
        ms(2),
    ));
    let turn = calling.turn().clone();

    assert_eq!(turn.record().started_ms, 40);
    assert_eq!(turn.record().latency_ms, 2);
    assert_eq!(turn.tool_uses().count(), 1);
    let finished = calling.finish(StopReason::MaxTurns, ms(0), None, None, None, None);
    assert_eq!(finished.transcript.turns(), [turn]);
    assert!(
        finished.transcript.turns()[0].tool_calls().is_empty(),
        "a pending run that finishes leaves its calls unanswered"
    );
}

#[test]
fn a_response_that_calls_no_tool_is_final_and_one_that_calls_a_tool_is_pending() {
    assert!(matches!(
        start().responded(
            response("Done.", &[], FinishReason::EndTurn, 1),
            ms(0),
            ms(1)
        ),
        Responded::Final(_)
    ));
    assert!(matches!(
        start().responded(
            response("On it.", &["bash"], FinishReason::ToolUse, 1),
            ms(0),
            ms(1)
        ),
        Responded::Pending(_)
    ));
}

#[test]
fn a_run_that_has_just_responded_counts_the_new_turns_usage() {
    let run = tool_turn(start(), &["bash"], &[ran(ToolCallEnd::Ok)]);
    let before = run.usage();

    let calling = calling(run.clone(), &["bash"]);
    let last = done(run, 7, ms(1));

    assert_eq!(calling.usage(), before + calling.turn().record().usage);
    assert_eq!(last.usage(), before + last.turn().record().usage);
    assert_eq!(last.turn().text(), "Done.");
}

#[test]
fn provider_latency_is_summed_and_its_maximum_kept_over_every_attempt() {
    let calls = response("On it.", &["bash"], FinishReason::ToolUse, 1);
    let mut run = start();
    run.failed_attempt(ms(900));
    let calling = pending(run.responded(calls, ms(0), ms(400)));
    let mut run = answered(calling, &[answer(ran(ToolCallEnd::Ok), "out", ms(0))]);
    run.failed_attempt(ms(200));

    let summary = finish(run.clone(), StopReason::ProviderError);
    assert_eq!(summary.provider_latency_total_ms, 1_500);
    assert_eq!(summary.provider_latency_max_ms, 900);

    let summary = done(run, 1, ms(1_200))
        .finish(StopReason::Completed, ms(0), None, None, None, None)
        .summary;
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
    let run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::Ok)]);
    let run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::Ok)]);

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
    let mut run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::Ok)]);
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
    let run = answered(
        calling(start(), &["bash", "bash", "read_file"]),
        &[
            answer(ran(ToolCallEnd::Ok), "12345678", ms(30)),
            answer(ran(ToolCallEnd::ToolError), "exit 1", ms(5)),
            answer(ran(ToolCallEnd::Ok), "0123456789", ms(2)),
        ],
    );

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
    let run = tool_turn(
        start(),
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

/// A call whose arguments didn't parse named a tool the run has, so it earns
/// its per-tool entry and isn't counted among the calls to a name the run
/// doesn't have. `lablet.tool_calls.unknown` says the model invented a name,
/// and a model that can't serialise for a real tool hasn't.
#[test]
fn a_call_whose_arguments_did_not_parse_is_not_a_call_to_an_unknown_tool() {
    let run = tool_turn(
        start(),
        &["bash", "rm_rf"],
        &[ToolCallStatus::MalformedInput, ToolCallStatus::Unknown],
    );

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.tool_calls_unknown, 1);
    assert_eq!(summary.tool_calls_errors, 2);
    assert_eq!(summary.per_tool.keys().collect::<Vec<_>>(), [&name("bash")]);
    assert_eq!(summary.per_tool[&name("bash")].errors, 1);
}

/// The summary asks the status whether a tool ran, not where it came from, so
/// an MCP tool earns its per-tool entry exactly as a built-in one does. Every
/// other test here runs built-in tools, which would leave the branch that
/// separates a call that ran from one that didn't tested on one source only.
#[test]
fn a_call_to_a_tool_served_over_mcp_earns_a_per_tool_entry_like_any_other() {
    let run = tool_turn(
        start(),
        &["bash", "read_file"],
        &[ran_over_mcp(ToolCallEnd::Ok), ran(ToolCallEnd::ToolError)],
    );

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.tool_calls_unknown, 0);
    assert_eq!(
        summary.per_tool.keys().collect::<Vec<_>>(),
        [&name("bash"), &name("read_file")]
    );
    assert_eq!(summary.per_tool[&name("bash")].calls, 1);
    assert_eq!(summary.per_tool[&name("bash")].errors, 0);
}

/// The summary asks the outcome what became of the call, not the tool list
/// what the name looks like. The two agree in a run, because the executor
/// resolves the name against that same list; this is what keeps them from
/// being two answers to one question.
#[test]
fn whether_a_call_was_to_a_tool_the_run_has_is_read_from_the_outcome_alone() {
    let run = tool_turn(start(), &["bash"], &[ToolCallStatus::Unknown]);

    let summary = finish(run, StopReason::Completed);
    assert!(summary.tools.contains(&name("bash")));
    assert_eq!(summary.tool_calls_unknown, 1);
    assert!(summary.per_tool.is_empty());
}

#[test]
fn tool_calls_that_never_ran_are_in_no_total() {
    let summary = calling(start(), &["task_complete"])
        .finish(StopReason::Completed, ms(0), None, None, None, None)
        .summary;
    assert_eq!(summary.outcome.tool_calls, 0);
    assert_eq!(summary.tool_calls_unknown, 0);
    assert_eq!(summary.tool_input_bytes, 0);
}

#[test]
fn consecutive_tool_errors_count_up_and_a_success_resets_them() {
    let run = start();
    let errors = |run: &Run| run.progress(ms(0)).consecutive_tool_errors;

    assert_eq!(errors(&run), 0);
    let run = tool_turn(
        run,
        &["bash", "no_such_tool"],
        &[ran(ToolCallEnd::Timeout), ToolCallStatus::Unknown],
    );
    assert_eq!(errors(&run), 2);
    let run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::Ok)]);
    assert_eq!(errors(&run), 0);
    let run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::Failed)]);
    assert_eq!(errors(&run), 1);
}

#[test]
fn progress_is_what_the_limits_are_held_against() {
    let run = start();
    assert_eq!(run.progress(ms(0)), Progress::default());

    let run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::ToolError)]);

    assert_eq!(
        run.progress(ms(830)),
        Progress {
            turns: 1,
            elapsed: ms(830),
            usage: Usage::from_inclusive(TokenCounts {
                input: 100,
                output: 20,
                reasoning: 0,
                cache_read: 0,
                cache_write: 0
            }),
            consecutive_tool_errors: 1,
        }
    );
}

#[test]
fn a_latency_is_truncated_as_it_is_recorded_so_totals_are_sums_of_whole_milliseconds() {
    let mut run = start();
    run.failed_attempt(Duration::from_micros(1_600));
    let calling = pending(run.responded(
        response("On it.", &["bash", "bash"], FinishReason::ToolUse, 1),
        ms(0),
        Duration::from_micros(1_999),
    ));
    let run = answered(
        calling,
        &[
            answer(ran(ToolCallEnd::Ok), "", Duration::from_micros(1_600)),
            answer(ran(ToolCallEnd::Ok), "", Duration::from_micros(1_600)),
        ],
    );

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
        Prompts {
            system: String::new(),
            task: "Go.".to_owned(),
        },
    );
    for _ in 0..2 {
        run.failed_attempt(Duration::MAX);
    }
    let calling = pending(run.responded(
        response("On it.", &["bash", "bash"], FinishReason::ToolUse, 1),
        ms(0),
        Duration::MAX,
    ));
    let run = answered(
        calling,
        &[
            answer(ran(ToolCallEnd::Ok), "", Duration::MAX),
            answer(ran(ToolCallEnd::Ok), "", Duration::MAX),
        ],
    );

    let summary = finish(run, StopReason::Timeout);
    assert_eq!(summary.timeout_ms, u64::MAX);
    assert_eq!(summary.provider_latency_total_ms, u64::MAX);
    assert_eq!(summary.provider_latency_max_ms, u64::MAX);
    assert_eq!(summary.tool_latency_total_ms, u64::MAX);
    assert_eq!(summary.per_tool[&name("bash")].latency_ms, u64::MAX);
}

#[test]
fn the_summarys_totals_are_those_of_the_transcript_it_comes_with() {
    let run = tool_turn(
        start(),
        &["bash", "read_file"],
        &[ran(ToolCallEnd::Ok), ran(ToolCallEnd::ToolError)],
    );
    let run = tool_turn(run, &["bash"], &[ran(ToolCallEnd::Ok)]);

    let finished = run.finish(StopReason::MaxTurns, ms(50), None, None, None, None);

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

/// The first response needs something from the user before it, and the task
/// is all a run has, so a blank one is refused before a run can buy a
/// provider call with it.
#[test]
fn a_blank_task_is_refused_before_a_run_can_be_built_from_it() {
    for task in ["", " ", "\t\n "] {
        assert_eq!(
            Prompts::new("You fix tests.", task),
            Err(BlankTask),
            "{task:?}"
        );
    }
    assert_eq!(BlankTask.to_string(), "the task prompt is blank");
}

#[test]
fn prompts_keep_both_strings_as_they_were_given() {
    let prompts = Prompts::new("You fix tests.", " Fix the test. ").expect("the task isn't blank");

    assert_eq!(prompts.system(), "You fix tests.");
    assert_eq!(
        prompts.task(),
        " Fix the test. ",
        "only blankness is refused; the text itself is the caller's"
    );
}

#[test]
fn a_run_with_no_system_prompt_is_allowed() {
    assert_eq!(
        Prompts::new("", "Fix the test.")
            .expect("the task isn't blank")
            .system(),
        ""
    );
}

#[test]
fn a_task_complete_call_whose_arguments_parsed_completes_an_explicit_run() {
    let calling = pending(
        start().responded(
            ProviderResponse::new(
                vec![
                    tool_use(0, "bash", ToolInput::Json(json!({ "n": 0 }))),
                    tool_use(1, "task_complete", ToolInput::Unparsed("{".to_owned())),
                    tool_use(
                        2,
                        "task_complete",
                        ToolInput::Json(json!({ "passed": true })),
                    ),
                    tool_use(
                        3,
                        "task_complete",
                        ToolInput::Json(json!({ "passed": false })),
                    ),
                ],
                Usage::default(),
                FinishReason::ToolUse,
                None,
                None,
            )
            .unwrap(),
            ms(0),
            ms(1),
        ),
    );

    assert_eq!(
        calling.completed_with(CompletionMode::Explicit),
        Some(&json!({ "passed": true })),
        "the first call to task_complete whose arguments parsed"
    );
    assert_eq!(calling.calls(CompletionMode::Explicit), Calls::TaskComplete);
    assert_eq!(calling.completed_with(CompletionMode::Natural), None);
    assert_eq!(calling.calls(CompletionMode::Natural), Calls::Tools);
}

/// In explicit mode the structured result is what the run is for, so a
/// `task_complete` call the run can't read isn't a completion. It's answered
/// like any other call with bad arguments, and the model can try again.
#[test]
fn a_task_complete_call_whose_arguments_did_not_parse_completes_nothing() {
    let calling = pending(
        start().responded(
            ProviderResponse::new(
                vec![tool_use(
                    0,
                    "task_complete",
                    ToolInput::Unparsed("{\"passed\": tr".to_owned()),
                )],
                Usage::default(),
                FinishReason::ToolUse,
                None,
                None,
            )
            .unwrap(),
            ms(0),
            ms(1),
        ),
    );

    assert_eq!(calling.completed_with(CompletionMode::Explicit), None);
    assert_eq!(calling.calls(CompletionMode::Explicit), Calls::Tools);
}

fn tool_use(n: usize, tool: &str, input: ToolInput) -> ContentBlock {
    ContentBlock::ToolUse(ToolUse {
        id: ToolCallId::new(format!("call_{n}")).unwrap(),
        name: name(tool),
        input,
    })
}

/// A future that's not ready `polls` times, waking its task each time as any
/// correct future does, and then finishes.
struct Yield(usize);

impl Future for Yield {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if self.0 == 0 {
            return Poll::Ready(());
        }
        self.0 -= 1;
        context.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Calls to a tool whose name starts `read` may run together.
fn reads_are_shared(call: &ToolUse) -> ToolConcurrency {
    if call.name.as_str().starts_with("read") {
        ToolConcurrency::Shared
    } else {
        ToolConcurrency::Exclusive
    }
}

/// Answers every call of `calling` after `polls(n)` polls, and returns the
/// run and the order calls started and ended in, as `+n` and `-n`.
fn answered_over_time(
    calling: Pending,
    max_concurrent: u32,
    polls: impl Fn(usize) -> usize,
) -> (Run, Vec<String>) {
    let log = RefCell::new(Vec::new());
    let schedule = Schedule {
        max_concurrent: nz(max_concurrent),
        concurrency: &reads_are_shared,
    };
    let run = block_on(calling.answer(schedule, |call| {
        let log = &log;
        let n = index(&call);
        let wait = polls(n);
        async move {
            log.borrow_mut().push(format!("+{n}"));
            Yield(wait).await;
            log.borrow_mut().push(format!("-{n}"));
            answer(ran(ToolCallEnd::Ok), &n.to_string(), ms(1))
        }
    }));
    (run, log.into_inner())
}

/// The most calls that were running at one time.
fn most_at_once(log: &[String]) -> usize {
    let mut running = 0;
    let mut most = 0;
    for entry in log {
        if entry.starts_with('+') {
            running += 1;
            most = most.max(running);
        } else {
            running -= 1;
        }
    }
    most
}

fn position(log: &[String], entry: &str) -> usize {
    log.iter().position(|e| e == entry).unwrap()
}

#[test]
fn consecutive_shared_calls_run_together_and_an_exclusive_call_runs_alone() {
    let calling = calling(
        start(),
        &["read_file", "read_file", "write_file", "read_file"],
    );

    let (_, log) = answered_over_time(calling, 10, |_| 2);

    assert!(
        position(&log, "+1") < position(&log, "-0"),
        "the two first reads overlap: {log:?}"
    );
    assert!(
        position(&log, "-0").max(position(&log, "-1")) < position(&log, "+2"),
        "the write starts after both reads end: {log:?}"
    );
    assert!(
        position(&log, "-2") < position(&log, "+3"),
        "the read after the write starts after it ends: {log:?}"
    );
}

#[test]
fn consecutive_exclusive_calls_each_run_alone() {
    let calling = calling(start(), &["write_file", "bash", "write_file"]);

    let (_, log) = answered_over_time(calling, 10, |_| 2);

    assert_eq!(log, ["+0", "-0", "+1", "-1", "+2", "-2"]);
}

/// The cap is a pool, not a window over the calls in order: when a later call
/// ends first, the next call starts in its slot without waiting for the
/// earliest.
#[test]
fn a_slow_call_does_not_hold_back_the_calls_after_it_in_its_group() {
    let calling = calling(start(), &["read_file"; 3]);

    let (run, log) = answered_over_time(calling, 2, |n| if n == 0 { 10 } else { 1 });

    assert!(
        position(&log, "+2") < position(&log, "-0"),
        "the third read starts once the second ends, while the first still runs: {log:?}"
    );
    let ids: Vec<&str> = run.transcript.turns()[0]
        .tool_calls()
        .iter()
        .map(|outcome| outcome.call_id.as_str())
        .collect();
    assert_eq!(ids, ["call_0", "call_1", "call_2"]);
}

#[test]
fn a_group_runs_no_more_calls_at_once_than_the_schedule_allows() {
    let calling = calling(start(), &["read_file"; 5]);

    let (_, capped) = answered_over_time(calling.clone(), 2, |_| 2);
    let (_, free) = answered_over_time(calling, 10, |_| 2);

    assert_eq!(most_at_once(&capped), 2, "{capped:?}");
    assert_eq!(most_at_once(&free), 5, "{free:?}");
}

#[test]
fn outcomes_are_in_call_order_and_answer_their_own_calls_whichever_finishes_first() {
    let calling = calling(start(), &["read_file", "read_file", "read_file"]);

    // The first call takes longest, so it finishes last.
    let (run, log) = answered_over_time(calling, 10, |n| 3 - n);

    assert_eq!(log.last().map(String::as_str), Some("-0"), "{log:?}");
    let turn = &run.transcript.turns()[0];
    let answered: Vec<(&str, &[ToolResultContent])> = turn
        .tool_calls()
        .iter()
        .map(|outcome| (outcome.call_id.as_str(), outcome.content.as_slice()))
        .collect();
    assert_eq!(
        answered,
        [
            ("call_0", &[ToolResultContent::Text("0".to_owned())][..]),
            ("call_1", &[ToolResultContent::Text("1".to_owned())][..]),
            ("call_2", &[ToolResultContent::Text("2".to_owned())][..]),
        ]
    );
}

#[test]
fn an_answer_reports_what_the_outcome_will_hold() {
    let answer = Answer::measured(
        ran(ToolCallEnd::ToolError),
        vec![ToolResultContent::Text("0123456789".to_owned())],
        Some(4),
        ms(20),
        ms(7),
    );
    let reported = (
        answer.status().clone(),
        answer.latency_ms(),
        answer.output_bytes(),
        answer.truncated_from_bytes(),
        answer.content().to_vec(),
    );

    let run = answered(calling(start(), &["bash"]), &[answer]);

    let outcome: &ToolCallOutcome = &run.transcript.turns()[0].tool_calls()[0];
    assert_eq!(
        reported,
        (
            outcome.status.clone(),
            outcome.latency_ms,
            outcome.output_bytes(),
            outcome.truncated_from_bytes,
            outcome.content.clone(),
        )
    );
    assert_eq!(outcome.started_ms, 20);
    assert_eq!(outcome.truncated_from_bytes, Some(10));
}

#[test]
fn a_schedule_prints_its_cap() {
    assert_eq!(
        format!("{:?}", one_at_a_time()),
        "Schedule { max_concurrent: 1, .. }"
    );
}
