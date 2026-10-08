//! A run around the loop, against fakes: what names it, the context its
//! root span is opened in, the root span and the wide event as they're
//! filled and timed, the transcript and when it's written, and the
//! cancellation each run sets. The spans are read back from the SDK's
//! in-memory exporter, and the wide event from a logger that keeps what
//! it's handed and passes it on to the SDK's.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lablet_model::{
    CacheScope, CompletionMode, ConfigDigest, ContentBlock, FinishReason, ModelRef, Prompts,
    ProviderApi, ProviderResponse, RequestParams, RunId, RunLabels, StopReason, Thinking,
    ToolConcurrency, ToolName, ToolSource, ToolSpec, Usage,
};
use lablet_policy::{RetryPolicy, RetrySettings, StopPolicy};
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::logs::LoggerProvider as _;
use opentelemetry::trace::{SpanId, Status, TracerProvider as _};
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLogRecord, SdkLoggerProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};

use super::*;
use crate::telemetry::generated::key;
use crate::telemetry::generated::{GenAiClientInferenceOperationDetails, LabletChat, LabletRun};
use crate::telemetry::{Attribute, Bridge, Record};
use crate::tests::fakes::{Answer, FakeCancel, FakeClock, FakeProvider, FakeTools, WALL_AT_ZERO};
use crate::{CallLimits, ModelProvider, ProviderError, ProviderRequest, ToolFilter};

const SYSTEM: &str = "You fix tests, tersely.";
const TASK: &str = "Fix the failing test in the parser.";
const CONFIG_DIGEST: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
const VERSION: &str = "0.4.2";
const PLACE: &str = "/runs/transcript.json";

/// When a run on a [`FakeClock`] starts.
fn started() -> SystemTime {
    UNIX_EPOCH + WALL_AT_ZERO
}

fn model() -> ModelRef {
    ModelRef {
        api: ProviderApi::Script,
        name: "scripted-1".to_owned(),
        replays_reasoning: false,
    }
}

/// A response that ends the run, after `latency`.
fn ends_after(latency: Duration) -> Answer {
    let response = ProviderResponse::new(
        vec![ContentBlock::Text("Nothing to fix.".to_owned())],
        Usage::default(),
        FinishReason::EndTurn,
        None,
        None,
    )
    .unwrap();
    Answer::Responds(Box::new(response), latency)
}

fn ends() -> Answer {
    ends_after(Duration::ZERO)
}

fn start() -> RunStart {
    RunStart {
        task: Prompts::new(String::new(), TASK).unwrap(),
        run_id: None,
        labels: RunLabels::default(),
        cancellation: None,
    }
}

fn named(run_id: &str) -> RunStart {
    RunStart {
        run_id: Some(RunId::new(run_id).unwrap()),
        ..start()
    }
}

/// What happened, in order, as the fakes that share it saw it.
#[derive(Debug, Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn push(&self, line: String) {
        self.0.lock().unwrap().push(line);
    }

    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// A logger that says in the log what it emits, keeps it, and passes it on
/// to the SDK's.
struct Listening {
    log: Log,
    kept: Arc<Mutex<Vec<Record>>>,
    to: Bridge<opentelemetry_sdk::logs::SdkLogger>,
}

impl Logger for Listening {
    fn emit(&self, record: Record) {
        self.log.push(format!("emit {}", record.name));
        self.kept.lock().unwrap().push(record.clone());
        self.to.emit(record);
    }
}

/// A transcript writer that says in the log what it's asked, gives the
/// place and the outcome of a write it's told to, and keeps what it's
/// handed.
struct Writer {
    log: Log,
    place: Result<PathBuf, TranscriptError>,
    written: Result<(), TranscriptError>,
    handed: Mutex<Vec<(PathBuf, RunTranscript)>>,
}

