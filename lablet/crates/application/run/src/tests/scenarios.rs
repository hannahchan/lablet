//! The loop, driven end to end against fakes, and read back from the spans
//! and records it emits through the SDK's in-memory exporters.
//!
//! What a run's spans and records don't carry has no scenario here: the
//! whole `RunContext` of a run's start, the request's `api`, `thinking` and
//! `cache_scope`, the name of an MCP tool's server and the wait a server
//! asked for are all read from the summary a run returns, which the
//! summary's own scenarios hold, since no signal of the loop's is the place
//! for them. The run's own span and its one row are the composition root's.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lablet_model::{
    CacheScope, CompletionMode, ContentBlock, Effort, Endpoint, FinishReason, ModelRef, OutputCap,
    OutputCut, OutputKeep, Prompts, ProviderApi, ProviderErrorKind, ProviderResponse, Rates,
    RequestParams, RunContext, RunId, RunLabels, RunSummary, StopReason, Thinking, TokenCounts,
    ToolCallEnd, ToolCallId, ToolCallStatus, ToolConcurrency, ToolInput, ToolName,
    ToolResultContent, ToolSource, ToolSpec, ToolUse, Usage,
};
use lablet_policy::{Pricing, RetryPolicy, RetrySettings, StopPolicy};
use opentelemetry::global::BoxedTracer;
use opentelemetry::logs::{AnyValue, LoggerProvider as _};
use opentelemetry::trace::noop::NoopTracer;
use opentelemetry::trace::{
    FutureExt as _, SpanContext, SpanKind, Status, TraceContextExt as _, Tracer as _,
    TracerProvider as _,
};
use opentelemetry::{Array, Context, StringValue, Value};
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLogRecord, SdkLoggerProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use serde_json::json;

use super::fakes::{Answer, Answers, FakeCancel, FakeClock, FakeProvider, FakeTools};
use crate::telemetry::Bridge;
use crate::telemetry::generated::{
    GenAiClientInferenceOperationDetails, GenAiClientOperationException, LabletChat,
    LabletExecuteTool, LabletRetry, key,
};
use crate::{CallLimits, ERROR_MESSAGE_MAX_BYTES, ProviderError, RunService, ToolFilter, ToolSet};

const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

const fn us(micros: u64) -> Duration {
    Duration::from_micros(micros)
}

fn nz(count: u32) -> std::num::NonZeroU32 {
    std::num::NonZeroU32::new(count).expect("a cap in these tests is above zero")
}

fn name(value: &str) -> ToolName {
    ToolName::new(value).expect("a test's tool name is valid")
}

fn spec(value: &str, source: ToolSource) -> ToolSpec {
    ToolSpec {
        name: name(value),
        description: format!("The {value} tool."),
        input_schema: serde_json::json!({ "type": "object" }),
        source,
        concurrency: ToolConcurrency::Exclusive,
    }
}

fn mcp(server: &str) -> ToolSource {
    ToolSource::Mcp {
        server: server.to_owned(),
    }
}

/// The id of every run here, unless a test gives its run another.
const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";

/// When every run here started: a fraction of a millisecond past a whole
/// one, so a time cut to the millisecond anywhere is a time that differs.
fn started() -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(1_790_000_000_000_123_456)
}

/// The digest every run's context gives its config.
const CONFIG_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn context() -> RunContext {
    RunContext {
        run_id: RunId::new(RUN).expect("a valid run id"),
        labels: RunLabels::default(),
        started: started(),
        config_digest: lablet_model::ConfigDigest::new(CONFIG_DIGEST).expect("a digest"),
        agent_version: "0.1.0".to_owned(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    }
}

fn prompts() -> Prompts {
    Prompts::new("You fix tests.", "Fix the failing test.").expect("the task isn't blank")
}

fn model() -> ModelRef {
    ModelRef {
        api: ProviderApi::Script,
        name: "fake-1".to_owned(),
        replays_reasoning: false,
    }
}

fn request() -> RequestParams {
    RequestParams {
        max_tokens: 4096,
        temperature: None,
        thinking: Thinking::ProviderDefault,
        effort: None,
        seed: None,
        cache_scope: CacheScope::Shared,
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
    cancel: Arc<FakeCancel>,
    tools: Vec<Arc<dyn crate::ToolExecutor>>,
    filter: ToolFilter,
    stop: StopPolicy,
    retry: RetryPolicy,
    pricing: Option<Pricing>,
    calls: CallLimits,
    secrets: Arc<lablet_model::Secrets>,
    context: RunContext,
    completion: CompletionMode,
    request: RequestParams,
    prompts: Prompts,
    /// Whether the loop is handed the API's no-op tracer in place of the
    /// SDK's and run in the empty context, as a host that installs no
    /// tracing runs it.
    noop_tracer: bool,
}

impl Harness {
    fn new(script: Vec<Answer>) -> Self {
        let clock = Arc::new(FakeClock::new());
        let provider = Arc::new(FakeProvider::new(model(), Arc::clone(&clock), script));
        Self {
            tools: vec![Arc::new(FakeTools::new(
                Arc::clone(&clock),
                vec![
                    spec("bash", ToolSource::Builtin),
                    spec("read_file", ToolSource::Builtin),
                ],
            ))],
            clock,
            provider,
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
                output_cap: Some(output_cap(50_000, OutputCut::Preview { bytes: 2_000 })),
                max_concurrent_tool_calls: nz(10),
            },
            secrets: Arc::default(),
            context: context(),
            completion: CompletionMode::Natural,
            request: request(),
            prompts: prompts(),
            noop_tracer: false,
        }
    }

    /// Runs the loop inside a root span of the test's own, as the
    /// composition root runs it inside the run's, so every span has the
    /// parent a real run's has, and keeps what the loop emitted.
    ///
    /// The span's context is read from the context the loop ran in, and
    /// the root span itself is dropped with it: the SDK ends and exports a
    /// dropped span, so `spans` leaves it out by its id.
    async fn run(self) -> Run {
        let tools = ToolSet::build(self.tools, &self.filter, self.completion, None)
            .await
            .expect("these fakes serve distinct names");
        let spans = InMemorySpanExporter::default();
        let records = InMemoryLogExporter::default();
        let trace_provider = SdkTracerProvider::builder()
            .with_simple_exporter(spans.clone())
            .build();
        let log_provider = SdkLoggerProvider::builder()
            .with_simple_exporter(records.clone())
            .build();
        let tracer = if self.noop_tracer {
            BoxedTracer::new(Box::new(NoopTracer::new()))
        } else {
            BoxedTracer::new(Box::new(trace_provider.tracer("lablet")))
        };
        let mut service = RunService::new(
            Arc::clone(&self.provider) as Arc<dyn crate::ModelProvider>,
            Arc::new(tools),
            tracer,
            Box::new(Bridge::new(log_provider.logger("lablet"))),
            Arc::clone(&self.clock) as Arc<dyn crate::Clock>,
            Arc::clone(&self.cancel) as Arc<dyn crate::Cancellation>,
            self.stop,
            self.retry,
            self.request,
            self.pricing,
            self.calls,
            self.secrets,
        );
        // A host with no tracing calls the loop in no span's context either.
        let within = if self.noop_tracer {
            Context::new()
        } else {
            Context::current_with_span(trace_provider.tracer("lablet").start("invoke_agent test"))
        };
        let finished = movable(
            service
                .run(self.context, self.prompts)
                .with_context(within.clone()),
        )
        .await;
        Run {
            finished,
            provider: self.provider,
            clock: self.clock,
            root: within.span().span_context().clone(),
            spans,
            records,
            _providers: (trace_provider, log_provider),
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
    provider: Arc<FakeProvider>,
    clock: Arc<FakeClock>,
    /// The span the loop ran inside, which every span of the run is a
    /// child of.
    root: SpanContext,
    spans: InMemorySpanExporter,
    records: InMemoryLogExporter,
    /// The providers the loop emitted through, held because an in-memory
    /// exporter forgets everything when its provider shuts down.
    _providers: (SdkTracerProvider, SdkLoggerProvider),
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

    /// Every span the loop ended, in the order they ended; the test's own
    /// root span, which ends after the run, isn't the loop's.
    fn spans(&self) -> Vec<SpanData> {
        self.spans
            .get_finished_spans()
            .expect("the exporter holds its spans")
            .into_iter()
            .filter(|span| span.span_context.span_id() != self.root.span_id())
            .collect()
    }

    /// The spans whose `gen_ai.operation.name` is `operation`, in the order
    /// they ended.
    fn operation(&self, operation: &str) -> Vec<SpanData> {
        self.spans()
            .into_iter()
            .filter(|span| text(span, key::GEN_AI_OPERATION_NAME) == operation)
            .collect()
    }

    /// The chat spans, in the order the attempts ended.
    fn chats(&self) -> Vec<SpanData> {
        self.operation(LabletChat::GEN_AI_OPERATION_NAME)
    }

    /// The tool spans, in the order the calls ended.
    fn tools(&self) -> Vec<SpanData> {
        self.operation(LabletExecuteTool::GEN_AI_OPERATION_NAME)
    }

    /// Every record the loop wrote, in order.
    fn records(&self) -> Vec<SdkLogRecord> {
        self.records
            .get_emitted_logs()
            .expect("the exporter holds its records")
            .into_iter()
            .map(|log| log.record)
            .collect()
    }

    /// The records named `name`, in order.
    fn records_of(&self, name: &str) -> Vec<SdkLogRecord> {
        self.records()
            .into_iter()
            .filter(|record| record.event_name() == Some(name))
            .collect()
    }

    /// The content records, in order.
    fn content(&self) -> Vec<SdkLogRecord> {
        self.records_of(GenAiClientInferenceOperationDetails::NAME)
    }

    /// The exception records, in order.
    fn exceptions(&self) -> Vec<SdkLogRecord> {
        self.records_of(GenAiClientOperationException::NAME)
    }
}

/// The value of the attribute `key` of `span`, when it has one.
fn attribute<'a>(span: &'a SpanData, key: &str) -> Option<&'a Value> {
    span.attributes
        .iter()
        .find(|attribute| attribute.key.as_str() == key)
        .map(|attribute| &attribute.value)
}

/// The text the attribute `key` of `span` holds.
fn text<'a>(span: &'a SpanData, key: &str) -> &'a str {
    match attribute(span, key) {
        Some(Value::String(text)) => text.as_str(),
        other => panic!("{key} of {} is {other:?}, not text", span.name),
    }
}

/// The count the attribute `key` of `span` holds.
fn count(span: &SpanData, key: &str) -> i64 {
    match attribute(span, key) {
        Some(Value::I64(count)) => *count,
        other => panic!("{key} of {} is {other:?}, not a count", span.name),
    }
}

/// The text the attribute `key` of `span` holds, when it holds one.
fn maybe_text(span: &SpanData, key: &str) -> Option<String> {
    attribute(span, key).map(|value| match value {
        Value::String(text) => text.as_str().to_owned(),
        other => panic!("{key} of {} is {other:?}, not text", span.name),
    })
}

/// A span's attributes by key, each as JSON, since the order isn't part of
/// the contract.
fn attributes(span: &SpanData) -> BTreeMap<String, serde_json::Value> {
    span.attributes
        .iter()
        .map(|attribute| (attribute.key.as_str().to_owned(), as_json(&attribute.value)))
        .collect()
}

fn as_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Bool(flag) => json!(flag),
        Value::I64(count) => json!(count),
        Value::F64(number) => json!(number),
        Value::String(text) => json!(text.as_str()),
        Value::Array(Array::String(texts)) => {
            json!(texts.iter().map(StringValue::as_str).collect::<Vec<_>>())
        }
        other => panic!("no attribute of the loop's is {other:?}"),
    }
}

/// A record's attributes by key, each as JSON.
fn record_attributes(record: &SdkLogRecord) -> BTreeMap<String, serde_json::Value> {
    record
        .attributes_iter()
        .map(|(key, value)| {
            let value = match value {
                AnyValue::String(text) => json!(text.as_str()),
                AnyValue::Int(count) => json!(count),
                AnyValue::Boolean(flag) => json!(flag),
                other => panic!("no attribute of a record of the loop's is {other:?}"),
            };
            (key.as_str().to_owned(), value)
        })
        .collect()
}

/// The text the attribute `key` of `record` holds.
fn record_text(record: &SdkLogRecord, key: &str) -> String {
    match record_attributes(record).remove(key) {
        Some(serde_json::Value::String(text)) => text,
        other => panic!("{key} of the record is {other:?}, not text"),
    }
}

/// The JSON the attribute `key` of `record` holds as text.
fn record_json(record: &SdkLogRecord, key: &str) -> serde_json::Value {
    serde_json::from_str(&record_text(record, key)).expect("the record holds JSON")
}

/// How far into the run `at` is, in whole milliseconds.
fn offset_ms(at: SystemTime) -> u64 {
    let since = at
        .duration_since(started())
        .expect("nothing of a run is timed before it started");
    u64::try_from(since.as_millis()).expect("a run's offsets are short")
}

/// When `span` started and how long it lasted, in milliseconds into the run.
fn timing(span: &SpanData) -> (u64, u64) {
    let started = offset_ms(span.start_time);
    (started, offset_ms(span.end_time) - started)
}

/// The turn and the attempt of a chat span.
fn numbered(span: &SpanData) -> (i64, i64) {
    (
        count(span, key::LABLET_TURN),
        count(span, key::LABLET_ATTEMPT),
    )
}

/// The call id of a tool span.
fn call_id(span: &SpanData) -> String {
    text(span, key::GEN_AI_TOOL_CALL_ID).to_owned()
}

/// The shape of a run's chat spans: each attempt's turn, number and
/// `error.type`, in the order the attempts ended.
fn chat_shape(run: &Run) -> Vec<(i64, i64, Option<String>)> {
    run.chats()
        .iter()
        .map(|chat| {
            let (turn, attempt) = numbered(chat);
            (turn, attempt, maybe_text(chat, key::ERROR_TYPE))
        })
        .collect()
}

/// The shape of a run's tool spans: each call's turn, id and status, in the
/// order the calls ended.
fn tool_shape(run: &Run) -> Vec<(i64, String, String)> {
    run.tools()
        .iter()
        .map(|tool| {
            (
                count(tool, key::LABLET_TURN),
                call_id(tool),
                text(tool, key::LABLET_TOOL_STATUS).to_owned(),
            )
        })
        .collect()
}

/// The id of each call that has a span, in the order the calls ended.
fn announced(run: &Run) -> Vec<String> {
    run.tools().iter().map(call_id).collect()
}

/// What the chat span of a failed attempt says of the failure and of what
/// the loop did next.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failed {
    error_type: String,
    message: String,
    will_retry: bool,
    backoff_ms: Option<i64>,
}

