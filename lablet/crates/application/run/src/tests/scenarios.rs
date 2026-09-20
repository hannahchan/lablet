//! The loop, driven end to end against fakes.

use std::sync::Arc;
use std::time::Duration;

use lablet_model::{
    CompletionMode, ContentBlock, Endpoint, FinishReason, ModelRef, Prompts, ProviderErrorKind,
    ProviderKind, ProviderResponse, Rates, RequestParams, RunContext, RunId, StopReason, Thinking,
    TokenCounts, ToolCallEnd, ToolCallId, ToolCallStatus, ToolInput, ToolName, ToolSource,
    ToolSpec, ToolUse, Usage,
};
use lablet_policy::{Pricing, RetryPolicy, StopPolicy};

use super::fakes::{Answer, Answers, FakeCancel, FakeClock, FakeProvider, FakeTools, Recorder};
use crate::{CallLimits, EventKind, RunService, ToolFilter, ToolSet};

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn nz(count: u32) -> std::num::NonZeroU32 {
    std::num::NonZeroU32::new(count).expect("a cap in these tests is above zero")
}

fn name(value: &str) -> ToolName {
    ToolName::new(value).expect("a test's tool name is valid")
}

fn spec(value: &str) -> ToolSpec {
    ToolSpec {
        name: name(value),
        description: format!("The {value} tool."),
        input_schema: serde_json::json!({ "type": "object" }),
        source: ToolSource::Builtin,
    }
}

fn context() -> RunContext {
    RunContext {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").expect("a valid run id"),
        config_digest: "0".repeat(64),
        agent_version: "0.1.0".to_owned(),
        resource: Vec::new(),
        transcript_path: None,
        skills_count: 0,
        mcp_servers: Vec::new(),
        capture_content: false,
    }
}

fn prompts() -> Prompts {
    Prompts::new("You fix tests.", "Fix the failing test.").expect("the task isn't blank")
}

/// A response that says `text` and calls each of `tools`, the n-th under the
/// id `call_n`.
fn says(text: &str, tools: &[&str], finish: FinishReason) -> ProviderResponse {
    let mut content = vec![ContentBlock::Text(text.to_owned())];
    content.extend(tools.iter().enumerate().map(|(n, tool)| {
        ContentBlock::ToolUse(ToolUse {
            id: ToolCallId::new(format!("call_{n}")).expect("a valid call id"),
            name: name(tool),
            input: ToolInput::Json(serde_json::json!({ "n": n })),
        })
    }));
    ProviderResponse::new(
        content,
        Usage::from_inclusive(TokenCounts {
            input: 100,
            output: 20,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        }),
        finish,
        None,
        None,
    )
    .expect("a test's response has distinct call ids")
}

/// Everything a run is built from, so a scenario states only what it varies.
struct Harness {
    clock: Arc<FakeClock>,
    provider: Arc<FakeProvider>,
    observer: Arc<Recorder>,
    cancel: Arc<FakeCancel>,
    tools: Vec<Arc<dyn crate::ToolExecutor>>,
    filter: ToolFilter,
    stop: StopPolicy,
    retry: RetryPolicy,
    pricing: Option<Pricing>,
    calls: CallLimits,
    context: RunContext,
    completion: CompletionMode,
}

impl Harness {
    fn new(script: Vec<Answer>) -> Self {
        let clock = Arc::new(FakeClock::new());
        let provider = Arc::new(FakeProvider::new(
            ModelRef {
                provider: ProviderKind::Fake,
                name: "fake-1".to_owned(),
            },
            Arc::clone(&clock),
            script,
        ));
        Self {
            tools: vec![Arc::new(FakeTools::new(
                Arc::clone(&clock),
                vec![spec("bash"), spec("read_file")],
            ))],
            clock,
            provider,
            observer: Arc::new(Recorder::new()),
            cancel: Arc::new(FakeCancel::never()),
            filter: ToolFilter::default(),
            stop: StopPolicy {
                max_turns: nz(10),
                timeout: Duration::from_secs(600),
                max_total_tokens: None,
                max_consecutive_tool_errors: nz(3),
            },
            retry: RetryPolicy::new(3, ms(100), ms(10_000), 2.0).expect("a valid retry policy"),
            pricing: None,
            calls: CallLimits {
                provider_timeout: Duration::from_secs(60),
                tool_timeout: Duration::from_secs(30),
                max_tool_output_bytes: Some(100_000),
            },
            context: context(),
            completion: CompletionMode::Natural,
        }
    }

    async fn run(self) -> Run {
        let tools = ToolSet::build(self.tools, &self.filter, self.completion, None)
            .await
            .expect("these fakes serve distinct names");
        let mut service = RunService::new(
            Arc::clone(&self.provider) as Arc<dyn crate::ModelProvider>,
            Arc::new(tools),
            Arc::clone(&self.observer) as Arc<dyn crate::RunObserver>,
            Arc::clone(&self.clock) as Arc<dyn crate::Clock>,
            Arc::clone(&self.cancel) as Arc<dyn crate::Cancellation>,
            self.stop,
            self.retry,
            RequestParams {
                max_tokens: 4096,
                temperature: None,
                thinking: Thinking::ProviderDefault,
                effort: None,
                seed: None,
            },
            self.pricing,
            self.calls,
        );
        let finished = service.run(self.context, prompts()).await;
        Run {
            finished,
            observer: self.observer,
            provider: self.provider,
            clock: self.clock,
        }
    }
}

struct Run {
    finished: lablet_model::FinishedRun,
    observer: Arc<Recorder>,
    provider: Arc<FakeProvider>,
    clock: Arc<FakeClock>,
}

impl Run {
    fn stop_reason(&self) -> StopReason {
        self.finished.summary.outcome.stop_reason()
    }

    fn error(&self) -> Option<&str> {
        self.finished.summary.outcome.error()
    }

    fn turns(&self) -> u32 {
        self.finished.summary.outcome.turns
    }
}

