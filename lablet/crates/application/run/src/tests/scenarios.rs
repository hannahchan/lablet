//! The loop, driven end to end against fakes.

use std::sync::Arc;
use std::time::Duration;

use lablet_model::{
    CompletionMode, ContentBlock, Endpoint, FinishReason, ModelRef, OutputCap, OutputCut,
    OutputKeep, Prompts, ProviderErrorKind, ProviderKind, ProviderResponse, Rates, RequestParams,
    RunContext, RunId, StopReason, Thinking, TokenCounts, ToolCallEnd, ToolCallId, ToolCallStatus,
    ToolConcurrency, ToolInput, ToolName, ToolSource, ToolSpec, ToolUse, Usage,
};
use lablet_policy::{Pricing, RetryPolicy, RetrySettings, StopPolicy};

use super::fakes::{Answer, Answers, FakeCancel, FakeClock, FakeProvider, FakeTools, Recorder};
use crate::{CallLimits, EventKind, ProviderError, RunService, ToolFilter, ToolSet};

const fn ms(millis: u64) -> Duration {
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
        concurrency: ToolConcurrency::Exclusive,
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
            reasoning: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
        }),
        finish,
        None,
        None,
    )
    .expect("a test's response has distinct call ids")
}

/// The same, from a provider that reported `counts`.
fn reporting(tools: &[&str], finish: FinishReason, counts: TokenCounts) -> ProviderResponse {
    let said = says("On it.", tools, finish);
    ProviderResponse::new(
        said.content().to_vec(),
        Usage::from_inclusive(counts),
        said.finish,
        None,
        None,
    )
    .expect("a test's response has distinct call ids")
}

/// Three retries, backing off from 100 ms to 10 s, with no jitter, so a
/// scenario that isn't about the jitter asserts its waits exactly.
const fn settings() -> RetrySettings {
    RetrySettings {
        max_retries: 3,
        base: ms(100),
        max: ms(10_000),
        factor: 2.0,
        hint_max: Duration::from_secs(60),
        jitter: 0.0,
    }
}

fn retrying(settings: RetrySettings) -> RetryPolicy {
    RetryPolicy::new(settings).expect("a valid retry policy")
}

/// A failure another attempt could answer, in a provider's own words.
fn overloaded() -> ProviderError {
    ProviderError::new(ProviderErrorKind::Retryable, "529 overloaded")
}

/// What a provider that reports every count says a call used.
fn tokens(input: u64, output: u64) -> Usage {
    Usage::from_inclusive(TokenCounts {
        input,
        output,
        reasoning: Some(0),
        cache_read: Some(0),
        cache_write: Some(0),
    })
}

fn output_cap(max_bytes: u64, cut: OutputCut) -> OutputCap {
    OutputCap::new(max_bytes, cut).expect("a preview in these tests is within its cap")
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
                max_turns: None,
                timeout: Duration::from_secs(600),
                max_total_tokens: None,
                max_consecutive_invalid_turns: Some(nz(3)),
            },
            retry: retrying(settings()),
            pricing: None,
            calls: CallLimits {
                provider_timeout: Duration::from_secs(60),
                tool_timeout: Duration::from_secs(30),
                output_cap: Some(output_cap(50_000, OutputCut::Preview { bytes: 2_000 })),
                max_concurrent_tool_calls: nz(10),
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
        let finished = movable(service.run(self.context, prompts())).await;
        Run {
            finished,
            observer: self.observer,
            provider: self.provider,
            clock: self.clock,
        }
    }
}

/// A composition root runs the loop on a multi-threaded runtime, which may move
/// the run's future between threads, so every scenario fails to compile if it
/// can't be.
const fn movable<F: Future + Send>(future: F) -> F {
    future
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

    /// The error of each failed attempt and the wait the loop said would
    /// follow it, in order.
    fn failures(&self) -> Vec<(ProviderError, Option<Duration>)> {
        self.observer
            .events()
            .into_iter()
            .filter_map(|event| match event.kind {
                EventKind::ProviderCallFailed { error, retry, .. } => Some((error, retry)),
                _ => None,
            })
            .collect()
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
    harness.stop.max_turns = Some(nz(2));

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

// E7: what a tool returned never stops a run.

/// Makes the answer a fake tool is scripted with, afresh for each call,
/// since an answer is handed over once.
type Scripted = fn() -> Answers;

/// A run with a cap of three invalid turns, whose model calls `bash` five
/// times and then ends, and whose `bash` answers `failure` every time.
async fn calling_a_tool_that_always(failure: impl Fn() -> Answers) -> Run {
    let mut script: Vec<Answer> = (0..5)
        .map(|_| Answer::now(says("Again.", &["bash"], FinishReason::ToolUse)))
        .collect();
    script.push(Answer::now(says("Done.", &[], FinishReason::EndTurn)));
    let mut harness = Harness::new(script);
    harness.stop.max_consecutive_invalid_turns = Some(nz(3));
    let tools = (0..5).fold(
        FakeTools::new(Arc::clone(&harness.clock), vec![spec("bash")]),
        |tools, _| tools.answers("bash", failure()),
    );
    harness.tools = vec![Arc::new(tools)];
    harness.run().await
}

#[tokio::test]
async fn a_tool_that_always_fails_never_stops_a_run_however_it_fails() {
    let failures: [(Scripted, ToolCallEnd); 3] = [
        (
            || Answers::ToolError("1 failed".to_owned()),
            ToolCallEnd::ToolError,
        ),
        (
            || Answers::Fails(crate::ToolErrorKind::Timeout, "took too long".to_owned()),
            ToolCallEnd::Timeout,
        ),
        (
            || {
                Answers::Fails(
                    crate::ToolErrorKind::Failed,
                    "the server is gone".to_owned(),
                )
            },
            ToolCallEnd::Failed,
        ),
    ];
    for (failure, ended) in failures {
        let run = calling_a_tool_that_always(failure).await;

        assert_eq!(run.stop_reason(), StopReason::Completed, "{ended}");
        assert_eq!(run.error(), None, "{ended}");
        assert_eq!(run.turns(), 6, "{ended}");
        assert_eq!(run.provider.calls(), 6, "{ended}");
        assert_eq!(run.finished.summary.tool_calls_errors, 5, "{ended}");
        let statuses: Vec<&ToolCallStatus> = run
            .finished
            .transcript
            .turns()
            .iter()
            .flat_map(lablet_model::Turn::tool_calls)
            .map(|outcome| &outcome.status)
            .collect();
        assert_eq!(
            statuses,
            [&ToolCallStatus::ran(ToolSource::Builtin, ended); 5],
            "every call reached the tool, so no turn was an invalid one"
        );
    }
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

// T10: the output cap, cutting to the start.

const SIXTEEN_BYTES: &str = "0123456789abcdef";
const TEN_BYTES: &str = "0123456789";

/// A run under `cap` whose first response calls `bash`, which writes 16
/// bytes, and then `read_file`, which writes 10. The executor keeps of each
/// what its call says to, or all of it when it's `hoarding`.
async fn writing(cap: Option<OutputCap>, hoarding: bool) -> (Run, Arc<FakeTools>) {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash", "read_file"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.calls.output_cap = cap;
    let tools = FakeTools::new(
        Arc::clone(&harness.clock),
        vec![spec("bash"), spec("read_file")],
    )
    .answers("bash", Answers::Text(SIXTEEN_BYTES.to_owned()))
    .answers("read_file", Answers::Text(TEN_BYTES.to_owned()));
    let tools = Arc::new(if hoarding { tools.hoarding() } else { tools });
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];
    (harness.run().await, tools)
}

/// The tool results the model was sent with its second call, which are the
/// first turn's.
fn results_sent(run: &Run) -> serde_json::Value {
    let sent = run.provider.sent();
    assert_eq!(sent.len(), 2, "the run made two provider calls");
    let last = sent[1]
        .as_array()
        .and_then(|messages| messages.last())
        .expect("the second call sent messages");
    last["user"]["tool_results"].clone()
}

/// A result that isn't an error, as the model is sent it.
fn result_of(call_id: &str, texts: &[&str]) -> serde_json::Value {
    let content: Vec<_> = texts
        .iter()
        .map(|text| serde_json::json!({ "text": text }))
        .collect();
    serde_json::json!({ "call_id": call_id, "content": content, "is_error": false })
}

#[tokio::test]
async fn an_output_over_the_cap_is_cut_to_its_start_and_one_that_fits_is_sent_whole() {
    let (run, _) = writing(Some(output_cap(10, OutputCut::Head)), false).await;

    assert_eq!(
        results_sent(&run),
        serde_json::json!([
            result_of(
                "call_0",
                &["0123456789", "[truncated: the first 10 of 16 bytes]"]
            ),
            result_of("call_1", &[TEN_BYTES]),
        ])
    );
    let outcomes = run.finished.transcript.turns()[0].tool_calls();
    assert_eq!(outcomes[0].truncated_from_bytes, Some(16));
    assert_eq!(outcomes[1].truncated_from_bytes, None);
    assert_eq!(run.finished.summary.tool_calls_truncated, 1);
    for outcome in outcomes {
        assert_eq!(
            outcome.status,
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok),
            "the loop cut the output; the tool itself succeeded"
        );
    }
    assert_eq!(run.finished.summary.tool_calls_errors, 0);
    assert_eq!(run.stop_reason(), StopReason::Completed);
}

/// The line is added on top of the cap, so the size that was sent is more
/// than the cap allows of the tool's own text.
#[tokio::test]
async fn an_observer_is_told_what_was_sent_of_each_output_and_how_large_a_cut_one_was() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash", "read_file"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.calls.output_cap = Some(output_cap(10, OutputCut::Head));
    harness.calls.max_concurrent_tool_calls = nz(1);
    harness.context.capture_content = true;
    harness.tools = vec![Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash"), spec("read_file")],
        )
        .answers("bash", Answers::Text(SIXTEEN_BYTES.to_owned()))
        .answers("read_file", Answers::Text(TEN_BYTES.to_owned())),
    )];

    let run = harness.run().await;

    let line = "[truncated: the first 10 of 16 bytes]";
    let text = |text: &str| lablet_model::ToolResultContent::Text(text.to_owned());
    let told: Vec<_> = run
        .observer
        .events()
        .into_iter()
        .filter_map(|event| match event.kind {
            EventKind::ToolCallFinished {
                output_bytes,
                truncated_from_bytes,
                output,
                ..
            } => Some((output_bytes, truncated_from_bytes, output)),
            _ => None,
        })
        .collect();
    assert_eq!(
        told,
        [
            (
                10 + line.len() as u64,
                Some(16),
                Some(vec![text(TEN_BYTES), text(line)])
            ),
            (10, None, Some(vec![text(TEN_BYTES)])),
        ]
    );
    assert_eq!(
        run.finished.summary.tool_output_bytes,
        20 + line.len() as u64
    );
}