/// The `lablet.retry` event of a failed attempt's span, and what it says.
fn retry(span: &SpanData) -> (bool, Option<i64>) {
    let [event] = span.events.events.as_slice() else {
        panic!("a failed attempt's span has one event: {:?}", span.events);
    };
    assert_eq!(event.name, LabletRetry::NAME);
    assert_eq!(
        event.timestamp, span.end_time,
        "the retry is decided as the attempt ends"
    );
    let value = |key: &str| {
        event
            .attributes
            .iter()
            .find(|attribute| attribute.key.as_str() == key)
            .map(|attribute| attribute.value.clone())
    };
    let Some(Value::Bool(will_retry)) = value(key::LABLET_RETRY_WILL_RETRY) else {
        panic!("the retry event says whether the loop retries");
    };
    let backoff_ms = value(key::LABLET_RETRY_BACKOFF_MS).map(|value| match value {
        Value::I64(backoff_ms) => backoff_ms,
        other => panic!("{other:?} isn't a count of milliseconds"),
    });
    (will_retry, backoff_ms)
}

/// Each failed attempt's span, read: its error type, its status message,
/// and the retry its event announced, in order.
fn failures(run: &Run) -> Vec<Failed> {
    run.chats()
        .iter()
        .filter(|chat| attribute(chat, key::ERROR_TYPE).is_some())
        .map(|chat| {
            let Status::Error { description } = &chat.status else {
                panic!(
                    "a failed attempt's span has an error status: {:?}",
                    chat.status
                );
            };
            let (will_retry, backoff_ms) = retry(chat);
            Failed {
                error_type: text(chat, key::ERROR_TYPE).to_owned(),
                message: description.to_string(),
                will_retry,
                backoff_ms,
            }
        })
        .collect()
}

/// The size of the request of each attempt, in the order the attempts ended.
fn request_bytes(run: &Run) -> Vec<u64> {
    run.chats()
        .iter()
        .map(|chat| u64::try_from(count(chat, key::LABLET_REQUEST_BYTES)).expect("a size"))
        .collect()
}

/// When each attempt began and how long it took, as its span says, with
/// how it ended, in the order the attempts ended.
fn attempts(run: &Run) -> Vec<(&'static str, u64, u64)> {
    run.chats()
        .iter()
        .map(|chat| {
            let ended = match &chat.status {
                Status::Unset => "answered",
                Status::Error { .. } => "failed",
                Status::Ok => panic!("no span of the loop's is marked ok"),
            };
            let (started_ms, latency_ms) = timing(chat);
            (ended, started_ms, latency_ms)
        })
        .collect()
}

/// When each call began and how long it took, as its span says, in the
/// order the calls ended.
fn tool_timings(run: &Run) -> Vec<(u64, u64)> {
    run.tools().iter().map(timing).collect()
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
        run.tools().is_empty(),
        "a call that never runs has no span either"
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
        chat_shape(&run),
        [(1, 1, None)],
        "the attempt answered, so its span says nothing went wrong"
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
        run.finished.summary.provider.retries, 2,
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
        run.finished.summary.provider.retries, 0,
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
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash", ToolSource::Builtin)],
        ),
        |tools, _| tools.answers("bash", failure()),
    );
    harness.tools = vec![Arc::new(tools)];
    harness.run().await
}

#[tokio::test]
async fn a_tool_that_always_fails_never_stops_a_run_however_it_fails() {
    for ended in ToolCallEnd::ALL {
        let failure: Scripted = match ended {
            // A call is stopped only when the run is cancelled, which ends
            // the run: C8's scenarios hold what becomes of it.
            ToolCallEnd::Ok | ToolCallEnd::Cancelled => continue,
            ToolCallEnd::ToolError => || Answers::ToolError("1 failed".to_owned()),
            ToolCallEnd::Timeout => {
                || Answers::Fails(crate::ToolErrorKind::Timeout, "took too long".to_owned())
            }
            ToolCallEnd::Failed => || {
                Answers::Fails(
                    crate::ToolErrorKind::Failed,
                    "the server is gone".to_owned(),
                )
            },
        };
        let run = calling_a_tool_that_always(failure).await;

        assert_eq!(run.stop_reason(), StopReason::Completed, "{ended}");
        assert_eq!(run.error(), None, "{ended}");
        assert_eq!(run.turns(), 6, "{ended}");
        assert_eq!(run.provider.calls(), 6, "{ended}");
        assert_eq!(run.finished.summary.tool_calls.errors, 5, "{ended}");
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
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash", ToolSource::Builtin)],
        )
        .answers(
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
    assert_eq!(run.finished.summary.tool_calls.errors, 1);
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
    assert_eq!(run.finished.summary.tool_calls.unknown, 1);
    assert_eq!(run.finished.summary.outcome.tool_calls, 2);
    assert_eq!(
        run.finished.summary.per_tool.keys().collect::<Vec<_>>(),
        [&name("bash")],
        "a name no tool has earns no key, so the wide event's keys stay bounded"
    );
    let outcome = &run.finished.transcript.turns()[0].tool_calls()[1];
    assert_eq!(outcome.status, ToolCallStatus::Unknown);
}

// C8: cancellation stops what's in flight, runs nothing after it, and the
// run still returns whole.

/// What the model would have been sent for each call of each turn.
fn contents(run: &Run) -> Vec<Vec<String>> {
    run.finished
        .transcript
        .turns()
        .iter()
        .map(|turn| {
            turn.tool_calls()
                .iter()
                .map(|outcome| match outcome.content.as_slice() {
                    [ToolResultContent::Text(text)] => text.clone(),
                    other => panic!("a call's content is one text block: {other:?}"),
                })
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn a_run_cancelled_once_its_response_came_starts_none_of_the_response_s_calls() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    // Answered once, before the first provider call, then true when the
    // call's turn comes.
    harness.cancel = Arc::new(FakeCancel::after(1));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.error(), None, "a cancelled run didn't fail");
    assert_eq!(
        run.provider.calls(),
        1,
        "the scripted second answer was never bought"
    );
    assert_eq!(
        statuses(&run),
        [vec!["not_run"]],
        "the turn has an outcome for its call, which never started"
    );
    assert_eq!(
        contents(&run),
        [vec![
            "the run was cancelled before this call started".to_owned()
        ]]
    );
    assert_eq!(chat_shape(&run), [(1, 1, None)]);
    assert!(
        run.tools().is_empty(),
        "nothing was started for the call, so no span reports it"
    );
    assert_eq!(run.finished.summary.outcome.tool_calls, 0);
}

#[tokio::test]
async fn a_run_cancelled_during_a_provider_attempt_drops_it_and_makes_no_other() {
    let cancel = Arc::new(FakeCancel::never());
    let mut harness = Harness::new(vec![
        Answer::Responds(
            Box::new(says("On it.", &["bash"], FinishReason::ToolUse)),
            ms(250),
        ),
        Answer::Fails(overloaded(), ms(40)),
        Answer::Cancels(Arc::clone(&cancel), us(70_500)),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    harness.cancel = cancel;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.error(), None);
    assert_eq!(
        run.provider.calls(),
        3,
        "no attempt followed the one in flight"
    );
    assert_eq!(
        run.provider.dropped(),
        ["x3"],
        "the attempt in flight was dropped"
    );
    assert_eq!(run.turns(), 1, "the first turn is whole");
    assert_eq!(statuses(&run), [vec!["ok"]]);
    assert_eq!(
        chat_shape(&run),
        [
            (1, 1, None),
            (2, 1, Some("retryable".to_owned())),
            (2, 2, Some("cancelled".to_owned())),
        ],
        "every attempt that started has a span that ends it"
    );
    assert_eq!(
        tool_shape(&run),
        [(1, "call_0".to_owned(), "ok".to_owned())]
    );
    let dropped = &run.chats()[2];
    assert_eq!(
        timing(dropped),
        (390, 70),
        "the attempt began after the backoff and was dropped when the run was cancelled"
    );
    assert_eq!(
        dropped.end_time,
        started() + us(460_500),
        "the dropped attempt's span ends when it was dropped, to the nanosecond"
    );
    assert_eq!(
        dropped.status,
        Status::error("the run was cancelled while the attempt was in flight")
    );
    assert_eq!(
        attribute(dropped, key::GEN_AI_USAGE_INPUT_TOKENS),
        None,
        "a dropped attempt reported nothing"
    );
    assert!(dropped.events.is_empty(), "no retry was decided");
    assert_declared(dropped, key::LABLET_CHAT_REQUIRED, key::LABLET_CHAT_KEYS);
    assert_eq!(
        run.exceptions().len(),
        1,
        "only the attempt that failed raised an exception"
    );
    let summary = &run.finished.summary;
    assert_eq!(
        summary.provider.retries, 1,
        "the attempt that was dropped was the call's second"
    );
    assert_eq!(
        (
            summary.provider.latency.total_ms(),
            summary.provider.latency.max_ms()
        ),
        (360, 250),
        "the attempt that was dropped took its time"
    );
    assert_eq!(
        summary.outcome.duration_ms, 460,
        "the run ended when the attempt was dropped"
    );
}

#[tokio::test]
async fn a_run_cancelled_during_its_first_provider_attempt_has_no_turn() {
    let cancel = Arc::new(FakeCancel::never());
    let mut harness = Harness::new(vec![
        Answer::Cancels(Arc::clone(&cancel), ms(70)),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    harness.cancel = cancel;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.provider.calls(), 1);
    assert_eq!(run.turns(), 0);
    assert_eq!(chat_shape(&run), [(1, 1, Some("cancelled".to_owned()))]);
    assert!(run.tools().is_empty());
    let summary = &run.finished.summary;
    assert_eq!(
        summary.provider.retries, 0,
        "a call's first attempt is no retry"
    );
    assert_eq!(summary.provider.latency.total_ms(), 70);
    assert_eq!(summary.failed_usage, None, "the attempt reported nothing");
}

#[tokio::test]
async fn a_run_cancelled_during_a_backoff_stops_waiting_and_makes_no_further_attempt() {
    let cancel = Arc::new(FakeCancel::never());
    let mut harness = Harness::new(vec![
        Answer::Fails(overloaded(), ms(40)),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    harness
        .clock
        .interrupts_the_next_sleep(Arc::clone(&cancel), ms(30));
    harness.cancel = cancel;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.provider.calls(), 1, "no attempt after the backoff");
    assert_eq!(run.clock.sleeps(), [ms(100)], "the loop began a wait");
    assert_eq!(
        run.finished.summary.outcome.duration_ms, 70,
        "and stopped 30 ms into it rather than at its end"
    );
    assert_eq!(run.turns(), 0);
    assert_eq!(chat_shape(&run), [(1, 1, Some("retryable".to_owned()))]);
    let [failed] = failures(&run).try_into().expect("one attempt failed");
    assert_eq!(
        (failed.will_retry, failed.backoff_ms),
        (true, Some(100)),
        "the failure's span announced the wait the loop began"
    );
    assert_eq!(run.finished.summary.provider.retries, 0);
}

#[tokio::test]
async fn a_run_cancelled_during_a_tool_call_stops_it_and_starts_no_later_group() {
    let cancel = Arc::new(FakeCancel::never());
    let mut harness = Harness::new(vec![
        Answer::now(says(
            "On it.",
            &["bash", "read_file"],
            FinishReason::ToolUse,
        )),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    let served = Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![
                spec("bash", ToolSource::Builtin),
                spec("read_file", ToolSource::Builtin),
            ],
        )
        .answers("bash", Answers::Stalls(Some(Arc::clone(&cancel)), ms(250))),
    );
    harness.tools = vec![Arc::clone(&served) as Arc<dyn crate::ToolExecutor>];
    harness.cancel = cancel;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.error(), None);
    assert_eq!(run.provider.calls(), 1, "no provider call followed");
    assert_eq!(
        statuses(&run),
        [vec!["cancelled", "not_run"]],
        "the turn has an outcome for every call: the one in flight was stopped, and the one \
         after it never started"
    );
    assert_eq!(
        run.finished.transcript.turns()[0].tool_calls()[0].status,
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Cancelled)
    );
    assert_eq!(
        contents(&run),
        [vec![
            "the run was cancelled while this call ran, and the call was stopped".to_owned(),
            "the run was cancelled before this call started".to_owned(),
        ]]
    );
    assert_eq!(
        served.spans(),
        ["+call_0", "xcall_0"],
        "the call in flight was dropped, and the later group reached no executor"
    );
    assert_eq!(chat_shape(&run), [(1, 1, None)]);
    assert_eq!(
        tool_shape(&run),
        [(1, "call_0".to_owned(), "cancelled".to_owned())],
        "the call in flight has a span, and the call after it never started"
    );
    let stopped = &run.tools()[0];
    assert_eq!(text(stopped, key::ERROR_TYPE), "cancelled");
    assert_eq!(stopped.status, Status::error(""));
    assert_eq!(
        timing(stopped),
        (0, 250),
        "the call was stopped when the run was cancelled"
    );
    assert_declared(
        stopped,
        key::LABLET_EXECUTE_TOOL_REQUIRED,
        key::LABLET_EXECUTE_TOOL_KEYS,
    );
    let summary = &run.finished.summary;
    assert_eq!(
        summary.outcome.tool_calls, 1,
        "the call that ran counts, and the one that never started doesn't"
    );
    assert_eq!(
        (summary.tool_calls.errors, summary.tool_calls.latency_ms),
        (1, 250)
    );
    assert_eq!(
        summary
            .per_tool
            .keys()
            .map(ToolName::as_str)
            .collect::<Vec<_>>(),
        ["bash"]
    );
    assert_eq!(summary.outcome.duration_ms, 250);
}

#[tokio::test]
async fn a_run_cancelled_while_a_shared_group_runs_stops_every_call_in_flight() {
    let cancel = Arc::new(FakeCancel::never());
    let mut harness = Harness::new(vec![
        Answer::now(says(
            "On it.",
            &["read_file", "read_file", "bash"],
            FinishReason::ToolUse,
        )),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    let mut read_file = spec("read_file", ToolSource::Builtin);
    read_file.concurrency = ToolConcurrency::Shared;
    let served = Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash", ToolSource::Builtin), read_file],
        )
        .answers("read_file", Answers::Stalls(None, Duration::ZERO))
        .answers(
            "read_file",
            Answers::Stalls(Some(Arc::clone(&cancel)), ms(40)),
        ),
    );
    harness.tools = vec![Arc::clone(&served) as Arc<dyn crate::ToolExecutor>];
    harness.cancel = cancel;

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.provider.calls(), 1, "no provider call followed");
    assert_eq!(
        statuses(&run),
        [vec!["cancelled", "cancelled", "not_run"]],
        "both calls of the group were in flight and stopped, and the call after the group \
         never started"
    );
    let mut spans = served.spans();
    spans.sort_unstable();
    assert_eq!(
        spans,
        ["+call_0", "+call_1", "xcall_0", "xcall_1"],
        "both calls were dropped, and bash reached no executor"
    );
    let mut stopped = tool_shape(&run);
    stopped.sort();
    assert_eq!(
        stopped,
        [
            (1, "call_0".to_owned(), "cancelled".to_owned()),
            (1, "call_1".to_owned(), "cancelled".to_owned()),
        ],
        "every call that started has a span, and it ended"
    );
    let latencies: Vec<u64> = run.finished.transcript.turns()[0]
        .tool_calls()
        .iter()
        .map(|outcome| outcome.latency_ms)
        .collect();
    assert_eq!(
        latencies,
        [40, 40, 0],
        "both ran until the run was cancelled"
    );
    assert_eq!(run.finished.summary.outcome.tool_calls, 2);
}

/// `bash` takes the rest of the run's time and then cancels the run, as a
/// Ctrl-C at the deadline would, so when `read_file`'s turn comes the run
/// is both cancelled and out of time.
#[tokio::test]
async fn a_call_whose_turn_comes_once_the_run_is_cancelled_and_out_of_time_is_said_to_be_cancelled()
{
    let cancel = Arc::new(FakeCancel::never());
    let mut harness = Harness::new(vec![Answer::now(says(
        "One.",
        &["bash", "read_file"],
        FinishReason::ToolUse,
    ))]);
    harness.tools = vec![Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![
                spec("bash", ToolSource::Builtin),
                spec("read_file", ToolSource::Builtin),
            ],
        )
        .answers(
            "bash",
            Answers::Stalls(Some(Arc::clone(&cancel)), harness.stop.timeout),
        ),
    )];
    harness.cancel = cancel;
    harness.context.capture_content = true;
    let timeout = u64::try_from(harness.stop.timeout.as_millis()).expect("fits");

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(
        run.finished.summary.outcome.duration_ms, timeout,
        "the run's time had gone when the second call's turn came"
    );
    assert_eq!(statuses(&run), [vec!["cancelled", "not_run"]]);
    assert_eq!(
        contents(&run)[0][1],
        "the run was cancelled before this call started",
        "cancellation comes before the timeout, as it does at points A and B"
    );
    assert_eq!(
        announced(&run),
        ["call_0"],
        "the call whose turn came too late has no span"
    );
    let spans = run.spans();
    let recorded_in: Vec<_> = run
        .content()
        .iter()
        .map(|record| {
            let context = record.trace_context().expect("in a span's context");
            spans
                .iter()
                .find(|span| span.span_context.span_id() == context.span_id)
                .map_or("the context the run was called in", |span| {
                    span.name.as_ref()
                })
        })
        .collect();
    assert_eq!(
        recorded_in,
        [
            "the context the run was called in",
            "chat fake-1",
            "execute_tool bash"
        ],
        "a call that was never run is in the transcript and nowhere else"
    );
}