// L1: a natural-mode run that calls one tool and then answers.

#[tokio::test]
async fn a_natural_run_ends_when_the_model_answers_without_calling_a_tool() {
    let run = Harness::new(vec![
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.turns(), 2);
    assert_eq!(run.finished.summary.outcome.tool_calls, 1);
    assert_eq!(run.finished.summary.outcome.result().text, "Done.");
    assert_eq!(run.error(), None);
    assert_eq!(run.finished.transcript.turns().len(), 2);
}

// L2: explicit mode completes only on task_complete.

#[tokio::test]
async fn explicit_mode_completes_when_the_model_calls_task_complete() {
    let mut harness = Harness::new(vec![
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says(
            "Done.",
            &[CompletionMode::TASK_COMPLETE],
            FinishReason::ToolUse,
        )),
    ]);
    harness.completion = CompletionMode::Explicit;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(
        run.finished.summary.outcome.result().structured,
        Some(serde_json::json!({ "n": 0 })),
        "the task_complete argument is the structured result"
    );
    assert_eq!(
        run.finished.summary.outcome.tool_calls, 1,
        "the intercepted call isn't executed, so it's in no total"
    );
}

#[tokio::test]
async fn explicit_mode_ends_without_completion_when_the_model_just_stops() {
    let mut harness = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))]);
    harness.completion = CompletionMode::Explicit;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::EndedWithoutCompletion);
    assert_eq!(run.finished.summary.outcome.result().structured, None);
}

// L3, L4: the caps.

#[tokio::test]
async fn the_turn_cap_stops_the_run_after_the_tool_phase_of_the_capped_turn() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Two.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Three.", &["bash"], FinishReason::ToolUse)),
    ]);
    harness.stop.max_turns = nz(2);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::MaxTurns);
    assert_eq!(run.turns(), 2);
    assert_eq!(run.provider.calls(), 2, "the capped turn's tools still ran");
    assert_eq!(run.finished.summary.outcome.tool_calls, 2);
}

#[tokio::test]
async fn the_token_budget_stops_the_run_before_the_call_that_would_pass_it() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Two.", &["bash"], FinishReason::ToolUse)),
    ]);
    harness.stop.max_total_tokens = Some(150);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::MaxTotalTokens);
    assert_eq!(
        run.turns(),
        2,
        "120 tokens was under the budget, so the second call was still made; 240 passed it"
    );
    assert_eq!(
        run.provider.calls(),
        2,
        "the budget stopped the run before a third call, not after it"
    );
}

// L8: the run timeout.

#[tokio::test]
async fn the_timeout_stops_the_run_at_the_instant_it_is_reached() {
    let mut harness = Harness::new(vec![
        Answer::Responds(
            Box::new(says("One.", &["bash"], FinishReason::ToolUse)),
            ms(400),
        ),
        Answer::now(says("Two.", &[], FinishReason::EndTurn)),
    ]);
    harness.stop.timeout = ms(300);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Timeout);
    assert_eq!(run.turns(), 1);
    assert_eq!(
        run.provider.calls(),
        1,
        "no provider call is made after the overrun"
    );
}

// L5, L6: a response the model didn't finish.

#[tokio::test]
async fn a_truncated_response_stops_the_run_and_none_of_its_tool_calls_run() {
    let run = Harness::new(vec![Answer::now(says(
        "Half a th",
        &["bash"],
        FinishReason::MaxTokens,
    ))])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::OutputTruncated);
    assert_eq!(
        run.finished.summary.outcome.tool_calls, 0,
        "a cut-off input can parse as a smaller one, so none of its calls runs"
    );
    assert!(run.finished.transcript.turns()[0].tool_calls().is_empty());
    assert!(
        !run.observer.names().contains(&"ToolCallStarted"),
        "a call that never runs is never announced either"
    );
}

#[tokio::test]
async fn a_refusal_stops_the_run_and_is_not_a_completed_one() {
    let run = Harness::new(vec![Answer::now(says(
        "I won't.",
        &[],
        FinishReason::Refusal,
    ))])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::Refused);
    assert_eq!(run.error(), None, "a refusal is a stop, not a failure");
    assert_ne!(
        run.stop_reason(),
        StopReason::Completed,
        "the CLI exits 2 on any stop reason but this one"
    );
}

#[tokio::test]
async fn a_response_cut_short_at_the_context_window_fails_the_run() {
    let run = Harness::new(vec![Answer::now(says(
        "",
        &[],
        FinishReason::ContextWindow,
    ))])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::ContextExhausted);
    assert_eq!(
        run.error(),
        Some("the response was cut short at the model's context window"),
        "a failure always carries an error, even when the loop had none to give"
    );
    assert_eq!(
        run.observer.names().last(),
        Some(&"RunFinished"),
        "a failed run still publishes its wide event"
    );
}

// E1 to E5: provider failures and retries.

#[tokio::test]
async fn a_retryable_failure_is_tried_again_after_the_policy_s_backoff() {
    let run = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.provider.calls(), 3);
    assert_eq!(
        run.clock.sleeps(),
        [ms(100), ms(200)],
        "the waits grow by the policy's factor and nothing else"
    );
    assert_eq!(
        run.finished.summary.provider_retries, 2,
        "a retry is an attempt beyond the first of its call"
    );
    assert_eq!(run.turns(), 1);
}

#[tokio::test]
async fn a_call_that_fails_every_attempt_exhausts_its_retries() {
    let run = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::fails(ProviderErrorKind::Retryable),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::RetriesExhausted);
    assert_eq!(
        run.provider.calls(),
        4,
        "max_retries of 3 allows four attempts"
    );
    assert_eq!(run.turns(), 0);
    assert!(run.error().is_some_and(|e| e.contains("fake provider")));
}

#[tokio::test]
async fn a_fatal_failure_is_not_tried_again() {
    let run = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Fatal),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::ProviderError);
    assert_eq!(run.provider.calls(), 1);
    assert_eq!(run.clock.sleeps(), [], "nothing was waited for");
    assert_eq!(
        run.finished.summary.provider_retries, 0,
        "a call that fails on its only attempt made no retry"
    );
}

