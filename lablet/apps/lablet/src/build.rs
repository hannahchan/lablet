//! `build` and `check`: a config in, a `Lablet` or a checked config out.
//! The library's composition: it checks the config through
//! `lablet-prepare`, builds the tools and the provider through the wiring
//! kernels, resolves the tracer, the logger and the propagator once, from
//! what the host handed in or else OpenTelemetry's globals, and connects
//! them to the loop and the runner itself.

use std::sync::Arc;

use lablet_config::{Config, Env, TelemetryFile, environment};
use lablet_prepare::{BuildError, Checked, Prepared, Telemetry, Wiring, prepare};
use lablet_run::telemetry::generated::SCHEMA_URL;
use lablet_run::telemetry::{Bridge, Logger, Record, SCOPE};
use lablet_run::{Clock, RunCancellation, RunService, Runner, Shared, ToolSet, TranscriptWriter};
use lablet_transcript_json::JsonTranscripts;
use opentelemetry::global::{BoxedSpan, BoxedTracer};
use opentelemetry::logs::LoggerProvider;
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::noop::NoopTextMapPropagator;
use opentelemetry::trace::{Span, SpanBuilder, Tracer, TracerProvider};
use opentelemetry::{Context, InstrumentationScope};

use crate::clock::TokioClock;
use crate::fallback::{self, GlobalPropagator};
use crate::lablet::Lablet;

/// How a [`Lablet`] is built: from a config and the logger provider its
/// records go to, with the tracer provider its spans go to and the
/// propagator its context is injected through each handed in or left to
/// OpenTelemetry's global one. Made by [`Lablet::builder`].
pub struct Builder {
    config: Config,
    /// lablet's logger, taken from the provider the host handed in.
    logger: Arc<dyn Logger>,
    /// The tracer of the provider the host handed in, when it handed one in.
    tracer: Option<BoxedTracer>,
    /// The propagator the host handed in, when it handed one in.
    propagator: Option<Arc<dyn TextMapPropagator + Send + Sync>>,
}

impl core::fmt::Debug for Builder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Builder")
            .field("config", &self.config.digest())
            .field("tracer_provider", &self.tracer.is_some())
            .field("propagator", &self.propagator.is_some())
            .finish_non_exhaustive()
    }
}

impl Lablet {
    /// A builder of a `Lablet` that runs as `config` says and emits its
    /// records through `logger_provider`, from which it takes one logger of
    /// lablet's instrumentation scope, here, and keeps nothing else. The
    /// provider is required, since OpenTelemetry's API has no global logger
    /// provider to fall back to; a host that wants spans and no records
    /// hands in the API's [`NoopLoggerProvider`].
    ///
    /// The spans go to the tracer provider handed to
    /// [`Builder::with_tracer_provider`], or else to OpenTelemetry's global
    /// tracer provider as it is when [`Builder::build`] is called, so a host
    /// sets its global providers before it builds a `Lablet`. A command's
    /// context is injected through the propagator handed to
    /// [`Builder::with_propagator`], or else through OpenTelemetry's global
    /// propagator as it is when the command starts.
    ///
    /// [`NoopLoggerProvider`]: opentelemetry::logs::NoopLoggerProvider
    ///
    /// A `Lablet` can't be built without a logger provider:
    ///
    /// ```compile_fail
    /// # async fn host(config: lablet::Config) -> Result<(), lablet::BuildError> {
    /// let lablet = lablet::Lablet::builder(config).build().await?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "a host hands its provider in, as OpenTelemetry's own setters take one, and lablet keeps the logger it takes from it rather than the provider"
    )]
    pub fn builder<P>(config: Config, logger_provider: P) -> Builder
    where
        P: LoggerProvider,
        P::Logger: Send + Sync + 'static,
    {
        Builder {
            config,
            logger: Arc::new(Bridge::new(logger_provider.logger_with_scope(scope()))),
            tracer: None,
            propagator: None,
        }
    }
}