// Cancellation leads the order at points A and B, and the stop policy never
// sees it, so only the loop can keep it first.

/// A run whose one response calls `bash`, which cancels the run while it
/// runs, as a Ctrl-C during the tool phase would.
fn cancelled_while_bash_runs() -> Harness {
    let mut harness = Harness::new(vec![Answer::now(says(
        "On it.",
        &["bash"],
        FinishReason::ToolUse,
    ))]);
    let cancel = Arc::new(FakeCancel::never());
    harness.tools = vec![Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash", ToolSource::Builtin)],
        )
        .answers(
            "bash",
            Answers::Cancelling(Arc::clone(&cancel), "bash ran".to_owned()),
        ),
    )];
    harness.cancel = cancel;
    harness
}

#[tokio::test]
async fn a_run_cancelled_during_the_tool_phase_of_its_last_allowed_turn_is_cancelled() {
    let mut harness = cancelled_while_bash_runs();
    harness.stop.max_turns = Some(nz(1));

    let run = harness.run().await;

    assert_eq!(
        run.stop_reason(),
        StopReason::Cancelled,
        "cancellation comes before the turn cap at point B"
    );
    assert_eq!(run.turns(), 1, "the turn was the last the cap allowed");
    assert_eq!(run.provider.calls(), 1);
    assert_eq!(statuses(&run), [vec!["ok"]], "the call ran to its end");
}

#[tokio::test]
async fn a_run_cancelled_in_the_turn_that_reached_its_token_budget_is_cancelled() {
    let mut harness = cancelled_while_bash_runs();
    harness.stop.max_total_tokens = Some(120);

    let run = harness.run().await;

    assert_eq!(
        run.stop_reason(),
        StopReason::Cancelled,
        "cancellation comes before the token budget at point B"
    );
    assert_eq!(
        run.finished.summary.outcome.usage.total(),
        120,
        "the turn's response reached the budget"
    );
    assert_eq!(run.provider.calls(), 1);
    assert_eq!(statuses(&run), [vec!["ok"]]);
}

#[tokio::test]
async fn a_run_cancelled_before_it_starts_with_no_time_to_run_is_cancelled() {
    let mut harness = Harness::new(vec![Answer::now(says(
        "Never reached.",
        &[],
        FinishReason::EndTurn,
    ))]);
    harness.cancel = Arc::new(FakeCancel::already());
    harness.stop.timeout = Duration::ZERO;

    let run = harness.run().await;

    assert_eq!(
        run.stop_reason(),
        StopReason::Cancelled,
        "cancellation comes before the timeout at point A"
    );
    assert_eq!(run.provider.calls(), 0);
    assert_eq!(run.error(), None);
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
        vec![
            spec("bash", ToolSource::Builtin),
            spec("read_file", ToolSource::Builtin),
        ],
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
    assert_eq!(run.finished.summary.tool_calls.truncated, 1);
    for outcome in outcomes {
        assert_eq!(
            outcome.status,
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok),
            "the loop cut the output; the tool itself succeeded"
        );
    }
    assert_eq!(run.finished.summary.tool_calls.errors, 0);
    assert_eq!(run.stop_reason(), StopReason::Completed);
}

/// The line is added on top of the cap, so the size that was sent is more
/// than the cap allows of the tool's own text.
#[tokio::test]
async fn the_tool_spans_say_what_was_sent_of_each_output_and_how_large_a_cut_one_was() {
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
            vec![
                spec("bash", ToolSource::Builtin),
                spec("read_file", ToolSource::Builtin),
            ],
        )
        .answers("bash", Answers::Text(SIXTEEN_BYTES.to_owned()))
        .answers("read_file", Answers::Text(TEN_BYTES.to_owned())),
    )];

    let run = harness.run().await;

    let line = "[truncated: the first 10 of 16 bytes]";
    let sent = |text: &str| json!({ "type": "text", "text": text });
    let told: Vec<_> = run
        .tools()
        .iter()
        .map(|tool| {
            (
                count(tool, key::LABLET_TOOL_OUTPUT_BYTES),
                attribute(tool, key::LABLET_TOOL_OUTPUT_TRUNCATED).cloned(),
                attribute(tool, key::LABLET_TOOL_OUTPUT_ORIGINAL_BYTES).cloned(),
            )
        })
        .collect();
    let line_bytes = i64::try_from(line.len()).expect("a short line");
    assert_eq!(
        told,
        [
            (
                10 + line_bytes,
                Some(Value::Bool(true)),
                Some(Value::I64(16))
            ),
            (10, Some(Value::Bool(false)), None),
        ]
    );
    let results: Vec<_> = run
        .content()
        .iter()
        .filter(|record| record_attributes(record).contains_key(key::GEN_AI_TOOL_CALL_RESULT))
        .map(|record| record_json(record, key::GEN_AI_TOOL_CALL_RESULT))
        .collect();
    assert_eq!(
        results,
        [
            json!({ "content": [sent(TEN_BYTES), sent(line)], "isError": false }),
            json!({ "content": [sent(TEN_BYTES)], "isError": false }),
        ],
        "the record of each call holds what the model was sent"
    );
    assert_eq!(
        run.finished.summary.tool_calls.output_bytes,
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
    assert_eq!(run.finished.summary.tool_calls.truncated, 0);
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
            vec![
                spec("bash", ToolSource::Builtin),
                spec("read_file", ToolSource::Builtin),
            ],
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
    assert_eq!(run.finished.summary.tool_calls.truncated, 3);
}

/// A 48-byte value of the run's, which an executor's message carries
/// astride the bound.
const VALUE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

/// The error result an executor's message becomes has the run's secrets cut
/// out of it and is then bounded, in that order. Bounded first, a value
/// astride the bound would be no value to the cut, and every byte of it
/// before the bound would reach the model.
#[tokio::test]
async fn an_executor_s_error_message_is_bounded_only_after_the_run_s_secrets_are_cut_out_of_it() {
    const MARKER: &str = lablet_model::Secrets::MARKER;
    assert_eq!(VALUE.len(), 48);
    let before = |bytes: usize| "x".repeat(bytes);
    for (message, sent) in [
        // The marker fits at the bound, where the value would have straddled
        // it.
        (
            format!("{}{VALUE}", before(ERROR_MESSAGE_MAX_BYTES - MARKER.len())),
            format!("{}{MARKER}", before(ERROR_MESSAGE_MAX_BYTES - MARKER.len())),
        ),
        // The marker lengthens the text past the bound, and the bound takes
        // the marker's end rather than the value's start.
        (
            format!("{}{VALUE}", before(2_040)),
            format!(
                "{}{}",
                before(2_040),
                &MARKER[..ERROR_MESSAGE_MAX_BYTES - 2_040]
            ),
        ),
        // A message with nothing to cut is bounded all the same.
        (before(3_000), before(ERROR_MESSAGE_MAX_BYTES)),
    ] {
        let mut harness = Harness::new(vec![
            Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
            Answer::now(says("Done.", &[], FinishReason::EndTurn)),
        ]);
        harness.secrets = Arc::new(lablet_model::Secrets::new([VALUE.to_owned()]));
        harness.tools = vec![Arc::new(
            FakeTools::new(
                Arc::clone(&harness.clock),
                vec![spec("bash", ToolSource::Builtin)],
            )
            .answers(
                "bash",
                Answers::Fails(crate::ToolErrorKind::Failed, message),
            ),
        )];

        let run = harness.run().await;

        let mut result = result_of("call_0", &[&sent]);
        result["is_error"] = serde_json::json!(true);
        assert_eq!(results_sent(&run), serde_json::json!([result]));
        assert_eq!(
            run.finished.summary.tool_calls.truncated, 0,
            "the bound is no cut of the cap's"
        );
    }
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
    assert_eq!(run.finished.summary.tool_calls.truncated, 1);
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
    assert_eq!(run.finished.summary.tool_calls.truncated, 1);
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

// The spans and records of a run.

#[tokio::test]
async fn the_spans_and_records_of_a_scripted_run_are_exactly_these() {
    let run = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ])
    .run()
    .await;

    assert_eq!(
        chat_shape(&run),
        [
            (1, 1, Some("retryable".to_owned())),
            (1, 2, None),
            (2, 1, None)
        ],
        "a span for each attempt, in the order they ended"
    );
    assert_eq!(
        tool_shape(&run),
        [(1, "call_0".to_owned(), "ok".to_owned())]
    );
    assert_eq!(run.spans().len(), 4);
    let events: Vec<_> = run.records().iter().map(SdkLogRecord::event_name).collect();
    assert_eq!(
        events,
        [Some(GenAiClientOperationException::NAME)],
        "content is off, so the one record is the failure's"
    );
}

#[tokio::test]
async fn every_span_and_record_carries_the_run_id() {
    let mut harness = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.context.capture_content = true;

    let run = harness.run().await;

    let spans = run.spans();
    let records = run.records();
    assert_eq!(
        (spans.len(), records.len()),
        (4, 1 + 1 + 3 + 1),
        "the tools offered, the exception, a record of each attempt and of the call"
    );
    for span in &spans {
        assert_eq!(
            text(span, key::GEN_AI_CONVERSATION_ID),
            RUN,
            "{}",
            span.name
        );
        assert_eq!(text(span, key::SESSION_ID), RUN, "{}", span.name);
    }
    for record in &records {
        let attributes = record_attributes(record);
        assert_eq!(attributes[key::GEN_AI_CONVERSATION_ID], json!(RUN));
        assert_eq!(attributes[key::SESSION_ID], json!(RUN));
    }
}

