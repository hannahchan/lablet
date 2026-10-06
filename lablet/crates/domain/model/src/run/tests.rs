use std::cell::RefCell;
use std::pin::Pin;
use std::task::{Context, Poll};

use serde_json::json;

use super::*;
use crate::tests::block_on;
use crate::{
    CacheScope, ContentBlock, Effort, FinishReason, KeptOutput, OutputCap, OutputCut, Prompts,
    ProviderApi, Rates, RunLabels, StopClass, Thinking, TokenCounts, ToolCallEnd, ToolCallId,
    ToolCallOutcome, ToolCallStatus, ToolResult, ToolResultContent, ToolSource, ToolStats,
};

fn nz(count: u32) -> NonZeroU32 {
    NonZeroU32::new(count).expect("the caps in these tests are all above zero")
}

/// A call to a tool the run offered from `source`, which ended `ended`.
fn ran(source: ToolSource, ended: ToolCallEnd) -> ToolCallStatus {
    ToolCallStatus::ran(source, ended)
}

fn docs_server() -> ToolSource {
    ToolSource::Mcp {
        server: "docs".to_owned(),
    }
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

fn labels() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: Some("terse-tool-descriptions".to_owned()),
        trial: Some("3".to_owned()),
    }
}

fn setup() -> RunSetup {
    RunSetup {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        labels: labels(),
        model: ModelRef {
            api: ProviderApi::Messages,
            name: "claude-sonnet-5".to_owned(),
            replays_reasoning: true,
        },
        endpoint: Some(Endpoint {
            host: "api.anthropic.com".to_owned(),
            port: 443,
        }),
        tools: vec![name("bash"), name("read_file")],
        tools_bytes: 312,
        tools_digest: "the digest of the tool specs".to_owned(),
        system_prompt_digest: "the digest of the system prompt".to_owned(),
        completion: CompletionMode::Explicit,
        max_turns: Some(nz(30)),
        timeout: Duration::from_secs(600),
        request: RequestParams {
            max_tokens: 4096,
            temperature: None,
            thinking: Thinking::Adaptive,
            effort: Some(Effort::High),
            seed: Some(7),
            cache_scope: CacheScope::Run,
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
/// under the id `call_n`, from a provider that reports every count.
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
            reasoning: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
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
        KeptOutput::whole(output),
        Some(OutputCap::new(8, OutputCut::Head).unwrap()),
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
    let setup = setup();

    assert_eq!(
        finished.summary,
        RunSummary {
            model: setup.model,
            endpoint: setup.endpoint,
            tools: setup.tools,
            completion: CompletionMode::Explicit,
            max_turns: Some(nz(30)),
            timeout_ms: 600_000,
            request: setup.request,
            prompt: PromptSizes {
                system_bytes: 14,
                user_bytes: 21,
                tools_bytes: 312,
            },
            tools_digest: "the digest of the tool specs".to_owned(),
            system_prompt_digest: "the digest of the system prompt".to_owned(),
            failed_usage: None,
            provider: ProviderTotals::default(),
            finish_reasons: Vec::new(),
            tool_calls: ToolCallTotals::default(),
            per_tool: BTreeMap::new(),
            rates: None,
            cost: None,
            outcome: RunOutcome::closing(OutcomeParts {
                run_id: setup.run_id,
                labels: labels(),
                stop_reason: StopReason::Cancelled,
                turns: 0,
                usage: Usage::default(),
                tool_calls: 0,
                duration_ms: 0,
                result: TaskResult {
                    text: String::new(),
                    structured: None,
                },
                error: None,
            }),
        }
    );
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
        assert_eq!(summary.prompt.system_bytes, 5);
        assert_eq!(summary.prompt.user_bytes, 6);
    }
}

#[test]
fn the_conversation_so_far_can_be_read_while_the_run_goes_on() {
    let run = start();
    assert_eq!(run.transcript().system(), "You fix tests.");
    assert!(run.transcript().turns().is_empty());

    let run = tool_turn(run, &["bash"], &[ran(ToolSource::Builtin, ToolCallEnd::Ok)]);
    assert_eq!(run.transcript().turns().len(), 1);
}

#[test]
fn the_prompt_is_the_input_of_the_first_turn_and_of_no_other() {
    let run = tool_turn(
        start(),
        &["bash"],
        &[ran(ToolSource::Builtin, ToolCallEnd::Ok)],
    );
    let run = tool_turn(run, &["bash"], &[ran(ToolSource::Builtin, ToolCallEnd::Ok)]);

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

    let run = tool_turn(
        run,
        &["bash"],
        &[ran(ToolSource::Builtin, ToolCallEnd::Timeout)],
    );

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

/// Each under its own name, in whichever state the run stopped: the two
/// digests are adjacent strings, and one written where the other belongs
/// would be a run that reports a prompt it never sent.
#[test]
fn the_summary_names_what_the_model_was_shown_as_the_run_was_set_up_with_it() {
    let waiting = finish(start(), StopReason::Cancelled);
    let answered = done(start(), 1, ms(1))
        .finish(StopReason::Completed, ms(0), None, None, None, None)
        .summary;
    let unanswered = calling(start(), &["bash"])
        .finish(StopReason::MaxTurns, ms(0), None, None, None, None)
        .summary;

    for summary in [waiting, answered, unanswered] {
        assert_eq!(summary.prompt.tools_bytes, setup().tools_bytes);
        assert_eq!(summary.tools_digest, setup().tools_digest);
        assert_eq!(summary.system_prompt_digest, setup().system_prompt_digest);
        assert_ne!(summary.tools_digest, summary.system_prompt_digest);
        assert_eq!(summary.model, setup().model);
        assert_eq!(summary.request.cache_scope, CacheScope::Run);
    }
}

#[test]
fn the_outcome_echoes_the_labels_the_run_was_set_up_with() {
    let prompts = || Prompts {
        system: String::new(),
        task: "Go.".to_owned(),
    };
    let partly = RunLabels {
        task: None,
        experiment: Some("terse-tool-descriptions".to_owned()),
        trial: None,
    };
    let close = |labels: RunLabels| {
        let setup = RunSetup { labels, ..setup() };
        let run = Run::start(setup, prompts());
        finish(run, StopReason::Cancelled).outcome.labels
    };

    assert_eq!(close(labels()), labels());
    assert_eq!(close(partly.clone()), partly);
    assert_eq!(close(RunLabels::default()), RunLabels::default());
    let answered =
        done(start(), 1, ms(1)).finish(StopReason::Completed, ms(0), None, None, None, None);
    assert_eq!(answered.summary.outcome.labels, labels());
    let unanswered =
        calling(start(), &["bash"]).finish(StopReason::MaxTurns, ms(0), None, None, None, None);
    assert_eq!(unanswered.summary.outcome.labels, labels());
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
    let run = tool_turn(
        start(),
        &["bash"],
        &[ran(ToolSource::Builtin, ToolCallEnd::Ok)],
    );
    assert_eq!(run.progress(ms(1_500)).turns, 1);
    let run = done(run, 180, ms(1));

    assert_eq!(
        run.usage(),
        Usage::from_inclusive(TokenCounts {
            input: 280,
            output: 40,
            reasoning: Some(0),
            cache_read: Some(0),
            cache_write: Some(0)
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
            reasoning: Some(0),
            cache_read: Some(0),
            cache_write: Some(0)
        })
    );
    assert_eq!(finished.transcript.turns().len(), 2);
}

/// A run whose first response called a tool and reported `first`, and whose
/// second has just been recorded, reporting `last`.
fn after(first: TokenCounts, last: TokenCounts) -> Final {
    let reporting = |tools: &[&str], finish: FinishReason, counts: TokenCounts| ProviderResponse {
        usage: Usage::from_inclusive(counts),
        ..response("On it.", tools, finish, 0)
    };
    let calling = pending(start().responded(
        reporting(&["bash"], FinishReason::ToolUse, first),
        ms(0),
        ms(1),
    ));
    let run = answered(
        calling,
        &[answer(
            ran(ToolSource::Builtin, ToolCallEnd::Ok),
            "out",
            ms(1),
        )],
    );
    final_run(run.responded(reporting(&[], FinishReason::EndTurn, last), ms(2), ms(1)))
}

#[test]
fn a_count_no_call_reported_is_missing_from_the_run_s_usage_and_its_outcome() {
    let bare = TokenCounts {
        input: 100,
        output: 20,
        ..TokenCounts::default()
    };
    let run = after(bare, bare);
    let nothing_reported = Usage {
        input_tokens: 200,
        output_tokens: 40,
        reasoning_output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    };

    assert_eq!(run.usage(), nothing_reported);
    let finished = run.finish(StopReason::Completed, ms(0), None, None, None, None);
    assert_eq!(finished.summary.outcome.usage, nothing_reported);
    assert_eq!(finished.transcript.turns()[1].record().usage.total(), 120);
}

#[test]
fn a_count_one_call_reported_is_in_the_run_s_usage_though_another_left_it_out() {
    let first = TokenCounts {
        input: 100,
        output: 20,
        reasoning: None,
        cache_read: Some(0),
        cache_write: Some(60),
    };
    let last = TokenCounts {
        input: 180,
        output: 20,
        reasoning: Some(12),
        cache_read: None,
        cache_write: Some(30),
    };
    let reported = Usage {
        input_tokens: 280,
        output_tokens: 40,
        reasoning_output_tokens: Some(12),
        cache_read_tokens: Some(0),
        cache_write_tokens: Some(90),
    };

    let run = after(first, last);

    assert_eq!(run.usage(), reported);
    let finished = run.finish(StopReason::Completed, ms(0), None, None, None, None);
    assert_eq!(finished.summary.outcome.usage, reported);
    let turns = finished.transcript.turns();
    assert_eq!(turns[0].record().usage, Usage::from_inclusive(first));
    assert_eq!(turns[1].record().usage, Usage::from_inclusive(last));
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
    let run = tool_turn(
        start(),
        &["bash"],
        &[ran(ToolSource::Builtin, ToolCallEnd::Ok)],
    );
    let before = run.usage();

    let calling = calling(run.clone(), &["bash"]);
    let last = done(run, 7, ms(1));

    assert_eq!(calling.usage(), before + calling.turn().record().usage);
    assert_eq!(last.usage(), before + last.turn().record().usage);
    assert_eq!(last.turn().text(), "Done.");
}

/// What a provider that reports every count says a failed attempt used.
fn billed(input: u64, output: u64) -> Usage {
    Usage::from_inclusive(TokenCounts {
        input,
        output,
        reasoning: Some(0),
        cache_read: Some(0),
        cache_write: Some(0),
    })
}

#[test]
fn what_a_failed_attempt_reported_is_spent_and_is_in_no_turn() {
    let mut run = start();
    run.failed_attempt(Duration::ZERO, ms(10), Some(billed(70, 5)));

    assert_eq!(run.usage(), Usage::default(), "no call has succeeded");
    assert_eq!(run.spent(), billed(70, 5));
    assert_eq!(run.progress(ms(10)).usage, billed(70, 5));

    let run = tool_turn(run, &["bash"], &[ran(ToolSource::Builtin, ToolCallEnd::Ok)]);

    assert_eq!(run.usage(), billed(100, 20));
    assert_eq!(run.spent(), billed(170, 25));
    assert_eq!(run.progress(ms(20)).usage, billed(170, 25));
    let finished = run.finish(StopReason::MaxTurns, ms(20), None, None, None, None);
    assert_eq!(finished.summary.failed_usage, Some(billed(70, 5)));
    assert_eq!(
        finished.summary.outcome.usage,
        billed(100, 20),
        "the outcome counts the calls that succeeded"
    );
    assert_eq!(
        finished.transcript.turns()[0].record().usage,
        billed(100, 20),
        "a turn's usage is that of the attempt that answered"
    );
}

#[test]
fn what_failed_attempts_reported_is_summed_over_every_call_of_the_run() {
    let mut run = start();
    run.failed_attempt(Duration::ZERO, ms(10), Some(billed(70, 5)));
    run.failed_attempt(Duration::ZERO, ms(10), None);
    let mut run = tool_turn(run, &["bash"], &[ran(ToolSource::Builtin, ToolCallEnd::Ok)]);
    run.failed_attempt(Duration::ZERO, ms(10), Some(billed(30, 0)));

    assert_eq!(run.spent(), billed(200, 25));
    assert_eq!(
        finish(run, StopReason::RetriesExhausted).failed_usage,
        Some(billed(100, 5))
    );
}

#[test]
fn failed_usage_is_absent_when_no_failed_attempt_reported_any() {
    let mut run = start();
    run.failed_attempt(Duration::ZERO, ms(10), None);
    run.failed_attempt(Duration::ZERO, ms(10), None);

    assert_eq!(run.spent(), Usage::default());
    assert_eq!(finish(run, StopReason::RetriesExhausted).failed_usage, None);
}

/// A failed attempt that reported zeros reported something, which isn't the
/// same as one that reported nothing.
#[test]
fn a_failed_attempt_that_reported_no_tokens_still_reported() {
    let mut run = start();
    run.failed_attempt(Duration::ZERO, ms(10), Some(Usage::default()));

    assert_eq!(
        finish(run, StopReason::RetriesExhausted).failed_usage,
        Some(Usage::default())
    );
}

#[test]
fn a_count_is_missing_from_the_failed_usage_only_when_no_failed_attempt_reported_it() {
    let mut run = start();
    run.failed_attempt(
        Duration::ZERO,
        ms(10),
        Some(Usage {
            input_tokens: 70,
            output_tokens: 0,
            reasoning_output_tokens: None,
            cache_read_tokens: Some(60),
            cache_write_tokens: None,
        }),
    );
    run.failed_attempt(
        Duration::ZERO,
        ms(10),
        Some(Usage {
            input_tokens: 70,
            output_tokens: 0,
            reasoning_output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: Some(10),
        }),
    );

    assert_eq!(
        finish(run, StopReason::RetriesExhausted).failed_usage,
        Some(Usage {
            input_tokens: 140,
            output_tokens: 0,
            reasoning_output_tokens: None,
            cache_read_tokens: Some(60),
            cache_write_tokens: Some(10),
        })
    );
}

/// The cost is priced from the state a run stopped in, so each state counts
/// the failed attempts beside the turn it's holding.
#[test]
fn a_run_that_has_just_responded_has_spent_what_its_failed_attempts_reported_too() {
    let mut run = tool_turn(
        start(),
        &["bash"],
        &[ran(ToolSource::Builtin, ToolCallEnd::Ok)],
    );
    run.failed_attempt(Duration::ZERO, ms(10), Some(billed(70, 5)));

    let calling = calling(run.clone(), &["bash"]);
    let last = done(run, 7, ms(1));

    assert_eq!(calling.usage(), billed(200, 40));
    assert_eq!(calling.spent(), billed(270, 45));
    assert_eq!(last.usage(), billed(107, 40));
    assert_eq!(last.spent(), billed(177, 45));
    assert_eq!(
        last.finish(StopReason::Completed, ms(0), None, None, None, None)
            .summary
            .failed_usage,
        Some(billed(70, 5))
    );
}

/// What comes back here is what the totals count of each attempt.
#[test]
fn a_failed_attempt_gives_back_its_timing_as_the_totals_count_it() {
    let mut run = start();

    let first = run.failed_attempt(
        Duration::from_micros(2_400_900),
        Duration::from_micros(90_700),
        None,
    );
    let second = run.failed_attempt(ms(2_600), ms(250), Some(billed(70, 5)));

    assert_eq!(
        first,
        FailedAttempt {
            started_ms: 2_400,
            latency_ms: 90,
        }
    );
    assert_eq!(
        second,
        FailedAttempt {
            started_ms: 2_600,
            latency_ms: 250,
        }
    );
    let summary = finish(run, StopReason::RetriesExhausted);
    assert_eq!(
        summary.provider.latency.total_ms(),
        first.latency_ms + second.latency_ms
    );
    assert_eq!(summary.provider.latency.max_ms(), second.latency_ms);
}

#[test]
fn provider_latency_is_summed_and_its_maximum_kept_over_every_attempt() {
    let calls = response("On it.", &["bash"], FinishReason::ToolUse, 1);
    let mut run = start();
    run.failed_attempt(Duration::ZERO, ms(900), None);
    let calling = pending(run.responded(calls, ms(0), ms(400)));
    let mut run = answered(
        calling,
        &[answer(
            ran(ToolSource::Builtin, ToolCallEnd::Ok),
            "out",
            ms(0),
        )],
    );
    run.failed_attempt(Duration::ZERO, ms(200), None);

    let summary = finish(run.clone(), StopReason::ProviderError);
    assert_eq!(summary.provider.latency.total_ms(), 1_500);
    assert_eq!(summary.provider.latency.max_ms(), 900);

    let summary = done(run, 1, ms(1_200))
        .finish(StopReason::Completed, ms(0), None, None, None, None)
        .summary;
    assert_eq!(summary.provider.latency.total_ms(), 2_700);
    assert_eq!(summary.provider.latency.max_ms(), 1_200);
}

#[test]
fn a_run_whose_only_provider_call_fails_took_no_turns_and_made_no_retries() {
    let mut run = start();
    run.failed_attempt(Duration::ZERO, ms(250), None);

    assert_eq!(run.progress(ms(250)).turns, 0);
    let summary = finish(run, StopReason::ProviderError);
    assert_eq!(summary.outcome.turns, 0);
    assert_eq!(summary.provider.retries, 0);
    assert_eq!(summary.provider.latency.total_ms(), 250);
    assert_eq!(summary.outcome.usage, Usage::default());
}

#[test]
fn a_turn_counts_the_attempts_of_its_call_and_the_next_call_starts_again() {
    let mut run = start();
    run.failed_attempt(Duration::ZERO, ms(10), None);
    run.failed_attempt(Duration::ZERO, ms(10), None);
    let run = tool_turn(run, &["bash"], &[ran(ToolSource::Builtin, ToolCallEnd::Ok)]);
    let run = tool_turn(run, &["bash"], &[ran(ToolSource::Builtin, ToolCallEnd::Ok)]);

    let attempts: Vec<u32> = run
        .transcript
        .turns()
        .iter()
        .map(|turn| turn.record().attempts)
        .collect();
    assert_eq!(attempts, [3, 1]);
    assert_eq!(finish(run, StopReason::Completed).provider.retries, 2);
}

#[test]
fn a_retry_is_an_attempt_made_beyond_the_first_of_its_call() {
    let mut run = start();
    run.failed_attempt(Duration::ZERO, ms(10), None);
    let mut run = tool_turn(run, &["bash"], &[ran(ToolSource::Builtin, ToolCallEnd::Ok)]);
    for _ in 0..4 {
        run.failed_attempt(Duration::ZERO, ms(10), None);
    }

    // One retry behind the turn, and three of the four failures of the last
    // call were followed by another attempt.
    let summary = finish(run, StopReason::RetriesExhausted);
    assert_eq!(summary.provider.retries, 4);
    assert_eq!(summary.outcome.turns, 1);
}

#[test]
fn tool_calls_add_to_the_totals_and_to_the_share_of_their_tool() {
    let run = answered(
        calling(start(), &["bash", "bash", "read_file"]),
        &[
            answer(
                ran(ToolSource::Builtin, ToolCallEnd::Ok),
                "12345678",
                ms(30),
            ),
            answer(
                ran(ToolSource::Builtin, ToolCallEnd::ToolError),
                "exit 1",
                ms(5),
            ),
            answer(
                ran(ToolSource::Builtin, ToolCallEnd::Ok),
                "0123456789",
                ms(2),
            ),
        ],
    );

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.outcome.tool_calls, 3);
    assert_eq!(summary.tool_calls.errors, 1);
    assert_eq!(summary.tool_calls.unknown, 0);
    assert_eq!(summary.tool_calls.truncated, 1);
    assert_eq!(summary.tool_calls.latency_ms, 37);
    // Each input is `{"n":0}` with its own digit.
    assert_eq!(summary.tool_calls.input_bytes, 3 * 7);
    // The third output was cut to 8 bytes and a 36-byte line.
    assert_eq!(summary.tool_calls.output_bytes, 8 + 6 + 8 + 36);
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
            ran(ToolSource::Builtin, ToolCallEnd::Ok),
            ToolCallStatus::Unknown,
            ToolCallStatus::Unknown,
        ],
    );

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.outcome.tool_calls, 3);
    assert_eq!(summary.tool_calls.errors, 2);
    assert_eq!(summary.tool_calls.unknown, 2);
    assert_eq!(summary.tool_calls.latency_ms, 3);
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
    assert_eq!(summary.tool_calls.unknown, 1);
    assert_eq!(summary.tool_calls.errors, 2);
    assert_eq!(summary.per_tool.keys().collect::<Vec<_>>(), [&name("bash")]);
    assert_eq!(summary.per_tool[&name("bash")].errors, 1);
}