#[tokio::test]
async fn a_request_the_provider_says_is_too_long_exhausts_the_context() {
    let run = Harness::new(vec![Answer::fails(ProviderErrorKind::ContextExhausted)])
        .run()
        .await;

    assert_eq!(run.stop_reason(), StopReason::ContextExhausted);
    assert_eq!(run.provider.calls(), 1);
}

#[tokio::test]
async fn a_backoff_that_would_reach_the_run_timeout_stops_the_run_instead_of_sleeping() {
    let mut harness = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    harness.stop.timeout = ms(50);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Timeout);
    assert_eq!(
        run.clock.sleeps(),
        [],
        "a wait known to reach the limit isn't taken"
    );
}

// E6 to E8: tool failures.

#[tokio::test]
async fn consecutive_tool_errors_stop_the_run_and_a_success_resets_the_count() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Two.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Three.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.stop.max_consecutive_tool_errors = nz(2);
    harness.tools = vec![Arc::new(
        FakeTools::new(Arc::clone(&harness.clock), vec![spec("bash")])
            .answers("bash", Answers::ToolError("no".to_owned()))
            .answers("bash", Answers::Text("yes".to_owned()))
            .answers("bash", Answers::ToolError("no".to_owned())),
    )];

    let run = harness.run().await;

    assert_eq!(
        run.stop_reason(),
        StopReason::Completed,
        "the success between them reset the count, so two errors never ran together"
    );
    assert_eq!(run.finished.summary.tool_calls_errors, 2);
}

#[tokio::test]
async fn the_tool_error_cap_stops_the_run_on_the_error_that_reaches_it() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Two.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Three.", &["bash"], FinishReason::ToolUse)),
    ]);
    harness.stop.max_consecutive_tool_errors = nz(2);
    harness.tools = vec![Arc::new(
        FakeTools::new(Arc::clone(&harness.clock), vec![spec("bash")])
            .answers("bash", Answers::ToolError("no".to_owned()))
            .answers("bash", Answers::ToolError("no".to_owned())),
    )];

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::ToolErrorsExhausted);
    assert_eq!(run.turns(), 2);
    assert_eq!(
        run.error(),
        Some("consecutive tool error results reached their cap")
    );
    assert_eq!(
        run.provider.calls(),
        2,
        "the error results that reached the cap are never sent back to the model"
    );
}

#[tokio::test]
async fn an_executor_that_fails_sends_the_model_an_error_result_rather_than_ending_the_run() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.tools = vec![Arc::new(
        FakeTools::new(Arc::clone(&harness.clock), vec![spec("bash")]).answers(
            "bash",
            Answers::Fails(crate::ToolErrorKind::Timeout, "took too long".to_owned()),
        ),
    )];

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    let outcome = &run.finished.transcript.turns()[0].tool_calls()[0];
    assert_eq!(
        outcome.status,
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Timeout)
    );
    assert_eq!(run.finished.summary.tool_calls_errors, 1);
}

// E10: a name the run doesn't offer.

#[tokio::test]
async fn a_call_to_a_name_the_run_does_not_offer_is_an_error_result_and_gets_no_per_tool_entry() {
    let run = Harness::new(vec![
        Answer::now(says("One.", &["bash", "invented"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.finished.summary.tool_calls_unknown, 1);
    assert_eq!(run.finished.summary.outcome.tool_calls, 2);
    assert_eq!(
        run.finished.summary.per_tool.keys().collect::<Vec<_>>(),
        [&name("bash")],
        "a name no tool has earns no key, so the wide event's keys stay bounded"
    );
    let outcome = &run.finished.transcript.turns()[0].tool_calls()[1];
    assert_eq!(outcome.status, ToolCallStatus::Unknown);
}

// C8: cancellation.

#[tokio::test]
async fn a_cancelled_run_stops_at_the_next_point_it_is_polled() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    // Answered twice before the first call, then true at the point after the
    // first tool phase.
    harness.cancel = Arc::new(FakeCancel::after(1));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.error(), None, "a cancelled run didn't fail");
    assert_eq!(
        run.provider.calls(),
        1,
        "the scripted second answer was never bought"
    );

    let turns = run.finished.transcript.turns();
    assert_eq!(
        turns.len(),
        1,
        "the transcript of a cancelled run comes back"
    );
    assert_eq!(
        turns[0].tool_calls().len(),
        1,
        "the call in flight ran to its end rather than being abandoned"
    );
    assert_eq!(
        turns[0].tool_calls()[0].status,
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok),
        "and its outcome was recorded"
    );
    assert_eq!(
        run.observer.names().last(),
        Some(&"RunFinished"),
        "a cancelled run still publishes its wide event"
    );
}

// T10: the output cap.

#[tokio::test]
async fn a_tool_s_output_is_cut_by_the_loop_and_the_outcome_holds_both_sizes() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.calls.max_tool_output_bytes = Some(4);
    harness.tools = vec![Arc::new(
        FakeTools::new(Arc::clone(&harness.clock), vec![spec("bash")])
            .answers("bash", Answers::Text("0123456789".to_owned())),
    )];

    let run = harness.run().await;

    let outcome = &run.finished.transcript.turns()[0].tool_calls()[0];
    assert_eq!(outcome.truncated_from_bytes, Some(10));
    assert_eq!(run.finished.summary.tool_calls_truncated, 1);
    assert!(
        outcome.output_bytes() > 4,
        "the marker naming what was cut is added on top of the cap"
    );
    assert_eq!(
        outcome.status,
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok),
        "the loop cut the output; the tool itself succeeded"
    );
}

// The event stream, and the summary that closes it.