impl Writer {
    fn at(log: &Log, place: &str) -> Self {
        Self {
            log: log.clone(),
            place: Ok(PathBuf::from(place)),
            written: Ok(()),
            handed: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl TranscriptWriter for Writer {
    fn place(&self, run_id: &RunId) -> Result<PathBuf, TranscriptError> {
        self.log.push(format!("place {run_id}"));
        self.place.clone()
    }

    async fn write(
        &self,
        place: PathBuf,
        transcript: RunTranscript,
    ) -> Result<(), TranscriptError> {
        self.log.push(format!("write {}", place.display()));
        self.handed.lock().unwrap().push((place, transcript));
        self.written.clone()
    }
}

/// A provider that answers as the fake does, and keeps the context each
/// attempt was made in.
struct Witness {
    provider: FakeProvider,
    contexts: Mutex<Vec<Context>>,
}

#[async_trait::async_trait]
impl ModelProvider for Witness {
    fn model(&self) -> &ModelRef {
        self.provider.model()
    }

    fn endpoint(&self) -> Option<lablet_model::Endpoint> {
        self.provider.endpoint()
    }

    async fn complete(
        &self,
        request: ProviderRequest<'_>,
    ) -> Result<ProviderResponse, ProviderError> {
        self.contexts.lock().unwrap().push(Context::current());
        self.provider.complete(request).await
    }
}

/// The loop a [`Lab`] runs: no retry, and no cap but the run's timeout.
fn loop_of(
    provider: Arc<dyn ModelProvider>,
    tools: Arc<ToolSet>,
    tracer: BoxedTracer,
    logger: Box<dyn Logger>,
    clock: Arc<dyn Clock>,
    cancellation: Arc<dyn Cancellation>,
) -> RunService {
    RunService::new(
        provider,
        tools,
        tracer,
        logger,
        clock,
        cancellation,
        StopPolicy {
            max_turns: None,
            timeout: Duration::from_secs(600),
            max_total_tokens: None,
            max_consecutive_invalid_turns: NonZeroU32::new(3),
        },
        RetryPolicy::new(RetrySettings {
            max_retries: 0,
            base: Duration::from_millis(100),
            max: Duration::from_secs(10),
            factor: 2.0,
            hint_max: Duration::from_secs(60),
            jitter: 0.0,
        })
        .unwrap(),
        RequestParams {
            max_tokens: 4_096,
            temperature: None,
            thinking: Thinking::ProviderDefault,
            effort: None,
            seed: None,
            cache_scope: CacheScope::Shared,
        },
        None,
        CallLimits {
            provider_timeout: Duration::from_secs(60),
            output_cap: None,
            max_concurrent_tool_calls: NonZeroU32::new(1).unwrap(),
        },
        Arc::default(),
    )
}

/// A runner over fakes, and what it emitted.
struct Lab {
    clock: Arc<FakeClock>,
    provider: Arc<Witness>,
    log: Log,
    wide: Arc<Mutex<Vec<Record>>>,
    spans: InMemorySpanExporter,
    records: InMemoryLogExporter,
    tracer_provider: SdkTracerProvider,
    _logger_provider: SdkLoggerProvider,
    runner: Runner,
}

impl Lab {
    /// A runner of `script`, whose transcripts go to `transcript`, with the
    /// run's content captured when `capture` says.
    async fn new(
        script: Vec<Answer>,
        transcript: Option<Arc<dyn TranscriptWriter>>,
        log: &Log,
    ) -> Self {
        Self::capturing(script, transcript, log, false).await
    }

    async fn capturing(
        script: Vec<Answer>,
        transcript: Option<Arc<dyn TranscriptWriter>>,
        log: &Log,
        capture: bool,
    ) -> Self {
        let clock = Arc::new(FakeClock::new());
        let provider = Arc::new(Witness {
            provider: FakeProvider::new(model(), Arc::clone(&clock), script),
            contexts: Mutex::new(Vec::new()),
        });
        let tools = Arc::new(FakeTools::new(
            Arc::clone(&clock),
            vec![ToolSpec {
                name: ToolName::new("bash").unwrap(),
                description: "The bash tool.".to_owned(),
                input_schema: serde_json::json!({ "type": "object" }),
                source: ToolSource::Builtin,
                concurrency: ToolConcurrency::Exclusive,
            }],
        ));
        let tools = Arc::new(
            ToolSet::build(
                vec![tools],
                &ToolFilter::default(),
                CompletionMode::Natural,
                None,
            )
            .await
            .unwrap(),
        );
        let spans = InMemorySpanExporter::default();
        let records = InMemoryLogExporter::default();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(spans.clone())
            .build();
        let logger_provider = SdkLoggerProvider::builder()
            .with_simple_exporter(records.clone())
            .build();
        let tracer = || BoxedTracer::new(Box::new(tracer_provider.tracer("lablet")));
        let logger = || Bridge::new(logger_provider.logger("lablet"));
        let cancellation = Arc::new(RunCancellation::default());
        let service = loop_of(
            Arc::clone(&provider) as Arc<dyn ModelProvider>,
            Arc::clone(&tools),
            tracer(),
            Box::new(logger()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&cancellation) as Arc<dyn Cancellation>,
        );
        let wide = Arc::new(Mutex::new(Vec::new()));
        let runner = Runner::new(
            service,
            tools,
            tracer(),
            Box::new(Listening {
                log: log.clone(),
                kept: Arc::clone(&wide),
                to: logger(),
            }),
            Arc::clone(&clock) as Arc<dyn Clock>,
            cancellation,
            Shared {
                system: SYSTEM.to_owned(),
                config_digest: ConfigDigest::new(CONFIG_DIGEST).unwrap(),
                capture_content: capture,
                transcript,
                agent_version: VERSION.to_owned(),
            },
        );
        Self {
            clock,
            provider,
            log: log.clone(),
            wide,
            spans,
            records,
            tracer_provider,
            _logger_provider: logger_provider,
            runner,
        }
    }

    /// The spans the runs ended, in the order they ended.
    fn spans(&self) -> Vec<SpanData> {
        self.spans.get_finished_spans().unwrap()
    }

    /// The root spans, in the order the runs ended.
    fn roots(&self) -> Vec<SpanData> {
        self.spans()
            .into_iter()
            .filter(|span| span.name == root_span::name())
            .collect()
    }

    /// The spans of the trace `root` is in.
    fn traced(&self, root: &SpanData) -> Vec<SpanData> {
        let trace = root.span_context.trace_id();
        self.spans()
            .into_iter()
            .filter(|span| span.span_context.trace_id() == trace)
            .collect()
    }

    /// The wide events, in the order the runs emitted them.
    fn wide(&self) -> Vec<Record> {
        self.wide.lock().unwrap().clone()
    }

    /// The records the SDK was handed, in order.
    fn records(&self) -> Vec<SdkLogRecord> {
        self.records
            .get_emitted_logs()
            .unwrap()
            .into_iter()
            .map(|log| log.record)
            .collect()
    }
}

/// The value the attribute `key` of `record` holds, when it holds one.
fn value_of<'a>(record: &'a Record, key: &str) -> Option<&'a crate::telemetry::Value> {
    record
        .attributes
        .iter()
        .find(|attribute| attribute.key() == key)
        .and_then(Attribute::value)
}

/// The value the attribute `key` of `span` holds, when it holds one.
fn span_value<'a>(span: &'a SpanData, key: &str) -> Option<&'a Value> {
    span.attributes
        .iter()
        .find(|attribute| attribute.key.as_str() == key)
        .map(|attribute| &attribute.value)
}

