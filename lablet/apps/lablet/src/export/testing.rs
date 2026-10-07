//! What the export module's tests share: exporters that keep what they're
//! handed, in memory, and a run of spans and records emitted as the loop
//! emits them, through a tracer and lablet's logger.

use std::ffi::OsString;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lablet_run::telemetry::{Attribute, Logger, Record};
use opentelemetry::global::BoxedTracer;
use opentelemetry::logs::Severity;
use opentelemetry::trace::{
    Span as _, SpanContext, SpanKind, Status, TraceContextExt as _, Tracer as _,
};
use opentelemetry::{Context, InstrumentationScope, KeyValue};

use super::telemetry::Telemetry;
use crate::otel_env::{OtelEnv, Sdk};

pub(crate) use lablet_test_support::Scratch;

pub(crate) const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
pub(crate) const OTHER_RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";
pub(crate) const VERSION: &str = "0.4.2";
pub(crate) const STARTED_UNIX_MS: u64 = 1_790_000_000_000;
/// How long the runs these tests emit last.
pub(crate) const DURATION_MS: u64 = 12_345;

/// The schema URL the composition root's scope carries in these tests. It
/// stands for the registry's, which the composition root hands over and
/// this module never reads.
pub(crate) const SCHEMA_URL: &str = "https://lablet.dev/schemas/test";

/// The key a test's spans and records carry their run under.
pub(crate) const RUN_KEY: &str = "lablet.test.run";

/// The event name of a run's wide event.
pub(crate) const WIDE: &str = "lablet.run";
/// The event name of a failed attempt's record.
pub(crate) const EXCEPTION: &str = "gen_ai.client.operation.exception";
/// The event name of a content record.
pub(crate) const CONTENT: &str = "gen_ai.client.inference.operation.details";

/// How many spans one run of [`emit`] opens: a root, two chats and a tool.
pub(crate) const SPANS_PER_RUN: usize = 4;
/// How many records one run of [`emit`] emits without content.
pub(crate) const RECORDS_PER_RUN: usize = 1;
/// How many records one run of [`emit`] emits with content captured.
pub(crate) const RECORDS_PER_CAPTURED_RUN: usize = 5;
/// How many content records one run of [`emit`] emits with content
/// captured, beside its failed attempt's record.
pub(crate) const CONTENT_PER_RUN: usize = 4;

/// Which records a run of [`emit`] emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Records {
    /// None: a run of spans alone, whose only record is the wide event.
    None,
    /// The failed attempt's record.
    Exception,
    /// The failed attempt's record and a content record for each span.
    Captured,
}

/// The scope the composition root hands over.
pub(crate) fn scope() -> InstrumentationScope {
    InstrumentationScope::builder("lablet")
        .with_version(VERSION)
        .with_schema_url(SCHEMA_URL)
        .build()
}

/// `ms` after the run started.
pub(crate) fn after(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(STARTED_UNIX_MS + ms)
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

/// Emits one run under the id `run` through `tracer` and `logger`, as the
/// loop and the composition root do: the root span, opened from an empty
/// context, two chat spans and a tool span beneath it, and the `records`
/// in their spans' contexts, each timed on the run's clock. Returns the
/// run's wide event as the composition root hands it over, in the root
/// span's context, timed at the run's end.
pub(crate) fn emit(
    tracer: &BoxedTracer,
    logger: &dyn Logger,
    run: &str,
    records: Records,
) -> Record {
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
            EXCEPTION,
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
    wide_event(run, root_context)
}

/// The wide event of the run `run`, as the composition root hands it over.
pub(crate) fn wide_event(run: &str, root: SpanContext) -> Record {
    Record {
        name: WIDE,
        severity: Severity::Info,
        at: after(DURATION_MS),
        span: root,
        attributes: vec![Attribute::of(RUN_KEY, run)],
    }
}

/// The SDK's settings an environment that holds `held`, and nothing else,
/// gives, read through the seam as a build reads them.
pub(crate) fn sdk_of(held: &[(&str, &str)]) -> Sdk {
    let env = |name: &str| {
        held.iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| OsString::from(value))
    };
    OtelEnv::read(&env).sdk
}

/// The network settings the config `text`, in YAML, and an environment
/// that holds `held`, and nothing else, resolve to, read through the seam
/// as a build reads them. They must export something.
pub(crate) fn otlp_of(text: &str, held: &[(&str, &str)]) -> super::OtlpSettings {
    let env = |name: &str| {
        held.iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| OsString::from(value))
    };
    let config = crate::config::Config::from_str(text, crate::config::Format::Yaml)
        .unwrap_or_else(|error| panic!("{text} is a config: {error}"));
    crate::otlp::settings(&config, &config, &OtelEnv::read(&env).exporter)
        .unwrap_or_else(|error| panic!("{text} is refused: {error}"))
        .unwrap_or_else(|| panic!("{text} exports nothing over the network"))
}

/// Begins the run `run` on `telemetry`, emits it through a tracer and a
/// logger of `telemetry`'s own, and returns its wide event.
pub(crate) fn emit_run(telemetry: &Telemetry, run: &str, records: Records) -> Record {
    telemetry.begin_run();
    emit(
        &telemetry.tracer(),
        telemetry.logger().as_ref(),
        run,
        records,
    )
}