#[tokio::test]
async fn without_a_cap_nothing_is_cut_and_an_executor_is_told_to_keep_everything() {
    let (run, tools) = writing(None, false).await;

    assert_eq!(
        results_sent(&run),
        serde_json::json!([
            result_of("call_0", &[SIXTEEN_BYTES]),
            result_of("call_1", &[TEN_BYTES]),
        ])
    );
    let outcomes = run.finished.transcript.turns()[0].tool_calls();
    assert_eq!(outcomes[0].truncated_from_bytes, None);
    assert_eq!(outcomes[1].truncated_from_bytes, None);
    assert_eq!(run.finished.summary.tool_calls_truncated, 0);
    let keeps: Vec<_> = tools.taken().iter().map(|call| call.keep).collect();
    assert_eq!(keeps, [None, None]);
    assert_eq!(tools.kept(), [16, 10]);
}

/// An error result is a tool's output like any other, and so is what the
/// loop says of a call that reached no tool. Cutting one changes nothing
/// about what became of the call.
#[tokio::test]
async fn an_error_result_is_cut_as_any_other_output_is() {
    let mut harness = Harness::new(vec![
        Answer::now(says(
            "One.",
            &["bash", "read_file", "invented"],
            FinishReason::ToolUse,
        )),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.calls.output_cap = Some(output_cap(10, OutputCut::Head));
    harness.tools = vec![Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash"), spec("read_file")],
        )
        .answers("bash", Answers::ToolError(SIXTEEN_BYTES.to_owned()))
        .answers(
            "read_file",
            Answers::Fails(crate::ToolErrorKind::Failed, SIXTEEN_BYTES.to_owned()),
        ),
    )];

    let run = harness.run().await;

    let cut = |call_id: &str, texts: &[&str]| {
        let mut result = result_of(call_id, texts);
        result["is_error"] = serde_json::json!(true);
        result
    };
    let sixteen = ["0123456789", "[truncated: the first 10 of 16 bytes]"];
    assert_eq!(
        results_sent(&run),
        serde_json::json!([
            cut("call_0", &sixteen),
            cut("call_1", &sixteen),
            cut(
                "call_2",
                &["no tool na", "[truncated: the first 10 of 45 bytes]"]
            ),
        ])
    );
    let statuses: Vec<_> = run.finished.transcript.turns()[0]
        .tool_calls()
        .iter()
        .map(|outcome| outcome.status.as_str())
        .collect();
    assert_eq!(statuses, ["tool_error", "failed", "unknown"]);
    assert_eq!(run.finished.summary.tool_calls_truncated, 3);
}

// T13: the other two ways to cut, and an executor that keeps only what the
// call says to.

#[tokio::test]
async fn head_tail_sends_both_ends_of_a_long_output_around_a_line_that_says_what_was_left_out() {
    let (run, tools) = writing(Some(output_cap(10, OutputCut::HeadTail)), true).await;

    assert_eq!(
        results_sent(&run),
        serde_json::json!([
            result_of(
                "call_0",
                &["01234", "[truncated: 6 of 16 bytes left out]", "bcdef"]
            ),
            result_of("call_1", &[TEN_BYTES]),
        ])
    );
    let outcomes = run.finished.transcript.turns()[0].tool_calls();
    assert_eq!(outcomes[0].truncated_from_bytes, Some(16));
    assert_eq!(outcomes[1].truncated_from_bytes, None);
    assert_eq!(run.finished.summary.tool_calls_truncated, 1);
    assert_eq!(tools.kept(), [16, 10], "the executor held every byte");
}

#[tokio::test]
async fn preview_sends_a_few_bytes_of_a_long_output_and_a_line_that_says_how_large_it_was() {
    let (run, tools) = writing(Some(output_cap(10, OutputCut::Preview { bytes: 4 })), true).await;

    assert_eq!(
        results_sent(&run),
        serde_json::json!([
            result_of(
                "call_0",
                &["0123", "[output too large: the first 4 of 16 bytes]"]
            ),
            result_of("call_1", &[TEN_BYTES]),
        ])
    );
    let outcomes = run.finished.transcript.turns()[0].tool_calls();
    assert_eq!(outcomes[0].truncated_from_bytes, Some(16));
    assert_eq!(outcomes[1].truncated_from_bytes, None);
    assert_eq!(run.finished.summary.tool_calls_truncated, 1);
    assert_eq!(tools.kept(), [16, 10], "the executor held every byte");
}

