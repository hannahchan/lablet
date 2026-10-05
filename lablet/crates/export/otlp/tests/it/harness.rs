//! A `Telemetry` built from a test's settings, and a run emitted through its
//! tracer and its logger as the loop and the composition root emit one:
//! the root span opened from an empty context, the chat and tool spans
//! beneath it, the records in their spans' contexts, each timed on the
//! run's clock, and the wide event as a function of the lost count.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lablet_conformance::otlp::Exported;
use lablet_model::RunId;
use lablet_otlp::{FileTarget, FlushError, OtlpSettings, Telemetry, WideEvent};
use lablet_run::telemetry::{Attribute, Logger, Record};
use lablet_test_support::Scratch;
use opentelemetry::global::BoxedTracer;
use opentelemetry::logs::Severity;
use opentelemetry::trace::{
    Span as _, SpanContext, SpanKind, Status, TraceContextExt as _, Tracer as _,
};
use opentelemetry::{Context, InstrumentationScope, KeyValue};

pub const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
pub const OTHER_RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";
pub const VERSION: &str = "0.4.2";
pub const STARTED_UNIX_MS: u64 = 1_790_000_000_000;
/// How long the runs these tests emit last.
pub const DURATION_MS: u64 = 12_345;

/// The schema URL the composition root's scope carries in these tests. It
/// stands for the registry's, which the composition root knows and this
/// crate doesn't.
pub const SCHEMA_URL: &str = "https://lablet.dev/schemas/test";

/// The key a test's spans and records carry their run under.
pub const RUN_KEY: &str = "lablet.test.run";
/// The key a test's wide event counts the lost records under.
pub const DROPPED_KEY: &str = "lablet.test.dropped";

/// The event name of a run's wide event.
pub const WIDE: &str = "lablet.run";
/// The event name of a content record.
pub const CONTENT: &str = "gen_ai.client.inference.operation.details";

/// How many spans one run opens: a root, two chats and a tool.
pub const SPANS_PER_RUN: usize = 4;
/// How many content records a run that captures content emits, beside its
/// failed attempt's record.
pub const CONTENT_PER_RUN: usize = 4;

/// Which records a run emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Records {
    /// None: a run of spans alone, whose only record is the wide event.
    None,
    /// The failed attempt's record.
    Exception,
    /// The failed attempt's record and a content record for each span.
    Captured,
}

/// The scope the composition root hands over.
pub fn scope() -> InstrumentationScope {
    InstrumentationScope::builder("lablet")
        .with_version(VERSION)
        .with_schema_url(SCHEMA_URL)
        .build()
}

/// `ms` after the run started.
pub fn after(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(STARTED_UNIX_MS + ms)
}

/// The file of the run `run` in `scratch`, when each run has its own.
pub fn file_of(scratch: &Scratch, run: &str) -> PathBuf {
    scratch.at(&format!("lablet-{run}.otlp.jsonl"))
}

/// What a test says of the telemetry it builds.
pub struct Settings {
    /// The file the telemetry exports to, when it exports to one.
    pub target: Option<FileTarget>,
    /// The collector the telemetry exports to, when it exports to one.
    pub otlp: Option<OtlpSettings>,
    /// The composer's resource attributes.
    pub resource: Vec<(String, String)>,
    /// How long a flush waits for a destination, in place of the builder's
    /// five seconds.
    pub flush_timeout: Option<Duration>,
    /// How long a shutdown waits, in place of the builder's five seconds.
    pub shutdown_timeout: Option<Duration>,
}

impl Settings {
    /// A file of its own for each run in `scratch`, no collector, and a
    /// resource the composer adds `team: evals` and
    /// `deployment.environment.name: ci` to.
    pub fn in_scratch(scratch: &Scratch) -> Self {
        Self {
            target: Some(FileTarget::EachRun {
                directory: scratch.path().to_owned(),
            }),
            otlp: None,
            resource: vec![
                ("team".to_owned(), "evals".to_owned()),
                ("deployment.environment.name".to_owned(), "ci".to_owned()),
            ],
            flush_timeout: None,
            shutdown_timeout: None,
        }
    }
}

/// The telemetry `settings` describe. With a collector it must be called
/// inside a tokio runtime.
pub fn built(settings: Settings) -> Telemetry {
    let Settings {
        target,
        otlp,
        resource,
        flush_timeout,
        shutdown_timeout,
    } = settings;
    let mut builder = Telemetry::builder(VERSION, scope()).resource(resource);
    if let Some(target) = target {
        builder = builder.file(target);
    }
    if let Some(settings) = otlp {
        builder = builder.otlp(settings);
    }
    if let Some(timeout) = flush_timeout {
        builder = builder.flush_timeout(timeout);
    }
    if let Some(timeout) = shutdown_timeout {
        builder = builder.shutdown_timeout(timeout);
    }
    builder.build().unwrap()
}