/// A call the loop rejected named a tool the run has, `task_complete`, so it
/// counts against that tool as a call whose arguments didn't parse counts
/// against the one it named, and it's an error result like any other.
#[test]
fn a_call_the_loop_rejected_counts_in_the_totals_and_against_the_tool_it_named() {
    let run = tool_turn(
        start(),
        &["bash", "task_complete"],
        &[
            ran(ToolSource::Builtin, ToolCallEnd::Ok),
            ToolCallStatus::Rejected,
        ],
    );

    let summary = finish(run, StopReason::MaxTurns);
    assert_eq!(summary.outcome.tool_calls, 2);
    assert_eq!(summary.tool_calls.errors, 1);
    assert_eq!(summary.tool_calls.unknown, 0);
    assert_eq!(summary.tool_calls.latency_ms, 2);
    // Each input is `{"n":0}` with its own digit, and each output is `out`.
    assert_eq!(summary.tool_calls.input_bytes, 2 * 7);
    assert_eq!(summary.tool_calls.output_bytes, 2 * 3);
    assert_eq!(
        summary.per_tool,
        BTreeMap::from([
            (
                name("bash"),
                ToolStats {
                    calls: 1,
                    errors: 0,
                    latency_ms: 1,
                }
            ),
            (
                name("task_complete"),
                ToolStats {
                    calls: 1,
                    errors: 1,
                    latency_ms: 1,
                }
            ),
        ])
    );
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
        &[
            ran(docs_server(), ToolCallEnd::Ok),
            ran(ToolSource::Builtin, ToolCallEnd::ToolError),
        ],
    );

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.tool_calls.unknown, 0);
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
    assert_eq!(summary.tool_calls.unknown, 1);
    assert!(summary.per_tool.is_empty());
}