/// Every content record of a run that calls a tool, so the prompts aren't
/// standing in for the response and the tool call beside them.
#[tokio::test]
async fn content_reaches_the_records_only_when_the_run_captures_it() {
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

    assert!(
        quiet.records().is_empty(),
        "nothing is written for a run that captures no content"
    );
    for span in quiet.spans().into_iter().chain(loud.spans()) {
        for key in HOLD_CONTENT {
            assert_eq!(attribute(&span, key), None, "{key} is on {}", span.name);
        }
    }
    let content = loud.content();
    assert_eq!(
        content.len(),
        1 + 2 + 1,
        "the tools offered, each attempt, and the call"
    );
    let offered = record_json(&content[0], key::GEN_AI_TOOL_DEFINITIONS);
    assert_eq!(offered[0]["name"], "bash");
    assert_eq!(offered[1]["name"], "read_file");
    assert_eq!(
        record_json(&content[1], key::GEN_AI_SYSTEM_INSTRUCTIONS),
        json!([{ "type": "text", "content": "You fix tests." }])
    );
    assert_eq!(
        record_json(&content[1], key::GEN_AI_INPUT_MESSAGES),
        json!([{ "role": "user", "parts": [{ "type": "text", "content": "Fix the failing test." }] }])
    );
    assert_eq!(
        record_json(&content[1], key::GEN_AI_OUTPUT_MESSAGES),
        json!([{ "role": "assistant", "parts": [
            { "type": "text", "content": "On it." },
            { "type": "tool_call", "id": "call_0", "name": "bash", "arguments": { "n": 0 } },
        ] }])
    );
    assert_eq!(
        record_json(&content[2], key::GEN_AI_TOOL_CALL_ARGUMENTS),
        json!({ "n": 0 }),
        "the call's arguments"
    );
    assert_eq!(
        record_json(&content[2], key::GEN_AI_TOOL_CALL_RESULT),
        json!({ "content": [{ "type": "text", "text": "bash ran" }], "isError": false }),
        "what the model was sent back"
    );
    assert_eq!(
        record_json(&content[3], key::GEN_AI_OUTPUT_MESSAGES),
        json!([{ "role": "assistant", "parts": [{ "type": "text", "content": "Done." }] }])
    );
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
/// call reported it, and each span holds what its own call reported.
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
    let cached: Vec<_> = run
        .chats()
        .iter()
        .map(|chat| attribute(chat, key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS).cloned())
        .collect();
    assert_eq!(
        cached,
        [Some(Value::I64(40)), None],
        "each span says what its own call reported, and no more"
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
    harness.stop.max_turns = Some(nz(7));
    harness.stop.timeout = ms(1_234);
    harness.provider = Arc::new(
        FakeProvider::new(
            model(),
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

    let specs: u64 = [
        spec("bash", ToolSource::Builtin),
        spec("read_file", ToolSource::Builtin),
    ]
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

/// O24: the loop runs each call under its span's context, which is how an
/// MCP executor gets something to propagate, and the two calls of a
/// concurrent group each find their own.
#[tokio::test]
async fn the_span_of_a_call_is_in_the_executor_s_current_context() {
    let mut harness = Harness::new(vec![
        Answer::now(says(
            "On it.",
            &["read_file", "read_file"],
            FinishReason::ToolUse,
        )),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    let tools = Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![ToolSpec {
                concurrency: ToolConcurrency::Shared,
                ..spec("read_file", ToolSource::Builtin)
            }],
        )
        .yielding(),
    );
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];

    let run = harness.run().await;

    let mut found = tools.found();
    found.sort_by(|one, other| one.0.cmp(&other.0));
    let mut exported: Vec<_> = run
        .tools()
        .iter()
        .map(|tool| (call_id(tool), tool.span_context.clone()))
        .collect();
    exported.sort_by(|one, other| one.0.cmp(&other.0));
    assert_eq!(
        found, exported,
        "each call found the span it was exported as"
    );
    let mut found_again = tools.found_again();
    found_again.sort_by(|one, other| one.0.cmp(&other.0));
    assert_eq!(
        found_again, exported,
        "each call found its own span again once it had yielded"
    );
    assert_eq!(found.len(), 2);
    for (call, context) in &found {
        assert!(context.is_valid(), "{call}");
        assert!(context.is_sampled(), "{call}");
        assert_eq!(context.trace_id(), run.root.trace_id(), "{call}");
    }
    assert_ne!(
        found[0].1.span_id(),
        found[1].1.span_id(),
        "the two calls of the group each found their own"
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
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash", ToolSource::Builtin)],
        )
        .taking(ms(250)),
    )];

    let run = harness.run().await;

    assert_eq!(
        run.finished.transcript.turns()[0].tool_calls()[0].latency_ms,
        250
    );
    assert_eq!(run.finished.summary.tool_calls.latency_ms, 250);
}

/// A host that installs no tracing hands the loop the API's no-op tracer
/// and calls it in no span's context, and an executor then finds no span to
/// propagate.
#[tokio::test]
async fn a_run_with_a_no_op_tracer_hands_the_executor_no_span() {
    let mut harness = Harness::new(vec![
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    let tools = Arc::new(FakeTools::new(
        Arc::clone(&harness.clock),
        vec![spec("bash", ToolSource::Builtin)],
    ));
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];
    harness.noop_tracer = true;

    let run = harness.run().await;

    let [(call, found)] = tools.found().try_into().expect("one call");
    assert_eq!(call, "call_0");
    assert!(!found.is_valid(), "{found:?}");
    assert!(run.spans().is_empty(), "the no-op tracer exports nothing");
    assert_eq!(run.stop_reason(), StopReason::Completed);
}

// L2: the intercepted call is never started, so it has no span.

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
        run.tools().is_empty(),
        "the loop intercepts it rather than running it, so it has no span"
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
    assert!(run.tools().is_empty(), "and has no span");
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
            vec![
                spec("bash", ToolSource::Builtin),
                spec("read_file", ToolSource::Builtin),
            ],
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
    assert_eq!(summary.provider.retries, 1);
    assert_eq!(summary.outcome.tool_calls, 2);
    assert_eq!(summary.tool_calls.errors, 1);
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

/// Each tool has an executor of its own, so the two calls take different
/// times; both are exclusive, so the second starts when the first ends. A
/// loop that stamped every call of a turn with one start, or one latency,
/// fails here.
#[tokio::test]
async fn each_call_of_a_turn_holds_its_own_start_and_latency() {
    let mut harness = Harness::new(vec![
        Answer::Responds(
            Box::new(says(
                "On it.",
                &["bash", "read_file"],
                FinishReason::ToolUse,
            )),
            ms(80),
        ),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.tools = vec![
        Arc::new(
            FakeTools::new(
                Arc::clone(&harness.clock),
                vec![spec("bash", ToolSource::Builtin)],
            )
            .taking(ms(250)),
        ),
        Arc::new(
            FakeTools::new(
                Arc::clone(&harness.clock),
                vec![spec("read_file", ToolSource::Builtin)],
            )
            .taking(ms(70)),
        ),
    ];

    let run = harness.run().await;
    let calls = run.finished.transcript.turns()[0].tool_calls();

    let timings: Vec<(&str, u64, u64)> = calls
        .iter()
        .map(|call| (call.call_id.as_str(), call.started_ms, call.latency_ms))
        .collect();
    assert_eq!(
        timings,
        [("call_0", 80, 250), ("call_1", 330, 70)],
        "the first call starts when the response arrives, the second when the first ends"
    );
    assert_eq!(tool_timings(&run), [(80, 250), (330, 70)]);
    assert_eq!(run.finished.summary.tool_calls.latency_ms, 320);
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
    assert_eq!(run.finished.summary.provider.retries, 6);
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
    let [failed] = failures(&run).try_into().expect("one attempt failed");
    assert_eq!(
        failed.message, "prompt is 205000 tokens, over the 200000 limit",
        "and so does the span's status"
    );
    assert_eq!(
        record_text(&run.exceptions()[0], key::EXCEPTION_MESSAGE),
        "prompt is 205000 tokens, over the 200000 limit",
        "and the exception record"
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
    assert_eq!(run.finished.summary.provider.retries, 1);
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
    assert_eq!(run.finished.summary.tool_calls.unknown, 1);
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
    assert_eq!(run.finished.summary.tool_calls.unknown, 1);
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
}

#[tokio::test]
async fn a_turn_in_which_a_call_reached_a_tool_ends_the_count_whatever_the_tool_returned() {
    for ended in ToolCallEnd::ALL {
        let (returned, reached): (Scripted, &str) = match ended {
            // A call is stopped only when the run is cancelled, which ends
            // the run before any count is read.
            ToolCallEnd::Cancelled => continue,
            ToolCallEnd::Ok => (|| Answers::Text("ok".to_owned()), "ok"),
            ToolCallEnd::ToolError => (|| Answers::ToolError("1 failed".to_owned()), "tool_error"),
            ToolCallEnd::Timeout => (
                || Answers::Fails(crate::ToolErrorKind::Timeout, "took too long".to_owned()),
                "timeout",
            ),
            ToolCallEnd::Failed => (
                || {
                    Answers::Fails(
                        crate::ToolErrorKind::Failed,
                        "the server is gone".to_owned(),
                    )
                },
                "failed",
            ),
        };
        let mut script = three_invalid_turns(Some(("read_file", parsed())));
        script.push(Answer::now(calling(&[("invented", parsed())])));
        script.push(Answer::now(says("Done.", &[], FinishReason::EndTurn)));
        let mut harness = Harness::new(script);
        harness.stop.max_consecutive_invalid_turns = Some(nz(3));
        harness.tools = vec![Arc::new(
            FakeTools::new(
                Arc::clone(&harness.clock),
                vec![
                    spec("bash", ToolSource::Builtin),
                    spec("read_file", ToolSource::Builtin),
                ],
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

/// One response that makes three invalid calls, and the response that ends
/// the run.
fn three_invalid_calls_at_once() -> Vec<Answer> {
    vec![
        Answer::now(calling(&[
            ("invented", parsed()),
            ("bash", unparsed()),
            ("also_invented", parsed()),
        ])),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]
}

#[tokio::test]
async fn one_response_that_makes_three_invalid_calls_is_one_invalid_turn() {
    let mut harness = Harness::new(three_invalid_calls_at_once());
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

/// `script`, then three more invalid turns, then the response that ends the
/// run: twice the turns a cap of three allows in a row.
fn and_three_more_invalid_turns(mut script: Vec<Answer>) -> Vec<Answer> {
    script.extend((0..3).map(|_| Answer::now(calling(&[("invented", parsed())]))));
    script.push(Answer::now(says("Done.", &[], FinishReason::EndTurn)));
    script
}

#[tokio::test]
async fn without_a_cap_on_invalid_turns_every_such_run_goes_on() {
    let scripts = [
        (and_three_more_invalid_turns(three_invalid_turns(None)), 7),
        (
            and_three_more_invalid_turns(three_invalid_turns(Some(("read_file", parsed())))),
            7,
        ),
        (three_invalid_calls_at_once(), 2),
    ];
    for (script, turns) in scripts {
        let mut harness = Harness::new(script);
        harness.stop.max_consecutive_invalid_turns = None;

        let run = harness.run().await;

        assert_eq!(run.stop_reason(), StopReason::Completed, "{turns} turns");
        assert_eq!(run.turns(), turns);
        assert_eq!(
            run.provider.calls(),
            turns as usize,
            "every response of the script was asked for"
        );
        assert_eq!(run.error(), None, "{turns} turns");
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
    assert_eq!(
        run.chats().len(),
        41,
        "a span for every attempt, however many"
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
        run.finished.summary.provider.latency.total_ms(),
        550,
        "a run where nothing failed still spent time in the provider"
    );
    assert_eq!(run.finished.summary.provider.latency.max_ms(), 400);
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

    assert_eq!(run.finished.summary.provider.latency.total_ms(), 150);
    assert_eq!(run.finished.summary.provider.latency.max_ms(), 90);
    assert_eq!(
        run.finished.transcript.turns()[0].record().started_ms,
        190,
        "the successful attempt began after the failure and its 100ms backoff"
    );
}

/// The summary's provider latencies count the failed attempts, so the spans
/// account for them only when a failed attempt's span lasts as long as the
/// attempt did. The slowest attempt of the first run is one that failed, and
/// the second run has no attempt but one that failed.
#[tokio::test]
async fn the_latencies_of_a_run_s_attempts_sum_to_the_summary_s_total_and_the_longest_is_its_max() {
    let failing_twice_and_then_once = vec![
        Answer::Fails(overloaded(), ms(90)),
        Answer::Fails(overloaded(), ms(250)),
        Answer::Responds(
            Box::new(says("On it.", &["bash"], FinishReason::ToolUse)),
            ms(60),
        ),
        Answer::Fails(overloaded(), ms(30)),
        Answer::Responds(Box::new(says("Done.", &[], FinishReason::EndTurn)), ms(120)),
    ];
    let failing_for_good = vec![Answer::Fails(
        ProviderError::new(ProviderErrorKind::Fatal, "bad request"),
        ms(70),
    )];
    let runs = [
        (
            failing_twice_and_then_once,
            vec![
                ("failed", 0, 90),
                ("failed", 190, 250),
                ("answered", 640, 60),
                ("failed", 700, 30),
                ("answered", 830, 120),
            ],
            (550, 250),
        ),
        (failing_for_good, vec![("failed", 0, 70)], (70, 70)),
    ];
    for (script, attempted, (total, max)) in runs {
        let run = Harness::new(script).run().await;

        let attempts = attempts(&run);
        assert_eq!(
            attempts, attempted,
            "each attempt began where the one before it ended, a backoff later when that one failed"
        );
        let latencies = attempts.iter().map(|(_, _, latency_ms)| *latency_ms);
        let summary = &run.finished.summary;
        assert_eq!(
            latencies.clone().sum::<u64>(),
            summary.provider.latency.total_ms()
        );
        assert_eq!(latencies.max(), Some(summary.provider.latency.max_ms()));
        assert_eq!(
            (
                summary.provider.latency.total_ms(),
                summary.provider.latency.max_ms()
            ),
            (total, max)
        );
    }
}

/// The retry event is the whole answer: `will_retry` with a backoff means
/// another attempt follows, and without one the call is over.
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

    let [failed] = failures(&run).try_into().expect("one attempt failed");
    assert_eq!(
        (failed.will_retry, failed.backoff_ms),
        (true, Some(100)),
        "the policy's first backoff"
    );
}

#[tokio::test]
async fn the_last_failed_attempt_reports_no_wait() {
    let run = Harness::new(vec![Answer::failing(ProviderError::new(
        ProviderErrorKind::Fatal,
        "bad request",
    ))])
    .run()
    .await;

    let [failed] = failures(&run).try_into().expect("one attempt failed");
    assert_eq!(
        (failed.will_retry, failed.backoff_ms),
        (false, None),
        "a fatal failure is never tried again"
    );
    assert_eq!(run.stop_reason(), StopReason::ProviderError);
}

/// The loop is the only hop between an executor and the call's span, so
/// anything it drops here can never become an `mcp.*` attribute.
#[tokio::test]
async fn what_a_tool_carried_back_over_mcp_reaches_the_tool_span() {
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
        FakeTools::new(Arc::clone(&clock), vec![spec("search", mcp("docs"))])
            .answers("search", Answers::OverMcp("found it".to_owned(), meta)),
    )];

    let run = harness.run().await;

    let [tool] = run.tools().try_into().expect("one call");
    let carried = attributes(&tool);
    assert_eq!(carried[key::MCP_METHOD_NAME], json!("tools/call"));
    assert_eq!(carried[key::MCP_SESSION_ID], json!("session-7"));
    assert_eq!(carried[key::MCP_PROTOCOL_VERSION], json!("2025-06-18"));
    assert_eq!(carried[key::JSONRPC_REQUEST_ID], json!("3"));
    assert_eq!(carried.get(key::RPC_RESPONSE_STATUS_CODE), None);
    assert_eq!(carried[key::NETWORK_TRANSPORT], json!("pipe"));
    assert_eq!(carried[key::LABLET_TOOL_STATUS], json!("ok"));
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
        FakeTools::new(Arc::clone(&clock), vec![spec("search", mcp("docs"))]).answers(
            "search",
            Answers::FailsOverMcp(
                crate::ToolErrorKind::Failed,
                "the server gave up".to_owned(),
                meta,
            ),
        ),
    )];

    let run = harness.run().await;

    let [tool] = run.tools().try_into().expect("one call");
    let carried = attributes(&tool);
    assert_eq!(carried[key::MCP_METHOD_NAME], json!("tools/call"));
    assert_eq!(carried.get(key::MCP_SESSION_ID), None);
    assert_eq!(carried.get(key::MCP_PROTOCOL_VERSION), None);
    assert_eq!(carried[key::JSONRPC_REQUEST_ID], json!("4"));
    assert_eq!(carried[key::RPC_RESPONSE_STATUS_CODE], json!("-32603"));
    assert_eq!(carried[key::NETWORK_TRANSPORT], json!("tcp"));
    assert_eq!(carried[key::LABLET_TOOL_STATUS], json!("failed"));
    assert_eq!(carried[key::ERROR_TYPE], json!("failed"));
}

/// The tool set is the one place a call's source comes from, and the loop
/// carries it to the call's record and to the call's span, whether the call
/// succeeded or failed. The span names the source and not the server, which
/// the record holds.
#[tokio::test]
async fn a_call_to_a_tool_served_over_mcp_is_recorded_with_its_server_and_its_span_names_the_source()
 {
    let mut harness = Harness::new(vec![
        Answer::now(says(
            "On it.",
            &["bash", "search", "search"],
            FinishReason::ToolUse,
        )),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.tools.push(Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("search", mcp("docs"))],
        )
        .answers("search", Answers::Text("found it".to_owned()))
        .answers(
            "search",
            Answers::Fails(
                crate::ToolErrorKind::Failed,
                "the server gave up".to_owned(),
            ),
        ),
    ));

    let run = harness.run().await;

    let recorded: Vec<&ToolCallStatus> = run.finished.transcript.turns()[0]
        .tool_calls()
        .iter()
        .map(|outcome| &outcome.status)
        .collect();
    assert_eq!(
        recorded,
        [
            &ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok),
            &ToolCallStatus::ran(mcp("docs"), ToolCallEnd::Ok),
            &ToolCallStatus::ran(mcp("docs"), ToolCallEnd::Failed),
        ]
    );
    let sources: Vec<_> = run
        .tools()
        .iter()
        .map(|tool| {
            (
                call_id(tool),
                maybe_text(tool, key::LABLET_TOOL_SOURCE),
                maybe_text(tool, key::GEN_AI_TOOL_TYPE),
            )
        })
        .collect();
    let builtin = (Some("builtin".to_owned()), Some("function".to_owned()));
    let served = (Some("mcp".to_owned()), Some("extension".to_owned()));
    assert_eq!(
        sources,
        [
            ("call_0".to_owned(), builtin.0, builtin.1),
            ("call_1".to_owned(), served.0.clone(), served.1.clone()),
            ("call_2".to_owned(), served.0, served.1),
        ]
    );
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

    let [tool] = run.tools().try_into().expect("one call");
    for key in OVER_MCP {
        assert_eq!(attribute(&tool, key), None, "{key}");
    }
    assert_eq!(text(&tool, key::LABLET_TOOL_STATUS), "unknown");
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
    assert!(
        run.spans().is_empty() && run.records().is_empty(),
        "the stop is read before an attempt is admitted, so nothing began"
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
    harness.cancel = Arc::new(FakeCancel::already());

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
    let tools = Arc::new(FakeTools::new(
        Arc::clone(&clock),
        vec![spec("bash", ToolSource::Builtin)],
    ));
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
    assert_eq!(run.finished.summary.tool_calls.errors, 1);
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
        vec![spec("write_file", ToolSource::Builtin)],
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
    assert_eq!(summary.tool_calls.errors, 3);
    assert_eq!(summary.tool_calls.unknown, 0);
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

/// A rejected call is a call the loop answered, so it has a span as a call
/// to an unknown name has. Only the call that completes the run has none.
/// Every call runs alone here, because the spans of calls that run together
/// end in no order a test may rely on.
#[tokio::test]
async fn a_rejected_call_has_a_span_and_the_intercepted_one_has_none() {
    let (mut harness, _) = explicit_with_write_file(completing_beside_other_calls());
    harness.calls.max_concurrent_tool_calls = nz(1);

    let run = harness.run().await;

    let spans: Vec<(i64, String, String, Option<String>, String, bool)> = run
        .tools()
        .iter()
        .map(|tool| {
            (
                count(tool, key::LABLET_TURN),
                call_id(tool),
                text(tool, key::GEN_AI_TOOL_NAME).to_owned(),
                maybe_text(tool, key::LABLET_TOOL_SOURCE),
                text(tool, key::LABLET_TOOL_STATUS).to_owned(),
                attribute(tool, key::MCP_METHOD_NAME).is_some(),
            )
        })
        .collect();
    let span = |turn, call: &str, name: &str, status: &str| {
        (
            turn,
            call.to_owned(),
            name.to_owned(),
            Some("builtin".to_owned()),
            status.to_owned(),
            false,
        )
    };
    assert_eq!(
        spans,
        [
            span(1, "call_0", "write_file", "ok"),
            span(1, "call_1", "task_complete", "rejected"),
            span(2, "call_0", "task_complete", "rejected"),
            span(2, "call_1", "task_complete", "rejected"),
        ],
        "turn 3's call completed the run, so it never began"
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
            vec![
                spec("bash", ToolSource::Builtin),
                spec(CompletionMode::TASK_COMPLETE, ToolSource::Builtin),
            ],
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
    assert_eq!(summary.tool_calls.errors, 0);
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
                    ..spec("read_file", ToolSource::Builtin)
                },
                spec("write_file", ToolSource::Builtin),
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
/// test that asserts spans exactly relies on. Each run has a clock of its
/// own, and nothing in either run takes any time, so every offset and
/// latency is 0 in both and the transcripts can be equal whole. Calls that
/// took time couldn't be: two that overlap don't start and end when the
/// same calls run one after the other do.
#[tokio::test]
async fn a_cap_of_one_ends_the_spans_in_call_order_and_gives_the_same_transcript() {
    let (alone, spans) = grouped(1).await;
    let (together, overlapping) = grouped(10).await;

    assert_eq!(
        spans,
        [
            "+call_0", "-call_0", "+call_1", "-call_1", "+call_2", "-call_2", "+call_3", "-call_3"
        ]
    );
    assert_eq!(
        announced(&alone),
        ["call_0", "call_1", "call_2", "call_3"],
        "each call's span ends before the next call's opens"
    );
    let held = |run: &Run| {
        let mut spans: Vec<String> = run
            .spans()
            .iter()
            .map(|span| format!("{:?}", attributes(span)))
            .collect();
        spans.sort();
        spans
    };
    assert_eq!(
        held(&alone),
        held(&together),
        "a cap changes the order the spans end in and nothing that they hold"
    );
    assert_ne!(spans, overlapping, "with 10 the first two reads overlap");
    assert_eq!(alone.finished.transcript, together.finished.transcript);
    let ids: Vec<&str> = alone.finished.transcript.turns()[0]
        .tool_calls()
        .iter()
        .map(|outcome| outcome.call_id.as_str())
        .collect();
    assert_eq!(ids, ["call_0", "call_1", "call_2", "call_3"]);
}

// E11: a retry is a provider call, so it's polled for cancellation.

#[tokio::test]
async fn a_run_cancelled_as_a_backoff_ends_makes_no_further_attempt() {
    let mut harness = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("Never reached.", &[], FinishReason::EndTurn)),
    ]);
    // Answered before the attempt and again once it failed, then true when
    // it's asked again, after the backoff.
    harness.cancel = Arc::new(FakeCancel::after(2));

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Cancelled);
    assert_eq!(run.provider.calls(), 1, "no attempt after the backoff");
    assert_eq!(run.clock.sleeps(), [ms(100)]);
    assert_eq!(run.turns(), 0);
    // The failure's span announced the retry, decided before the wait; the
    // run's outcome says why the retry never came.
    let [failed] = failures(&run).try_into().expect("one attempt failed");
    assert_eq!((failed.will_retry, failed.backoff_ms), (true, Some(100)));
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
    let [failed] = failures(&run).try_into().expect("one attempt failed");
    assert!(
        !failed.will_retry,
        "the call was over when the attempt failed"
    );
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
    let [failed] = failures(&run).try_into().expect("one attempt failed");
    assert_eq!((failed.will_retry, failed.backoff_ms), (true, Some(5_000)));
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
    assert_eq!(run.finished.summary.provider.retries, 0);
    let [failed] = failures(&run).try_into().expect("one attempt failed");
    assert_eq!((failed.will_retry, failed.backoff_ms), (false, None));
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
    let failed = &run.chats()[0];
    assert_eq!(text(failed, key::ERROR_TYPE), "retryable");
    assert_eq!(
        (
            count(failed, key::GEN_AI_USAGE_INPUT_TOKENS),
            count(failed, key::GEN_AI_USAGE_OUTPUT_TOKENS),
        ),
        (70, 5),
        "the failed attempt's span holds what it used"
    );
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
    assert_eq!(run.finished.summary.provider.retries, 0);
    assert_eq!(run.turns(), 0);
    let [failed] = failures(&run).try_into().expect("one attempt failed");
    assert_eq!(failed.error_type, "auth");
    assert_eq!((failed.will_retry, failed.backoff_ms), (false, None));
    assert_eq!(
        record_text(&run.exceptions()[0], key::EXCEPTION_TYPE),
        "auth"
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
    let waits = run.clock.sleeps();
    let announced: Vec<Option<i64>> = failures(&run)
        .into_iter()
        .map(|failed| failed.backoff_ms)
        .collect();
    let whole: Vec<Option<i64>> = waits
        .iter()
        .map(|wait| Some(i64::try_from(wait.as_millis()).expect("a wait of some ms")))
        .collect();
    assert_eq!(
        announced, whole,
        "each span announces the wait the loop took, in whole milliseconds"
    );
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
        let [failed] = failures(&run).try_into().expect("one attempt failed");
        assert!(!failed.will_retry, "{max_retries}");
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
        let [failed] = failures(&run).try_into().expect("one attempt failed");
        assert!(!failed.will_retry, "{max_retries}");
    }
}

/// Only a failure that could be retried is held to point A. One that
/// couldn't is why the run ended, however long the attempt took.
#[tokio::test]
async fn a_failure_no_attempt_could_answer_ends_the_run_as_itself_though_the_time_is_up() {
    for kind in ProviderErrorKind::ALL {
        let reason = match kind {
            ProviderErrorKind::ContextExhausted => StopReason::ContextExhausted,
            ProviderErrorKind::Auth | ProviderErrorKind::Fatal => StopReason::ProviderError,
            ProviderErrorKind::Retryable | ProviderErrorKind::Malformed => continue,
        };
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

// E16: no call's deadline is later than the run's, and a call whose turn
// comes when the run's time has gone is never run.

const fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

const NEVER_STARTED: &str = "the run reached its timeout before this call started";

/// A run with a timeout of 10 s and an output cap of 4 bytes, whose one
/// response arrives 9 s in and makes two calls to `tool`, each of which
/// would take `each`; and the executor, which serves `tool` as `concurrency`
/// says.
async fn two_calls_nine_seconds_in(
    tool: &str,
    concurrency: ToolConcurrency,
    each: Duration,
) -> (Run, Arc<FakeTools>) {
    let mut harness = Harness::new(vec![Answer::Responds(
        Box::new(says("On it.", &[tool, tool], FinishReason::ToolUse)),
        secs(9),
    )]);
    let tools = Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![ToolSpec {
                concurrency,
                ..spec(tool, ToolSource::Builtin)
            }],
        )
        .taking(each),
    );
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];
    harness.stop.timeout = secs(10);
    harness.calls.output_cap = Some(output_cap(4, OutputCut::Head));
    harness.calls.max_concurrent_tool_calls = nz(1);
    (harness.run().await, tools)
}

/// The id and the deadline of every call an executor was handed, in order.
fn deadlines(tools: &FakeTools) -> Vec<(String, Duration)> {
    tools
        .taken()
        .iter()
        .map(|call| (call.id.as_str().to_owned(), call.deadline))
        .collect()
}

#[tokio::test]
async fn a_call_whose_turn_comes_when_the_run_s_time_has_gone_is_never_run() {
    let (run, tools) = two_calls_nine_seconds_in("bash", ToolConcurrency::Exclusive, secs(1)).await;

    assert_eq!(
        deadlines(&tools),
        [("call_0".to_owned(), secs(1))],
        "the first call has the second the run has left, and the executor never sees the other"
    );
    let outcomes = run.finished.transcript.turns()[0].tool_calls();
    assert_eq!(
        outcomes[0].status,
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Timeout),
        "the first call took all of its second, so it reached its deadline and timed out"
    );
    assert_eq!(
        (outcomes[0].started_ms, outcomes[0].latency_ms),
        (9_000, 1_000)
    );
    assert_eq!(
        outcomes[1],
        lablet_model::ToolCallOutcome {
            call_id: ToolCallId::new("call_1").expect("a valid call id"),
            status: ToolCallStatus::NotRun,
            started_ms: 10_000,
            latency_ms: 0,
            truncated_from_bytes: None,
            content: vec![lablet_model::ToolResultContent::Text(
                NEVER_STARTED.to_owned()
            )],
        },
        "nothing of the call was there for the output cap to cut"
    );
    assert_eq!(run.stop_reason(), StopReason::Timeout);
    assert_eq!(run.error(), None);
    assert_eq!(run.provider.calls(), 1, "point B stops the run");
    assert_eq!(run.finished.summary.outcome.duration_ms, 10_000);
}

#[tokio::test]
async fn a_call_that_was_never_run_has_no_span_and_no_total() {
    let (run, _) = two_calls_nine_seconds_in("bash", ToolConcurrency::Exclusive, secs(1)).await;

    assert_eq!(chat_shape(&run), [(1, 1, None)]);
    assert_eq!(
        tool_shape(&run),
        [(1, "call_0".to_owned(), "timeout".to_owned())],
        "the call that ran has a span, and the one that never started has none"
    );
    let summary = &run.finished.summary;
    assert_eq!(summary.outcome.tool_calls, 1);
    assert_eq!(
        summary.tool_calls.errors, 1,
        "the first call timed out at the run's deadline"
    );
    assert_eq!(summary.tool_calls.unknown, 0);
    assert_eq!(
        summary.tool_calls.truncated, 1,
        "`bash was stopped at its deadline` is over the cap"
    );
    assert_eq!(summary.tool_calls.latency_ms, 1_000);
    assert_eq!(
        summary.tool_calls.input_bytes, 7,
        "the first call's `{{\"n\":0}}`"
    );
    assert_eq!(
        summary.tool_calls.output_bytes,
        4 + 36,
        "`bash` and `[truncated: the first 4 of 32 bytes]`"
    );
    assert_eq!(
        summary.per_tool,
        std::collections::BTreeMap::from([(
            name("bash"),
            lablet_model::ToolStats {
                calls: 1,
                errors: 1,
                latency_ms: 1_000,
            }
        )])
    );
}

/// A place in a group is a turn too: with one call running at a time, the
/// second read waits for the first, and the time has gone when it ends.
#[tokio::test]
async fn a_call_still_waiting_for_a_place_in_its_group_when_the_time_goes_is_never_run() {
    let (run, tools) =
        two_calls_nine_seconds_in("read_file", ToolConcurrency::Shared, secs(1)).await;

    assert_eq!(deadlines(&tools), [("call_0".to_owned(), secs(1))]);
    assert_eq!(statuses(&run), [vec!["timeout", "not_run"]]);
    assert_eq!(run.stop_reason(), StopReason::Timeout);
}

/// The executor takes no longer than the deadline, so the run ends when its
/// timeout says and not when the tool would have.
#[tokio::test]
async fn a_tool_that_would_outlast_the_run_is_stopped_when_the_run_s_time_is_up() {
    let (run, tools) = two_calls_nine_seconds_in("bash", ToolConcurrency::Exclusive, secs(5)).await;

    assert_eq!(deadlines(&tools), [("call_0".to_owned(), secs(1))]);
    let outcomes = run.finished.transcript.turns()[0].tool_calls();
    assert_eq!(
        outcomes[0].status,
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Timeout)
    );
    assert_eq!(outcomes[0].latency_ms, 1_000);
    assert_eq!(outcomes[1].status, ToolCallStatus::NotRun);
    assert_eq!(run.stop_reason(), StopReason::Timeout);
    assert_eq!(run.finished.summary.outcome.duration_ms, 10_000);
}

/// The time left is read when a call's turn comes, so a call that runs
/// after another has what the other left it.
#[tokio::test]
async fn a_tool_call_is_given_the_time_the_run_has_left_when_its_turn_comes() {
    let mut harness = Harness::new(vec![
        Answer::Responds(
            Box::new(says("On it.", &["bash", "bash"], FinishReason::ToolUse)),
            secs(3),
        ),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    let tools = Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash", ToolSource::Builtin)],
        )
        .taking(secs(2)),
    );
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];
    harness.stop.timeout = secs(10);

    let run = harness.run().await;

    assert_eq!(
        deadlines(&tools),
        [
            ("call_0".to_owned(), secs(7)),
            ("call_1".to_owned(), secs(5)),
        ]
    );
    assert_eq!(statuses(&run), [vec!["ok", "ok"], vec![]]);
    assert_eq!(run.stop_reason(), StopReason::Completed);
}

