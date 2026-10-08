//! `build` and `check`: a config in, a `Lablet` or a checked config out.
//! The library's composition: it checks the config through
//! `lablet-prepare`, builds the tools and the provider through the wiring
//! kernels with the propagator the host handed in, and connects them, with
//! the tracer and the logger the host handed in, to the loop and the runner
//! itself.

use std::sync::Arc;

use lablet_clock_tokio::TokioClock;
use lablet_config::{Config, Env, TelemetryFile, environment};
use lablet_prepare::{BuildError, Checked, Prepared, Telemetry, Wiring, prepare};
use lablet_run::telemetry::{Logger, Record};
use lablet_run::{Clock, RunCancellation, RunService, Runner, Shared, ToolSet, TranscriptWriter};
use lablet_transcript_json::JsonTranscripts;
use opentelemetry::Context;
use opentelemetry::global::{BoxedSpan, BoxedTracer};
use opentelemetry::trace::noop::NoopTextMapPropagator;
use opentelemetry::trace::{SpanBuilder, Tracer};

use crate::lablet::Lablet;
use crate::otel::Otel;

/// How a [`Lablet`] is built: from a config and the host's [`Otel`]. Made
/// by [`Lablet::builder`].
///
/// It's a builder so that each further input a host can hand in is a
/// method of its own, which leaves a host that hands in none of them
/// unchanged.
pub struct Builder {
    config: Config,
    otel: Otel,
}

impl core::fmt::Debug for Builder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Builder")
            .field("config", &self.config.digest())
            .field("otel", &self.otel)
            .finish()
    }
}

impl Lablet {
    /// A builder of a `Lablet` that runs as `config` says on the host's
    /// OpenTelemetry, `otel`: its spans go to `otel`'s tracer provider, its
    /// records to its logger provider, and a command's context is injected
    /// through its propagator.
    ///
    /// A `Lablet` can't be built without the host's OpenTelemetry:
    ///
    /// ```compile_fail,E0061
    /// # async fn host(config: lablet::Config) -> Result<(), lablet::BuildError> {
    /// let lablet = lablet::Lablet::builder(config).build().await?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn builder(config: Config, otel: Otel) -> Builder {
        Builder { config, otel }
    }
}

impl Builder {
    /// The `Lablet`.
    ///
    /// # Errors
    ///
    /// What [`build`] returns.
    pub async fn build(self) -> Result<Lablet, BuildError> {
        build_in(self, &environment).await
    }
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
    // A check starts no command, so there's no context to inject.
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

/// A `Lablet` that runs as `config` says on the host's OpenTelemetry,
/// `otel`: [`Lablet::builder`] with nothing else handed in.
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
///
/// Returns [`BuildError::Tools`] when the tools the run's executors serve
/// can't be settled into one set.
pub async fn build(config: Config, otel: Otel) -> Result<Lablet, BuildError> {
    Lablet::builder(config, otel).build().await
}

pub(crate) async fn build_in(builder: Builder, env: Env<'_>) -> Result<Lablet, BuildError> {
    let Builder { config, otel } = builder;
    let Otel {
        tracer,
        logger,
        propagator,
    } = otel;
    let prepared = prepared(&config, env)?;
    let tools = lablet_tools_wiring::tools(&config, &prepared, propagator).await?;
    let wiring = prepared.wiring()?;
    Ok(wire(&config, wiring, tools, tracer, logger))
}

/// The `Lablet` a checked config comes to, which offers `tools` and whose
/// runs open their spans through `tracer` and emit their records through
/// `logger`.
pub(crate) fn wire(
    config: &Config,
    wiring: Wiring,
    tools: Arc<ToolSet>,
    tracer: Arc<BoxedTracer>,
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
    let tracer = SharedTracer(tracer);

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
/// both open their spans through, and every `Lablet` of one [`Otel`]
/// shares.
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