/// The cap's worth of the start is kept whatever the cut, so the 10-byte
/// output arrives whole under a preview of 4 and is sent whole.
#[tokio::test]
async fn an_executor_that_keeps_what_its_call_says_to_gives_the_results_of_one_that_kept_it_all() {
    for (cut, keep, kept) in [
        (OutputCut::HeadTail, OutputKeep { head: 10, tail: 5 }, 15),
        (
            OutputCut::Preview { bytes: 4 },
            OutputKeep { head: 10, tail: 0 },
            10,
        ),
        (OutputCut::Head, OutputKeep { head: 10, tail: 0 }, 10),
    ] {
        let (whole, _) = writing(Some(output_cap(10, cut)), true).await;
        let (fed, tools) = writing(Some(output_cap(10, cut)), false).await;

        let keeps: Vec<_> = tools.taken().iter().map(|call| call.keep).collect();
        assert_eq!(keeps, [Some(keep), Some(keep)], "{cut:?}");
        assert_eq!(tools.kept(), [kept, 10], "{cut:?}");
        assert_eq!(results_sent(&fed), results_sent(&whole), "{cut:?}");
        assert_eq!(
            fed.finished.transcript.turns()[0].tool_calls(),
            whole.finished.transcript.turns()[0].tool_calls(),
            "{cut:?}"
        );
        assert_eq!(fed.finished.summary, whole.finished.summary, "{cut:?}");
    }
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

/// The cost is priced before the run is closed, from the state it stopped in,
/// so each state has to count the turn it's holding and the failed attempts
/// behind it. A run stopped at a response's calls holds a turn the transcript
/// doesn't have yet.
#[tokio::test]
async fn a_run_is_priced_on_all_it_spent_whichever_state_it_stopped_in() {
    let pricing = Pricing::new(Rates::new(4.0, 16.0, 0.5, 5.0).expect("ordinary rates"));
    let billed = || Answer::failing(overloaded().with_usage(tokens(70, 5)));
    let answered = || Answer::now(says("On it.", &["bash"], FinishReason::ToolUse));
    let stopped_at = [
        // The response called no tool.
        vec![
            billed(),
            Answer::now(says("Done.", &[], FinishReason::EndTurn)),
        ],
        // The response's calls were cut short, so they never ran.
        vec![
            billed(),
            Answer::now(says("On it.", &["bash"], FinishReason::MaxTokens)),
        ],
        // Waiting for a response, after a turn whose calls ran.
        vec![
            billed(),
            answered(),
            Answer::fails(ProviderErrorKind::Fatal),
        ],
    ];

    for script in stopped_at {
        let mut harness = Harness::new(script);
        harness.pricing = Some(pricing);

        let run = harness.run().await;

        let usage = run.finished.summary.outcome.usage;
        assert_eq!(usage, tokens(100, 20), "{:?}", run.stop_reason());
        assert_eq!(
            run.finished.summary.cost,
            pricing.cost(&tokens(170, 25)),
            "{:?}",
            run.stop_reason()
        );
    }
}

/// A provider that reports no cache count has left a gap, not a zero: the
/// gap reaches the outcome as it is, and the cost prices the input whole.
#[tokio::test]
async fn a_run_whose_provider_reported_no_cache_count_is_priced_on_its_whole_input() {
    let counts = TokenCounts {
        input: 1_000_000,
        output: 250_000,
        ..TokenCounts::default()
    };
    let mut harness = Harness::new(vec![Answer::now(reporting(
        &[],
        FinishReason::EndTurn,
        counts,
    ))]);
    harness.pricing = Some(Pricing::new(
        Rates::new(4.0, 16.0, 0.5, 5.0).expect("ordinary rates"),
    ));

    let run = harness.run().await;

    let summary = &run.finished.summary;
    assert_eq!(
        summary.outcome.usage,
        Usage {
            input_tokens: 1_000_000,
            output_tokens: 250_000,
            reasoning_output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        }
    );
    assert_eq!(
        summary.cost.map(lablet_model::Cost::usd),
        Some(4.0 + 4.0),
        "the input at 4 and a quarter of a million output tokens at 16"
    );
}

/// Several servers answer to one provider name and report different things,
/// and so can the calls of one run. The run's totals hold a count when any
/// call reported it, and the observer is told what the caller is handed.
#[tokio::test]
async fn a_run_reports_a_count_when_any_of_its_calls_reported_it() {
    let first = TokenCounts {
        input: 100,
        output: 20,
        reasoning: None,
        cache_read: Some(40),
        cache_write: None,
    };
    let second = TokenCounts {
        input: 150,
        output: 10,
        reasoning: None,
        cache_read: None,
        cache_write: None,
    };
    let run = Harness::new(vec![
        Answer::now(reporting(&["bash"], FinishReason::ToolUse, first)),
        Answer::now(reporting(&[], FinishReason::EndTurn, second)),
    ])
    .run()
    .await;

    let reported = Usage {
        input_tokens: 250,
        output_tokens: 30,
        reasoning_output_tokens: None,
        cache_read_tokens: Some(40),
        cache_write_tokens: None,
    };
    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.finished.summary.outcome.usage, reported);
    let turns = run.finished.transcript.turns();
    assert_eq!(turns[0].record().usage, Usage::from_inclusive(first));
    assert_eq!(turns[1].record().usage, Usage::from_inclusive(second));
    let events = run.observer.events();
    let Some(EventKind::RunFinished { summary, .. }) = events.last().map(|e| &e.kind) else {
        panic!("the run's last event is RunFinished");
    };
    assert_eq!(summary.outcome.usage, reported);
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
    harness.stop.max_turns = Some(nz(7));
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

    assert_eq!(run.finished.summary.max_turns, Some(nz(7)));
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
    harness.retry = retrying(RetrySettings {
        max_retries: 0,
        ..settings()
    });

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
    let run = Harness::new(vec![Answer::failing(ProviderError::new(
        ProviderErrorKind::ContextExhausted,
        "prompt is 205000 tokens, over the 200000 limit",
    ))])
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
    let run = Harness::new(vec![Answer::failing(ProviderError::new(
        ProviderErrorKind::Fatal,
        "404 no model named fake-2",
    ))])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::ProviderError);
    assert_eq!(run.error(), Some("404 no model named fake-2"));
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

// E8: a name the run doesn't offer is an error result, and a turn that made
// no other call is an invalid turn.

#[tokio::test]
async fn the_model_is_sent_an_error_result_for_a_name_the_run_does_not_offer() {
    let run = Harness::new(vec![
        Answer::now(says("One.", &["invented"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    let sent = run.provider.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[1].as_array().and_then(|messages| messages.last()),
        Some(&serde_json::json!({
            "user": {
                "tool_results": [{
                    "call_id": "call_0",
                    "content": [{ "text": "no tool named invented is offered by this run" }],
                    "is_error": true,
                }],
                "input": [],
            },
        })),
        "the second call's last message is the result of the first turn's call"
    );
}

#[tokio::test]
async fn a_turn_whose_only_call_names_no_tool_is_an_invalid_turn() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["invented"], FinishReason::ToolUse)),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    harness.stop.max_consecutive_invalid_turns = Some(nz(1));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::InvalidCallsExhausted);
    assert_eq!(run.turns(), 1);
    assert_eq!(run.provider.calls(), 1);
    assert_eq!(run.finished.summary.tool_calls_unknown, 1);
}

