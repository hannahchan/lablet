//! The providers and the destinations behind them: what the composition
//! root builds once, hands the loop a tracer and a logger from, and flushes
//! after each run.

use std::time::Duration;

use lablet_run::telemetry::{Bridge, Logger, Record};
use opentelemetry::InstrumentationScope;
use opentelemetry::global::BoxedTracer;
use opentelemetry::logs::{LoggerProvider as _, NoopLoggerProvider};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::trace::noop::NoopTracer;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{BatchLogProcessor, LogExporter, SdkLoggerProvider};
use opentelemetry_sdk::trace::{BatchSpanProcessor, SdkTracerProvider, SpanExporter};

use super::file::{FileLogExporter, FileSpanExporter, FileTarget, Sink};
use super::network::{OtelBuildError, OtlpSettings, exporters};
use crate::otel_env::Sdk;

/// What a flush or a shutdown couldn't export, as the SDK said it. Nothing
/// about a run changes for it: it's for whoever reports to the person who
/// ran lablet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("telemetry wasn't exported whole: {}", failures.join("; "))]
pub(crate) struct FlushError {
    failures: Vec<String>,
}

impl FlushError {
    /// What failed, one entry for each provider whose processors said so:
    /// `spans: ` or `log records: `, then the SDK's error.
    #[must_use]
    pub(crate) fn failures(&self) -> &[String] {
        &self.failures
    }
}

/// What a shutdown gives each processor to export what it holds and stop.
const SHUTDOWN: Duration = Duration::from_secs(5);

/// One destination's processors, made once the SDK's settings are known.
#[cfg(test)]
type Hooked = Box<dyn FnOnce(&Sdk) -> (BatchSpanProcessor, BatchLogProcessor) + Send>;

/// What a [`Telemetry`] is built from.
pub(crate) struct TelemetryBuilder {
    scope: InstrumentationScope,
    resource: Resource,
    file: Option<FileTarget>,
    otlp: Option<OtlpSettings>,
    sdk: Sdk,
    disabled: bool,
    #[cfg(test)]
    hooked: Vec<Hooked>,
}

impl std::fmt::Debug for TelemetryBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryBuilder")
            .field("scope", &self.scope)
            .field("resource", &self.resource)
            .field("file", &self.file)
            .field("otlp", &self.otlp)
            .field("sdk", &self.sdk)
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl TelemetryBuilder {
    /// The resource every export describes itself with, in place of an
    /// empty one.
    #[must_use]
    pub(crate) fn resource(mut self, resource: Resource) -> Self {
        self.resource = resource;
        self
    }

    /// Exports to `target` as OTLP/JSON lines.
    #[must_use]
    pub(crate) fn file(mut self, target: FileTarget) -> Self {
        self.file = Some(target);
        self
    }

    /// Exports to an OTLP collector over the network, as `settings` says.
    /// Each header's value is a secret from here on: no `Debug` or message
    /// shows one. A header that isn't one is refused by [`Self::build`].
    #[must_use]
    pub(crate) fn otlp(mut self, settings: OtlpSettings) -> Self {
        self.otlp = Some(settings);
        self
    }

    /// The sampler, the span limits and the batch processors' settings, in
    /// place of the specification's defaults.
    #[must_use]
    pub(crate) fn sdk(mut self, sdk: Sdk) -> Self {
        self.sdk = sdk;
        self
    }

    /// No telemetry at all, whatever else is said: the API's no-op tracer
    /// and a logger that drops what it's handed, as the specification's
    /// no-op SDK is, so nothing is written or sent and no span has an id of
    /// its own. A span opened in a context with a span keeps that span's
    /// context.
    #[must_use]
    pub(crate) fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// Exports to `spans` and `records` too, as a destination registered
    /// where the network's is, after the file's.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn exporting_to<S, L>(mut self, spans: S, records: L) -> Self
    where
        S: SpanExporter + 'static,
        L: LogExporter + 'static,
    {
        self.hooked
            .push(Box::new(move |sdk| processors(sdk, spans, records)));
        self
    }

    /// The providers, with a batch processor of the SDK's for each
    /// destination and signal. With a network destination it must be
    /// called inside a tokio runtime, which the gRPC channel's worker runs
    /// on.
    ///
    /// # Errors
    ///
    /// Returns an [`OtelBuildError`] when the network exporters can't be
    /// made: an endpoint isn't one the exporter accepts, a header isn't one
    /// a header may have, TLS to an endpoint can't be set up, or a client
    /// can't be made.
    pub(crate) fn build(self) -> Result<Telemetry, OtelBuildError> {
        let Self {
            scope,
            resource,
            file,
            otlp,
            sdk,
            disabled,
            #[cfg(test)]
            hooked,
        } = self;
        if disabled {
            return Ok(Telemetry {
                scope,
                providers: None,
                sink: None,
            });
        }

        // A provider flushes and stops its processors one after the other,
        // in the order they were added, so the file's come first: it never
        // waits on the network.
        let sink = file.map(Sink::new);
        let mut span_processors = Vec::new();
        let mut log_processors = Vec::new();
        if let Some(sink) = &sink {
            span_processors.push(span_processor(&sdk, FileSpanExporter::new(sink.clone())));
            log_processors.push(log_processor(&sdk, FileLogExporter::new(sink.clone())));
        }
        if let Some(settings) = otlp {
            let (spans, records) = exporters(&settings)?;
            span_processors.extend(spans.map(|spans| span_processor(&sdk, spans)));
            log_processors.extend(records.map(|records| log_processor(&sdk, records)));
        }
        #[cfg(test)]
        for hook in hooked {
            let (spans, records) = hook(&sdk);
            span_processors.push(spans);
            log_processors.push(records);
        }

        let mut tracer_provider = SdkTracerProvider::builder()
            .with_sampler(sdk.sampling.sampler())
            .with_span_limits(sdk.limits.span_limits())
            .with_resource(resource.clone());
        for processor in span_processors {
            tracer_provider = tracer_provider.with_span_processor(processor);
        }
        let mut logger_provider = SdkLoggerProvider::builder().with_resource(resource);
        for processor in log_processors {
            logger_provider = logger_provider.with_log_processor(processor);
        }
        Ok(Telemetry {
            scope,
            providers: Some(Providers {
                tracer: tracer_provider.build(),
                logger: logger_provider.build(),
            }),
            sink,
        })
    }
}

