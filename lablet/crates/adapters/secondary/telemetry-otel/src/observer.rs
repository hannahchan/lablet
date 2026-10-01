//! The observer: a run's events in, spans and log records out.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use lablet_model::{RunId, ToolCallId};
use lablet_run::{EventKind, RunEvent, RunObserver, TraceContext};
use lablet_telemetry_registry::{SCHEMA_URL, attribute as key};
use opentelemetry::logs::{Logger as _, LoggerProvider as _};
use opentelemetry::trace::TraceId;
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::logs::{LogExporter, SdkLogger, SdkLoggerProvider};
use opentelemetry_sdk::resource::TelemetryResourceDetector;
use opentelemetry_sdk::trace::SpanExporter;

use crate::file::{FileLogExporter, FileSpanExporter, FileTarget, Sink};
use crate::network::{Network, OtelBuildError, OtlpSettings};
use crate::pipeline::{Lost, RecordQueue, SpanQueue};
use crate::run::{Attempt, CallEnd, Closed, OpenRun, Opening};
use crate::signal::Signals;
use crate::wide::wide_event;

/// The name of the service and of the instrumentation scope.
const LABLET: &str = "lablet";

/// How long a flush and a shutdown are each given unless the builder says
/// otherwise, so a destination that doesn't answer costs seconds.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// What a flush or a shutdown couldn't export. Nothing about a run changes
/// for it: it's for whoever reports to the person who ran lablet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("telemetry wasn't exported whole: {}", failures.join("; "))]
pub struct FlushError {
    failures: Vec<String>,
}

impl FlushError {
    /// What failed, one entry for each queue that did.
    #[must_use]
    pub fn failures(&self) -> &[String] {
        &self.failures
    }
}

/// How one destination and its three queues are named in what a flush
/// reports.
#[derive(Debug, Clone, Copy)]
struct Names {
    destination: &'static str,
    spans: &'static str,
    records: &'static str,
    wide: &'static str,
}

const FILE: Names = Names {
    destination: "file",
    spans: "spans",
    records: "log records",
    wide: "the wide event",
};

const OTLP: Names = Names {
    destination: "otlp",
    spans: "otlp spans",
    records: "otlp log records",
    wide: "the otlp wide event",
};

/// Where a run's signals go: three queues, each with a thread of the SDK's
/// behind it, one for spans, one for log records and one that only the wide
/// event goes through, and the destination's own count of what they lost.
struct Destination {
    names: Names,
    spans: SpanQueue,
    records: SdkLoggerProvider,
    records_logger: SdkLogger,
    wide: SdkLoggerProvider,
    wide_logger: SdkLogger,
    lost: Arc<Lost>,
}

impl std::fmt::Debug for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Destination")
            .field("names", &self.names)
            .finish_non_exhaustive()
    }
}

type MakeDestination = Box<dyn FnOnce(&Resource, &InstrumentationScope) -> Destination + Send>;

/// What an [`OtelObserver`] is built from.
pub struct OtelObserverBuilder {
    version: String,
    resource: Vec<(String, String)>,
    file: Option<FileTarget>,
    otlp: Option<OtlpSettings>,
    flush_timeout: Duration,
    shutdown_timeout: Duration,
    destinations: Vec<MakeDestination>,
}

impl std::fmt::Debug for OtelObserverBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtelObserverBuilder")
            .field("version", &self.version)
            .field("resource", &self.resource)
            .field("file", &self.file)
            .field("otlp", &self.otlp)
            .field("flush_timeout", &self.flush_timeout)
            .field("shutdown_timeout", &self.shutdown_timeout)
            .finish_non_exhaustive()
    }
}

impl OtelObserverBuilder {
    /// The composer's own resource attributes, which every export carries
    /// beside `service.name`, `service.version` and what the SDK says of
    /// itself. A key of lablet's own keeps lablet's value.
    #[must_use]
    pub fn resource(mut self, attributes: Vec<(String, String)>) -> Self {
        self.resource = attributes;
        self
    }