#[test]
fn tool_calls_that_never_ran_are_in_no_total() {
    let summary = calling(start(), &["task_complete"])
        .finish(StopReason::Completed, ms(0), None, None, None, None)
        .summary;
    assert_eq!(summary.outcome.tool_calls, 0);
    assert_eq!(summary.tool_calls.unknown, 0);
    assert_eq!(summary.tool_calls.input_bytes, 0);
}

/// The call that was never run is given a latency and an output over the
/// cap here, which the loop never gives one, so that each total is seen to
/// leave it out rather than to add nothing.
#[test]
fn a_call_that_was_never_run_is_in_the_transcript_and_in_no_total() {
    let run = answered(
        calling(start(), &["bash", "read_file", "no_such_tool"]),
        &[
            answer(ran(ToolSource::Builtin, ToolCallEnd::Ok), "done", ms(30)),
            answer(ToolCallStatus::NotRun, "0123456789", ms(5)),
            answer(ToolCallStatus::NotRun, "0123456789", ms(5)),
        ],
    );

    let finished = run.finish(StopReason::Timeout, ms(0), None, None, None, None);

    let outcomes = finished.transcript.turns()[0].tool_calls();
    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| outcome.status.as_str())
            .collect::<Vec<_>>(),
        ["ok", "not_run", "not_run"]
    );
    assert_eq!(outcomes[1].call_id.as_str(), "call_1");
    let summary = finished.summary;
    assert_eq!(summary.outcome.tool_calls, 1);
    assert_eq!(summary.tool_calls.errors, 0);
    assert_eq!(summary.tool_calls.unknown, 0);
    assert_eq!(summary.tool_calls.truncated, 0);
    assert_eq!(summary.tool_calls.latency_ms, 30);
    // The one input counted is `{"n":0}`, and the one output is `done`.
    assert_eq!(summary.tool_calls.input_bytes, 7);
    assert_eq!(summary.tool_calls.output_bytes, 4);
    assert_eq!(
        summary.per_tool,
        BTreeMap::from([(
            name("bash"),
            ToolStats {
                calls: 1,
                errors: 0,
                latency_ms: 30,
            }
        )])
    );
}

