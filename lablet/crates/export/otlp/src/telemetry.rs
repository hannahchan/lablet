//! The destinations and the providers behind them: what the composition
//! root builds once, hands the loop a tracer and a logger from, and flushes
//! after each run.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use lablet_model::RunId;
use lablet_run::telemetry::{Bridge, Logger, Record};
use opentelemetry::global::BoxedTracer;
use opentelemetry::logs::LoggerProvider as _;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::logs::{LogExporter, LogProcessor as _, SdkLogger, SdkLoggerProvider};
use opentelemetry_sdk::resource::{EnvResourceDetector, TelemetryResourceDetector};
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider, SpanExporter, SpanProcessor as _};

use crate::file::{FileLogExporter, FileSpanExporter, FileTarget, Sink};
use crate::network::{Network, OtelBuildError, OtlpSettings, validate};
use crate::pipeline::{Lost, RecordQueue, SpanQueue};

/// The name of the service.
const LABLET: &str = "lablet";

/// The resource keys lablet sets itself, as the resource conventions name
/// them. They're the service's, not a signal's: the registry's attributes
/// are named by the crates that emit them.
const SERVICE_NAME: &str = "service.name";
const SERVICE_VERSION: &str = "service.version";

/// How many attributes and how many events a span keeps. A run is one
/// trace whose wide event counts every span in it, so an attribute a limit
/// cut would be a step the record says happened and the file doesn't hold;
/// the limits are set here so `OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT` and
/// `OTEL_SPAN_EVENT_COUNT_LIMIT` can't lower them.
const SPAN_LIMIT: u32 = 128;

/// How long a flush and a shutdown are each given unless the builder says
/// otherwise, so a destination that doesn't answer costs seconds.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// The run's wide event, as the composition root hands it over: a function
/// of the count of records a destination lost. Each destination calls it
/// with its own count once its own flush has ended, and emits the record it
/// returns through a logger of its own, in an export of its own.
pub type WideEvent = Box<dyn Fn(u64) -> Record + Send + Sync>;

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
///
/// The span queue and the record queue are processors of the providers
/// every destination shares, so the loop's tracer and logger reach each
/// destination; the wide event's queue has a provider of its own, so the
/// event is never in a batch with a content record.
struct Destination {
    names: Names,
    spans: SpanQueue,
    records: RecordQueue,
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

/// What a [`Telemetry`] is built from.
pub struct TelemetryBuilder {
    version: String,
    scope: InstrumentationScope,
    resource: Vec<(String, String)>,
    file: Option<FileTarget>,
    otlp: Option<OtlpSettings>,
    flush_timeout: Duration,
    shutdown_timeout: Duration,
    destinations: Vec<MakeDestination>,
}

impl std::fmt::Debug for TelemetryBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryBuilder")
            .field("version", &self.version)
            .field("scope", &self.scope)
            .field("resource", &self.resource)
            .field("file", &self.file)
            .field("otlp", &self.otlp)
            .field("flush_timeout", &self.flush_timeout)
            .field("shutdown_timeout", &self.shutdown_timeout)
            .finish_non_exhaustive()
    }
}

impl TelemetryBuilder {
    /// The composer's own resource attributes, which every export carries
    /// beside `service.name`, `service.version` and what the SDK says of
    /// itself, over those `OTEL_RESOURCE_ATTRIBUTES` holds when it's set. A
    /// key the environment names too takes the composer's value, and a key
    /// of lablet's own keeps lablet's value whichever names it.
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

    /// How long [`Telemetry::flush`] waits for a destination, in place of
    /// five seconds.
    #[must_use]
    pub const fn flush_timeout(mut self, timeout: Duration) -> Self {
        self.flush_timeout = timeout;
        self
    }

    /// How long [`Telemetry::shutdown`] waits, in place of five seconds.
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