/// The value of a ULID's first ten digits, which is when it was made, in
/// milliseconds since the Unix epoch.
fn ulid_time(id: &str) -> u128 {
    const DIGITS: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    assert_eq!(id.len(), 26, "{id}");
    id.chars().take(10).fold(0, |time, digit| {
        time * 32 + u128::try_from(DIGITS.find(digit).unwrap()).unwrap()
    })
}

fn unix_ms(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH).unwrap().as_millis()
}

#[tokio::test]
async fn a_run_without_an_id_gets_a_fresh_ulid_that_holds_when_it_started() {
    let mut lab = Lab::new(vec![ends(), ends()], None, &Log::default()).await;

    let first = lab
        .runner
        .run(start())
        .await
        .finished
        .summary
        .outcome
        .run_id;
    let second = lab
        .runner
        .run(start())
        .await
        .finished
        .summary
        .outcome
        .run_id;

    assert_ne!(first, second);
    let roots = lab.roots();
    assert_eq!(roots.len(), 2);
    for (run_id, root) in [&first, &second].into_iter().zip(&roots) {
        assert_eq!(
            span_value(root, key::GEN_AI_CONVERSATION_ID),
            Some(&Value::from(run_id.as_str().to_owned()))
        );
        assert_eq!(
            ulid_time(run_id.as_str()),
            unix_ms(root.start_time),
            "the root span starts when the id says the run did"
        );
        for span in lab.traced(root) {
            for label in [
                key::LABLET_TASK_ID,
                key::LABLET_EXPERIMENT_ID,
                key::LABLET_TRIAL,
            ] {
                assert!(span_value(&span, label).is_none(), "{label}");
            }
        }
    }
    for wide in lab.wide() {
        assert_eq!(value_of(&wide, key::LABLET_RUN_TRANSCRIPT_PATH), None);
    }
    assert!(
        lab.records()
            .iter()
            .all(|record| record.event_name() != Some(GenAiClientInferenceOperationDetails::NAME)),
        "no content is captured"
    );
}