#[test]
fn a_run_without_a_turn_cap_has_a_summary_that_holds_none() {
    let run = Run::start(
        RunSetup {
            max_turns: None,
            ..setup()
        },
        Prompts {
            system: String::new(),
            task: "Go.".to_owned(),
        },
    );

    assert_eq!(finish(run, StopReason::Completed).max_turns, None);
}

/// The invalid turns in a row that `run` ends with.
fn invalid_turns(run: &Run) -> u32 {
    run.progress(ms(0)).consecutive_invalid_turns
}

/// A turn whose one call is answered `status`.
fn turn_answered(run: Run, status: ToolCallStatus) -> Run {
    tool_turn(run, &["bash"], &[status])
}

#[test]
fn invalid_turns_in_a_row_count_up_whichever_way_the_model_got_each_call_wrong() {
    let run = start();
    assert_eq!(invalid_turns(&run), 0);

    let run = turn_answered(run, ToolCallStatus::Unknown);
    assert_eq!(invalid_turns(&run), 1);
    let run = turn_answered(run, ToolCallStatus::MalformedInput);
    assert_eq!(invalid_turns(&run), 2);
    let run = tool_turn(
        run,
        &["no_such_tool", "bash"],
        &[ToolCallStatus::Unknown, ToolCallStatus::MalformedInput],
    );
    assert_eq!(invalid_turns(&run), 3);
    let run = tool_turn(
        run,
        &["task_complete", "task_complete"],
        &[ToolCallStatus::Rejected, ToolCallStatus::Rejected],
    );
    assert_eq!(invalid_turns(&run), 4);
}