#[tokio::test]
async fn the_event_stream_of_a_scripted_run_is_exactly_this() {
    let run = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(
        run.observer.names(),
        [
            "RunStarted",
            "TurnStarted",
            "ProviderCallStarted",
            "ProviderCallFailed",
            "ProviderCallStarted",
            "ProviderCallFinished",
            "ToolCallStarted",
            "ToolCallFinished",
            "TurnStarted",
            "ProviderCallStarted",
            "ProviderCallFinished",
            "RunFinished",
        ]
    );
}

#[tokio::test]
async fn every_event_carries_the_run_id_and_the_summary_closes_the_stream() {
    let run = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))])
        .run()
        .await;

    let events = run.observer.events();
    assert!(events.iter().all(|event| event.run_id == context().run_id));
    let Some(EventKind::RunFinished { summary, .. }) = events.last().map(|e| &e.kind) else {
        panic!("the last event is RunFinished");
    };
    assert_eq!(**summary, run.finished.summary);
}

/// Every content-bearing field of every event, on a run that calls a tool, so
/// the prompts aren't standing in for the response and the tool call beside
/// them.
#[tokio::test]
async fn content_reaches_an_observer_only_when_the_run_captures_it() {
    let script = || {
        vec![
            Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
            Answer::now(says("Done.", &[], FinishReason::EndTurn)),
        ]
    };
    let quiet = Harness::new(script()).run().await;
    let mut loud = Harness::new(script());
    loud.context.capture_content = true;
    let loud = loud.run().await;

    for (run, captured) in [(&quiet, false), (&loud, true)] {
        let events = run.observer.events();
        let Some(EventKind::RunStarted {
            system_prompt,
            prompt,
            ..
        }) = events.first().map(|e| e.kind.clone())
        else {
            panic!("the first event is RunStarted");
        };
        assert_eq!(system_prompt.is_some(), captured, "the system prompt");
        assert_eq!(prompt.is_some(), captured, "the task prompt");

        let present = |carried: Vec<bool>, what: &str| {
            assert!(!carried.is_empty(), "no event carried {what}");
            assert!(
                carried.iter().all(|had| *had == captured),
                "{what}: {carried:?} with capture {captured}"
            );
        };
        present(
            events
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::ProviderCallFinished { response, .. } => Some(response.is_some()),
                    _ => None,
                })
                .collect(),
            "the response",
        );
        present(
            events
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::ToolCallStarted { input, .. } => Some(input.is_some()),
                    _ => None,
                })
                .collect(),
            "the call's arguments",
        );
        present(
            events
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::ToolCallFinished { output, .. } => Some(output.is_some()),
                    _ => None,
                })
                .collect(),
            "what the model was sent back",
        );
    }
}

// Pricing, and what the summary reports about the run it measured.

#[tokio::test]
async fn a_priced_run_reports_its_cost_and_the_rates_it_was_priced_at() {
    let mut harness = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))]);
    let rates = Rates::new(4.0, 16.0, 0.5, 5.0).expect("ordinary rates");
    harness.pricing = Some(Pricing::new(rates));

    let run = harness.run().await;

    assert_eq!(run.finished.summary.rates, Some(rates));
    assert!(
        run.finished
            .summary
            .cost
            .is_some_and(|cost| cost.usd() > 0.0)
    );
}

#[tokio::test]
async fn an_unpriced_run_reports_neither_a_cost_nor_rates() {
    let run = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))])
        .run()
        .await;

    assert_eq!(run.finished.summary.cost, None);
    assert_eq!(run.finished.summary.rates, None);
}

#[tokio::test]
async fn the_summary_reports_the_limits_the_run_actually_enforced() {
    let mut harness = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))]);
    harness.stop.max_turns = nz(7);
    harness.stop.timeout = ms(1_234);
    harness.provider = Arc::new(
        FakeProvider::new(
            ModelRef {
                provider: ProviderKind::Fake,
                name: "fake-1".to_owned(),
            },
            Arc::clone(&harness.clock),
            vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))],
        )
        .at(Endpoint {
            host: "localhost".to_owned(),
            port: 11434,
        }),
    );

    let run = harness.run().await;

    assert_eq!(run.finished.summary.max_turns, nz(7));
    assert_eq!(run.finished.summary.timeout_ms, 1_234);
    assert_eq!(
        run.finished.summary.endpoint,
        Some(Endpoint {
            host: "localhost".to_owned(),
            port: 11434
        })
    );
    assert_eq!(
        run.finished.summary.tools,
        vec![name("bash"), name("read_file")]
    );
}

// Request bytes: what the loop measures, and that it measures each message once.

fn request_bytes(run: &Run) -> Vec<u64> {
    run.observer
        .events()
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::ProviderCallStarted { request_bytes, .. } => Some(request_bytes),
            _ => None,
        })
        .collect()
}

