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
    Prompts {
        system: "You fix tests.".to_owned(),
        task: "Fix the failing test.".to_owned(),
    }
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
                completion: CompletionMode::Natural,
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
        }
    }

    async fn run(self) -> Run {
        let tools = ToolSet::build(self.tools, &self.filter, self.stop.completion, None)
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
    harness.stop.completion = CompletionMode::Explicit;

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
    harness.stop.completion = CompletionMode::Explicit;

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
}

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

#[tokio::test]
async fn content_reaches_an_observer_only_when_the_run_captures_it() {
    let quiet = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))])
        .run()
        .await;
    let mut loud = Harness::new(vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))]);
    loud.context.capture_content = true;
    let loud = loud.run().await;

    for (run, captured) in [(&quiet, false), (&loud, true)] {
        let Some(EventKind::RunStarted {
            system_prompt,
            prompt,
            ..
        }) = run.observer.events().first().map(|e| e.kind.clone())
        else {
            panic!("the first event is RunStarted");
        };
        assert_eq!(system_prompt.is_some(), captured);
        assert_eq!(prompt.is_some(), captured);
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
    let first = prompts().system.len() as u64 + specs + prompt.to_string().len() as u64;

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

    let set = ToolSet::build(
        harness.tools,
        &harness.filter,
        harness.stop.completion,
        None,
    )
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