/// The call beside the rejected one reached a tool, which is what the
/// rejection asked the model to wait for.
#[test]
fn a_turn_whose_completion_call_was_rejected_beside_a_call_that_ran_is_not_an_invalid_turn() {
    let run = turn_answered(start(), ToolCallStatus::Unknown);
    assert_eq!(invalid_turns(&run), 1);

    let run = tool_turn(
        run,
        &["bash", "task_complete"],
        &[
            ran(ToolSource::Builtin, ToolCallEnd::Ok),
            ToolCallStatus::Rejected,
        ],
    );
    assert_eq!(invalid_turns(&run), 0);
}

#[test]
fn a_turn_counts_once_however_many_invalid_calls_it_made() {
    let run = tool_turn(
        start(),
        &["no_such_tool", "nor_this", "bash"],
        &[
            ToolCallStatus::Unknown,
            ToolCallStatus::Unknown,
            ToolCallStatus::MalformedInput,
        ],
    );

    assert_eq!(invalid_turns(&run), 1);
}

#[test]
fn a_turn_in_which_a_call_reached_a_tool_ends_the_count_whatever_the_tool_returned() {
    for ended in ToolCallEnd::ALL {
        for reached in [ran(ToolSource::Builtin, ended), ran(docs_server(), ended)] {
            let run = turn_answered(start(), ToolCallStatus::Unknown);
            let run = turn_answered(run, ToolCallStatus::MalformedInput);
            assert_eq!(invalid_turns(&run), 2);

            let run = turn_answered(run, reached.clone());
            assert_eq!(invalid_turns(&run), 0, "{reached}");

            let run = turn_answered(run, ToolCallStatus::Unknown);
            assert_eq!(
                invalid_turns(&run),
                1,
                "{reached}: the count starts again after the turn that ended it"
            );
        }
    }
}