    /// Exports to `target` as OTLP/JSON lines.
    #[must_use]
    pub fn file(mut self, target: FileTarget) -> Self {
        self.file = Some(target);
        self
    }

    /// Exports to an OTLP collector over the network, as `settings` says.
    /// Each header's value is a secret from here on: no `Debug` or message
    /// shows one. A header that isn't one is refused by [`Self::build`].
    #[must_use]
    pub fn otlp(mut self, settings: OtlpSettings) -> Self {
        self.otlp = Some(settings);
        self
    }

    /// How long [`OtelObserver::flush`] waits for a destination, in place
    /// of five seconds.
    #[must_use]
    pub const fn flush_timeout(mut self, timeout: Duration) -> Self {
        self.flush_timeout = timeout;
        self
    }

    /// How long [`OtelObserver::shutdown`] waits, in place of five seconds.
    #[must_use]
    pub const fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Exports to these three exporters, the third of which is handed the
    /// wide event and nothing else, named as the file's queues are.
    #[cfg(test)]
    pub(crate) fn exporting_to<S, L>(mut self, spans: S, records: L, wide: L) -> Self
    where
        S: SpanExporter + 'static,
        L: LogExporter + 'static,
    {
        self.destinations
            .push(destination(FILE, spans, records, wide));
        self
    }

    /// As [`Self::exporting_to`], named as the network's queues are.
    #[cfg(test)]
    pub(crate) fn exporting_over_the_network_to<S, L>(
        mut self,
        spans: S,
        records: L,
        wide: L,
    ) -> Self
    where
        S: SpanExporter + 'static,
        L: LogExporter + 'static,
    {
        self.destinations
            .push(destination(OTLP, spans, records, wide));
        self
    }

    /// The observer, with a thread behind each of its queues. With a
    /// network destination it must be called inside a tokio runtime, which
    /// the gRPC channel's worker runs on.
    ///
    /// # Errors
    ///
    /// Returns an [`OtelBuildError`] when the network exporters can't be
    /// made: the endpoint isn't one the exporter accepts, or a header isn't
    /// one a header may have.
    pub fn build(self) -> Result<OtelObserver, OtelBuildError> {
        let Self {
            version,
            resource,
            file,
            otlp,
            flush_timeout,
            shutdown_timeout,
            mut destinations,
        } = self;
        let scope = InstrumentationScope::builder(LABLET)
            .with_version(version.clone())
            .with_schema_url(SCHEMA_URL)
            .build();
        let resource = Resource::builder_empty()
            .with_attributes(
                resource
                    .into_iter()
                    .map(|(key, value)| KeyValue::new(key, value)),
            )
            .with_detector(Box::new(TelemetryResourceDetector))
            .with_attributes([
                KeyValue::new(key::SERVICE_NAME, LABLET),
                KeyValue::new(key::SERVICE_VERSION, version),
            ])
            .build();

        if let Some(settings) = otlp {
            let (spans, records, wide) = Network::new(settings)?.exporters()?;
            destinations.push(destination(OTLP, spans, records, wide));
        }
        let sink = file.map(Sink::new);
        if let Some(sink) = &sink {
            destinations.push(destination(
                FILE,
                FileSpanExporter::new(sink.clone()),
                FileLogExporter::new(sink.clone()),
                FileLogExporter::new(sink.clone()),
            ));
        }
        let destinations = destinations
            .into_iter()
            .map(|make| make(&resource, &scope))
            .collect();

        Ok(OtelObserver {
            inner: Arc::new(Inner {
                scope,
                destinations,
                sink,
                flush_timeout,
                shutdown_timeout,
                state: Mutex::new(State::default()),
            }),
        })
    }
}