#[tokio::test]
async fn a_run_with_an_id_and_labels_is_known_by_them_and_its_signals_carry_what_it_shares() {
    let labels = RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: Some("tool-descriptions-v2".to_owned()),
        trial: Some("seed-42".to_owned()),
    };
    let mut lab = Lab::capturing(vec![ends()], None, &Log::default(), true).await;

    let finished = lab
        .runner
        .run(RunStart {
            labels: labels.clone(),
            ..named("the-named-run")
        })
        .await
        .finished;

    assert_eq!(finished.summary.outcome.run_id.as_str(), "the-named-run");
    assert_eq!(finished.summary.outcome.labels, labels);
    let shown = lab.provider.provider.shown();
    assert_eq!(
        shown[0].system, SYSTEM,
        "the task runs under the system prompt"
    );
    assert_eq!(finished.transcript.system(), SYSTEM);
    let [root] = lab.roots().try_into().unwrap();
    for (key, value) in [
        (key::GEN_AI_CONVERSATION_ID, "the-named-run"),
        (key::LABLET_CONFIG_DIGEST, CONFIG_DIGEST),
        (key::GEN_AI_AGENT_VERSION, VERSION),
        (key::LABLET_TASK_ID, "fix-failing-test"),
        (key::LABLET_EXPERIMENT_ID, "tool-descriptions-v2"),
        (key::LABLET_TRIAL, "seed-42"),
        (key::LABLET_RUN_STOP_REASON, "completed"),
    ] {
        assert_eq!(
            span_value(&root, key),
            Some(&Value::from(value.to_owned())),
            "{key}"
        );
    }
    assert_eq!(root.status, Status::Unset);
    let [wide] = lab.wide().try_into().unwrap();
    assert_eq!(wide.name, LabletRun::NAME);
    assert_eq!(wide.span, root.span_context);
    assert_eq!(
        value_of(&wide, key::LABLET_RESULT_TEXT),
        Some(&crate::telemetry::Value::Text("Nothing to fix.".to_owned())),
        "the run captures content"
    );
    assert!(
        lab.records()
            .iter()
            .any(|record| record.event_name() == Some(GenAiClientInferenceOperationDetails::NAME)),
        "the run captures content"
    );
}

/// The fake clock moves only by what the script says, so the attempt and
/// the run last exactly a millisecond and a half, and the run starts at the
/// one reading of the wall clock it made, a fraction of a millisecond past a
/// whole one, so a start cut to the millisecond would differ.
#[tokio::test]
async fn the_run_s_own_span_and_its_wide_event_are_timed_to_the_nanosecond_from_its_start() {
    let lasting = Duration::from_micros(1_500);
    let mut lab = Lab::new(vec![ends_after(lasting)], None, &Log::default()).await;

    let finished = lab.runner.run(start()).await.finished;

    assert_eq!(finished.duration, lasting);
    let [root] = lab.roots().try_into().unwrap();
    let [chat] = lab
        .spans()
        .into_iter()
        .filter(|span| {
            span_value(span, key::GEN_AI_OPERATION_NAME)
                == Some(&Value::from(LabletChat::GEN_AI_OPERATION_NAME))
        })
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert_eq!(root.start_time, started());
    assert_eq!(
        (root.start_time, root.end_time),
        (chat.start_time, chat.end_time),
        "the run began and ended with its one attempt, timed by the loop"
    );
    assert_eq!(
        root.end_time.duration_since(root.start_time).unwrap(),
        lasting
    );
    let [wide] = lab.wide().try_into().unwrap();
    assert_eq!(wide.at, root.end_time);
    let [exported] = lab
        .records()
        .into_iter()
        .filter(|record| record.event_name() == Some(LabletRun::NAME))
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert_eq!(
        (exported.timestamp(), exported.observed_timestamp()),
        (Some(root.end_time), Some(root.end_time))
    );
    assert_eq!(
        value_of(&wide, key::LABLET_RUN_DURATION_MS),
        Some(&crate::telemetry::Value::Int(1))
    );
}