#[test]
fn where_a_valid_call_sits_among_a_turn_s_calls_does_not_decide_the_count() {
    let reached = ran(ToolSource::Builtin, ToolCallEnd::ToolError);
    let orders = [
        [reached.clone(), ToolCallStatus::Unknown],
        [ToolCallStatus::Unknown, reached],
    ];
    for statuses in orders {
        let run = turn_answered(start(), ToolCallStatus::Unknown);
        let run = tool_turn(run, &["bash", "bash"], &statuses);

        assert_eq!(invalid_turns(&run), 0, "{statuses:?}");
    }
}

/// The model got the first call wrong and had no chance to get the second
/// one right, so the turn says nothing about whether it can call a tool.
#[test]
fn a_turn_the_timeout_cut_short_is_never_an_invalid_turn() {
    let run = turn_answered(start(), ToolCallStatus::Unknown);
    let run = turn_answered(run, ToolCallStatus::MalformedInput);
    assert_eq!(invalid_turns(&run), 2);

    let cut_short = tool_turn(
        run.clone(),
        &["no_such_tool", "bash"],
        &[ToolCallStatus::Unknown, ToolCallStatus::NotRun],
    );
    assert_eq!(invalid_turns(&cut_short), 0);

    let never_begun = turn_answered(run, ToolCallStatus::NotRun);
    assert_eq!(invalid_turns(&never_begun), 0);
}