    /// The destinations, with a thread behind each of their queues, and the
    /// providers over them. With a network destination it must be called
    /// inside a tokio runtime, which the gRPC channel's worker runs on.
    ///
    /// # Errors
    ///
    /// Returns an [`OtelBuildError`] when the network exporters can't be
    /// made: the endpoint isn't one the exporter accepts, a header isn't
    /// one a header may have, or TLS to the endpoint can't be set up.
    pub fn build(self) -> Result<Telemetry, OtelBuildError> {
        let Self {
            version,
            scope,
            resource,
            file,
            otlp,
            flush_timeout,
            shutdown_timeout,
            mut destinations,
        } = self;
        // A later source wins a key an earlier one set. So the environment's
        // attributes are defaults beneath the composer's, as every `OTEL_*`
        // variable lablet inherits is, and what the SDK says of itself and
        // lablet's own two keys come last, so neither source can rename the
        // service or misstate the SDK. `OTEL_SERVICE_NAME` goes unread: the
        // detector that reads it isn't one of these.
        let resource = Resource::builder_empty()
            .with_detector(Box::new(EnvResourceDetector::new()))
            .with_attributes(
                resource
                    .into_iter()
                    .map(|(key, value)| KeyValue::new(key, value)),
            )
            .with_detector(Box::new(TelemetryResourceDetector))
            .with_attributes([
                KeyValue::new(SERVICE_NAME, LABLET),
                KeyValue::new(SERVICE_VERSION, version),
            ])
            .build();

        if let Some(settings) = otlp {
            // What a check of the settings refuses, the build refuses first,
            // so the two can't come apart. Only what the check can't do
            // without making it is the build's alone: the trust roots of a
            // gRPC exporter that speaks TLS, and the HTTP client.
            validate(&settings)?;
            let (spans, records, wide) = Network::new(&settings)?.exporters()?;
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
        let destinations: Vec<Destination> = destinations
            .into_iter()
            .map(|make| make(&resource, &scope))
            .collect();

        // The sampler and the limits are set after the SDK has read its
        // environment, so the variables that would lower them can't.
        let mut tracer_provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_max_attributes_per_span(SPAN_LIMIT)
            .with_max_events_per_span(SPAN_LIMIT)
            .with_resource(resource.clone());
        let mut records_provider = SdkLoggerProvider::builder().with_resource(resource);
        for destination in &destinations {
            tracer_provider = tracer_provider.with_span_processor(destination.spans.clone());
            records_provider = records_provider.with_log_processor(destination.records.clone());
        }

        let destinations_flushing: Vec<Vec<thread::JoinHandle<()>>> =
            destinations.iter().map(|_| Vec::new()).collect();
        Ok(Telemetry {
            inner: Arc::new(Inner {
                scope,
                tracer_provider: tracer_provider.build(),
                records_provider: records_provider.build(),
                destinations,
                sink,
                flush_timeout,
                shutdown_timeout,
                flushing: Mutex::new(destinations_flushing),
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
        // The wide event isn't one of the records its own count is of, so
        // what becomes of it is counted apart and reported by the flush.
        let wide = SdkLoggerProvider::builder()
            .with_resource(resource.clone())
            .with_log_processor(RecordQueue::new(
                names.destination,
                wide,
                resource,
                &Arc::new(Lost::default()),
            ))
            .build();
        Destination {
            names,
            spans: SpanQueue::new(names.destination, spans, resource, &lost),
            records: RecordQueue::new(names.destination, records, resource, &lost),
            wide_logger: wide.logger_with_scope(scope.clone()),
            wide,
            lost,
        }
    })
}

#[derive(Debug)]
struct Inner {
    scope: InstrumentationScope,
    tracer_provider: SdkTracerProvider,
    records_provider: SdkLoggerProvider,
    destinations: Vec<Destination>,
    sink: Option<Sink>,
    flush_timeout: Duration,
    shutdown_timeout: Duration,
    /// For each destination, the flush threads that may still be running:
    /// a flush that gave up on a destination left its thread, which makes
    /// the wide event of the run it took once the destination answers.
    /// A shutdown waits for them before it stops that destination's
    /// queues, or a wide event made late would be emitted into a queue
    /// already stopped.
    flushing: Mutex<Vec<Vec<thread::JoinHandle<()>>>>,
}

/// The destinations a run's telemetry goes to, and the providers behind
/// them.
///
/// The loop opens its spans through [`Telemetry::tracer`] and emits its
/// records through [`Telemetry::logger`], which only queue what they're
/// handed, so neither waits for an export. Whoever runs the loop calls
/// [`Telemetry::begin_run`] before a run and [`Telemetry::flush`] when it
/// has returned, which is when the run's wide event is made and exported,
/// and [`Telemetry::shutdown`] before the process exits. A `Telemetry`
/// dropped without a shutdown is stopped by the SDK instead, which exports
/// what the queues hold but costs the thread that drops it up to the SDK's
/// five seconds for each of its two providers, ten in all, when a
/// destination doesn't answer.
#[derive(Debug, Clone)]
pub struct Telemetry {
    inner: Arc<Inner>,
}

impl Telemetry {
    /// A builder for lablet `version`, which is the `service.version` of
    /// the resource, emitting every span and record under `scope`, the
    /// instrumentation scope the composition root makes. The resource is
    /// the service's and the scope is the instrumentation's, so each
    /// carries a version of its own, and the composition root hands the
    /// same one to both; nothing here refuses two that differ.
    pub fn builder(version: impl Into<String>, scope: InstrumentationScope) -> TelemetryBuilder {
        TelemetryBuilder {
            version: version.into(),
            scope,
            resource: Vec::new(),
            file: None,
            otlp: None,
            flush_timeout: SHUTDOWN_TIMEOUT,
            shutdown_timeout: SHUTDOWN_TIMEOUT,
            destinations: Vec::new(),
        }
    }

    /// A tracer of the scope the composition root handed over, whose spans
    /// reach every destination. Every span it opens is sampled, and a span
    /// keeps up to 128 attributes and 128 events whatever the environment
    /// says.
    #[must_use]
    pub fn tracer(&self) -> BoxedTracer {
        BoxedTracer::new(Box::new(
            self.inner
                .tracer_provider
                .tracer_with_scope(self.inner.scope.clone()),
        ))
    }

    /// Lablet's logger, of the scope the composition root handed over,
    /// whose records reach every destination.
    #[must_use]
    pub fn logger(&self) -> Box<dyn Logger> {
        Box::new(Bridge::new(
            self.inner
                .records_provider
                .logger_with_scope(self.inner.scope.clone()),
        ))
    }

    /// Called before every run, whatever the file target. With a directory
    /// of per-run files it names the run's file; with a fixed path or
    /// standard error it lets go of the file that's open, so that the run's
    /// first line opens the path again and a file moved between two runs
    /// keeps the first run while the path gets the second. Nothing is
    /// opened here, and a `Telemetry` with no file does nothing.
    pub fn begin_run(&self, run_id: &RunId) {
        if let Some(sink) = &self.inner.sink {
            sink.start(run_id);
        }
    }

    /// Exports what the run so far gave rise to, to each destination on its
    /// own: its spans and its log records, then, in an export of its own,
    /// the run's wide event, which `wide` makes from that destination's own
    /// count of what it lost.
    ///
    /// The wide event is made once everything else of its run has been
    /// exported to the destination or lost, so its count is whole.
    ///
    /// Each destination is waited for no longer than the builder's flush
    /// timeout, five seconds unless it says otherwise, and none waits on
    /// another: when this returns, a file the telemetry exports to holds
    /// every line of the run, whatever a collector did. A destination that
    /// didn't answer in time is left to its thread, which still makes the
    /// wide event once the destination answers.
    ///
    /// # Errors
    ///
    /// Returns a [`FlushError`] that names each queue whose export failed,
    /// and each destination that didn't end in time.
    pub async fn flush(&self, wide: WideEvent) -> Result<(), FlushError> {
        let inner = Arc::clone(&self.inner);
        let wide = Arc::new(wide);
        blocking(move || inner.flush(Some(&wide))).await
    }

    /// Waits for the threads of the flushes that gave up, so that a wide
    /// event made late is in its queue first, then flushes what the queues
    /// still hold, and then stops every queue's thread, and waits for all
    /// that no longer than the builder's shutdown timeout, five seconds
    /// unless it says otherwise. So a destination that doesn't
    /// answer can't hold a process that's ready to exit: what it still
    /// holds then is left to a thread that nothing waits for. Spans and
    /// records that come after a shutdown that ended are dropped.
    ///
    /// # Errors
    ///
    /// Returns a [`FlushError`] that names each queue whose export failed,
    /// or that didn't stop in time, or that says the shutdown didn't end.
    pub async fn shutdown(&self) -> Result<(), FlushError> {
        let inner = Arc::clone(&self.inner);
        blocking(move || inner.shutdown()).await
    }
}

/// Runs `work` where it may wait: a queue answers a flush from its own
/// thread, and waiting for it on the runtime's would hold up whatever else
/// runs there.
async fn blocking(work: impl FnOnce() -> Vec<String> + Send + 'static) -> Result<(), FlushError> {
    let failures = tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| vec![format!("the flush didn't run to its end: {error}")]);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(FlushError { failures })
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
    /// side, and then the wide event `wide` makes, when there is one, with
    /// the count of what was lost by then.
    fn flush(&self, wide: Option<&WideEvent>) -> Vec<String> {
        let mut failures = Vec::new();
        let (spans, records) = thread::scope(|scope| {
            let records = scope.spawn(|| self.records.force_flush());
            let spans = self.spans.force_flush();
            let records = records.join().unwrap_or_else(|_| {
                Err(OTelSdkError::InternalFailure(
                    "the flush didn't run to its end".to_owned(),
                ))
            });
            (spans, records)
        });
        note(&mut failures, self.names.spans, spans);
        note(&mut failures, self.names.records, records);

        if let Some(wide) = wide {
            wide(self.lost.take()).emit_through(&self.wide_logger);
        }
        note(&mut failures, self.names.wide, self.wide.force_flush());
        failures
    }

    /// Stops the three queues, each given `timeout`.
    fn stop(&self, timeout: Duration, failures: &mut Vec<String>) {
        note(
            failures,
            self.names.spans,
            self.spans.shutdown_with_timeout(timeout),
        );
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
    fn flushing(&self) -> MutexGuard<'_, Vec<Vec<thread::JoinHandle<()>>>> {
        self.flushing.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Flushes each destination on a thread of its own, and waits for them
    /// all for as long as a flush is given. A destination that hasn't
    /// answered by then is left to its thread, and said to have held the
    /// flush up; the others are whole.
    fn flush(self: &Arc<Self>, wide: Option<&Arc<WideEvent>>) -> Vec<String> {
        let timeout = self.flush_timeout;
        let (done, answers) = mpsc::channel();
        let mut failures = Vec::new();
        let mut waiting = Vec::new();
        for (index, destination) in self.destinations.iter().enumerate() {
            let inner = Arc::clone(self);
            let wide = wide.map(Arc::clone);
            let done = done.clone();
            let started = thread::Builder::new()
                .name(format!(
                    "lablet-telemetry-flush-{}",
                    destination.names.destination
                ))
                .spawn(move || {
                    // Once the caller has stopped waiting there's no one to
                    // answer, and nothing to do about it.
                    let _ = done.send((index, inner.destinations[index].flush(wide.as_deref())));
                });
            match started {
                Ok(thread) => {
                    let mut flushing = self.flushing();
                    flushing[index].retain(|thread| !thread.is_finished());
                    flushing[index].push(thread);
                    waiting.push(index);
                }
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
    /// is the caller's. Then marks the providers shut down, so that their
    /// drop doesn't stop the queues again.
    fn stop(&self) -> Vec<String> {
        let timeout = self.shutdown_timeout;
        let flushing: Vec<_> = self.flushing().iter_mut().map(std::mem::take).collect();
        let failures = thread::scope(|scope| {
            let stopping: Vec<_> = self
                .destinations
                .iter()
                .zip(flushing)
                .map(|(destination, flushing)| {
                    scope.spawn(move || {
                        // A flush thread's failures were said, or given up
                        // on, by the flush that started it.
                        for thread in flushing {
                            let _ = thread.join();
                        }
                        let mut failures = destination.flush(None);
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
        });
        // Every queue was told to stop above and answers a provider at
        // once, so this only marks the providers, which otherwise stop
        // their processors again when they're dropped, each for up to the
        // SDK's five seconds. What the queues said of their stop is in
        // `failures`; a provider has nothing to add, and a second shutdown
        // of a provider says only that it was shut down already.
        let _ = self.tracer_provider.shutdown_with_timeout(timeout);
        let _ = self.records_provider.shutdown_with_timeout(timeout);
        failures
    }
}

#[cfg(test)]
mod tests;