/// Point R reads the response alone and lets this run go on. The question
/// asked after it is what stops the run, with the turn's call unanswered,
/// as any stop ahead of a tool phase leaves it.
#[tokio::test]
async fn no_tool_starts_in_a_run_whose_response_used_up_its_time() {
    let mut harness = Harness::new(vec![Answer::Responds(
        Box::new(says("On it.", &["bash"], FinishReason::ToolUse)),
        secs(10),
    )]);
    let tools = Arc::new(FakeTools::new(
        Arc::clone(&harness.clock),
        vec![spec("bash", ToolSource::Builtin)],
    ));
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];
    harness.stop.timeout = secs(10);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Timeout);
    assert_eq!(run.turns(), 1);
    assert_eq!(run.provider.calls(), 1);
    assert_eq!(tools.taken(), [], "no executor was reached");
    assert!(run.tools().is_empty(), "and no call has a span");
    let turn = &run.finished.transcript.turns()[0];
    assert_eq!(turn.tool_uses().count(), 1);
    assert_eq!(turn.tool_calls(), [], "the call stays unanswered");
    assert_eq!(run.finished.summary.outcome.tool_calls, 0);
}

/// Each attempt's deadline is taken when the attempt begins, a retry's
/// too: the third attempt begins 45.1 s into a run of 100 s.
#[tokio::test]
async fn a_provider_attempt_is_given_the_shorter_of_the_provider_timeout_and_the_time_left() {
    let mut harness = Harness::new(vec![
        Answer::Responds(
            Box::new(says("On it.", &["bash"], FinishReason::ToolUse)),
            secs(30),
        ),
        Answer::Fails(overloaded(), secs(15)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.stop.timeout = secs(100);
    harness.calls.provider_timeout = secs(60);

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(run.clock.sleeps(), [ms(100)]);
    assert_eq!(run.provider.deadlines(), [secs(60), secs(60), ms(54_900)]);
}

// O11: the labels a run was asked for under. Every span and record carries
// them, and the outcome holds a copy.

fn labelled() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: Some("terse-tool-descriptions".to_owned()),
        trial: Some("3".to_owned()),
    }
}

/// A run that answers at once, and one whose only provider call fails, so
/// the labels aren't those of a run that went well alone.
fn a_short_run_and_a_failed_one() -> [Vec<Answer>; 2] {
    [
        vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))],
        vec![Answer::fails(ProviderErrorKind::Fatal)],
    ]
}