/// The measure is the system prompt, every tool spec, and every message, as
/// the model's own serde form. A test that only asserted "it grows" would
/// pass with the counting removed, so the first call's size is computed here
/// from the same parts.
#[tokio::test]
async fn request_bytes_are_the_system_prompt_the_specs_and_the_messages() {
    let run = Harness::new(vec![
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    let specs: u64 = [spec("bash"), spec("read_file")]
        .iter()
        .map(|spec| {
            serde_json::to_string(spec)
                .expect("a spec serialises")
                .len() as u64
        })
        .sum();
    let prompt = serde_json::json!({
        "user": { "tool_results": [], "input": [{ "text": "Fix the failing test." }] }
    });
    let first = prompts().system().len() as u64 + specs + prompt.to_string().len() as u64;

    let sizes = request_bytes(&run);
    assert_eq!(sizes.len(), 2);
    assert_eq!(sizes[0], first);

    // The second call adds the first turn's response and the user message
    // holding its tool result. Both are stated here rather than asserted to
    // be "bigger", so dropping the running sum of settled messages fails.
    let turn = &run.finished.transcript.turns()[0];
    let assistant = serde_json::json!({ "assistant": turn.response() });
    let results = serde_json::json!({
        "user": {
            "tool_results": [{
                "call_id": "call_0",
                "content": [{ "text": "bash ran" }],
                "is_error": false,
            }],
            "input": [],
        }
    });
    assert_eq!(
        sizes[1],
        first + assistant.to_string().len() as u64 + results.to_string().len() as u64
    );
}

#[tokio::test]
async fn a_retried_call_measures_the_same_request_again() {
    let run = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    let sizes = request_bytes(&run);
    assert_eq!(sizes.len(), 2);
    assert_eq!(
        sizes[0], sizes[1],
        "a failed attempt added nothing to the conversation"
    );
}

/// The loop asks the observer for the span it opened and puts it on the call,
/// which is how an MCP executor gets something to propagate.
#[tokio::test]
async fn the_span_an_observer_opens_reaches_the_executor() {
    let mut harness = Harness::new(vec![
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    let tools = Arc::new(FakeTools::new(
        Arc::clone(&harness.clock),
        vec![spec("bash")],
    ));
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];
    let tracing: Arc<dyn crate::RunObserver> = Arc::new(super::fakes::Tracer);

    let set = ToolSet::build(harness.tools, &harness.filter, harness.completion, None)
        .await
        .expect("distinct names");
    let mut service = RunService::new(
        Arc::clone(&harness.provider) as Arc<dyn crate::ModelProvider>,
        Arc::new(set),
        tracing,
        Arc::clone(&harness.clock) as Arc<dyn crate::Clock>,
        Arc::clone(&harness.cancel) as Arc<dyn crate::Cancellation>,
        harness.stop,
        harness.retry,
        RequestParams {
            max_tokens: 4096,
            temperature: None,
            thinking: Thinking::ProviderDefault,
            effort: None,
            seed: None,
        },
        None,
        harness.calls,
    );
    service.run(harness.context, prompts()).await;

    let taken = tools.taken();
    assert_eq!(taken.len(), 1);
    assert_eq!(
        taken[0]
            .trace_context
            .as_ref()
            .map(|c| c.traceparent.as_str()),
        Some("00-trace-call_0-01")
    );
    assert_eq!(
        taken[0].deadline,
        Duration::from_secs(30),
        "the tool call carries the run's per-call deadline"
    );
}

/// The clock the run reads is the loop's, so a tool that takes time shows up
/// in the outcome without a test waiting for it.
#[tokio::test]
async fn a_tool_that_takes_time_is_reported_as_taking_it() {
    let mut harness = Harness::new(vec![
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.tools = vec![Arc::new(
        FakeTools::new(Arc::clone(&harness.clock), vec![spec("bash")]).taking(ms(250)),
    )];

    let run = harness.run().await;

    assert_eq!(
        run.finished.transcript.turns()[0].tool_calls()[0].latency_ms,
        250
    );
    assert_eq!(run.finished.summary.tool_latency_total_ms, 250);
}

#[tokio::test]
async fn an_observer_that_keeps_no_spans_hands_the_executor_nothing() {
    let mut harness = Harness::new(vec![
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    let tools = Arc::new(FakeTools::new(
        Arc::clone(&harness.clock),
        vec![spec("bash")],
    ));
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];

    harness.run().await;

    assert_eq!(tools.taken()[0].trace_context, None);
}

// L2: the intercepted call is never started, so an observer never sees it.

#[tokio::test]
async fn the_intercepted_completion_call_is_never_started() {
    let mut harness = Harness::new(vec![Answer::now(says(
        "Done.",
        &[CompletionMode::TASK_COMPLETE],
        FinishReason::ToolUse,
    ))]);
    harness.completion = CompletionMode::Explicit;

    let run = harness.run().await;

    assert_eq!(run.finished.summary.outcome.tool_calls, 0);
    assert!(
        !run.observer.names().contains(&"ToolCallStarted"),
        "the loop intercepts it rather than running it"
    );
}

// L6, L7: a response the model didn't finish, with and without calls.

#[tokio::test]
async fn a_truncated_response_with_no_tool_call_still_truncates_the_run() {
    let run = Harness::new(vec![Answer::now(says(
        "Half a th",
        &[],
        FinishReason::MaxTokens,
    ))])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::OutputTruncated);
    assert_eq!(run.turns(), 1);
}

/// A cut-off response can end inside the completion call's arguments, so a
/// `task_complete` in one doesn't complete the run and its argument isn't the
/// result.
#[tokio::test]
async fn a_truncated_response_that_called_task_complete_does_not_complete_the_run() {
    let mut harness = Harness::new(vec![Answer::now(says(
        "Done",
        &[CompletionMode::TASK_COMPLETE],
        FinishReason::MaxTokens,
    ))]);
    harness.completion = CompletionMode::Explicit;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::OutputTruncated);
    assert_eq!(
        run.finished.summary.outcome.result().structured,
        None,
        "only a completed run carries a structured result"
    );
    assert_eq!(
        run.finished.summary.outcome.tool_calls, 0,
        "the cut-off completion call doesn't run either"
    );
    assert!(
        !run.observer.names().contains(&"ToolCallStarted"),
        "and isn't announced"
    );
}

// L9: both spellings of a refusal, and the turn that records it.

#[tokio::test]
async fn a_content_filter_refuses_the_run_and_its_tool_calls_do_not_run() {
    let refusal = ProviderResponse::new(
        vec![ContentBlock::ToolUse(ToolUse {
            id: ToolCallId::new("call_0").expect("a valid call id"),
            name: name("bash"),
            input: ToolInput::Json(serde_json::json!({})),
        })],
        Usage::default(),
        FinishReason::from("content_filter".to_owned()),
        None,
        None,
    )
    .expect("distinct call ids");

    let run = Harness::new(vec![Answer::now(refusal)]).run().await;

    assert_eq!(run.stop_reason(), StopReason::Refused);
    assert_eq!(run.finished.summary.outcome.tool_calls, 0);
    assert_eq!(
        run.finished.transcript.turns()[0].record().finish,
        FinishReason::Refusal,
        "the transcript records why, so a grader can tell a refusal from an end"
    );
}

// L10: the transcript, the events and the summary all describe one run.

#[tokio::test]
async fn the_summary_is_the_sum_of_the_transcript_beside_it() {
    let mut harness = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::Responds(
            Box::new(says(
                "On it.",
                &["bash", "read_file"],
                FinishReason::ToolUse,
            )),
            ms(80),
        ),
        Answer::Responds(Box::new(says("Done.", &[], FinishReason::EndTurn)), ms(40)),
    ]);
    harness.tools = vec![Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash"), spec("read_file")],
        )
        .answers("bash", Answers::Text("ok".to_owned()))
        .answers("read_file", Answers::ToolError("no such file".to_owned())),
    )];

    let run = harness.run().await;
    let turns = run.finished.transcript.turns();
    let summary = &run.finished.summary;

    assert_eq!(turns.len(), 2);
    assert_eq!(
        turns[0].record().attempts,
        2,
        "the failed attempt is counted"
    );
    assert_eq!(turns[0].tool_calls().len(), 2);
    assert_eq!(
        turns[0].tool_calls()[0].status,
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok)
    );
    assert_eq!(
        turns[0].tool_calls()[1].status,
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::ToolError)
    );
    assert_eq!(
        turns[0].tool_calls()[0].call_id.as_str(),
        "call_0",
        "the outcomes are in call order"
    );
    assert_eq!(turns[0].tool_calls()[1].call_id.as_str(), "call_1");
    assert_eq!(
        turns[0].tool_calls()[0].started_ms,
        turns[0].record().started_ms + turns[0].record().latency_ms,
        "the first call began when the response that asked for it arrived"
    );
    assert!(
        turns[0].tool_calls()[1].started_ms >= turns[0].tool_calls()[0].started_ms,
        "calls within a turn run in order, so the second began no earlier"
    );

    // Every total is the sum over the turns it describes.
    assert_eq!(summary.provider_retries, 1);
    assert_eq!(summary.outcome.tool_calls, 2);
    assert_eq!(summary.tool_calls_errors, 1);
    assert_eq!(
        summary.outcome.usage,
        turns
            .iter()
            .fold(Usage::default(), |sum, turn| sum + turn.record().usage)
    );
    assert_eq!(
        summary.finish_reasons,
        turns
            .iter()
            .map(|t| t.record().finish.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        summary.per_tool.keys().collect::<Vec<_>>(),
        [&name("bash"), &name("read_file")]
    );
}