impl Builder {
    /// Sends the spans of every run to `provider`, from which one tracer of
    /// lablet's instrumentation scope is taken, here, in place of the global
    /// tracer provider's.
    #[must_use]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "a host hands its provider in, as OpenTelemetry's own `set_tracer_provider` takes one, and lablet keeps the tracer it takes from it rather than the provider"
    )]
    pub fn with_tracer_provider<P, T, S>(mut self, provider: P) -> Self
    where
        P: TracerProvider<Tracer = T> + Send + Sync + 'static,
        T: Tracer<Span = S> + Send + Sync + 'static,
        S: Span + Send + Sync + 'static,
    {
        self.tracer = Some(BoxedTracer::new(Box::new(
            provider.tracer_with_scope(scope()),
        )));
        self
    }

    /// Injects the context of each process a run starts through
    /// `propagator`, in place of OpenTelemetry's global propagator: a
    /// `bash` command's environment is given the context of its tool span,
    /// as the command starts.
    #[must_use]
    pub fn with_propagator<P>(mut self, propagator: P) -> Self
    where
        P: TextMapPropagator + Send + Sync + 'static,
    {
        self.propagator = Some(Arc::new(propagator));
        self
    }

    /// The `Lablet`. With no tracer provider handed in, OpenTelemetry's
    /// global tracer provider is read once, here. With no propagator handed
    /// in, OpenTelemetry's global propagator is asked at each injection, and
    /// the variables it names are those it names here.
    ///
    /// # Errors
    ///
    /// What [`build`] returns.
    pub async fn build(self) -> Result<Lablet, BuildError> {
        build_in(self, &environment).await
    }
}

/// lablet's instrumentation scope: its name, its version and the schema
/// URL of the registry its signals are declared in.
fn scope() -> InstrumentationScope {
    InstrumentationScope::builder(SCOPE)
        .with_version(crate::VERSION)
        .with_schema_url(SCHEMA_URL)
        .build()
}

/// Checks `config` whole, as [`build`] does, and stops before the provider
/// is selected, so the answer is the same for every provider: one this
/// lablet has no adapter for yet passes, and no call reaches a provider.
///
/// It substitutes `${VAR}`, checks every setting, that the provider's key
/// variable is set when the provider needs a key, that each file the config
/// names can be read, the fake provider's script among them, and the root
/// holds none of lablet's own, and builds the tool set, so the tools a run
/// is offered are listed. It checks nothing that only configured an
/// OpenTelemetry SDK, which a library run leaves to its host: the
/// `telemetry` section's file, endpoint and resource are passed as they
/// are, a `${VAR}` in the file or the resource isn't substituted, and no
/// `OTEL_*` variable but those the secrets name and the capture variable is
/// read.
///
/// # Errors
///
/// Returns what [`build`] returns, but for [`BuildError::Unsupported`] of
/// the provider.
pub async fn check(config: &Config) -> Result<Checked, BuildError> {
    check_in(config, &environment).await
}

/// [`check`], where `env` is lablet's environment.
pub(crate) async fn check_in(config: &Config, env: Env<'_>) -> Result<Checked, BuildError> {
    let prepared = prepared(config, env)?;
    // A check starts no command, so there's no context to inject, and no
    // global to read for one.
    let untraced = Arc::new(NoopTextMapPropagator::new());
    let tools = lablet_tools_wiring::tools(config, &prepared, untraced).await?;
    Ok(Checked::new(
        config.resolved(),
        tools.specs().to_vec(),
        prepared.into_secrets(),
    ))
}

/// The check of `config` in `env`, gone on with what a library run reads of
/// the telemetry: the capture variable alone, since the host's SDK reads
/// the rest, and no file of lablet's own.
fn prepared(config: &Config, env: Env<'_>) -> Result<Prepared, BuildError> {
    // The file and the resource only configured an SDK, so a `${VAR}` in
    // them isn't substituted, and an unset one isn't refused. The endpoint
    // and the headers stay, since their secrets are still cut.
    let mut checked = config.clone();
    checked.telemetry.file = TelemetryFile::default();
    checked.telemetry.resource.clear();
    Ok(prepare(&checked, env)?.finish(
        &checked,
        env,
        Telemetry {
            capture_content: lablet_otel_env::capture_content(env),
            ..Telemetry::default()
        },
    ))
}