/// A destination of three exporters, made once the resource they describe
/// their exports with and the scope they're emitted under are known.
fn destination<S, L>(names: Names, spans: S, records: L, wide: L) -> MakeDestination
where
    S: SpanExporter + 'static,
    L: LogExporter + 'static,
{
    Box::new(move |resource, scope| {
        let lost = Arc::new(Lost::default());
        let provider = |queue: RecordQueue| {
            SdkLoggerProvider::builder()
                .with_resource(resource.clone())
                .with_log_processor(queue)
                .build()
        };
        let records = provider(RecordQueue::new(
            names.destination,
            records,
            resource,
            &lost,
        ));
        // The wide event isn't one of the records its own count is of, so
        // what becomes of it is counted apart and reported by the flush.
        let wide = provider(RecordQueue::new(
            names.destination,
            wide,
            resource,
            &Arc::new(Lost::default()),
        ));
        Destination {
            names,
            spans: SpanQueue::new(names.destination, spans, resource, &lost),
            records_logger: records.logger_with_scope(scope.clone()),
            records,
            wide_logger: wide.logger_with_scope(scope.clone()),
            wide,
            lost,
        }
    })
}

#[derive(Debug, Default)]
struct State {
    /// The run in progress. An observer watches one loop, which makes one
    /// run at a time.
    open: Option<OpenRun>,
    /// The runs that have ended since the last flush.
    closed: Vec<Closed>,
}

#[derive(Debug)]
struct Inner {
    scope: InstrumentationScope,
    destinations: Vec<Destination>,
    sink: Option<Sink>,
    flush_timeout: Duration,
    shutdown_timeout: Duration,
    state: Mutex<State>,
}

/// The telemetry observer: it maps a run's events to OpenTelemetry spans
/// and log records, once, and hands them to its exporters.
///
/// [`RunObserver::on`] only queues what an event gave rise to, so it never
/// waits for an export. Whoever runs the loop calls
/// [`OtelObserver::flush`] when a run has returned, which is when the
/// run's wide event is made and exported.
#[derive(Debug, Clone)]
pub struct OtelObserver {
    inner: Arc<Inner>,
}

impl OtelObserver {
    /// An observer of lablet `version`, which is the `service.version` of
    /// its resource.
    pub fn builder(version: impl Into<String>) -> OtelObserverBuilder {
        OtelObserverBuilder {
            version: version.into(),
            resource: Vec::new(),
            file: None,
            otlp: None,
            flush_timeout: SHUTDOWN_TIMEOUT,
            shutdown_timeout: SHUTDOWN_TIMEOUT,
            destinations: Vec::new(),
        }
    }

    /// Exports what the runs so far gave rise to, to each destination on
    /// its own: its spans and its log records, then, in an export of its
    /// own, the wide event of each run that has ended, with that
    /// destination's own count of what it lost.
    ///
    /// The wide event is made once everything else of its run has been
    /// exported to the destination or lost, so its count is whole. A run
    /// whose observer is never flushed has its wide event when the observer
    /// shuts down.
    ///
    /// Each destination is waited for no longer than the builder's flush
    /// timeout, five seconds unless it says otherwise, and none waits on
    /// another: when this returns, a file the observer exports to holds
    /// every line of the runs that have ended, whatever a collector did.
    ///
    /// # Errors
    ///
    /// Returns a [`FlushError`] that names each queue whose export failed,
    /// and each destination that didn't end in time.
    pub async fn flush(&self) -> Result<(), FlushError> {
        self.blocking(Inner::flush).await
    }

    /// Flushes as [`OtelObserver::flush`] does, and then stops every
    /// queue's thread, and waits for that no longer than the builder's
    /// shutdown timeout, five seconds unless it says otherwise. So a
    /// destination that doesn't answer can't hold a process that's ready
    /// to exit: what it still holds then is left to a thread that nothing
    /// waits for. Events that come after a shutdown that ended are dropped.
    ///
    /// # Errors
    ///
    /// Returns a [`FlushError`] that names each queue whose export failed,
    /// or that didn't stop in time, or that says the shutdown didn't end.
    pub async fn shutdown(&self) -> Result<(), FlushError> {
        self.blocking(Inner::shutdown).await
    }