// E2, E3: what the retry budget counts, and what it belongs to.

#[tokio::test]
async fn a_retry_budget_of_zero_gives_up_on_the_first_error() {
    let mut harness = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    harness.retry = RetryPolicy::new(0, ms(100), ms(10_000), 2.0).expect("a valid policy");

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::RetriesExhausted);
    assert_eq!(run.provider.calls(), 1);
}

/// The budget belongs to a call, not to the run: three calls each surviving
/// two failures is a completed run, not an exhausted one.
#[tokio::test]
async fn the_retry_budget_starts_again_for_every_provider_call() {
    let mut script = Vec::new();
    for turn in 0..3 {
        script.push(Answer::fails(ProviderErrorKind::Retryable));
        script.push(Answer::fails(ProviderErrorKind::Retryable));
        let finish = if turn == 2 {
            FinishReason::EndTurn
        } else {
            FinishReason::ToolUse
        };
        let tools: &[&str] = if turn == 2 { &[] } else { &["bash"] };
        script.push(Answer::now(says("On it.", tools, finish)));
    }

    let run = Harness::new(script).run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.provider.calls(), 9);
    assert_eq!(run.finished.summary.provider_retries, 6);
    assert_eq!(run.turns(), 3);
}

// E4, E5, E6: what each failure says, and which are tried again.

#[tokio::test]
async fn a_failure_reports_the_providers_own_words() {
    let run = Harness::new(vec![Answer::Fails(
        ProviderErrorKind::ContextExhausted,
        "prompt is 205000 tokens, over the 200000 limit".to_owned(),
        Duration::ZERO,
    )])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::ContextExhausted);
    assert_eq!(
        run.error(),
        Some("prompt is 205000 tokens, over the 200000 limit"),
        "lablet's own sentence is for a failure that came without one"
    );
    assert_eq!(
        run.observer.names().last(),
        Some(&"RunFinished"),
        "a failed run still publishes its wide event"
    );
}

#[tokio::test]
async fn a_fatal_failure_reports_what_the_provider_said() {
    let run = Harness::new(vec![Answer::Fails(
        ProviderErrorKind::Fatal,
        "401 unauthorized".to_owned(),
        Duration::ZERO,
    )])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::ProviderError);
    assert_eq!(run.error(), Some("401 unauthorized"));
    assert_eq!(run.turns(), 0);
}

/// A response the adapter couldn't map needn't recur, so it's tried again.
#[tokio::test]
async fn a_malformed_response_is_tried_again() {
    let run = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Malformed),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.provider.calls(), 2);
    assert_eq!(run.finished.summary.provider_retries, 1);
}

// E8: an unknown name is an error result, and errors are errors whatever
// made them.

#[tokio::test]
async fn calls_to_a_name_the_run_does_not_offer_count_toward_the_tool_error_cap() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["invented"], FinishReason::ToolUse)),
        Answer::now(says("Two.", &["invented"], FinishReason::ToolUse)),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    harness.stop.max_consecutive_tool_errors = nz(2);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::ToolErrorsExhausted);
    assert_eq!(run.finished.summary.tool_calls_unknown, 2);
}

// T10: the exact bytes the model is sent.