#[tokio::test]
async fn a_turn_that_names_no_tool_and_also_reaches_one_is_not_an_invalid_turn() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["invented", "bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.stop.max_consecutive_invalid_turns = Some(nz(1));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.turns(), 2);
    assert_eq!(run.finished.summary.tool_calls_unknown, 1);
}

// E12: the cap on invalid turns in a row.

const INVALID_TURNS_REACHED_THEIR_CAP: &str =
    "the turns in a row in which no call reached a tool reached their cap";

/// A response whose calls are `calls`, the n-th under the id `call_n`: a
/// name and the arguments the model wrote for it, parsed or not.
fn calling(calls: &[(&str, ToolInput)]) -> ProviderResponse {
    let content = calls
        .iter()
        .enumerate()
        .map(|(n, (tool, input))| {
            ContentBlock::ToolUse(ToolUse {
                id: ToolCallId::new(format!("call_{n}")).expect("a valid call id"),
                name: name(tool),
                input: input.clone(),
            })
        })
        .collect();
    ProviderResponse::new(content, tokens(100, 20), FinishReason::ToolUse, None, None)
        .expect("a test's response has distinct call ids")
}

fn parsed() -> ToolInput {
    ToolInput::Json(serde_json::json!({}))
}

fn unparsed() -> ToolInput {
    ToolInput::Unparsed("{\"cmd\": ".to_owned())
}

/// Three turns in a row that reach no tool, each its own way: a name no
/// tool has, arguments that don't parse, and two unknown names at once. The
/// second turn also makes `beside`, when there is one.
fn three_invalid_turns(beside: Option<(&str, ToolInput)>) -> Vec<Answer> {
    let mut second = vec![("bash", unparsed())];
    second.extend(beside);
    vec![
        Answer::now(calling(&[("invented", parsed())])),
        Answer::now(calling(&second)),
        Answer::now(calling(&[
            ("invented", parsed()),
            ("also_invented", parsed()),
        ])),
    ]
}

/// The status of every call of every turn, turn by turn.
fn statuses(run: &Run) -> Vec<Vec<&str>> {
    run.finished
        .transcript
        .turns()
        .iter()
        .map(|turn| {
            turn.tool_calls()
                .iter()
                .map(|outcome| outcome.status.as_str())
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn the_third_invalid_turn_in_a_row_stops_the_run_and_no_provider_call_follows() {
    let mut script = three_invalid_turns(None);
    script.push(Answer::now(says(
        "Never reached.",
        &[],
        FinishReason::EndTurn,
    )));
    let mut harness = Harness::new(script);
    harness.stop.max_consecutive_invalid_turns = Some(nz(3));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::InvalidCallsExhausted);
    assert_eq!(run.error(), Some(INVALID_TURNS_REACHED_THEIR_CAP));
    assert_eq!(run.turns(), 3);
    assert_eq!(
        run.provider.calls(),
        3,
        "the results of the third turn are never sent"
    );
    assert_eq!(
        statuses(&run),
        [
            vec!["unknown"],
            vec!["malformed_input"],
            vec!["unknown", "unknown"]
        ],
        "every call was answered, the third turn's among them"
    );
    assert_eq!(run.observer.names().last(), Some(&"RunFinished"));
}

#[tokio::test]
async fn a_turn_in_which_a_call_reached_a_tool_ends_the_count_whatever_the_tool_returned() {
    let returns: [(Scripted, &str); 4] = [
        (|| Answers::Text("ok".to_owned()), "ok"),
        (|| Answers::ToolError("1 failed".to_owned()), "tool_error"),
        (
            || Answers::Fails(crate::ToolErrorKind::Timeout, "took too long".to_owned()),
            "timeout",
        ),
        (
            || {
                Answers::Fails(
                    crate::ToolErrorKind::Failed,
                    "the server is gone".to_owned(),
                )
            },
            "failed",
        ),
    ];
    for (returned, reached) in returns {
        let mut script = three_invalid_turns(Some(("read_file", parsed())));
        script.push(Answer::now(calling(&[("invented", parsed())])));
        script.push(Answer::now(says("Done.", &[], FinishReason::EndTurn)));
        let mut harness = Harness::new(script);
        harness.stop.max_consecutive_invalid_turns = Some(nz(3));
        harness.tools = vec![Arc::new(
            FakeTools::new(
                Arc::clone(&harness.clock),
                vec![spec("bash"), spec("read_file")],
            )
            .answers("read_file", returned()),
        )];

        let run = harness.run().await;

        assert_eq!(
            run.stop_reason(),
            StopReason::Completed,
            "{reached}: the two invalid turns after it are a count of two, not of three"
        );
        assert_eq!(run.turns(), 5, "{reached}");
        assert_eq!(
            statuses(&run),
            [
                vec!["unknown"],
                vec!["malformed_input", reached],
                vec!["unknown", "unknown"],
                vec!["unknown"],
                vec![],
            ],
            "{reached}: every call was answered, and the last turn made none"
        );
    }
}

#[tokio::test]
async fn one_response_that_makes_three_invalid_calls_is_one_invalid_turn() {
    let mut harness = Harness::new(vec![
        Answer::now(calling(&[
            ("invented", parsed()),
            ("bash", unparsed()),
            ("also_invented", parsed()),
        ])),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.stop.max_consecutive_invalid_turns = Some(nz(3));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.turns(), 2);
    assert_eq!(
        statuses(&run)[0],
        ["unknown", "malformed_input", "unknown"],
        "the model is told of all three before anything is counted against it"
    );
}

#[tokio::test]
async fn without_a_cap_on_invalid_turns_every_such_run_goes_on() {
    let beside = [None, Some(("read_file", parsed()))];
    for beside in beside {
        let mut script = three_invalid_turns(beside);
        script.extend((0..3).map(|_| Answer::now(calling(&[("invented", parsed())]))));
        script.push(Answer::now(says("Done.", &[], FinishReason::EndTurn)));
        let mut harness = Harness::new(script);
        harness.stop.max_consecutive_invalid_turns = None;

        let run = harness.run().await;

        assert_eq!(run.stop_reason(), StopReason::Completed);
        assert_eq!(run.turns(), 7);
        assert_eq!(run.error(), None);
    }
}

// L14: a run without a turn cap.

#[tokio::test]
async fn a_run_without_a_turn_cap_goes_on_until_the_model_ends_it() {
    let mut script: Vec<Answer> = (0..40)
        .map(|_| Answer::now(says("Again.", &["bash"], FinishReason::ToolUse)))
        .collect();
    script.push(Answer::now(says("Done.", &[], FinishReason::EndTurn)));
    let mut harness = Harness::new(script);
    harness.stop.max_turns = None;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.turns(), 41);
    assert_eq!(run.provider.calls(), 41);
    assert_eq!(run.finished.summary.outcome.tool_calls, 40);
    assert_eq!(run.finished.summary.max_turns, None);
    let events = run.observer.events();
    let Some(EventKind::RunFinished { summary, .. }) = events.last().map(|e| &e.kind) else {
        panic!("the run's last event is RunFinished");
    };
    assert_eq!(
        summary.max_turns, None,
        "what an observer is handed holds no cap either"
    );
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
            ProviderError::new(ProviderErrorKind::Retryable, "overloaded"),
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
        Answer::failing(ProviderError::new(
            ProviderErrorKind::Retryable,
            "overloaded",
        )),
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
    let run = Harness::new(vec![Answer::failing(ProviderError::new(
        ProviderErrorKind::Fatal,
        "bad request",
    ))])
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
                concurrency: ToolConcurrency::Exclusive,
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
                concurrency: ToolConcurrency::Exclusive,
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

// L12: a `task_complete` call whose arguments didn't parse completes nothing.

/// In explicit mode the structured result is what the run is for, so a
/// `task_complete` call whose arguments didn't parse isn't a completion. It's
/// answered like any other call with bad arguments, the response's other
/// calls run, and the model can call it again.
#[tokio::test]
async fn a_task_complete_call_whose_arguments_did_not_parse_is_answered_and_the_run_goes_on() {
    let argument = serde_json::json!({ "passed": true });
    let first = ProviderResponse::new(
        vec![
            ContentBlock::ToolUse(ToolUse {
                id: ToolCallId::new("call_0").expect("a valid call id"),
                name: name("bash"),
                input: ToolInput::Json(serde_json::json!({ "command": "cargo test" })),
            }),
            ContentBlock::ToolUse(ToolUse {
                id: ToolCallId::new("call_1").expect("a valid call id"),
                name: ToolName::task_complete(),
                input: ToolInput::Unparsed("{\"passed\": tr".to_owned()),
            }),
        ],
        Usage::default(),
        FinishReason::ToolUse,
        None,
        None,
    )
    .expect("distinct call ids");
    let second = ProviderResponse::new(
        vec![ContentBlock::ToolUse(ToolUse {
            id: ToolCallId::new("call_2").expect("a valid call id"),
            name: ToolName::task_complete(),
            input: ToolInput::Json(argument.clone()),
        })],
        Usage::default(),
        FinishReason::ToolUse,
        None,
        None,
    )
    .expect("one call");
    let mut harness = Harness::new(vec![Answer::now(first), Answer::now(second)]);
    harness.completion = CompletionMode::Explicit;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.turns(), 2);
    let outcome = &run.finished.summary.outcome;
    assert_eq!(outcome.result().structured, Some(argument));
    assert_eq!(
        outcome.tool_calls, 2,
        "turn 1's calls were both answered; turn 2's completion call is intercepted"
    );
    let answered = run.finished.transcript.turns()[0].tool_calls();
    assert_eq!(answered[0].status.as_str(), "ok", "the other call ran");
    assert_eq!(answered[1].status, ToolCallStatus::MalformedInput);
    assert!(answered[1].status.is_error(), "it counts as a tool error");
    assert_eq!(run.finished.summary.tool_calls_errors, 1);
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
            reasoning: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
        }),
        FinishReason::ToolUse,
        None,
        None,
    )
    .expect("a test's response has distinct call ids")
}

