//! What this crate's tests share: exporters that keep what they're handed,
//! in memory, and a run of spans and records emitted as the loop emits them,
//! through a tracer and lablet's logger.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lablet_run::telemetry::{Attribute, Logger, Record};
use opentelemetry::global::BoxedTracer;
use opentelemetry::logs::Severity;
use opentelemetry::trace::{
    Span as _, SpanContext, SpanKind, Status, TraceContextExt as _, Tracer as _,
};
use opentelemetry::{Context, InstrumentationScope, KeyValue};

use crate::telemetry::{Telemetry, WideEvent};

pub(crate) use lablet_test_support::Scratch;

pub(crate) const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
pub(crate) const OTHER_RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";
pub(crate) const VERSION: &str = "0.4.2";
pub(crate) const STARTED_UNIX_MS: u64 = 1_790_000_000_000;
/// How long the runs these tests emit last.
pub(crate) const DURATION_MS: u64 = 12_345;

/// The schema URL the composition root's scope carries in these tests. It
/// stands for the registry's, which the composition root knows and this
/// crate doesn't.
pub(crate) const SCHEMA_URL: &str = "https://lablet.dev/schemas/test";

/// The key a test's spans and records carry their run under.
pub(crate) const RUN_KEY: &str = "lablet.test.run";
/// The key a test's wide event counts the lost records under.
pub(crate) const DROPPED_KEY: &str = "lablet.test.dropped";

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
/// context, two chat spans and a tool span beneath it, a failed attempt's
/// record in the first chat's context and, with `content`, a content record
/// for each span, each timed on the run's clock. Returns the run's wide
/// event as the composition root hands it over: a function of the count of
/// lost records, in the root span's context, timed at the run's end.
///
/// The integration tests emit the same run from `tests/it/harness.rs`,
/// since that target can't reach a `#[cfg(test)]` module; a change to the
/// run's shape is made in both.
pub(crate) fn emit(
    tracer: &BoxedTracer,
    logger: &dyn Logger,
    run: &str,
    content: bool,
) -> WideEvent {
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
    logger.emit(record(
        EXCEPTION,
        after(47),
        failed.span_context().clone(),
        run,
    ));
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
pub(crate) fn wide_event(run: &str, root: SpanContext) -> WideEvent {
    let run = run.to_owned();
    Box::new(move |lost| Record {
        name: WIDE,
        severity: Severity::Info,
        at: after(DURATION_MS),
        span: root.clone(),
        attributes: vec![
            Attribute::of(RUN_KEY, run.as_str()),
            Attribute::of(DROPPED_KEY, lost),
        ],
    })
}

/// Begins the run `run` on `telemetry`, emits it through a tracer and a
/// logger of `telemetry`'s own, and returns its wide event.
pub(crate) fn emit_run(telemetry: &Telemetry, run: &str, content: bool) -> WideEvent {
    telemetry.begin_run(&lablet_model::RunId::new(run).unwrap_or_else(|error| {
        panic!("{run} is a run id: {error}");
    }));
    emit(
        &telemetry.tracer(),
        telemetry.logger().as_ref(),
        run,
        content,
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

    /// One log record, and the scope it was emitted under.
    pub(crate) type Logged = (SdkLogRecord, InstrumentationScope);

    /// One export, as the exporter it went to was handed it.
    #[derive(Debug, Clone)]
    pub(crate) enum Export {
        Spans(Vec<SpanData>),
        Records(Vec<Logged>),
        /// What the exporter of the wide event was handed.
        Wide(Vec<Logged>),
    }

    #[derive(Debug, Default)]
    struct Kept {
        exports: Mutex<Vec<Export>>,
        resources: Mutex<Vec<Resource>>,
        refuses_spans: AtomicBool,
        refuses_records: AtomicBool,
        refuses_wide: AtomicBool,
        held: Mutex<bool>,
        released: Condvar,
        /// How many exports are waiting to be released.
        exporting: Mutex<usize>,
        began: Condvar,
    }

    /// The memory of one destination's three exporters, which says what
    /// they were handed and decides what becomes of it.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct Memory(Arc<Kept>);

    #[derive(Debug)]
    pub(crate) struct Spans(Memory);

    #[derive(Debug)]
    pub(crate) struct Records {
        memory: Memory,
        wide: bool,
    }

    impl Memory {
        pub(crate) fn spans(&self) -> Spans {
            Spans(self.clone())
        }

        pub(crate) fn records(&self) -> Records {
            Records {
                memory: self.clone(),
                wide: false,
            }
        }

        pub(crate) fn wide(&self) -> Records {
            Records {
                memory: self.clone(),
                wide: true,
            }
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

        /// From now on, or no longer, an export of the wide event fails.
        pub(crate) fn refuse_wide(&self, refuses: bool) {
            self.0.refuses_wide.store(refuses, Ordering::SeqCst);
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

        /// Returns once an export is waiting to be released, which an
        /// export to a held destination does for as long as it's held.
        pub(crate) fn wait_until_exporting(&self) {
            let exporting = self.0.exporting.lock().unwrap();
            drop(
                self.0
                    .began
                    .wait_while(exporting, |exporting| *exporting == 0)
                    .unwrap(),
            );
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
                    Export::Records(_) | Export::Wide(_) => None,
                })
                .flatten()
                .collect()
        }

        /// Every log record so far but the wide events, in the order they
        /// were exported.
        pub(crate) fn exported_records(&self) -> Vec<Logged> {
            self.exports()
                .into_iter()
                .filter_map(|export| match export {
                    Export::Records(records) => Some(records),
                    Export::Spans(_) | Export::Wide(_) => None,
                })
                .flatten()
                .collect()
        }

        /// Every wide event so far, in the order they were exported.
        pub(crate) fn exported_wide(&self) -> Vec<Logged> {
            self.exports()
                .into_iter()
                .filter_map(|export| match export {
                    Export::Wide(records) => Some(records),
                    Export::Spans(_) | Export::Records(_) => None,
                })
                .flatten()
                .collect()
        }

        /// The resource each exporter was told its exports come from.
        pub(crate) fn resources(&self) -> Vec<Resource> {
            self.0.resources.lock().unwrap().clone()
        }

        fn export(&self, export: Export, refuses: &AtomicBool) -> OTelSdkResult {
            *self.0.exporting.lock().unwrap() += 1;
            self.0.began.notify_all();
            let held = self.0.held.lock().unwrap();
            drop(self.0.released.wait_while(held, |held| *held).unwrap());
            *self.0.exporting.lock().unwrap() -= 1;
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
            future::ready(if self.wide {
                self.memory
                    .export(Export::Wide(records), &self.memory.0.refuses_wide)
            } else {
                self.memory
                    .export(Export::Records(records), &self.memory.0.refuses_records)
            })
        }

        fn set_resource(&mut self, resource: &Resource) {
            self.memory.described(resource);
        }
    }
}