#[tokio::test]
async fn output_over_the_cap_is_cut_at_the_cap_and_says_what_was_cut() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.calls.max_tool_output_bytes = Some(10);
    harness.tools = vec![Arc::new(
        FakeTools::new(Arc::clone(&harness.clock), vec![spec("bash")])
            .answers("bash", Answers::Text("0123456789abcdef".to_owned())),
    )];

    let run = harness.run().await;

    let outcome = &run.finished.transcript.turns()[0].tool_calls()[0];
    assert_eq!(
        outcome.content,
        vec![
            lablet_model::ToolResultContent::Text("0123456789".to_owned()),
            lablet_model::ToolResultContent::Text(
                "[truncated: the first 10 of 16 bytes]".to_owned()
            ),
        ]
    );
    assert_eq!(outcome.truncated_from_bytes, Some(16));
}

#[tokio::test]
async fn output_that_just_fits_the_cap_is_not_cut() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.calls.max_tool_output_bytes = Some(10);
    harness.tools = vec![Arc::new(
        FakeTools::new(Arc::clone(&harness.clock), vec![spec("bash")])
            .answers("bash", Answers::Text("0123456789".to_owned())),
    )];

    let run = harness.run().await;

    let outcome = &run.finished.transcript.turns()[0].tool_calls()[0];
    assert_eq!(outcome.truncated_from_bytes, None);
    assert_eq!(run.finished.summary.tool_calls_truncated, 0);
}

/// The central measurement of the provider group: how long its calls took.
/// A run where nothing fails must still report it, and a turn must say when
/// its attempt began rather than when it ended.
#[tokio::test]
async fn a_turn_records_when_its_attempt_began_and_how_long_it_took() {
    let run = Harness::new(vec![
        Answer::Responds(
            Box::new(says("On it.", &["bash"], FinishReason::ToolUse)),
            ms(400),
        ),
        Answer::Responds(Box::new(says("Done.", &[], FinishReason::EndTurn)), ms(150)),
    ])
    .run()
    .await;

    let turns = run.finished.transcript.turns();
    assert_eq!(turns[0].record().started_ms, 0);
    assert_eq!(turns[0].record().latency_ms, 400);
    assert_eq!(
        turns[1].record().started_ms,
        400,
        "the second attempt began where the first ended"
    );
    assert_eq!(turns[1].record().latency_ms, 150);

    assert_eq!(
        run.finished.summary.provider_latency_total_ms, 550,
        "a run where nothing failed still spent time in the provider"
    );
    assert_eq!(run.finished.summary.provider_latency_max_ms, 400);
}

/// The totals count every attempt, not only the ones that answered.
#[tokio::test]
async fn provider_latency_counts_the_failed_attempts_too() {
    let run = Harness::new(vec![
        Answer::Fails(
            ProviderErrorKind::Retryable,
            "overloaded".to_owned(),
            ms(90),
        ),
        Answer::Responds(Box::new(says("Done.", &[], FinishReason::EndTurn)), ms(60)),
    ])
    .run()
    .await;

    assert_eq!(run.finished.summary.provider_latency_total_ms, 150);
    assert_eq!(run.finished.summary.provider_latency_max_ms, 90);
    assert_eq!(
        run.finished.transcript.turns()[0].record().started_ms,
        190,
        "the successful attempt began after the failure and its 100ms backoff"
    );
}