#[test]
fn turns_of_error_results_are_never_invalid_turns() {
    let mut run = start();
    for ended in ToolCallEnd::ALL {
        if ended == ToolCallEnd::Ok {
            continue;
        }
        run = turn_answered(run, ran(ToolSource::Builtin, ended));
        assert_eq!(invalid_turns(&run), 0, "{ended}");
    }
}

#[test]
fn progress_is_what_the_limits_are_held_against() {
    let run = start();
    assert_eq!(run.progress(ms(0)), Progress::default());

    let run = tool_turn(
        run,
        &["bash"],
        &[ran(ToolSource::Builtin, ToolCallEnd::ToolError)],
    );
    let mut run = tool_turn(run, &["no_such_tool"], &[ToolCallStatus::Unknown]);
    run.failed_attempt(Duration::ZERO, ms(10), Some(billed(70, 5)));

    assert_eq!(
        run.progress(ms(830)),
        Progress {
            turns: 2,
            elapsed: ms(830),
            usage: billed(270, 45),
            consecutive_invalid_turns: 1,
        }
    );
}

#[test]
fn a_latency_is_truncated_as_it_is_recorded_so_totals_are_sums_of_whole_milliseconds() {
    let mut run = start();
    run.failed_attempt(Duration::ZERO, Duration::from_micros(1_600), None);
    let calling = pending(run.responded(
        response("On it.", &["bash", "bash"], FinishReason::ToolUse, 1),
        ms(0),
        Duration::from_micros(1_999),
    ));
    let run = answered(
        calling,
        &[
            answer(
                ran(ToolSource::Builtin, ToolCallEnd::Ok),
                "",
                Duration::from_micros(1_600),
            ),
            answer(
                ran(ToolSource::Builtin, ToolCallEnd::Ok),
                "",
                Duration::from_micros(1_600),
            ),
        ],
    );

    let summary = finish(run, StopReason::Completed);
    assert_eq!(summary.provider.latency.total_ms(), 2);
    assert_eq!(summary.provider.latency.max_ms(), 1);
    assert_eq!(summary.tool_calls.latency_ms, 2);
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
        run.failed_attempt(Duration::ZERO, Duration::MAX, None);
    }
    let calling = pending(run.responded(
        response("On it.", &["bash", "bash"], FinishReason::ToolUse, 1),
        ms(0),
        Duration::MAX,
    ));
    let run = answered(
        calling,
        &[
            answer(ran(ToolSource::Builtin, ToolCallEnd::Ok), "", Duration::MAX),
            answer(ran(ToolSource::Builtin, ToolCallEnd::Ok), "", Duration::MAX),
        ],
    );

    let summary = finish(run, StopReason::Timeout);
    assert_eq!(summary.timeout_ms, u64::MAX);
    assert_eq!(summary.provider.latency.total_ms(), u64::MAX);
    assert_eq!(summary.provider.latency.max_ms(), u64::MAX);
    assert_eq!(summary.tool_calls.latency_ms, u64::MAX);
    assert_eq!(summary.per_tool[&name("bash")].latency_ms, u64::MAX);
}