// L13: a `task_complete` call made beside other calls completes nothing.

const CALL_IT_ON_ITS_OWN: &str =
    "call task_complete on its own, once your other calls have returned";

fn completing(argument: serde_json::Value) -> (&'static str, ToolInput) {
    (CompletionMode::TASK_COMPLETE, ToolInput::Json(argument))
}

/// An explicit run over an executor that serves `write_file`, and the
/// executor, so a scenario can say which calls reached it.
fn explicit_with_write_file(script: Vec<Answer>) -> (Harness, Arc<FakeTools>) {
    let mut harness = Harness::new(script);
    harness.completion = CompletionMode::Explicit;
    let tools = Arc::new(FakeTools::new(
        Arc::clone(&harness.clock),
        vec![spec("write_file")],
    ));
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];
    (harness, tools)
}

/// The script of L13: `write_file` beside the completion call, then the
/// completion call twice, then the completion call alone. Each completion
/// call has an argument of its own, so the result says which one was taken.
fn completing_beside_other_calls() -> Vec<Answer> {
    vec![
        Answer::now(calling(&[
            ("write_file", parsed()),
            completing(serde_json::json!({ "answer": "first" })),
        ])),
        Answer::now(calling(&[
            completing(serde_json::json!({ "answer": "second" })),
            completing(serde_json::json!({ "answer": "third" })),
        ])),
        Answer::now(calling(&[completing(
            serde_json::json!({ "answer": "alone" }),
        )])),
    ]
}

/// The name of every call an executor was handed, in order.
fn reached(tools: &FakeTools) -> Vec<String> {
    tools
        .taken()
        .iter()
        .map(|call| call.name.to_string())
        .collect()
}

/// A run that completed on a response that also asked for work would report
/// the work as done. So the completion call is answered `rejected`, the
/// response's other calls run, and the run completes when the call comes
/// alone.
#[tokio::test]
async fn a_completion_call_beside_other_calls_is_rejected_and_the_others_run() {
    let (harness, tools) = explicit_with_write_file(completing_beside_other_calls());

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.turns(), 3);
    assert_eq!(run.provider.calls(), 3);
    let outcome = &run.finished.summary.outcome;
    assert_eq!(
        outcome.result().structured,
        Some(serde_json::json!({ "answer": "alone" })),
        "the argument of the call that came alone, and of no call before it"
    );
    assert_eq!(
        statuses(&run),
        [vec!["ok", "rejected"], vec!["rejected", "rejected"], vec![]],
        "the call that completed the run was intercepted, so it has no outcome"
    );
    assert_eq!(
        reached(&tools),
        ["write_file"],
        "the other call ran, and no completion call reached an executor"
    );

    let rejected = &run.finished.transcript.turns()[0].tool_calls()[1];
    assert_eq!(rejected.status, ToolCallStatus::Rejected);
    assert_eq!(rejected.call_id.as_str(), "call_1");
    assert_eq!(
        rejected.content,
        [lablet_model::ToolResultContent::Text(
            CALL_IT_ON_ITS_OWN.to_owned()
        )]
    );
    assert_eq!(rejected.latency_ms, 0, "nothing ran, so nothing took time");
    for rejected in run.finished.transcript.turns()[1].tool_calls() {
        assert_eq!(rejected.status, ToolCallStatus::Rejected);
        assert_eq!(
            rejected.content,
            [lablet_model::ToolResultContent::Text(
                CALL_IT_ON_ITS_OWN.to_owned()
            )]
        );
    }
}

/// A rejected call is answered, so it's in every total an answered call is
/// in: the count, the errors, and the share of the tool it named. It named a
/// tool the run offers, so it isn't a call to an unknown one.
#[tokio::test]
async fn a_rejected_call_counts_in_the_totals_and_against_task_complete() {
    let (harness, _) = explicit_with_write_file(completing_beside_other_calls());

    let run = harness.run().await;

    let summary = &run.finished.summary;
    assert_eq!(
        summary.outcome.tool_calls, 4,
        "two calls in each of the first two turns; the third turn's is intercepted"
    );
    assert_eq!(summary.tool_calls_errors, 3);
    assert_eq!(summary.tool_calls_unknown, 0);
    assert_eq!(
        summary.per_tool,
        std::collections::BTreeMap::from([
            (
                ToolName::task_complete(),
                lablet_model::ToolStats {
                    calls: 3,
                    errors: 3,
                    latency_ms: 0,
                }
            ),
            (
                name("write_file"),
                lablet_model::ToolStats {
                    calls: 1,
                    errors: 0,
                    latency_ms: 0,
                }
            ),
        ])
    );
}

/// The model is told, as an error result, so it can make the call again once
/// the results of its other calls are in front of it.
#[tokio::test]
async fn the_model_is_sent_the_rejection_as_an_error_result_beside_the_other_results() {
    let (harness, _) = explicit_with_write_file(completing_beside_other_calls());

    let run = harness.run().await;

    let sent = run.provider.sent();
    assert_eq!(
        sent[1].as_array().and_then(|messages| messages.last()),
        Some(&serde_json::json!({
            "user": {
                "tool_results": [
                    {
                        "call_id": "call_0",
                        "content": [{ "text": "write_file ran" }],
                        "is_error": false,
                    },
                    {
                        "call_id": "call_1",
                        "content": [{ "text": CALL_IT_ON_ITS_OWN }],
                        "is_error": true,
                    },
                ],
                "input": [],
            },
        }))
    );
}