/// The retry field is the whole answer: a wait means another attempt follows,
/// and its absence means the call is over.
#[tokio::test]
async fn a_failed_attempt_reports_the_wait_before_the_attempt_that_follows() {
    let run = Harness::new(vec![
        Answer::Fails(ProviderErrorKind::Retryable, "overloaded".to_owned(), ms(0)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    let waits: Vec<Option<Duration>> = run
        .observer
        .events()
        .into_iter()
        .filter_map(|event| match event.kind {
            EventKind::ProviderCallFailed { retry, .. } => Some(retry),
            _ => None,
        })
        .collect();

    assert_eq!(waits, [Some(ms(100))], "the policy's first backoff");
}

#[tokio::test]
async fn the_last_failed_attempt_reports_no_wait() {
    let fail = || Answer::Fails(ProviderErrorKind::Fatal, "bad request".to_owned(), ms(0));
    let run = Harness::new(vec![fail()]).run().await;

    let waits: Vec<Option<Duration>> = run
        .observer
        .events()
        .into_iter()
        .filter_map(|event| match event.kind {
            EventKind::ProviderCallFailed { retry, .. } => Some(retry),
            _ => None,
        })
        .collect();

    assert_eq!(waits, [None], "a fatal failure is never tried again");
    assert_eq!(run.stop_reason(), StopReason::ProviderError);
}

/// The loop is the only hop between an executor and an observer, so anything
/// it drops here can never become an `mcp.*` attribute.
#[tokio::test]
async fn what_a_tool_carried_back_over_mcp_reaches_the_observer() {
    let meta = crate::McpCallMeta {
        method: "tools/call".to_owned(),
        session_id: Some("session-7".to_owned()),
        protocol_version: Some("2025-06-18".to_owned()),
        jsonrpc_request_id: Some("3".to_owned()),
        rpc_status_code: None,
        transport: crate::NetworkTransport::Pipe,
    };
    let clock = Arc::new(FakeClock::new());
    let mut harness = Harness::new(vec![
        Answer::now(says("On it.", &["search"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.tools = vec![Arc::new(
        FakeTools::new(
            Arc::clone(&clock),
            vec![ToolSpec {
                name: ToolName::new("search").expect("a test's tool name is valid"),
                description: "The search tool.".to_owned(),
                input_schema: serde_json::json!({ "type": "object" }),
                source: ToolSource::Mcp {
                    server: "docs".to_owned(),
                },
            }],
        )
        .answers(
            "search",
            Answers::OverMcp("found it".to_owned(), meta.clone()),
        ),
    )];

    let run = harness.run().await;

    let carried: Vec<Option<crate::McpCallMeta>> = run
        .observer
        .events()
        .into_iter()
        .filter_map(|event| match event.kind {
            EventKind::ToolCallFinished { mcp, .. } => Some(mcp),
            _ => None,
        })
        .collect();

    assert_eq!(carried, [Some(meta)]);
}

/// A call that failed still has a span, so its metadata comes back too.
#[tokio::test]
async fn a_tool_that_failed_over_mcp_still_reports_how_it_was_reached() {
    let meta = crate::McpCallMeta {
        method: "tools/call".to_owned(),
        session_id: None,
        protocol_version: None,
        jsonrpc_request_id: Some("4".to_owned()),
        rpc_status_code: Some("-32603".to_owned()),
        transport: crate::NetworkTransport::Tcp,
    };
    let clock = Arc::new(FakeClock::new());
    let mut harness = Harness::new(vec![
        Answer::now(says("On it.", &["search"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.tools = vec![Arc::new(
        FakeTools::new(
            Arc::clone(&clock),
            vec![ToolSpec {
                name: ToolName::new("search").expect("a test's tool name is valid"),
                description: "The search tool.".to_owned(),
                input_schema: serde_json::json!({ "type": "object" }),
                source: ToolSource::Mcp {
                    server: "docs".to_owned(),
                },
            }],
        )
        .answers(
            "search",
            Answers::FailsOverMcp(
                crate::ToolErrorKind::Failed,
                "the server gave up".to_owned(),
                meta.clone(),
            ),
        ),
    )];

    let run = harness.run().await;

    let carried: Vec<Option<crate::McpCallMeta>> = run
        .observer
        .events()
        .into_iter()
        .filter_map(|event| match event.kind {
            EventKind::ToolCallFinished { mcp, .. } => Some(mcp),
            _ => None,
        })
        .collect();

    assert_eq!(carried, [Some(meta)]);
}

/// The loop answers an unknown name itself, so no executor was reached and
/// there's no transport to report.
#[tokio::test]
async fn a_call_the_loop_answered_itself_carries_no_transport() {
    let run = Harness::new(vec![
        Answer::now(says("On it.", &["no_such_tool"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    let carried: Vec<Option<crate::McpCallMeta>> = run
        .observer
        .events()
        .into_iter()
        .filter_map(|event| match event.kind {
            EventKind::ToolCallFinished { mcp, .. } => Some(mcp),
            _ => None,
        })
        .collect();

    assert_eq!(carried, [None]);
}

/// Point A is the only place these can be read, because every later point
/// comes after a call the run hasn't earned.
#[tokio::test]
async fn a_zero_timeout_stops_the_run_before_its_first_provider_call() {
    let mut harness = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))]);
    harness.stop.timeout = Duration::ZERO;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Timeout);
    assert_eq!(run.turns(), 0);
    assert_eq!(run.provider.calls(), 0, "nothing was bought");
    assert_eq!(
        run.observer.names(),
        ["RunStarted", "RunFinished"],
        "the stop is read before a turn is announced, so no turn began"
    );
}

#[tokio::test]
async fn a_zero_token_budget_stops_the_run_before_its_first_provider_call() {
    let mut harness = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))]);
    harness.stop.max_total_tokens = Some(0);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::MaxTotalTokens);
    assert_eq!(run.provider.calls(), 0);
}

#[tokio::test]
async fn a_run_cancelled_before_its_first_poll_never_calls_the_provider() {
    let mut harness = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))]);
    harness.cancel = Arc::new(FakeCancel::after(0));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.provider.calls(), 0);
    assert_eq!(run.error(), None);
}

/// A call whose arguments didn't parse never reaches an executor: it has no
/// latency of its own, and the model is sent its own text back so it can
/// correct itself.
#[tokio::test]
async fn a_call_whose_arguments_did_not_parse_is_malformed_input_and_never_runs() {
    let clock = Arc::new(FakeClock::new());
    let mut harness = Harness::new(vec![
        Answer::now(calls_with_unparsed_input("bash", "{\"cmd\": ")),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    let tools = Arc::new(FakeTools::new(Arc::clone(&clock), vec![spec("bash")]));
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];

    let run = harness.run().await;

    let calls = run.finished.transcript.turns()[0].tool_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status, ToolCallStatus::MalformedInput);
    assert_eq!(calls[0].latency_ms, 0, "nothing ran, so nothing took time");
    assert!(tools.taken().is_empty(), "the executor was never reached");

    let lablet_model::ToolResultContent::Text(sent) = &calls[0].content[0];
    assert!(sent.contains("weren't valid JSON"), "{sent}");
    assert!(
        sent.contains("{\"cmd\": "),
        "the model sees its own text: {sent}"
    );
    assert_eq!(run.stop_reason(), StopReason::Completed);
}

/// A cut-off response can end inside `task_complete`'s own arguments. The call
/// still completes the run in explicit mode, because the model said it was
/// done, but nothing parsed, so the result carries no structured value.
#[tokio::test]
async fn a_task_complete_call_whose_arguments_did_not_parse_completes_without_a_result() {
    let mut harness = Harness::new(vec![Answer::now(calls_with_unparsed_input(
        CompletionMode::TASK_COMPLETE,
        "{\"passed\": tr",
    ))]);
    harness.completion = CompletionMode::Explicit;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(
        run.finished.summary.outcome.result().structured,
        None,
        "nothing parsed, so there is no argument to report"
    );
    assert_eq!(
        run.finished.summary.outcome.tool_calls, 0,
        "the completion call is intercepted rather than run"
    );
}

/// A response whose only call is unparsed, so `tool_uses` has one entry the
/// loop resolves by name before it reads the arguments.
fn calls_with_unparsed_input(tool: &str, text: &str) -> ProviderResponse {
    ProviderResponse::new(
        vec![
            ContentBlock::Text("On it.".to_owned()),
            ContentBlock::ToolUse(ToolUse {
                id: ToolCallId::new("call_0").expect("a valid call id"),
                name: name(tool),
                input: ToolInput::Unparsed(text.to_owned()),
            }),
        ],
        Usage::from_inclusive(TokenCounts {
            input: 100,
            output: 20,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        }),
        FinishReason::ToolUse,
        None,
        None,
    )
    .expect("a test's response has distinct call ids")
}