#[tokio::test]
async fn the_root_span_is_the_child_of_the_context_the_run_is_called_in() {
    let mut lab = Lab::new(vec![ends(), ends()], None, &Log::default()).await;
    let host = lab.tracer_provider.tracer("host").start("host operation");
    let host_span = host.span_context().clone();
    let calling = Context::current_with_span(host).with_baggage([KeyValue::new("tenant", "acme")]);

    lab.runner.run(start()).with_context(calling).await;
    lab.runner.run(start()).await;

    let [under_host, alone] = lab.roots().try_into().unwrap();
    assert_eq!(under_host.parent_span_id, host_span.span_id());
    assert_eq!(
        under_host.span_context.trace_id(),
        host_span.trace_id(),
        "the run is in the host's trace"
    );
    assert_eq!(alone.parent_span_id, SpanId::INVALID);
    assert_ne!(alone.span_context.trace_id(), host_span.trace_id());
    for root in [&under_host, &alone] {
        let loop_spans: Vec<SpanData> = lab
            .traced(root)
            .into_iter()
            .filter(|span| {
                span.span_context != root.span_context
                    && span.span_context.span_id() != host_span.span_id()
            })
            .collect();
        assert!(!loop_spans.is_empty());
        for span in loop_spans {
            assert_eq!(
                span.parent_span_id,
                root.span_context.span_id(),
                "{}",
                span.name
            );
            assert!(!span.parent_span_is_remote, "{}", span.name);
        }
    }
    let contexts = lab.provider.contexts.lock().unwrap().clone();
    let [in_host, in_none]: [Context; 2] = contexts.try_into().unwrap();
    assert_eq!(
        in_host.baggage().get("tenant"),
        Some(&opentelemetry::StringValue::from("acme")),
        "the context the run is called in reaches the loop, its baggage with it"
    );
    assert_eq!(in_host.span().span_context(), &under_host.span_context);
    assert_eq!(in_none.baggage().len(), 0);
    assert_eq!(in_none.span().span_context(), &alone.span_context);
}

#[tokio::test]
async fn the_transcript_is_written_before_the_wide_event_is_emitted() {
    let log = Log::default();
    let writer = Arc::new(Writer::at(&log, PLACE));
    let mut lab = Lab::new(
        vec![ends()],
        Some(Arc::clone(&writer) as Arc<dyn TranscriptWriter>),
        &log,
    )
    .await;

    let ran = lab.runner.run(named("run-a")).await;

    assert_eq!(ran.transcript, None);
    assert_eq!(
        lab.log.lines(),
        [
            "place run-a".to_owned(),
            format!("write {PLACE}"),
            format!("emit {}", LabletRun::NAME),
        ]
    );
    let [(place, handed)] = writer.handed.lock().unwrap().clone().try_into().unwrap();
    assert_eq!(place, PathBuf::from(PLACE));
    assert_eq!(handed.context.run_id.as_str(), "run-a");
    assert_eq!(handed.context.started, started());
    assert_eq!(handed.context.transcript_path, Some(PathBuf::from(PLACE)));
    assert_eq!(handed.context.config_digest.as_str(), CONFIG_DIGEST);
    assert_eq!(handed.context.agent_version, VERSION);
    assert!(!handed.context.capture_content);
    assert_eq!(handed.model, model());
    let offered: Vec<&str> = handed.tools.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(offered, ["bash"]);
    assert_eq!(handed.task, TASK);
    assert_eq!(handed.transcript, ran.finished.transcript);
    let [wide] = lab.wide().try_into().unwrap();
    assert_eq!(
        value_of(&wide, key::LABLET_RUN_TRANSCRIPT_PATH),
        Some(&crate::telemetry::Value::Text(PLACE.to_owned()))
    );
}