/// The labels every span and every record of `run` carries, by key, one
/// map for each signal.
fn labels_on(run: &Run) -> Vec<BTreeMap<String, serde_json::Value>> {
    let labels = [
        key::LABLET_TASK_ID,
        key::LABLET_EXPERIMENT_ID,
        key::LABLET_TRIAL,
    ];
    run.spans()
        .iter()
        .map(attributes)
        .chain(run.records().iter().map(record_attributes))
        .map(|attributes| {
            attributes
                .into_iter()
                .filter(|(key, _)| labels.contains(&key.as_str()))
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn the_labels_of_the_context_are_in_the_outcome_and_on_every_span_and_record() {
    let partly = RunLabels {
        experiment: None,
        ..labelled()
    };
    for labels in [labelled(), partly] {
        for script in a_short_run_and_a_failed_one() {
            let mut harness = Harness::new(script);
            harness.context.labels = labels.clone();
            harness.context.capture_content = true;

            let run = harness.run().await;

            assert_eq!(run.finished.summary.outcome.labels, labels);
            let expected: BTreeMap<String, serde_json::Value> = [
                (key::LABLET_TASK_ID, &labels.task),
                (key::LABLET_EXPERIMENT_ID, &labels.experiment),
                (key::LABLET_TRIAL, &labels.trial),
            ]
            .into_iter()
            .filter_map(|(key, value)| Some((key.to_owned(), json!(value.as_ref()?))))
            .collect();
            let signals = labels_on(&run);
            assert!(
                signals.len() >= 3,
                "a span and two records at least: {signals:?}"
            );
            for on in signals {
                assert_eq!(on, expected);
            }
        }
    }
}

/// How an outcome without labels is written down is the outcome document's
/// to hold, which the loop doesn't know.
#[tokio::test]
async fn a_run_asked_for_under_no_label_has_none_in_its_outcome_or_on_its_signals() {
    for script in a_short_run_and_a_failed_one() {
        let mut harness = Harness::new(script);
        harness.context.capture_content = true;

        let run = harness.run().await;

        assert_eq!(run.finished.summary.outcome.labels, RunLabels::default());
        let signals = labels_on(&run);
        assert!(signals.len() >= 3, "{signals:?}");
        for on in signals {
            assert_eq!(on, BTreeMap::new());
        }
    }
}

// O12: the digests of what the model was shown. Each digest below is what
// `shasum -a 256` gives for the bytes written out beside it, so these hold
// which hash it is, what's hashed and how it's written, where a test that
// only compared two runs would hold none of the three.

const BASH_SPEC: &str = r#"{"name":"bash","description":"The bash tool.","input_schema":{"type":"object"},"source":"builtin","concurrency":"exclusive"}"#;
const READ_FILE_SPEC: &str = r#"{"name":"read_file","description":"The read_file tool.","input_schema":{"type":"object"},"source":"builtin","concurrency":"exclusive"}"#;
const DIGEST_OF_BASH_THEN_READ_FILE: &str =
    "75502ef02ef4cc26f3b03bd9bf53705051bb3e0f81846f14abea2ed4405735db";
const DIGEST_OF_READ_FILE_THEN_BASH: &str =
    "57c11b3c1a827fcf4e5b3683d74dbce28d3646e326a62e2657c3ba88e5be64b0";
const DIGEST_OF_YOU_FIX_TESTS: &str =
    "0897299beed5fb8987943bb73e81f94c9b18c4caeb6abe5eee6009f4c180810d";
const DIGEST_OF_NOTHING: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn answering_at_once() -> Vec<Answer> {
    vec![Answer::now(says("Done.", &[], FinishReason::EndTurn))]
}

/// A run offered `specs` under `system`, which answers at once.
async fn shown(specs: Vec<ToolSpec>, system: &str) -> Run {
    let mut harness = Harness::new(answering_at_once());
    harness.tools = vec![Arc::new(FakeTools::new(Arc::clone(&harness.clock), specs))];
    harness.prompts = Prompts::new(system, "Fix the failing test.").expect("the task isn't blank");
    harness.run().await
}

/// The specs the provider was sent on the run's first attempt, as the bytes
/// the digest is of.
fn specs_sent(run: &Run) -> String {
    run.provider.shown()[0]
        .tools
        .iter()
        .map(|spec| serde_json::to_string(spec).expect("a spec serialises"))
        .collect()
}

fn summary(run: &Run) -> &RunSummary {
    &run.finished.summary
}

#[tokio::test]
async fn the_tools_digest_is_the_sha_256_of_the_specs_sent_as_compact_json_in_the_order_offered() {
    let run = Harness::new(answering_at_once()).run().await;

    assert_eq!(specs_sent(&run), format!("{BASH_SPEC}{READ_FILE_SPEC}"));
    assert_eq!(summary(&run).tools_digest, DIGEST_OF_BASH_THEN_READ_FILE);
    assert_eq!(
        summary(&run).prompt.tools_bytes,
        (BASH_SPEC.len() + READ_FILE_SPEC.len()) as u64
    );
}

#[tokio::test]
async fn the_system_prompt_digest_is_the_sha_256_of_the_system_prompt_sent() {
    let run = Harness::new(answering_at_once()).run().await;

    assert_eq!(run.provider.shown()[0].system, "You fix tests.");
    assert_eq!(summary(&run).system_prompt_digest, DIGEST_OF_YOU_FIX_TESTS);
}

#[tokio::test]
async fn a_run_shown_no_tools_and_no_system_prompt_reports_the_digest_of_nothing_for_each() {
    let run = shown(Vec::new(), "").await;

    assert_eq!(specs_sent(&run), "");
    assert_eq!(run.provider.shown()[0].system, "");
    assert_eq!(summary(&run).prompt.tools_bytes, 0);
    assert_eq!(summary(&run).tools_digest, DIGEST_OF_NOTHING);
    assert_eq!(summary(&run).system_prompt_digest, DIGEST_OF_NOTHING);
}

#[tokio::test]
async fn the_order_the_specs_are_offered_in_is_part_of_their_digest() {
    let run = shown(
        vec![
            spec("read_file", ToolSource::Builtin),
            spec("bash", ToolSource::Builtin),
        ],
        "You fix tests.",
    )
    .await;

    assert_eq!(specs_sent(&run), format!("{READ_FILE_SPEC}{BASH_SPEC}"));
    assert_eq!(summary(&run).tools_digest, DIGEST_OF_READ_FILE_THEN_BASH);
    assert_eq!(
        summary(&run).prompt.tools_bytes,
        (BASH_SPEC.len() + READ_FILE_SPEC.len()) as u64,
        "the size can't tell the two orders apart, which is what the digest is for"
    );
}

/// The two descriptions are as long as each other, so nothing but the
/// digest tells the two tool sets apart.
#[tokio::test]
async fn one_tool_set_run_twice_has_one_digest_and_one_changed_description_gives_another() {
    let described = |description: &str| {
        vec![
            spec("bash", ToolSource::Builtin),
            ToolSpec {
                description: description.to_owned(),
                ..spec("read_file", ToolSource::Builtin)
            },
        ]
    };
    let first = shown(described("Reads a file."), "You fix tests.").await;
    let again = shown(described("Reads a file."), "You fix tests.").await;
    let changed = shown(described("Reads a path."), "You fix tests.").await;

    assert_eq!(summary(&first).tools_digest, summary(&again).tools_digest);
    assert_ne!(summary(&first).tools_digest, summary(&changed).tools_digest);
    assert_eq!(
        summary(&first).prompt.tools_bytes,
        summary(&changed).prompt.tools_bytes
    );
    for run in [&first, &again, &changed] {
        assert_eq!(summary(run).system_prompt_digest, DIGEST_OF_YOU_FIX_TESTS);
    }
}

#[tokio::test]
async fn one_system_prompt_run_twice_has_one_digest_and_one_changed_word_gives_another() {
    let specs = || {
        vec![
            spec("bash", ToolSource::Builtin),
            spec("read_file", ToolSource::Builtin),
        ]
    };
    let first = shown(specs(), "You fix tests.").await;
    let again = shown(specs(), "You fix tests.").await;
    let changed = shown(specs(), "You fix tasks.").await;

    assert_eq!(
        summary(&first).system_prompt_digest,
        summary(&again).system_prompt_digest
    );
    assert_ne!(
        summary(&first).system_prompt_digest,
        summary(&changed).system_prompt_digest
    );
    assert_eq!(
        summary(&first).prompt.system_bytes,
        summary(&changed).prompt.system_bytes
    );
    for run in [&first, &again, &changed] {
        assert_eq!(summary(run).tools_digest, DIGEST_OF_BASH_THEN_READ_FILE);
    }
}

/// The digests are of what the model is shown and of nothing else about the
/// run, or the same variant run as two trials would look like two variants.
#[tokio::test]
async fn two_runs_shown_the_same_have_the_same_digests_whatever_else_tells_them_apart() {
    let mut other = Harness::new(vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    other.context.run_id = RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNED").expect("a valid run id");
    other.context.labels = labelled();
    other.request.cache_scope = CacheScope::Run;
    other.prompts = Prompts::new("You fix tests.", "Fix the other failing test.")
        .expect("the task isn't blank");

    let other = other.run().await;

    assert_eq!(summary(&other).tools_digest, DIGEST_OF_BASH_THEN_READ_FILE);
    assert_eq!(
        summary(&other).system_prompt_digest,
        DIGEST_OF_YOU_FIX_TESTS
    );
}

/// An explicit run offers `task_complete` itself, and the model is shown it
/// like any tool an executor serves.
#[tokio::test]
async fn the_completion_tool_a_run_offers_is_among_the_specs_its_digest_is_of() {
    let mut explicit = Harness::new(answering_at_once());
    explicit.completion = CompletionMode::Explicit;

    let explicit = explicit.run().await;

    let sent = specs_sent(&explicit);
    let own = sent
        .strip_prefix(&format!("{BASH_SPEC}{READ_FILE_SPEC}"))
        .expect("the executor's specs come first");
    assert!(own.starts_with(r#"{"name":"task_complete","#), "{own}");
    assert_eq!(summary(&explicit).prompt.tools_bytes, sent.len() as u64);
    assert_ne!(
        summary(&explicit).tools_digest,
        DIGEST_OF_BASH_THEN_READ_FILE
    );
}

// The cache key: the run id when the run has a cache of its own, and no part
// of what the digests and the sizes are of.

fn failing_once_then_taking_two_turns() -> Vec<Answer> {
    vec![
        Answer::fails(ProviderErrorKind::Retryable),
        Answer::now(says("On it.", &["bash"], FinishReason::ToolUse)),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]
}

async fn scoped(scope: CacheScope) -> Run {
    let mut harness = Harness::new(failing_once_then_taking_two_turns());
    harness.request.cache_scope = scope;
    harness.run().await
}

fn cache_keys(run: &Run) -> Vec<Option<String>> {
    run.provider
        .shown()
        .into_iter()
        .map(|shown| shown.cache_key)
        .collect()
}

#[tokio::test]
async fn a_run_with_a_cache_of_its_own_sends_its_id_as_the_cache_key_of_every_attempt() {
    let run = scoped(CacheScope::Run).await;

    assert_eq!(
        cache_keys(&run),
        vec![Some("01K5F3Z8Q4X9T2M7B6W1R0VNEC".to_owned()); 3],
        "the attempt that failed, the one that followed it, and the next turn's"
    );
    assert_eq!(summary(&run).request.cache_scope, CacheScope::Run);
}

#[tokio::test]
async fn a_run_that_shares_its_cache_sends_no_cache_key() {
    let run = scoped(CacheScope::Shared).await;

    assert_eq!(cache_keys(&run), vec![None; 3]);
    assert_eq!(summary(&run).request.cache_scope, CacheScope::Shared);
}

#[tokio::test]
async fn the_cache_key_is_no_part_of_the_system_prompt_nor_of_its_size_or_its_digest() {
    let shared = scoped(CacheScope::Shared).await;
    let own = scoped(CacheScope::Run).await;

    for run in [&shared, &own] {
        let systems: Vec<String> = run
            .provider
            .shown()
            .into_iter()
            .map(|shown| shown.system)
            .collect();
        assert_eq!(systems, ["You fix tests."; 3]);
        assert_eq!(run.finished.transcript.system(), "You fix tests.");
        assert_eq!(summary(run).prompt.system_bytes, 14);
        assert_eq!(summary(run).system_prompt_digest, DIGEST_OF_YOU_FIX_TESTS);
        assert_eq!(summary(run).tools_digest, DIGEST_OF_BASH_THEN_READ_FILE);
    }
    assert_eq!(request_bytes(&shared), request_bytes(&own));
    assert_eq!(shared.finished.transcript, own.finished.transcript);
}

// O15: how the model was reached. The provider says which API it speaks and
// whether it sends reasoning back, and the cache scope is the service's own.

#[tokio::test]
async fn the_summary_names_the_api_whether_reasoning_is_sent_back_and_the_cache_scope() {
    let reached = ModelRef {
        api: ProviderApi::ChatCompletions,
        name: "qwen3".to_owned(),
        replays_reasoning: true,
    };
    let mut harness = Harness::new(Vec::new());
    harness.provider = Arc::new(FakeProvider::new(
        reached.clone(),
        Arc::clone(&harness.clock),
        answering_at_once(),
    ));
    harness.request.cache_scope = CacheScope::Run;
    let asked = harness.request.clone();

    let run = harness.run().await;

    assert_eq!(summary(&run).model, reached);
    assert_eq!(summary(&run).request, asked);
    let [chat] = run.chats().try_into().expect("one attempt");
    assert_eq!(
        text(&chat, key::GEN_AI_PROVIDER_NAME),
        "openai",
        "the span names the provider the API belongs to"
    );
    assert_eq!(text(&chat, key::GEN_AI_REQUEST_MODEL), "qwen3");
}

#[tokio::test]
async fn a_scripted_run_that_shares_its_cache_says_so_in_its_summary() {
    let run = Harness::new(answering_at_once()).run().await;

    assert_eq!(summary(&run).model.api, ProviderApi::Script);
    assert!(!summary(&run).model.replays_reasoning);
    assert_eq!(summary(&run).request.cache_scope, CacheScope::Shared);
}

// The loop cuts the run's secrets out of every text it writes itself.

/// A value of lablet's that no text the model is sent may hold.
const SECRET: &str = "sk-0123456789abcdef-a-key-no-model-reads";

/// The run's secrets reach every executor on its call, and are cut out of
/// an executor's error message and out of the arguments of a call that
/// didn't parse, which the loop echoes to the model.
#[tokio::test]
async fn the_secrets_a_run_holds_reach_the_executor_and_are_cut_out_of_the_loop_s_own_texts() {
    let mut harness = Harness::new(vec![
        Answer::now(says("One.", &["bash"], FinishReason::ToolUse)),
        Answer::now(calls_with_unparsed_input(
            "read_file",
            &format!("{{\"path\": \"{SECRET}"),
        )),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    let secrets = Arc::new(lablet_model::Secrets::new([SECRET.to_owned()]));
    harness.secrets = Arc::clone(&secrets);
    let tools = Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![
                spec("bash", ToolSource::Builtin),
                spec("read_file", ToolSource::Builtin),
            ],
        )
        .answers(
            "bash",
            Answers::Fails(
                crate::ToolErrorKind::Failed,
                format!("bash couldn't be started: {SECRET} isn't a shell"),
            ),
        ),
    );
    harness.tools = vec![Arc::clone(&tools) as Arc<dyn crate::ToolExecutor>];

    let run = harness.run().await;

    assert_eq!(run.stop_reason(), StopReason::Completed);
    assert_eq!(
        contents(&run),
        [
            vec!["bash couldn't be started: [secret withheld] isn't a shell".to_owned()],
            vec![
                "the arguments weren't valid JSON, so read_file wasn't called: {\"path\": \
                 \"[secret withheld]"
                    .to_owned()
            ],
            vec![],
        ]
    );
    let taken = tools.taken();
    assert_eq!(
        taken.len(),
        1,
        "the unparsed call never reached the executor"
    );
    assert!(
        Arc::ptr_eq(&taken[0].secrets, &secrets),
        "the executor was handed the run's secrets"
    );
}

// The spans and records, one by one: what each says, read from the exporters
// the loop emitted through, and the keys each carries.

/// The attributes of a tool span that a call over MCP carries back.
const OVER_MCP: [&str; 6] = [
    key::JSONRPC_REQUEST_ID,
    key::MCP_METHOD_NAME,
    key::MCP_PROTOCOL_VERSION,
    key::MCP_SESSION_ID,
    key::NETWORK_TRANSPORT,
    key::RPC_RESPONSE_STATUS_CODE,
];

/// The keys that hold content, which no span carries.
const HOLD_CONTENT: [&str; 6] = [
    key::GEN_AI_SYSTEM_INSTRUCTIONS,
    key::GEN_AI_INPUT_MESSAGES,
    key::GEN_AI_OUTPUT_MESSAGES,
    key::GEN_AI_TOOL_DEFINITIONS,
    key::GEN_AI_TOOL_CALL_ARGUMENTS,
    key::GEN_AI_TOOL_CALL_RESULT,
];

/// `span` carries every key of `required`, which the registry requires of
/// its kind, and no key that isn't one of `declared`, which it declares on
/// it.
fn assert_declared(span: &SpanData, required: &[&str], declared: &[&str]) {
    let keys: std::collections::BTreeSet<&str> = span
        .attributes
        .iter()
        .map(|attribute| attribute.key.as_str())
        .collect();
    for key in required {
        assert!(keys.contains(key), "{key} is missing from {}", span.name);
    }
    for key in &keys {
        assert!(
            declared.contains(key),
            "{key} isn't declared on {}",
            span.name
        );
    }
}

/// The attributes `run` writes on every span and record, as JSON by key.
fn joined(run: &Run) -> Vec<(&'static str, serde_json::Value)> {
    vec![
        (key::GEN_AI_CONVERSATION_ID, json!(RUN)),
        (key::SESSION_ID, json!(RUN)),
        (key::LABLET_CONFIG_DIGEST, json!(CONFIG_DIGEST)),
        (key::LABLET_TASK_ID, json!("fix-failing-test")),
        (key::LABLET_EXPERIMENT_ID, json!("terse-tool-descriptions")),
        (key::LABLET_TRIAL, json!("3")),
    ]
    .into_iter()
    .filter(|_| run.finished.summary.outcome.labels == labelled())
    .collect()
}

/// A map of attributes from `pairs`, for an exact comparison.
fn map(pairs: Vec<(&str, serde_json::Value)>) -> BTreeMap<String, serde_json::Value> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

/// A run that fails once with usage, answers with a response that names its
/// id and model and calls `bash`, which takes a second, and `invented`, and
/// then ends; asked for with every request parameter set, from a provider
/// reached over the network, under every label, with every call running
/// alone, so the spans end in call order.
fn fully_described() -> Harness {
    let answered = ProviderResponse::new(
        says("On it.", &["bash", "invented"], FinishReason::ToolUse)
            .content()
            .to_vec(),
        Usage {
            input_tokens: 1_000,
            output_tokens: 50,
            reasoning_output_tokens: Some(5),
            cache_read_tokens: Some(200),
            cache_write_tokens: Some(30),
        },
        FinishReason::ToolUse,
        Some("msg_01".to_owned()),
        Some("fake-2026-09".to_owned()),
    )
    .expect("distinct call ids");
    let mut harness = Harness::new(Vec::new());
    harness.provider = Arc::new(
        FakeProvider::new(
            model(),
            Arc::clone(&harness.clock),
            vec![
                Answer::Fails(
                    overloaded().with_usage(Usage {
                        input_tokens: 800,
                        output_tokens: 0,
                        reasoning_output_tokens: None,
                        cache_read_tokens: Some(600),
                        cache_write_tokens: None,
                    }),
                    ms(40),
                ),
                Answer::Responds(Box::new(answered), ms(250)),
                Answer::Responds(Box::new(says("Done.", &[], FinishReason::EndTurn)), ms(120)),
            ],
        )
        .at(Endpoint {
            host: "localhost".to_owned(),
            port: 11434,
        }),
    );
    harness.request.temperature = Some(0.2);
    harness.request.seed = Some(7);
    harness.request.effort = Some(Effort::High);
    harness.context.labels = labelled();
    harness.calls.max_concurrent_tool_calls = nz(1);
    harness.tools = vec![Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![
                spec("bash", ToolSource::Builtin),
                spec("read_file", ToolSource::Builtin),
            ],
        )
        .answers("bash", Answers::Text("ok".to_owned()))
        .taking(ms(1_000)),
    )];
    harness
}

/// What every chat span of [`fully_described`] says the run asked for.
fn asked() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        (key::GEN_AI_OPERATION_NAME, json!("chat")),
        (key::LABLET_CHAT_PURPOSE, json!("turn")),
        (key::GEN_AI_PROVIDER_NAME, json!("fake")),
        (key::GEN_AI_REQUEST_MODEL, json!("fake-1")),
        (key::GEN_AI_REQUEST_MAX_TOKENS, json!(4_096)),
        (key::GEN_AI_REQUEST_TEMPERATURE, json!(0.2)),
        (key::GEN_AI_REQUEST_SEED, json!(7)),
        (key::GEN_AI_REQUEST_REASONING_LEVEL, json!("high")),
        (key::SERVER_ADDRESS, json!("localhost")),
        (key::SERVER_PORT, json!(11_434)),
    ]
}

#[tokio::test]
async fn every_span_is_a_sampled_child_of_the_context_the_run_was_called_in() {
    let run = fully_described().run().await;

    let spans = run.spans();
    assert_eq!(spans.len(), 3 + 2);
    for span in &spans {
        assert_eq!(span.parent_span_id, run.root.span_id(), "{}", span.name);
        assert_eq!(span.span_context.trace_id(), run.root.trace_id());
        assert!(span.span_context.is_sampled(), "{}", span.name);
        assert!(span.span_context.is_valid());
    }
    let named: Vec<_> = spans
        .iter()
        .map(|span| (span.name.as_ref(), span.span_kind.clone()))
        .collect();
    assert_eq!(
        named,
        [
            ("chat fake-1", SpanKind::Client),
            ("chat fake-1", SpanKind::Client),
            ("execute_tool bash", SpanKind::Internal),
            ("execute_tool invented", SpanKind::Internal),
            ("chat fake-1", SpanKind::Client),
        ]
    );
    for chat in run.chats() {
        assert_declared(&chat, key::LABLET_CHAT_REQUIRED, key::LABLET_CHAT_KEYS);
    }
    for tool in run.tools() {
        assert_declared(
            &tool,
            key::LABLET_EXECUTE_TOOL_REQUIRED,
            key::LABLET_EXECUTE_TOOL_KEYS,
        );
    }
}

/// Every time a span, an event or a record carries is the run's start plus
/// what the loop measured on its clock, to the nanosecond, while each `*_ms`
/// value is the whole milliseconds of the time or the length it measures.
#[tokio::test]
async fn a_span_event_and_record_are_timed_to_the_nanosecond_and_their_ms_values_are_cut() {
    let mut harness = Harness::new(vec![
        Answer::Fails(overloaded(), us(250)),
        Answer::Responds(
            Box::new(says("On it.", &["bash"], FinishReason::ToolUse)),
            us(250),
        ),
        Answer::Responds(Box::new(says("Done.", &[], FinishReason::EndTurn)), us(125)),
    ]);
    harness.tools = vec![Arc::new(
        FakeTools::new(
            Arc::clone(&harness.clock),
            vec![spec("bash", ToolSource::Builtin)],
        )
        .taking(us(1_500)),
    )];
    harness.context.capture_content = true;

    let run = harness.run().await;

    let at = |offset: Duration| started() + offset;
    let lasted = |span: &SpanData| span.end_time.duration_since(span.start_time).unwrap();
    let [failed, answered, done] = run.chats().try_into().expect("three attempts");
    let [tool] = run.tools().try_into().expect("one call");
    assert_eq!(run.clock.sleeps(), [ms(100)]);
    assert_eq!(
        (failed.start_time, failed.end_time),
        (at(ms(0)), at(us(250)))
    );
    assert_eq!(answered.start_time, at(us(100_250)));
    assert_eq!(lasted(&answered), us(250));
    assert_eq!(
        (tool.start_time, tool.end_time),
        (at(us(100_500)), at(ms(102)))
    );
    assert_eq!(
        (done.start_time, done.end_time),
        (at(ms(102)), at(us(102_125)))
    );
    assert_eq!(run.finished.duration, us(102_125));

    let [retry] = failed.events.events.as_slice() else {
        panic!("one event: {:?}", failed.events);
    };
    assert_eq!(retry.timestamp, at(us(250)));
    let [exception] = run.exceptions().try_into().expect("one exception");
    assert_eq!(exception.timestamp(), Some(at(us(250))));
    assert_eq!(exception.observed_timestamp(), Some(at(us(250))));
    let times: Vec<_> = run
        .content()
        .iter()
        .map(|record| (record.timestamp(), record.observed_timestamp()))
        .collect();
    let both = |offset| (Some(at(offset)), Some(at(offset)));
    assert_eq!(
        times,
        [
            both(ms(0)),
            both(us(250)),
            both(us(100_500)),
            both(ms(102)),
            both(us(102_125))
        ],
        "the tools offered, then each attempt and the call as it ended"
    );

    let summary = &run.finished.summary;
    let latency = summary.provider.latency;
    assert_eq!((latency.total_ms(), latency.max_ms()), (0, 0));
    assert_eq!(summary.tool_calls.latency_ms, 1);
    assert_eq!(summary.outcome.duration_ms, 102);
    let turns = run.finished.transcript.turns();
    let record = turns[0].record();
    assert_eq!((record.started_ms, record.latency_ms), (100, 0));
    let outcome = &turns[0].tool_calls()[0];
    assert_eq!((outcome.started_ms, outcome.latency_ms), (100, 1));
    let backoff = retry
        .attributes
        .iter()
        .find(|attribute| attribute.key.as_str() == key::LABLET_RETRY_BACKOFF_MS)
        .map(|attribute| attribute.value.clone());
    assert_eq!(backoff, Some(Value::I64(100)));
}

#[tokio::test]
async fn a_chat_span_says_what_the_attempt_was_asked_and_what_it_answered() {
    let run = fully_described().run().await;

    let answered = &run.chats()[1];
    let mut expected = joined(&run);
    expected.extend(asked());
    expected.extend([
        (key::LABLET_TURN, json!(1)),
        (key::LABLET_ATTEMPT, json!(2)),
        (key::LABLET_REQUEST_BYTES, json!(request_bytes(&run)[1])),
        (key::GEN_AI_RESPONSE_ID, json!("msg_01")),
        (key::GEN_AI_RESPONSE_MODEL, json!("fake-2026-09")),
        (key::GEN_AI_RESPONSE_FINISH_REASONS, json!(["tool_use"])),
        (key::GEN_AI_USAGE_INPUT_TOKENS, json!(1_000)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, json!(50)),
        (key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS, json!(5)),
        (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, json!(200)),
        (key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS, json!(30)),
    ]);
    assert_eq!(attributes(answered), map(expected));
    assert_eq!(answered.status, Status::Unset);
    assert!(answered.events.is_empty());
    assert_eq!(answered.name, "chat fake-1");
    assert_eq!(
        timing(answered),
        (140, 250),
        "the attempt began after the backoff"
    );
    let bytes = request_bytes(&run);
    assert_eq!(
        bytes[0], bytes[1],
        "a retry sends what the attempt before it sent"
    );
    assert!(bytes[1] < bytes[2], "{bytes:?}");
}

#[tokio::test]
async fn the_span_of_a_failed_attempt_says_how_it_failed_and_what_the_loop_did_next() {
    let run = fully_described().run().await;

    let failed = &run.chats()[0];
    let mut expected = joined(&run);
    expected.extend(asked());
    expected.extend([
        (key::LABLET_TURN, json!(1)),
        (key::LABLET_ATTEMPT, json!(1)),
        (key::LABLET_REQUEST_BYTES, json!(request_bytes(&run)[0])),
        (key::ERROR_TYPE, json!("retryable")),
        (key::GEN_AI_USAGE_INPUT_TOKENS, json!(800)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, json!(0)),
        (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, json!(600)),
    ]);
    assert_eq!(attributes(failed), map(expected));
    assert_eq!(failed.status, Status::error("529 overloaded"));
    assert_eq!(timing(failed), (0, 40));
    let [event] = failed.events.events.as_slice() else {
        panic!("one event: {:?}", failed.events);
    };
    assert_eq!(event.name, LabletRetry::NAME);
    assert_eq!(event.timestamp, failed.end_time);
    let retry: BTreeMap<_, _> = event
        .attributes
        .iter()
        .map(|attribute| (attribute.key.as_str().to_owned(), as_json(&attribute.value)))
        .collect();
    assert_eq!(
        retry,
        map(vec![
            (key::LABLET_ATTEMPT, json!(1)),
            (key::LABLET_RETRY_WILL_RETRY, json!(true)),
            (key::LABLET_RETRY_BACKOFF_MS, json!(100)),
        ])
    );

    let [exception] = run.exceptions().try_into().expect("one exception");
    let context = exception.trace_context().expect("in the span's context");
    assert_eq!(context.trace_id, failed.span_context.trace_id());
    assert_eq!(context.span_id, failed.span_context.span_id());
    assert_eq!(context.trace_flags, Some(failed.span_context.trace_flags()));
    assert_eq!(exception.timestamp(), Some(failed.end_time));
    assert_eq!(exception.observed_timestamp(), Some(failed.end_time));
    assert_eq!(
        exception.severity_number(),
        Some(opentelemetry::logs::Severity::Warn)
    );
    assert_eq!(exception.severity_text(), Some("WARN"));
    let mut expected = joined(&run);
    expected.extend([
        (key::GEN_AI_OPERATION_NAME, json!("chat")),
        (key::EXCEPTION_TYPE, json!("retryable")),
        (key::EXCEPTION_MESSAGE, json!("529 overloaded")),
        (key::GEN_AI_PROVIDER_NAME, json!("fake")),
        (key::GEN_AI_REQUEST_MODEL, json!("fake-1")),
        (key::LABLET_TURN, json!(1)),
        (key::LABLET_ATTEMPT, json!(1)),
    ]);
    assert_eq!(record_attributes(&exception), map(expected));
}

#[tokio::test]
async fn a_tool_span_says_what_was_called_and_what_became_of_the_call() {
    let run = fully_described().run().await;

    let tools = run.tools();
    let mut expected = joined(&run);
    expected.extend([
        (key::GEN_AI_OPERATION_NAME, json!("execute_tool")),
        (key::GEN_AI_TOOL_NAME, json!("bash")),
        (key::GEN_AI_TOOL_CALL_ID, json!("call_0")),
        (key::GEN_AI_TOOL_TYPE, json!("function")),
        (key::GEN_AI_TOOL_DESCRIPTION, json!("The bash tool.")),
        (key::LABLET_TOOL_SOURCE, json!("builtin")),
        (key::LABLET_TURN, json!(1)),
        (key::LABLET_TOOL_STATUS, json!("ok")),
        (key::LABLET_TOOL_INPUT_BYTES, json!(7)),
        (key::LABLET_TOOL_OUTPUT_BYTES, json!(2)),
        (key::LABLET_TOOL_OUTPUT_TRUNCATED, json!(false)),
        (key::LABLET_TOOL_IS_ERROR, json!(false)),
    ]);
    assert_eq!(attributes(&tools[0]), map(expected));
    assert_eq!(tools[0].status, Status::Unset);
    assert_eq!(tools[0].name, "execute_tool bash");
    assert_eq!(
        timing(&tools[0]),
        (390, 1_000),
        "the call began when the response came and took the tool's second"
    );

    let unknown = &tools[1];
    assert_eq!(text(unknown, key::ERROR_TYPE), "unknown");
    assert_eq!(text(unknown, key::LABLET_TOOL_STATUS), "unknown");
    assert_eq!(
        attribute(unknown, key::LABLET_TOOL_IS_ERROR),
        Some(&Value::Bool(true))
    );
    assert_eq!(count(unknown, key::LABLET_TURN), 1);
    for key in [
        key::GEN_AI_TOOL_TYPE,
        key::GEN_AI_TOOL_DESCRIPTION,
        key::LABLET_TOOL_SOURCE,
    ] {
        assert_eq!(attribute(unknown, key), None, "{key}");
    }
    assert_eq!(unknown.status, Status::error(""));
    assert_eq!(timing(unknown), (1_390, 0));
    assert_eq!(unknown.name, "execute_tool invented");
}

#[tokio::test]
async fn a_tool_span_says_where_its_tool_comes_from() {
    let mut harness = Harness::new(vec![
        Answer::now(says(
            "On it.",
            &["bash", "search", "invented"],
            FinishReason::ToolUse,
        )),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.tools.push(Arc::new(FakeTools::new(
        Arc::clone(&harness.clock),
        vec![spec("search", mcp("docs"))],
    )));

    let run = harness.run().await;

    let sources: Vec<_> = run
        .tools()
        .iter()
        .map(|tool| {
            (
                text(tool, key::GEN_AI_TOOL_NAME).to_owned(),
                maybe_text(tool, key::GEN_AI_TOOL_TYPE),
                maybe_text(tool, key::LABLET_TOOL_SOURCE),
                maybe_text(tool, key::GEN_AI_TOOL_DESCRIPTION),
            )
        })
        .collect();
    assert_eq!(
        sources,
        [
            (
                "bash".to_owned(),
                Some("function".to_owned()),
                Some("builtin".to_owned()),
                Some("The bash tool.".to_owned()),
            ),
            (
                "search".to_owned(),
                Some("extension".to_owned()),
                Some("mcp".to_owned()),
                Some("The search tool.".to_owned()),
            ),
            ("invented".to_owned(), None, None, None),
        ]
    );
}

#[tokio::test]
async fn each_record_of_content_is_in_the_context_of_the_span_the_content_belongs_to() {
    let mut harness = fully_described();
    harness.context.capture_content = true;

    let run = harness.run().await;

    let spans = run.spans();
    let belongs: Vec<_> = run
        .content()
        .iter()
        .map(|record| {
            let context = record.trace_context().expect("in a span's context");
            let attributes = record_attributes(record);
            let held: Vec<_> = HOLD_CONTENT
                .into_iter()
                .filter(|key| attributes.contains_key(*key))
                .collect();
            if context.span_id == run.root.span_id() {
                assert_eq!(context.trace_id, run.root.trace_id());
                assert_eq!(record.timestamp(), Some(started()));
                assert_eq!(attributes.get(key::LABLET_TURN), None);
                return ("the context the run was called in", held);
            }
            let span = spans
                .iter()
                .find(|span| span.span_context.span_id() == context.span_id)
                .expect("a span of the run");
            assert_eq!(context.trace_id, span.span_context.trace_id());
            assert_eq!(context.trace_flags, Some(span.span_context.trace_flags()));
            assert_eq!(record.timestamp(), Some(span.end_time), "{}", span.name);
            assert_eq!(record.observed_timestamp(), Some(span.end_time));
            assert_eq!(
                attributes[key::GEN_AI_OPERATION_NAME],
                json!(text(span, key::GEN_AI_OPERATION_NAME))
            );
            assert_eq!(
                attributes[key::LABLET_TURN],
                json!(count(span, key::LABLET_TURN))
            );
            (span.name.as_ref(), held)
        })
        .collect();

    let chat = "chat fake-1";
    let sent = vec![key::GEN_AI_SYSTEM_INSTRUCTIONS, key::GEN_AI_INPUT_MESSAGES];
    let exchanged = vec![
        key::GEN_AI_SYSTEM_INSTRUCTIONS,
        key::GEN_AI_INPUT_MESSAGES,
        key::GEN_AI_OUTPUT_MESSAGES,
    ];
    let called = vec![
        key::GEN_AI_TOOL_CALL_ARGUMENTS,
        key::GEN_AI_TOOL_CALL_RESULT,
    ];
    assert_eq!(
        belongs,
        [
            (
                "the context the run was called in",
                vec![key::GEN_AI_TOOL_DEFINITIONS]
            ),
            (chat, sent),
            (chat, exchanged.clone()),
            ("execute_tool bash", called.clone()),
            ("execute_tool invented", called),
            (chat, exchanged),
        ]
    );
    for span in &spans {
        for key in HOLD_CONTENT {
            assert_eq!(attribute(span, key), None, "{key} is on {}", span.name);
        }
    }
}

#[tokio::test]
async fn the_records_of_a_run_hold_the_conversation_as_the_model_was_sent_it() {
    let mut harness = fully_described();
    harness.context.capture_content = true;

    let run = harness.run().await;

    let records = run.content();
    assert_eq!(
        record_json(&records[0], key::GEN_AI_TOOL_DEFINITIONS),
        json!([
            { "type": "function", "name": "bash", "description": "The bash tool.", "parameters": { "type": "object" } },
            { "type": "function", "name": "read_file", "description": "The read_file tool.", "parameters": { "type": "object" } },
        ])
    );
    let text = |text: &str| json!({ "type": "text", "content": text });
    let result = |text: &str, failed: bool| json!({ "content": [{ "type": "text", "text": text }], "isError": failed });
    let failed = &records[1];
    assert_eq!(
        record_json(failed, key::GEN_AI_SYSTEM_INSTRUCTIONS),
        json!([text("You fix tests.")])
    );
    assert_eq!(
        record_json(failed, key::GEN_AI_INPUT_MESSAGES),
        json!([{ "role": "user", "parts": [text("Fix the failing test.")] }])
    );
    assert_eq!(
        record_attributes(failed).get(key::GEN_AI_OUTPUT_MESSAGES),
        None,
        "a failed attempt answered nothing"
    );
    let last = records.last().expect("the last attempt's record");
    assert_eq!(
        record_json(last, key::GEN_AI_INPUT_MESSAGES),
        json!([
            { "role": "user", "parts": [text("Fix the failing test.")] },
            { "role": "assistant", "parts": [
                text("On it."),
                { "type": "tool_call", "id": "call_0", "name": "bash", "arguments": { "n": 0 } },
                { "type": "tool_call", "id": "call_1", "name": "invented", "arguments": { "n": 1 } },
            ] },
            { "role": "tool", "parts": [
                { "type": "tool_call_response", "id": "call_0", "response": result("ok", false) },
                { "type": "tool_call_response", "id": "call_1", "response": result("no tool named invented is offered by this run", true) },
            ] },
        ])
    );
    assert_eq!(
        record_json(last, key::GEN_AI_OUTPUT_MESSAGES),
        json!([{ "role": "assistant", "parts": [text("Done.")] }])
    );
    let of_bash = &records[3];
    assert_eq!(record_text(of_bash, key::GEN_AI_TOOL_NAME), "bash");
    assert_eq!(record_text(of_bash, key::GEN_AI_TOOL_CALL_ID), "call_0");
    assert_eq!(
        record_json(of_bash, key::GEN_AI_TOOL_CALL_ARGUMENTS),
        json!({ "n": 0 })
    );
    assert_eq!(
        record_json(of_bash, key::GEN_AI_TOOL_CALL_RESULT),
        result("ok", false)
    );
    let of_invented = &records[4];
    assert_eq!(
        record_json(of_invented, key::GEN_AI_TOOL_CALL_RESULT),
        result("no tool named invented is offered by this run", true)
    );
}

/// A call whose arguments didn't parse has no arguments to record, and a
/// record of its result all the same.
#[tokio::test]
async fn the_record_of_a_call_whose_arguments_did_not_parse_holds_no_arguments() {
    let mut harness = Harness::new(vec![
        Answer::now(calls_with_unparsed_input("bash", "{\"cmd\": ")),
        Answer::now(says("Done.", &[], FinishReason::EndTurn)),
    ]);
    harness.context.capture_content = true;

    let run = harness.run().await;

    let records = run.content();
    let of_bash = &records[2];
    assert_eq!(record_text(of_bash, key::GEN_AI_TOOL_CALL_ID), "call_0");
    assert_eq!(
        record_attributes(of_bash).get(key::GEN_AI_TOOL_CALL_ARGUMENTS),
        None
    );
    let result = record_json(of_bash, key::GEN_AI_TOOL_CALL_RESULT);
    assert_eq!(result["isError"], true);
    assert_eq!(statuses(&run), [vec!["malformed_input"], vec![]]);
}