/// Exporters that keep what they're handed, in memory.
pub(crate) mod memory {
    use std::future::{self, Future};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};

    use opentelemetry::InstrumentationScope;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
    use opentelemetry_sdk::logs::{LogBatch, LogExporter, SdkLogRecord};
    use opentelemetry_sdk::trace::{SpanData, SpanExporter};

    use super::WIDE;

    /// One log record, and the scope it was emitted under.
    pub(crate) type Logged = (SdkLogRecord, InstrumentationScope);

    /// One export, as the exporter it went to was handed it.
    #[derive(Debug, Clone)]
    pub(crate) enum Export {
        Spans(Vec<SpanData>),
        Records(Vec<Logged>),
    }

    #[derive(Debug, Default)]
    struct Kept {
        exports: Mutex<Vec<Export>>,
        resources: Mutex<Vec<Resource>>,
        refuses_spans: AtomicBool,
        refuses_records: AtomicBool,
        held: Mutex<bool>,
        released: Condvar,
    }

    /// The memory of one destination's two exporters, which says what
    /// they were handed and decides what becomes of it.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct Memory(Arc<Kept>);

    #[derive(Debug)]
    pub(crate) struct Spans(Memory);

    #[derive(Debug)]
    pub(crate) struct Records(Memory);

    impl Memory {
        pub(crate) fn spans(&self) -> Spans {
            Spans(self.clone())
        }

        pub(crate) fn records(&self) -> Records {
            Records(self.clone())
        }

        /// From now on, or no longer, an export of spans fails, as one to
        /// a destination that can't be written does.
        pub(crate) fn refuse_spans(&self, refuses: bool) {
            self.0.refuses_spans.store(refuses, Ordering::SeqCst);
        }

        /// From now on, or no longer, an export of log records fails.
        pub(crate) fn refuse_records(&self, refuses: bool) {
            self.0.refuses_records.store(refuses, Ordering::SeqCst);
        }

        /// From now on an export waits, as one to a destination that
        /// doesn't answer does, until [`Memory::release`].
        pub(crate) fn hold(&self) {
            *self.0.held.lock().unwrap() = true;
        }

        pub(crate) fn release(&self) {
            *self.0.held.lock().unwrap() = false;
            self.0.released.notify_all();
        }

        /// Every export so far, in order.
        pub(crate) fn exports(&self) -> Vec<Export> {
            self.0.exports.lock().unwrap().clone()
        }

        /// Every span so far, in the order they were exported.
        pub(crate) fn exported_spans(&self) -> Vec<SpanData> {
            self.exports()
                .into_iter()
                .filter_map(|export| match export {
                    Export::Spans(spans) => Some(spans),
                    Export::Records(_) => None,
                })
                .flatten()
                .collect()
        }

        /// Every log record so far, the wide events among them, in the
        /// order they were exported.
        fn exported_logs(&self) -> Vec<Logged> {
            self.exports()
                .into_iter()
                .filter_map(|export| match export {
                    Export::Records(records) => Some(records),
                    Export::Spans(_) => None,
                })
                .flatten()
                .collect()
        }

        /// Every log record so far but the wide events, in the order they
        /// were exported.
        pub(crate) fn exported_records(&self) -> Vec<Logged> {
            self.exported_logs()
                .into_iter()
                .filter(|(record, _)| record.event_name() != Some(WIDE))
                .collect()
        }

        /// Every wide event so far, in the order they were exported.
        pub(crate) fn exported_wide(&self) -> Vec<Logged> {
            self.exported_logs()
                .into_iter()
                .filter(|(record, _)| record.event_name() == Some(WIDE))
                .collect()
        }

        /// The resource each exporter was told its exports come from.
        pub(crate) fn resources(&self) -> Vec<Resource> {
            self.0.resources.lock().unwrap().clone()
        }

        fn export(&self, export: Export, refuses: &AtomicBool) -> OTelSdkResult {
            let held = self.0.held.lock().unwrap();
            drop(self.0.released.wait_while(held, |held| *held).unwrap());
            if refuses.load(Ordering::SeqCst) {
                return Err(OTelSdkError::InternalFailure(
                    "the destination can't be written".to_owned(),
                ));
            }
            self.0.exports.lock().unwrap().push(export);
            Ok(())
        }

        fn described(&self, resource: &Resource) {
            self.0.resources.lock().unwrap().push(resource.clone());
        }
    }

    impl SpanExporter for Spans {
        fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
            future::ready(self.0.export(Export::Spans(batch), &self.0.0.refuses_spans))
        }

        fn set_resource(&mut self, resource: &Resource) {
            self.0.described(resource);
        }
    }

    impl LogExporter for Records {
        fn export(&self, batch: LogBatch<'_>) -> impl Future<Output = OTelSdkResult> + Send {
            let records = batch
                .iter()
                .map(|(record, scope)| (record.clone(), scope.clone()))
                .collect();
            future::ready(
                self.0
                    .export(Export::Records(records), &self.0.0.refuses_records),
            )
        }

        fn set_resource(&mut self, resource: &Resource) {
            self.0.described(resource);
        }
    }
}