    /// Runs `work` where it may wait: a queue answers a flush from its own
    /// thread, and waiting for it on the runtime's would hold up whatever
    /// else runs there.
    async fn blocking(&self, work: fn(&Arc<Inner>) -> Vec<String>) -> Result<(), FlushError> {
        let inner = Arc::clone(&self.inner);
        let failures = tokio::task::spawn_blocking(move || work(&inner))
            .await
            .unwrap_or_else(|error| vec![format!("the flush didn't run to its end: {error}")]);
        if failures.is_empty() {
            Ok(())
        } else {
            Err(FlushError { failures })
        }
    }
}

/// Adds what went wrong with `queue`, if anything did, to `failures`.
fn note(failures: &mut Vec<String>, queue: &str, result: OTelSdkResult) {
    if let Err(error) = result {
        failures.push(format!("{queue}: {error}"));
    }
}

impl Destination {
    /// Exports what the queues hold, the spans and the records side by
    /// side, and then the wide event of each run in `closed`, made with the
    /// count of what was lost by then.
    fn flush(&self, closed: &[Closed]) -> Vec<String> {
        let mut failures = Vec::new();
        let (spans, records) = thread::scope(|scope| {
            let records = scope.spawn(|| self.records.force_flush());
            let spans = self.spans.flush();
            let records = records.join().unwrap_or_else(|_| {
                Err(OTelSdkError::InternalFailure(
                    "the flush didn't run to its end".to_owned(),
                ))
            });
            (spans, records)
        });
        note(&mut failures, self.names.spans, spans);
        note(&mut failures, self.names.records, records);

        for closed in closed {
            let event = wide_event(closed, self.lost.take());
            self.wide_logger
                .emit(event.into_sdk(closed.trace, &self.wide_logger));
        }
        note(&mut failures, self.names.wide, self.wide.force_flush());
        failures
    }

    /// Stops the three queues, each given `timeout`.
    fn stop(&self, timeout: Duration, failures: &mut Vec<String>) {
        note(failures, self.names.spans, self.spans.shutdown(timeout));
        note(
            failures,
            self.names.records,
            self.records.shutdown_with_timeout(timeout),
        );
        note(
            failures,
            self.names.wide,
            self.wide.shutdown_with_timeout(timeout),
        );
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues what an event gave rise to, at every destination, the
    /// records ahead of the span they're in the context of.
    fn queue(&self, trace: TraceId, signals: Signals) {
        let Signals { spans, records } = signals;
        let Some((last, others)) = self.destinations.split_last() else {
            return;
        };
        for record in records {
            for destination in others {
                let logger = &destination.records_logger;
                logger.emit(record.clone().into_sdk(trace, logger));
            }
            last.records_logger
                .emit(record.into_sdk(trace, &last.records_logger));
        }
        for span in spans {
            let data = span.into_data(trace, &self.scope);
            for destination in others {
                destination.spans.end(data.clone());
            }
            last.spans.end(data);
        }
    }

    /// The runs that have ended by now, whose wide events the next flush
    /// makes: everything else of them is in the queues. A run that ends
    /// while the queues are flushed may have its root span in none of the
    /// exports, so it waits for the flush after.
    fn take_closed(&self) -> Vec<Closed> {
        std::mem::take(&mut self.state().closed)
    }

    /// Flushes each destination on a thread of its own, and waits for them
    /// all for as long as a flush is given. A destination that hasn't
    /// answered by then is left to its thread, and said to have held the
    /// flush up; the others are whole.
    fn flush(self: &Arc<Self>) -> Vec<String> {
        let closed = Arc::new(self.take_closed());
        let timeout = self.flush_timeout;
        let (done, answers) = mpsc::channel();
        let mut failures = Vec::new();
        let mut waiting = Vec::new();
        for (index, destination) in self.destinations.iter().enumerate() {
            let inner = Arc::clone(self);
            let closed = Arc::clone(&closed);
            let done = done.clone();
            let started = thread::Builder::new()
                .name(format!(
                    "lablet-telemetry-flush-{}",
                    destination.names.destination
                ))
                .spawn(move || {
                    // Once the caller has stopped waiting there's no one to
                    // answer, and nothing to do about it.
                    let _ = done.send((index, inner.destinations[index].flush(&closed)));
                });
            match started {
                Ok(_) => waiting.push(index),
                Err(error) => failures.push(format!(
                    "{}: the flush couldn't start: {error}",
                    destination.names.destination
                )),
            }
        }
        drop(done);

        let deadline = Instant::now() + timeout;
        let mut answered = BTreeMap::new();
        let mut unanswered = format!("the flush didn't end within {timeout:?}");
        while answered.len() < waiting.len() {
            match answers.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((index, failed)) => {
                    answered.insert(index, failed);
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    "the flush didn't run to its end".clone_into(&mut unanswered);
                    break;
                }
            }
        }
        for index in waiting {
            match answered.remove(&index) {
                Some(failed) => failures.extend(failed),
                None => failures.push(format!(
                    "{}: {unanswered}",
                    self.destinations[index].names.destination
                )),
            }
        }
        failures
    }