/// A rejected call is a call the loop answered, so an observer sees it begin
/// and end as it sees a call to an unknown name. Only the call that completes
/// the run is never announced. Every call runs alone here, because the events
/// of calls that run together come in no order a test may rely on.
#[tokio::test]
async fn a_rejected_call_is_announced_and_the_intercepted_one_is_not() {
    let (mut harness, _) = explicit_with_write_file(completing_beside_other_calls());
    harness.calls.max_concurrent_tool_calls = nz(1);

    let run = harness.run().await;

    let events = run.observer.events();
    let started: Vec<(u32, &str, &str, Option<&ToolSource>)> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ToolCallStarted {
                turn,
                call_id,
                name,
                source,
                ..
            } => Some((*turn, call_id.as_str(), name.as_str(), source.as_ref())),
            _ => None,
        })
        .collect();
    let finished: Vec<(u32, &str, &str, bool)> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ToolCallFinished {
                turn,
                call_id,
                status,
                mcp,
                ..
            } => Some((*turn, call_id.as_str(), status.as_str(), mcp.is_some())),
            _ => None,
        })
        .collect();

    let builtin = Some(&ToolSource::Builtin);
    assert_eq!(
        started,
        [
            (1, "call_0", "write_file", builtin),
            (1, "call_1", "task_complete", builtin),
            (2, "call_0", "task_complete", builtin),
            (2, "call_1", "task_complete", builtin),
        ],
        "turn 3's call completed the run, so it never began"
    );
    assert_eq!(
        finished,
        [
            (1, "call_0", "ok", false),
            (1, "call_1", "rejected", false),
            (2, "call_0", "rejected", false),
            (2, "call_1", "rejected", false),
        ]
    );
}

/// Both calls of L13's second turn were ones the model got wrong, so the
/// turn is an invalid one. Its first turn isn't: a call of it reached a tool.
#[tokio::test]
async fn a_turn_whose_calls_were_all_rejected_is_an_invalid_turn() {
    let (mut harness, _) = explicit_with_write_file(completing_beside_other_calls());
    harness.stop.max_consecutive_invalid_turns = Some(nz(1));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::InvalidCallsExhausted);
    assert_eq!(run.error(), Some(INVALID_TURNS_REACHED_THEIR_CAP));
    assert_eq!(
        run.turns(),
        2,
        "the first turn's write_file ran, so the count began at the second"
    );
    assert_eq!(run.provider.calls(), 2, "no provider call follows the stop");
    assert_eq!(
        statuses(&run),
        [vec!["ok", "rejected"], vec!["rejected", "rejected"]]
    );
    assert_eq!(run.finished.summary.outcome.result().structured, None);
}

/// Where the completion call sits among a response's calls decides nothing:
/// one made first is rejected as one made last is, and the call after it
/// still runs.
#[tokio::test]
async fn a_completion_call_made_ahead_of_another_call_is_rejected_too() {
    let (harness, tools) = explicit_with_write_file(vec![
        Answer::now(calling(&[
            completing(serde_json::json!({ "answer": "early" })),
            ("write_file", parsed()),
        ])),
        Answer::now(says("Stopping here.", &[], FinishReason::EndTurn)),
    ]);

    let run = harness.run().await;

    assert_eq!(
        run.stop_reason(),
        StopReason::EndedWithoutCompletion,
        "the rejected call completed nothing, and no other call followed it"
    );
    assert_eq!(run.turns(), 2);
    assert_eq!(statuses(&run), [vec!["rejected", "ok"], vec![]]);
    assert_eq!(reached(&tools), ["write_file"]);
    assert_eq!(run.finished.summary.outcome.result().structured, None);
}

// L15: only explicit mode intercepts the name.

/// A natural run over an executor that serves `bash` and a tool of its own
/// named `task_complete`, and the executor.
fn natural_serving_task_complete(script: Vec<Answer>) -> (Harness, Arc<FakeTools>) {
    let harness = Harness::new(script);
    let tools = Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash"), spec(CompletionMode::TASK_COMPLETE)],
        )
        .answers(
            CompletionMode::TASK_COMPLETE,
            Answers::Text("noted".to_owned()),
        ),
    );
    let harness = Harness {
        tools: vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>],
        ..harness
    };
    (harness, tools)
}