#[tokio::test]
async fn a_transcript_that_cannot_be_written_is_returned_and_the_wide_event_still_names_its_path() {
    let log = Log::default();
    let failed = TranscriptError::Unwritten("the disk is full".to_owned());
    let writer = Writer {
        written: Err(failed.clone()),
        ..Writer::at(&log, PLACE)
    };
    let mut lab = Lab::new(vec![ends()], Some(Arc::new(writer)), &log).await;

    let ran = lab.runner.run(start()).await;

    assert_eq!(ran.transcript, Some(failed));
    assert_eq!(
        ran.finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
    let [wide] = lab.wide().try_into().unwrap();
    assert_eq!(
        value_of(&wide, key::LABLET_RUN_TRANSCRIPT_PATH),
        Some(&crate::telemetry::Value::Text(PLACE.to_owned()))
    );
}

#[tokio::test]
async fn a_transcript_with_no_place_for_the_run_is_returned_never_written_and_named_nowhere() {
    let log = Log::default();
    let refused = TranscriptError::NoPlace("the run id holds a `/`".to_owned());
    let writer = Writer {
        place: Err(refused.clone()),
        ..Writer::at(&log, PLACE)
    };
    let mut lab = Lab::new(vec![ends()], Some(Arc::new(writer)), &log).await;

    let ran = lab.runner.run(named("run-a")).await;

    assert_eq!(ran.transcript, Some(refused));
    assert_eq!(
        lab.log.lines(),
        [
            "place run-a".to_owned(),
            format!("emit {}", LabletRun::NAME)
        ]
    );
    let [wide] = lab.wide().try_into().unwrap();
    assert_eq!(value_of(&wide, key::LABLET_RUN_TRANSCRIPT_PATH), None);
}

#[tokio::test]
async fn a_run_with_no_transcript_writer_names_no_transcript_path() {
    let mut lab = Lab::new(vec![ends()], None, &Log::default()).await;

    let ran = lab.runner.run(start()).await;

    assert_eq!(ran.transcript, None);
    let [wide] = lab.wide().try_into().unwrap();
    assert_eq!(value_of(&wide, key::LABLET_RUN_TRANSCRIPT_PATH), None);
}

#[tokio::test]
async fn a_run_s_cancellation_reaches_only_that_run() {
    let mut lab = Lab::new(vec![ends(), ends()], None, &Log::default()).await;
    let fired = Arc::new(FakeCancel::already());

    let cancelled = lab
        .runner
        .run(RunStart {
            cancellation: Some(fired),
            ..start()
        })
        .await;
    let after = lab.runner.run(start()).await;
    let unfired = lab
        .runner
        .run(RunStart {
            cancellation: Some(Arc::new(FakeCancel::never())),
            ..start()
        })
        .await;

    assert_eq!(
        cancelled.finished.summary.outcome.stop_reason(),
        StopReason::Cancelled
    );
    assert_eq!(
        after.finished.summary.outcome.stop_reason(),
        StopReason::Completed,
        "a handle of an earlier run never reaches the next"
    );
    assert_eq!(
        unfired.finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
    let roots = lab.roots();
    assert_eq!(roots.len(), 3);
    let cancelled_root = &roots[0];
    assert_eq!(
        span_value(cancelled_root, key::ERROR_TYPE),
        Some(&Value::from("cancelled"))
    );
    assert_eq!(cancelled_root.status, Status::error(String::new()));
    let wide = lab.wide();
    assert_eq!(
        value_of(&wide[0], key::LABLET_RUN_STOP_REASON),
        Some(&crate::telemetry::Value::Text("cancelled".to_owned()))
    );
    assert_eq!(lab.clock.wall(), started(), "no run took any time");
}