    /// Stops the queues from a thread of its own, and waits for that for
    /// as long as a shutdown is given. The SDK waits five seconds for a
    /// queue's flush whatever it's told, so a destination that doesn't
    /// answer holds that thread for longer, and never the caller.
    fn shutdown(self: &Arc<Self>) -> Vec<String> {
        let timeout = self.shutdown_timeout;
        let (stopped, answer) = mpsc::sync_channel(1);
        let inner = Arc::clone(self);
        let stopping = thread::Builder::new()
            .name("lablet-telemetry-shutdown".to_owned())
            .spawn(move || {
                // Once the caller has stopped waiting there's no one to
                // answer, and nothing to do about it.
                let _ = stopped.send(inner.stop());
            });
        if let Err(error) = stopping {
            return vec![format!("the shutdown couldn't start: {error}")];
        }
        answer.recv_timeout(timeout).unwrap_or_else(|error| {
            vec![match error {
                RecvTimeoutError::Timeout => format!("the shutdown didn't end within {timeout:?}"),
                RecvTimeoutError::Disconnected => "the shutdown didn't run to its end".to_owned(),
            }]
        })
    }

    /// Flushes and stops each destination on a thread of its own, so none
    /// waits on another, and waits for them all: the shutdown's own bound
    /// is the caller's.
    fn stop(&self) -> Vec<String> {
        let closed = self.take_closed();
        let timeout = self.shutdown_timeout;
        thread::scope(|scope| {
            let stopping: Vec<_> = self
                .destinations
                .iter()
                .map(|destination| {
                    let closed = &closed;
                    scope.spawn(move || {
                        let mut failures = destination.flush(closed);
                        destination.stop(timeout, &mut failures);
                        failures
                    })
                })
                .collect();
            stopping
                .into_iter()
                .zip(&self.destinations)
                .flat_map(|(stopped, destination)| {
                    stopped.join().unwrap_or_else(|_| {
                        vec![format!(
                            "{}: the shutdown didn't run to its end",
                            destination.names.destination
                        )]
                    })
                })
                .collect()
        })
    }

    /// Starts the run `opening` describes, in place of any run that was in
    /// progress: a run that never ended has no root span, and none of its
    /// calls that were still open has a span either.
    fn started(&self, state: &mut State, opening: Opening) {
        if let Some(sink) = &self.sink {
            sink.start(&opening.context.run_id);
        }
        let (run, signals) = OpenRun::open(opening);
        self.queue(run.trace(), signals);
        state.open = Some(run);
    }