/// The batch span processor of one destination, as the SDK's settings say.
fn span_processor(sdk: &Sdk, spans: impl SpanExporter + 'static) -> BatchSpanProcessor {
    BatchSpanProcessor::builder(spans)
        .with_batch_config(sdk.spans.span_config())
        .build()
}

/// The batch log record processor of one destination, as the SDK's
/// settings say.
fn log_processor(sdk: &Sdk, records: impl LogExporter + 'static) -> BatchLogProcessor {
    BatchLogProcessor::builder(records)
        .with_batch_config(sdk.logs.log_config())
        .build()
}

/// The batch processors of one destination, as the SDK's settings say.
#[cfg(test)]
fn processors<S, L>(sdk: &Sdk, spans: S, records: L) -> (BatchSpanProcessor, BatchLogProcessor)
where
    S: SpanExporter + 'static,
    L: LogExporter + 'static,
{
    (span_processor(sdk, spans), log_processor(sdk, records))
}

/// The providers a run's telemetry goes through, and the destinations
/// behind them.
///
/// The loop opens its spans through [`Telemetry::tracer`] and emits its
/// records through [`Telemetry::logger`], which only queue what they're
/// handed, so neither waits for an export. Whoever runs the loop calls
/// [`Telemetry::begin_run`] before a run and [`Telemetry::flush`] when it
/// has returned, with the run's wide event, and [`Telemetry::shutdown`]
/// before the process exits. A `Telemetry` dropped without a shutdown is
/// shut down by the SDK on the thread that drops it, which exports what the
/// processors hold but costs that thread up to five seconds for each
/// processor, in turn, when a destination doesn't answer.
#[derive(Debug, Clone)]
pub(crate) struct Telemetry {
    scope: InstrumentationScope,
    /// `None` when there's no telemetry at all.
    providers: Option<Providers>,
    sink: Option<Sink>,
}

/// The two providers every destination is behind.
#[derive(Debug, Clone)]
struct Providers {
    tracer: SdkTracerProvider,
    logger: SdkLoggerProvider,
}

impl Telemetry {
    /// A builder emitting every span and record under `scope`, the
    /// instrumentation scope the composition root makes.
    pub(crate) fn builder(scope: InstrumentationScope) -> TelemetryBuilder {
        TelemetryBuilder {
            scope,
            resource: Resource::builder_empty().build(),
            file: None,
            otlp: None,
            sdk: Sdk::default(),
            disabled: false,
            #[cfg(test)]
            hooked: Vec::new(),
        }
    }