fn of_run(run: &str) -> KeyValue {
    KeyValue::new(RUN_KEY, run.to_owned())
}

fn record(name: &'static str, at: SystemTime, span: SpanContext, run: &str) -> Record {
    Record {
        name,
        severity: Severity::Warn,
        at,
        span,
        attributes: vec![Attribute::of(RUN_KEY, run)],
    }
}

/// Emits one run under the id `run` through `tracer` and `logger`, and
/// returns its wide event as the composition root hands it over.
///
/// The crate's unit tests emit the same run from `src/testing.rs`, which an
/// integration target can't reach; a change to the run's shape is made in
/// both.
pub fn emit(tracer: &BoxedTracer, logger: &dyn Logger, run: &str, records: Records) -> WideEvent {
    let content = records == Records::Captured;
    let root = tracer
        .span_builder("invoke_agent lablet")
        .with_kind(SpanKind::Internal)
        .with_start_time(after(0))
        .with_attributes([of_run(run)])
        .start_with_context(tracer, &Context::new());
    let cx = Context::new().with_span(root);
    let root_context = cx.span().span_context().clone();
    if content {
        logger.emit(record(CONTENT, after(1), root_context.clone(), run));
    }

    let mut failed = tracer
        .span_builder("chat scripted-1")
        .with_kind(SpanKind::Client)
        .with_start_time(after(7))
        .with_attributes([of_run(run)])
        .start_with_context(tracer, &cx);
    failed.add_event_with_timestamp(
        "lablet.retry",
        after(47),
        vec![KeyValue::new("lablet.test.attempt", 1_i64)],
    );
    failed.set_status(Status::error("529 overloaded"));
    if records != Records::None {
        logger.emit(record(
            "gen_ai.client.operation.exception",
            after(47),
            failed.span_context().clone(),
            run,
        ));
    }
    if content {
        logger.emit(record(
            CONTENT,
            after(47),
            failed.span_context().clone(),
            run,
        ));
    }
    failed.end_with_timestamp(after(47));

    let mut answered = tracer
        .span_builder("chat scripted-1")
        .with_kind(SpanKind::Client)
        .with_start_time(after(2_047))
        .with_attributes([of_run(run)])
        .start_with_context(tracer, &cx);
    if content {
        logger.emit(record(
            CONTENT,
            after(2_297),
            answered.span_context().clone(),
            run,
        ));
    }
    answered.end_with_timestamp(after(2_297));

    let mut tool = tracer
        .span_builder("execute_tool bash")
        .with_kind(SpanKind::Internal)
        .with_start_time(after(2_300))
        .with_attributes([of_run(run)])
        .start_with_context(tracer, &cx);
    if content {
        logger.emit(record(
            CONTENT,
            after(3_300),
            tool.span_context().clone(),
            run,
        ));
    }
    tool.end_with_timestamp(after(3_300));

    cx.span().end_with_timestamp(after(DURATION_MS));
    let run = run.to_owned();
    Box::new(move |lost| Record {
        name: WIDE,
        severity: Severity::Info,
        at: after(DURATION_MS),
        span: root_context.clone(),
        attributes: vec![
            Attribute::of(RUN_KEY, run.as_str()),
            Attribute::of(DROPPED_KEY, lost),
        ],
    })
}

/// Begins the run `run` on `telemetry`, emits it through a tracer and a
/// logger of `telemetry`'s own, and returns its wide event.
pub fn emit_run(telemetry: &Telemetry, run: &str, records: Records) -> WideEvent {
    telemetry.begin_run(&RunId::new(run).unwrap());
    emit(
        &telemetry.tracer(),
        telemetry.logger().as_ref(),
        run,
        records,
    )
}

/// One run under the id `run`, flushed with its wide event, as
/// `Lablet::run` does.
pub async fn run(telemetry: &Telemetry, run: &str, records: Records) -> Result<(), FlushError> {
    let wide = emit_run(telemetry, run, records);
    telemetry.flush(wide).await
}

/// The runs whose wide events `exported` holds, in order.
pub fn runs_of(exported: &Exported) -> Vec<&str> {
    exported
        .records_of(WIDE)
        .into_iter()
        .map(|wide| wide.attributes[RUN_KEY].as_str().unwrap())
        .collect()
}

/// The queues each failure of `error` names.
pub fn queues(error: &FlushError) -> Vec<&str> {
    error
        .failures()
        .iter()
        .map(|failure| failure.split(": ").next().unwrap())
        .collect()
}