/// A `Lablet` that runs as `config` says and emits its records through
/// `logger_provider`, its spans through OpenTelemetry's global tracer
/// provider as it is now, and a command's context through OpenTelemetry's
/// global propagator as it is when the command starts: [`Lablet::builder`]
/// with nothing else handed in.
///
/// Library mode configures no OpenTelemetry SDK: the host's SDK samples,
/// exports and flushes what lablet emits. The config's `telemetry.file`,
/// `telemetry.otlp` and `telemetry.resource` do nothing, and so do the
/// `OTEL_*` variables the SDK reads, `OTEL_SDK_DISABLED`, and `TRACEPARENT`,
/// `TRACESTATE` and `BAGGAGE` as the run's parent, and none of them is
/// warned about.
/// `telemetry.capture_content` and then
/// `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` still decide whether
/// content is captured, and the secrets the config names are still withheld
/// from commands and cut from what they print, the values of
/// `telemetry.otlp.headers` and the user information of
/// `telemetry.otlp.endpoint` among them, and so are the values of the OTLP
/// header variables and the user information of the endpoint variables.
///
/// # Errors
///
/// Returns [`BuildError::Config`] for a config that states what a config
/// may not: a setting a rule refuses, a `${VAR}` whose variable isn't set,
/// or a file the config names that can't be read, a script that's refused,
/// or a root that isn't a directory. The config is checked whole before
/// the provider is selected, so the answer doesn't depend on which
/// adapters exist.
///
/// Returns [`BuildError::KeyVariable`] when the provider needs a key and
/// the variable `model.api_key_env` names isn't set, or holds nothing.
///
/// Returns [`BuildError::Unsupported`] for a config that selects an
/// adapter this lablet doesn't have yet, or that states a setting only a
/// later one applies.
///
/// Returns [`BuildError::RootHolds`] when `tools.builtin.root` holds the
/// config, the system prompt's file, the task prompt's file or the
/// transcript.
///
/// Returns [`BuildError::UnknownTool`] when `tools.allow` or `tools.deny`
/// names a tool the run doesn't have.
pub async fn build<P>(config: Config, logger_provider: P) -> Result<Lablet, BuildError>
where
    P: LoggerProvider,
    P::Logger: Send + Sync + 'static,
{
    Lablet::builder(config, logger_provider).build().await
}

pub(crate) async fn build_in(builder: Builder, env: Env<'_>) -> Result<Lablet, BuildError> {
    let Builder {
        config,
        logger,
        tracer,
        propagator,
    } = builder;
    let prepared = prepared(&config, env)?;
    let propagator = propagator.unwrap_or_else(|| Arc::new(GlobalPropagator::new()));
    let tools = lablet_tools_wiring::tools(&config, &prepared, propagator).await?;
    let wiring = prepared.wiring()?;
    let tracer = tracer.unwrap_or_else(|| fallback::global_tracer(scope()));
    Ok(wire(&config, wiring, tools, tracer, logger))
}

/// The `Lablet` a checked config comes to, which offers `tools` and whose
/// runs open their spans through `tracer` and emit their records through
/// `logger`.
pub(crate) fn wire(
    config: &Config,
    wiring: Wiring,
    tools: Arc<ToolSet>,
    tracer: BoxedTracer,
    logger: Arc<dyn Logger>,
) -> Lablet {
    let Wiring {
        real,
        settings,
        script,
        system,
        capture_content,
        secrets,
    } = wiring;
    let (provider, played) = lablet_provider_wiring::fake(real.model.name.clone(), script);
    let clock: Arc<dyn Clock> = Arc::new(TokioClock);
    let tracer = SharedTracer(Arc::new(tracer));

    let cancellation = Arc::new(RunCancellation::default());
    let service = RunService::new(
        provider,
        Arc::clone(&tools),
        tracer.boxed(),
        Box::new(SharedLogger(Arc::clone(&logger))),
        Arc::clone(&clock),
        Arc::clone(&cancellation) as _,
        settings.stop,
        settings.retry,
        settings.request,
        settings.pricing,
        settings.calls,
        Arc::new(secrets.values),
    );
    let config_digest = config.digest();
    let transcript = real
        .run
        .transcript_path
        .clone()
        .zip(config.run.transcript_path.clone())
        .map(|(real, written)| {
            Arc::new(JsonTranscripts::new(real, written)) as Arc<dyn TranscriptWriter>
        });
    let runner = Runner::new(
        service,
        Arc::clone(&tools),
        tracer.boxed(),
        Box::new(SharedLogger(logger)),
        clock,
        cancellation,
        Shared {
            system,
            config_digest: config_digest.clone(),
            capture_content,
            transcript,
            agent_version: crate::VERSION.to_owned(),
        },
    );
    Lablet::new(runner, played, tools, config_digest)
}

/// The one tracer a `Lablet` was built with, which the loop and the runner
/// both open their spans through.
#[derive(Clone)]
struct SharedTracer(Arc<BoxedTracer>);

impl SharedTracer {
    fn boxed(&self) -> BoxedTracer {
        BoxedTracer::new(Box::new(self.clone()))
    }
}

impl Tracer for SharedTracer {
    type Span = BoxedSpan;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &Context) -> BoxedSpan {
        self.0.build_with_context(builder, parent_cx)
    }
}

/// The one logger a `Lablet` was built with, which the loop and the runner
/// both emit their records through.
struct SharedLogger(Arc<dyn Logger>);

impl Logger for SharedLogger {
    fn emit(&self, record: Record) {
        self.0.emit(record);
    }
}

#[cfg(test)]
mod tests;