    /// A tracer of the scope the composition root handed over, whose spans
    /// reach every destination, sampled and limited as the SDK's settings
    /// say.
    #[must_use]
    pub(crate) fn tracer(&self) -> BoxedTracer {
        match &self.providers {
            Some(providers) => BoxedTracer::new(Box::new(
                providers.tracer.tracer_with_scope(self.scope.clone()),
            )),
            None => BoxedTracer::new(Box::new(NoopTracer::new())),
        }
    }

    /// Lablet's logger, of the scope the composition root handed over,
    /// whose records reach every destination.
    #[must_use]
    pub(crate) fn logger(&self) -> Box<dyn Logger> {
        match &self.providers {
            Some(providers) => Box::new(Bridge::new(
                providers.logger.logger_with_scope(self.scope.clone()),
            )),
            None => Box::new(Bridge::new(
                NoopLoggerProvider::new().logger_with_scope(self.scope.clone()),
            )),
        }
    }

    /// Called before every run. It lets go of the file that's open, so
    /// that the run's first line opens the path again and a file moved
    /// between two runs keeps the first run while the path gets the
    /// second. Nothing is opened here, and a `Telemetry` with no file does
    /// nothing.
    pub(crate) fn begin_run(&self) {
        if let Some(sink) = &self.sink {
            sink.start();
        }
    }

    /// Emits `wide`, the run's wide event, through lablet's logger, and
    /// then flushes both providers, side by side. Each flushes the file's
    /// processor before the network's, so when this returns the file holds
    /// the run whole, its wide event included, whatever a collector did.
    ///
    /// The SDK gives each processor's flush five seconds, so a collector
    /// that never answers costs this about five seconds.
    ///
    /// # Errors
    ///
    /// Returns a [`FlushError`] holding what each provider's processors
    /// said of a flush that failed or gave up.
    pub(crate) async fn flush(&self, wide: Record) -> Result<(), FlushError> {
        self.logger().emit(wide);
        self.flush_leftovers().await
    }

    /// Flushes both providers, side by side, as [`Self::flush`] does, with
    /// no wide event: what a run whose future was dropped left, its open
    /// spans ended as they were dropped.
    ///
    /// # Errors
    ///
    /// As [`Self::flush`].
    pub(crate) async fn flush_leftovers(&self) -> Result<(), FlushError> {
        let Some(Providers { tracer, logger }) = self.providers.clone() else {
            return Ok(());
        };
        both(move || tracer.force_flush(), move || logger.force_flush()).await
    }

    /// Flushes what the processors still hold and stops them, both
    /// providers side by side, each processor given five seconds.
    /// So a collector that never answers holds this about five seconds, and
    /// what it still holds then is left to a thread that nothing waits for.
    /// Spans and records that come after a shutdown are dropped.
    ///
    /// # Errors
    ///
    /// Returns a [`FlushError`] holding what each provider said of a
    /// processor that failed to export or to stop in time, or that the
    /// provider was shut down already.
    pub(crate) async fn shutdown(&self) -> Result<(), FlushError> {
        let Some(Providers { tracer, logger }) = self.providers.clone() else {
            return Ok(());
        };
        both(
            move || tracer.shutdown_with_timeout(SHUTDOWN),
            move || logger.shutdown_with_timeout(SHUTDOWN),
        )
        .await
    }
}

/// Runs `spans` and `records` side by side where they may wait, since a
/// processor answers from a thread of its own and waiting for it on the
/// runtime's would hold up whatever else runs there, and gathers what each
/// said went wrong.
async fn both(
    spans: impl FnOnce() -> OTelSdkResult + Send + 'static,
    records: impl FnOnce() -> OTelSdkResult + Send + 'static,
) -> Result<(), FlushError> {
    let (spans, records) = tokio::join!(
        tokio::task::spawn_blocking(spans),
        tokio::task::spawn_blocking(records)
    );
    let failures: Vec<String> = [("spans", spans), ("log records", records)]
        .into_iter()
        .filter_map(|(provider, done)| match done {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(format!("{provider}: {error}")),
            Err(error) => Some(format!("{provider}: it didn't run to its end: {error}")),
        })
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(FlushError { failures })
    }
}

#[cfg(test)]
mod tests;