#[test]
fn the_summarys_totals_are_those_of_the_transcript_it_comes_with() {
    let run = tool_turn(
        start(),
        &["bash", "read_file"],
        &[
            ran(ToolSource::Builtin, ToolCallEnd::Ok),
            ran(ToolSource::Builtin, ToolCallEnd::ToolError),
        ],
    );
    let run = tool_turn(run, &["bash"], &[ran(ToolSource::Builtin, ToolCallEnd::Ok)]);

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
        summary.provider.latency.total_ms(),
        turns
            .iter()
            .map(|turn| turn.record().latency_ms)
            .sum::<u64>()
    );
    assert_eq!(
        summary.tool_calls.latency_ms,
        outcomes().map(|outcome| outcome.latency_ms).sum::<u64>()
    );
    assert_eq!(
        summary.tool_calls.output_bytes,
        outcomes().map(ToolCallOutcome::output_bytes).sum::<u64>()
    );
    assert_eq!(
        summary.tool_calls.input_bytes,
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
fn a_task_keeps_its_text_under_another_system_prompt() {
    let prompts = Prompts::new("", " Fix the test. ")
        .expect("the task isn't blank")
        .with_system("You fix tests.");

    assert_eq!(prompts.system(), "You fix tests.");
    assert_eq!(prompts.task(), " Fix the test. ");
}

/// A run whose last response made `calls`, and nothing else.
fn having_called(calls: Vec<ContentBlock>) -> Pending {
    pending(start().responded(
        ProviderResponse::new(calls, Usage::default(), FinishReason::ToolUse, None, None).unwrap(),
        ms(0),
        ms(1),
    ))
}

fn parsed(argument: serde_json::Value) -> ToolInput {
    ToolInput::Json(argument)
}

#[test]
fn a_task_complete_call_made_on_its_own_completes_an_explicit_run() {
    let calling = having_called(vec![tool_use(
        0,
        "task_complete",
        parsed(json!({ "passed": true })),
    )]);

    assert_eq!(
        calling.completed_with(CompletionMode::Explicit),
        Some(&json!({ "passed": true }))
    );
    assert_eq!(calling.calls(CompletionMode::Explicit), Calls::TaskComplete);
}

/// Natural mode has no completion call, so a tool an executor serves under
/// the name is a tool like any other and its call is one to run.
#[test]
fn a_task_complete_call_completes_nothing_in_natural_mode() {
    let calling = having_called(vec![tool_use(
        0,
        "task_complete",
        parsed(json!({ "passed": true })),
    )]);

    assert_eq!(calling.completed_with(CompletionMode::Natural), None);
    assert_eq!(calling.calls(CompletionMode::Natural), Calls::Tools);
}

#[test]
fn a_call_to_another_tool_made_on_its_own_completes_nothing() {
    let calling = having_called(vec![tool_use(0, "bash", parsed(json!({ "n": 0 })))]);

    assert_eq!(calling.completed_with(CompletionMode::Explicit), None);
    assert_eq!(calling.calls(CompletionMode::Explicit), Calls::Tools);
}

/// A run that completed on such a response would report work as done that
/// the response only asked for. Where the completion call sits among the
/// others decides nothing, and the other call may be a second completion
/// call.
#[test]
fn a_task_complete_call_made_beside_another_call_completes_nothing() {
    let argument = || parsed(json!({ "passed": true }));
    let responses = [
        vec![
            tool_use(0, "task_complete", argument()),
            tool_use(1, "bash", parsed(json!({ "n": 1 }))),
        ],
        vec![
            tool_use(0, "bash", parsed(json!({ "n": 0 }))),
            tool_use(1, "task_complete", argument()),
        ],
        vec![
            tool_use(0, "task_complete", argument()),
            tool_use(1, "task_complete", parsed(json!({ "passed": false }))),
        ],
        vec![
            tool_use(0, "task_complete", argument()),
            tool_use(1, "task_complete", ToolInput::Unparsed("{".to_owned())),
        ],
        vec![
            tool_use(0, "task_complete", ToolInput::Unparsed("{".to_owned())),
            tool_use(1, "task_complete", argument()),
        ],
    ];
    for calls in responses {
        let calling = having_called(calls);

        assert_eq!(
            calling.completed_with(CompletionMode::Explicit),
            None,
            "{:?}",
            calling.turn().response()
        );
        assert_eq!(
            calling.calls(CompletionMode::Explicit),
            Calls::Tools,
            "{:?}",
            calling.turn().response()
        );
    }
}

/// In explicit mode the structured result is what the run is for, so a
/// `task_complete` call the run can't read isn't a completion. It's answered
/// like any other call with bad arguments, and the model can try again.
#[test]
fn a_task_complete_call_whose_arguments_did_not_parse_completes_nothing() {
    let calling = having_called(vec![tool_use(
        0,
        "task_complete",
        ToolInput::Unparsed("{\"passed\": tr".to_owned()),
    )]);

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
            answer(
                ran(ToolSource::Builtin, ToolCallEnd::Ok),
                &n.to_string(),
                ms(1),
            )
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
        ran(ToolSource::Builtin, ToolCallEnd::ToolError),
        KeptOutput::whole("0123456789"),
        Some(OutputCap::new(4, OutputCut::Head).unwrap()),
        ms(20),
        ms(7),
    );
    let reported = (
        answer.status().clone(),
        answer.started_ms(),
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
            outcome.started_ms,
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