    /// Makes of the run in progress what `event` gives rise to, when that
    /// run is `run_id`. An event of any other run has no root span to be a
    /// child of, so nothing comes of it.
    fn during(
        &self,
        state: &mut State,
        run_id: &RunId,
        event: impl FnOnce(&mut OpenRun) -> Signals,
    ) {
        if let Some(run) = state.open.as_mut().filter(|run| run.is(run_id)) {
            let signals = event(run);
            self.queue(run.trace(), signals);
        }
    }

    /// One match over every kind of event, so that a kind the loop gains
    /// doesn't compile until the observer says what becomes of it.
    #[expect(
        clippy::too_many_lines,
        reason = "one match over every kind of event, whose arms each take their event apart"
    )]
    fn on(&self, event: RunEvent) {
        let RunEvent { run_id, kind } = event;
        let state = &mut *self.state();
        match kind {
            EventKind::RunStarted {
                context,
                model,
                endpoint,
                request,
                tools,
                system_prompt,
                prompt,
            } => self.started(
                state,
                Opening {
                    context: *context,
                    model,
                    endpoint,
                    request,
                    tools,
                    system_prompt,
                    prompt,
                },
            ),
            // There's no turn span: a turn is the index its spans carry.
            EventKind::TurnStarted { turn: _ } => {}
            EventKind::ProviderCallStarted {
                turn,
                attempt,
                request_bytes,
            } => self.during(state, &run_id, |run| {
                run.attempt_began(turn, attempt, request_bytes);
                Signals::default()
            }),
            EventKind::ProviderCallFinished {
                turn,
                attempt,
                record,
                response,
            } => self.during(state, &run_id, |run| {
                run.attempt_answered(turn, attempt, *record, response.as_deref())
            }),
            EventKind::ProviderCallFailed {
                turn,
                attempt,
                error,
                started_ms,
                latency_ms,
                retry,
            } => self.during(state, &run_id, |run| {
                let attempt = Attempt {
                    turn,
                    number: attempt,
                    started_ms,
                    latency_ms,
                };
                run.attempt_failed(attempt, &error, retry)
            }),
            EventKind::ProviderCallCancelled {
                turn,
                attempt,
                started_ms,
                latency_ms,
            } => self.during(state, &run_id, |run| {
                run.attempt_cancelled(Attempt {
                    turn,
                    number: attempt,
                    started_ms,
                    latency_ms,
                })
            }),
            // The turn is the one the call's last event names.
            EventKind::ToolCallStarted {
                turn: _,
                call_id,
                name,
                source,
                input_bytes,
                input,
            } => self.during(state, &run_id, |run| {
                run.call_began(call_id, name, source, input_bytes, input.as_ref());
                Signals::default()
            }),
            EventKind::ToolCallFinished {
                turn,
                call_id,
                status,
                started_ms,
                latency_ms,
                output_bytes,
                truncated_from_bytes,
                mcp,
                output,
            } => self.during(state, &run_id, |run| {
                let end = CallEnd {
                    turn,
                    status,
                    started_ms,
                    latency_ms,
                    output_bytes,
                    truncated_from_bytes,
                    mcp,
                    output,
                };
                run.call_ended(&call_id, end)
            }),
            EventKind::RunFinished { context, summary } => {
                if let Some(run) = state.open.take_if(|run| run.is(&run_id)) {
                    let trace = run.trace();
                    let (signals, closed) = run.close(*context, *summary);
                    self.queue(trace, signals);
                    state.closed.push(closed);
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl RunObserver for OtelObserver {
    async fn on(&self, event: RunEvent) {
        self.inner.on(event);
    }

    fn trace_context(&self, call_id: &ToolCallId) -> Option<TraceContext> {
        self.inner
            .state()
            .open
            .as_ref()
            .and_then(|run| run.propagated(call_id))
    }
}

#[cfg(test)]
mod tests;