/// Natural mode has no completion call, so the name is the executor's to
/// serve and a call to it is a call to run, whatever it was made beside.
#[tokio::test]
async fn in_natural_mode_a_tool_named_task_complete_runs_beside_another_call() {
    let (harness, tools) = natural_serving_task_complete(vec![
        Answer::now(calling(&[
            ("bash", parsed()),
            completing(serde_json::json!({ "answer": "not a result" })),
        ])),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.turns(), 2);
    assert_eq!(
        statuses(&run),
        [vec!["ok", "ok"], vec![]],
        "both calls ran, and neither was rejected"
    );
    assert_eq!(reached(&tools), ["bash", "task_complete"]);
    assert_eq!(
        tools.taken()[1].input,
        serde_json::json!({ "answer": "not a result" }),
        "the executor is handed the arguments the model wrote"
    );
    let answered = &run.finished.transcript.turns()[0].tool_calls()[1];
    assert_eq!(
        answered.content,
        [lablet_model::ToolResultContent::Text("noted".to_owned())],
        "the model is sent what the tool returned"
    );
    let summary = &run.finished.summary;
    assert_eq!(summary.outcome.tool_calls, 2);
    assert_eq!(summary.tool_calls_errors, 0);
    assert_eq!(
        summary.outcome.result().structured,
        None,
        "its argument is a tool's input, not the run's result"
    );
}

/// The same call made alone would complete an explicit run at point R. In
/// natural mode it's a tool call like any other: it runs, and the run goes
/// on to the response that ends it.
#[tokio::test]
async fn in_natural_mode_a_tool_named_task_complete_runs_when_called_alone() {
    let (harness, tools) = natural_serving_task_complete(vec![
        Answer::now(calling(&[completing(
            serde_json::json!({ "answer": "not a result" }),
        )])),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.turns(), 2, "the call didn't end the run");
    assert_eq!(run.provider.calls(), 2);
    assert_eq!(statuses(&run), [vec!["ok"], vec![]]);
    assert_eq!(reached(&tools), ["task_complete"]);
    assert_eq!(run.finished.summary.outcome.tool_calls, 1);
    assert_eq!(run.finished.summary.outcome.result().structured, None);
}

// L11: a turn's tool calls run in groups.

/// A run whose first response calls `read_file`, `read_file`, `write_file`,
/// and `read_file`, on an executor whose calls yield part-way, with reads
/// shared and writes exclusive; and when each call started and ended.
async fn grouped(max_concurrent: u32) -> (Run, Vec<String>) {
    let mut harness = Harness::new(vec![
        Answer::now(says(
            "Looking.",
            &["read_file", "read_file", "write_file", "read_file"],
            FinishReason::ToolUse,
        )),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    let tools = Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![
                ToolSpec {
                    concurrency: ToolConcurrency::Shared,
                    ..spec("read_file")
                },
                spec("write_file"),
            ],
        )
        .yielding(),
    );
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];
    harness.calls.max_concurrent_tool_calls = nz(max_concurrent);

    let run = harness.run().await;
    (run, tools.spans())
}

fn at(spans: &[String], span: &str) -> usize {
    spans
        .iter()
        .position(|s| s == span)
        .expect("every call starts and ends")
}

#[tokio::test]
async fn consecutive_reads_run_together_and_a_write_runs_alone() {
    let (run, spans) = grouped(10).await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert!(
        at(&spans, "+call_1") < at(&spans, "-call_0"),
        "the first two reads overlap: {spans:?}"
    );
    assert!(
        at(&spans, "-call_0").max(at(&spans, "-call_1")) < at(&spans, "+call_2"),
        "the write starts after both reads have ended: {spans:?}"
    );
    assert!(
        at(&spans, "-call_2") < at(&spans, "+call_3"),
        "the read after the write starts after it ends: {spans:?}"
    );
    let ids: Vec<&str> = run.finished.transcript.turns()[0]
        .tool_calls()
        .iter()
        .map(|outcome| outcome.call_id.as_str())
        .collect();
    assert_eq!(ids, ["call_0", "call_1", "call_2", "call_3"]);
}

/// A cap of 1 is the loop before calls could run together, which is what a
/// test that asserts events exactly relies on. The outcomes are compared
/// rather than the whole transcript: the fakes share one clock, so calls that
/// overlap can't be given timings of their own.
#[tokio::test]
async fn a_cap_of_one_runs_every_call_alone_and_records_the_same_outcomes() {
    let (alone, spans) = grouped(1).await;
    let (together, _) = grouped(10).await;

    assert_eq!(
        spans,
        [
            "+call_0", "-call_0", "+call_1", "-call_1", "+call_2", "-call_2", "+call_3", "-call_3"
        ]
    );
    let tool_events: Vec<&str> = alone
        .observer
        .names()
        .into_iter()
        .filter(|name| name.starts_with("ToolCall"))
        .collect();
    assert_eq!(
        tool_events,
        ["ToolCallStarted", "ToolCallFinished"].repeat(4),
        "each call's events close before the next call's open"
    );
    let outcomes = |run: &Run| {
        run.finished.transcript.turns()[0]
            .tool_calls()
            .iter()
            .map(|outcome| {
                (
                    outcome.call_id.clone(),
                    outcome.status.clone(),
                    outcome.content.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(outcomes(&alone), outcomes(&together));
}

// E11: a retry is a provider call, so it's polled for cancellation.

#[tokio::test]
async fn a_run_cancelled_during_a_backoff_makes_no_further_attempt() {
    let mut harness = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    // Answered before the attempt and again once it failed, then true at the
    // poll after the backoff.
    harness.cancel = Arc::new(FakeCancel::after(2));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.provider.calls(), 1, "no attempt after the backoff");
    assert_eq!(run.clock.sleeps(), [ms(100)]);
    assert_eq!(run.turns(), 0);
    // The failure reported the decision to retry, made before the wait; the
    // run's end says why the retry never came.
    let events = run.observer.events();
    let [.., failed, finished] = events.as_slice() else {
        panic!("a run that failed once has at least two events");
    };
    assert!(matches!(
        failed.kind,
        EventKind::ProviderCallFailed {
            retry: Some(wait),
            ..
        } if wait == ms(100)
    ));
    assert!(matches!(finished.kind, EventKind::RunFinished { .. }));
}

#[tokio::test]
async fn a_run_cancelled_while_an_attempt_was_failing_stops_before_the_wait() {
    let mut harness = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    // Answered before the attempt, then true at the poll once it failed.
    harness.cancel = Arc::new(FakeCancel::after(1));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.provider.calls(), 1);
    assert_eq!(run.clock.sleeps(), [], "a cancelled run waits for nothing");
    let [(_, retry)] = run.failures().try_into().expect("one attempt failed");
    assert_eq!(retry, None, "the call was over when the attempt failed");
}

// E13: the server's hint.

#[tokio::test]
async fn a_retry_waits_as_long_as_the_server_asked_when_that_is_longer_than_the_backoff() {
    let run = Harness::new(vec![
        Answer::failing(overloaded().with_retry_after(Duration::from_secs(5))),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.clock.sleeps(), [Duration::from_secs(5)]);
    let [(error, retry)] = run.failures().try_into().expect("one attempt failed");
    assert_eq!(error.retry_after, Some(Duration::from_secs(5)));
    assert_eq!(retry, Some(Duration::from_secs(5)));
    assert_eq!(
        run.finished.transcript.turns()[0].record().started_ms,
        5_000,
        "the attempt that answered began once the wait was over"
    );
}

#[tokio::test]
async fn a_retry_waits_the_backoff_when_the_server_asked_for_less() {
    let run = Harness::new(vec![
        Answer::failing(overloaded().with_retry_after(ms(40))),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.clock.sleeps(), [ms(100)]);
}

#[tokio::test]
async fn a_hint_longer_than_the_cap_on_hints_exhausts_the_retries_without_waiting() {
    let mut harness = Harness::new(vec![
        Answer::failing(overloaded().with_retry_after(Duration::from_secs(120))),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    harness.retry = retrying(RetrySettings {
        hint_max: Duration::from_secs(60),
        ..settings()
    });

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::RetriesExhausted);
    assert_eq!(run.error(), Some("529 overloaded"));
    assert_eq!(run.provider.calls(), 1, "three retries were left");
    assert_eq!(run.clock.sleeps(), [], "lablet won't make the wait");
    assert_eq!(run.finished.summary.provider_retries, 0);
    let [(_, retry)] = run.failures().try_into().expect("one attempt failed");
    assert_eq!(retry, None);
}

// E14: what a failed attempt was billed.

#[tokio::test]
async fn what_a_failed_attempt_used_is_kept_apart_from_the_usage_and_priced_with_it() {
    let pricing = Pricing::new(Rates::new(4.0, 16.0, 0.5, 5.0).expect("ordinary rates"));
    let mut harness = Harness::new(vec![
        Answer::failing(overloaded().with_usage(tokens(70, 5))),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.pricing = Some(pricing);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    let summary = &run.finished.summary;
    assert_eq!(
        summary.outcome.usage,
        tokens(100, 20),
        "the call that succeeded, alone"
    );
    assert_eq!(summary.failed_usage, Some(tokens(70, 5)));
    assert_eq!(summary.cost, pricing.cost(&tokens(170, 25)));
    assert_ne!(summary.cost, pricing.cost(&tokens(100, 20)));
    assert_eq!(
        run.finished.transcript.turns()[0].record().usage,
        tokens(100, 20),
        "a turn's usage is that of the attempt that answered"
    );
    let [(error, _)] = run.failures().try_into().expect("one attempt failed");
    assert_eq!(error.usage, Some(tokens(70, 5)));
}

#[tokio::test]
async fn a_run_none_of_whose_failed_attempts_reported_usage_has_no_failed_usage() {
    let run = Harness::new(vec![
        Answer::failing(overloaded()),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.finished.summary.failed_usage, None);
}

/// The failed attempt used 75 tokens and the turn 120, so only the two
/// together reach a budget of 190.
#[tokio::test]
async fn the_token_budget_counts_what_a_failed_attempt_used() {
    let script = || {
        vec![
            Answer::failing(overloaded().with_usage(tokens(70, 5))),
            Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
            Answer::now(says("Done.", &[], FinishReason::EndTurn)),
        ]
    };
    let mut reached = Harness::new(script());
    reached.stop.max_total_tokens = Some(190);
    let mut spared = Harness::new(script());
    spared.stop.max_total_tokens = Some(196);

    let reached = reached.run().await;
    let spared = spared.run().await;

    assert_eq!(reached.stop_reason(), StopReason::MaxTotalTokens);
    assert_eq!(reached.provider.calls(), 2, "no call after the tool phase");
    assert_eq!(reached.turns(), 1);
    assert_eq!(spared.stop_reason(), StopReason::Completed);
    assert_eq!(spared.provider.calls(), 3);
}

// E15: a rejected key.

#[tokio::test]
async fn a_rejected_key_fails_the_run_at_once_and_the_failed_attempt_says_auth() {
    let run = Harness::new(vec![
        Answer::failing(ProviderError::new(
            ProviderErrorKind::Auth,
            "401 invalid x-api-key",
        )),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.stop_reason(), StopReason::ProviderError);
    assert_eq!(run.error(), Some("401 invalid x-api-key"));
    assert_eq!(run.provider.calls(), 1);
    assert_eq!(run.clock.sleeps(), [], "nothing was waited for");
    assert_eq!(run.finished.summary.provider_retries, 0);
    assert_eq!(run.turns(), 0);
    let [(error, retry)] = run.failures().try_into().expect("one attempt failed");
    assert_eq!(error.kind, ProviderErrorKind::Auth);
    assert_eq!(retry, None);
    assert_eq!(
        run.observer.names().last(),
        Some(&"RunFinished"),
        "a run the provider turned away is still a run, with its wide event"
    );
}

// E17: jitter.

/// Three failures and then an answer, with waits spread by up to a quarter,
/// under the run id `run_id`.
async fn spread(run_id: &str) -> Vec<Duration> {
    let mut harness = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.retry = retrying(RetrySettings {
        jitter: 0.25,
        ..settings()
    });
    harness.context.run_id = RunId::new(run_id).expect("a valid run id");

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    let waits: Vec<Duration> = run
        .failures()
        .into_iter()
        .filter_map(|(_, retry)| retry)
        .collect();
    assert_eq!(run.clock.sleeps(), waits, "the loop waits what it reported");
    waits
}

#[tokio::test]
async fn each_wait_is_the_backoff_and_up_to_a_quarter_more_and_two_runs_wait_differently() {
    let one = spread("01K5F3Z8Q4X9T2M7B6W1R0VNEC").await;
    let other = spread("01K5F3Z8Q4X9T2M7B6W1R0VNED").await;
    let again = spread("01K5F3Z8Q4X9T2M7B6W1R0VNEC").await;

    for waits in [&one, &other] {
        assert_eq!(waits.len(), 3);
        for (wait, backoff) in waits.iter().zip([100, 200, 400]) {
            assert!(
                (ms(backoff)..ms(backoff + backoff / 4)).contains(wait),
                "{wait:?} after a backoff of {backoff} ms"
            );
        }
    }
    for (wait, others) in one.iter().zip(&other) {
        assert_ne!(wait, others, "runs that fail together come back apart");
    }
    assert_eq!(one, again, "a replayed run waits as it did");
}

#[tokio::test]
async fn without_jitter_each_wait_is_the_backoff_exactly() {
    let run = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(run.clock.sleeps(), [ms(100), ms(200), ms(400)]);
}

/// The jitter of a wait is read from the run, the turn and the attempt, in
/// that order, so no two waits of a run share one.
#[tokio::test]
async fn the_salt_of_a_wait_is_of_the_run_the_turn_and_the_attempt_that_failed() {
    let spread = RetrySettings {
        jitter: 0.25,
        ..settings()
    };
    let mut harness = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.retry = retrying(spread);
    let run_id = harness.context.run_id.clone();

    let run = harness.run().await;

    let wait = |turn, attempt| {
        retrying(spread)
            .next(
                attempt,
                ProviderErrorKind::Retryable,
                None,
                run_id.salt(turn, attempt),
            )
            .expect("the call has retries left")
    };
    assert_eq!(run.clock.sleeps(), [wait(1, 1), wait(1, 2), wait(2, 1)]);
    assert_ne!(wait(1, 2), wait(2, 1));
    assert_ne!(wait(1, 1), wait(2, 1));
}

#[tokio::test]
async fn a_wait_the_server_asked_for_is_spread_too() {
    let mut harness = Harness::new(vec![
        Answer::failing(overloaded().with_retry_after(Duration::from_secs(5))),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.retry = retrying(RetrySettings {
        jitter: 0.25,
        ..settings()
    });

    let run = harness.run().await;

    let [wait] = run.clock.sleeps().try_into().expect("one wait");
    assert!(wait > Duration::from_secs(5), "{wait:?}");
    assert!(wait < ms(6_250), "{wait:?}");
}

// E18: point A is asked once an attempt has failed, before the retry policy.

#[tokio::test]
async fn a_failed_attempt_that_used_up_the_run_s_time_is_a_timeout_with_a_retry_left_or_without() {
    for max_retries in [0, 3] {
        let mut harness = Harness::new(vec![
            Answer::Fails(overloaded(), Duration::from_secs(10)),
            Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
        ]);
        harness.stop.timeout = Duration::from_secs(10);
        harness.retry = retrying(RetrySettings {
            max_retries,
            ..settings()
        });

        let run = harness.run().await;

        assert_eq!(run.stop_reason(), StopReason::Timeout, "{max_retries}");
        assert_eq!(run.error(), None, "{max_retries}");
        assert_eq!(run.provider.calls(), 1, "{max_retries}");
        assert_eq!(run.clock.sleeps(), [], "{max_retries}");
        let [(_, retry)] = run.failures().try_into().expect("one attempt failed");
        assert_eq!(retry, None, "{max_retries}");
    }
}

#[tokio::test]
async fn a_failed_attempt_whose_usage_reaches_the_token_budget_is_the_last_attempt() {
    for max_retries in [0, 3] {
        let mut harness = Harness::new(vec![
            Answer::failing(overloaded().with_usage(tokens(140, 10))),
            Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
        ]);
        harness.stop.max_total_tokens = Some(150);
        harness.retry = retrying(RetrySettings {
            max_retries,
            ..settings()
        });

        let run = harness.run().await;

        assert_eq!(
            run.stop_reason(),
            StopReason::MaxTotalTokens,
            "{max_retries}"
        );
        assert_eq!(run.provider.calls(), 1, "{max_retries}");
        assert_eq!(run.clock.sleeps(), [], "{max_retries}");
        assert_eq!(run.finished.summary.failed_usage, Some(tokens(140, 10)));
        assert_eq!(run.finished.summary.outcome.usage, Usage::default());
        let [(_, retry)] = run.failures().try_into().expect("one attempt failed");
        assert_eq!(retry, None, "{max_retries}");
    }
}

/// Only a failure that could be retried is held to point A. One that
/// couldn't is why the run ended, however long the attempt took.
#[tokio::test]
async fn a_failure_no_attempt_could_answer_ends_the_run_as_itself_though_the_time_is_up() {
    for (kind, reason) in [
        (
            ProviderErrorKind::ContextExhausted,
            StopReason::ContextExhausted,
        ),
        (ProviderErrorKind::Auth, StopReason::ProviderError),
        (ProviderErrorKind::Fatal, StopReason::ProviderError),
    ] {
        let mut harness = Harness::new(vec![Answer::Fails(
            ProviderError::new(kind, "the provider's own words"),
            Duration::from_secs(10),
        )]);
        harness.stop.timeout = Duration::from_secs(10);

        let run = harness.run().await;

        assert_eq!(run.stop_reason(), reason, "{kind}");
        assert_eq!(run.error(), Some("the provider's own words"), "{kind}");
    }
}
