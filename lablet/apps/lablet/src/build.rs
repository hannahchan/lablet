//! `build` and `check`: a config in, a `Lablet` or a checked config out.
//! The library's composition: it checks the config through
//! `lablet-prepare`, builds the tools and the provider through the wiring
//! kernels with the propagator the host handed in, and connects them, with
//! the tracer and the logger the host handed in, to the loop and the runner
//! itself. It reads no environment variable: the config was filled by its
//! host, and the environment a run's processes start from is what the host
//! handed in.

use std::ffi::OsString;
use std::sync::Arc;

use lablet_clock_tokio::TokioClock;
use lablet_config::Config;
use lablet_prepare::{Base, BuildError, Checked, Prepared, Telemetry, Wiring, prepare};
use lablet_run::telemetry::{Logger, Record};
use lablet_run::{Clock, RunCancellation, RunService, Runner, Shared, ToolSet, TranscriptWriter};
use lablet_tools_wiring::Built;
use lablet_transcript_json::JsonTranscripts;
use opentelemetry::Context;
use opentelemetry::global::{BoxedSpan, BoxedTracer};
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::{SpanBuilder, Tracer};

use crate::lablet::Lablet;
use crate::otel::Otel;

/// How a [`Lablet`] is built: from a config and the host's [`Otel`], and
/// the environment its processes start from. Made by [`Lablet::builder`].
///
/// It's a builder so that each further input a host can hand in is a
/// method of its own, which leaves a host that hands in none of them
/// unchanged.
pub struct Builder {
    config: Config,
    otel: Otel,
    environment: Base,
}

impl core::fmt::Debug for Builder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let environment: Option<Vec<&OsString>> = self
            .environment
            .as_ref()
            .map(|environment| environment.iter().map(|(name, _)| name).collect());
        f.debug_struct("Builder")
            .field("config", &self.config.digest())
            .field("otel", &self.otel)
            .field("environment", &environment)
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
        Builder {
            config,
            otel,
            environment: None,
        }
    }
}

impl Builder {
    /// The environment every process a run starts begins from, which a
    /// config that enables `bash` needs: `bash` is found on its `PATH`, or,
    /// when it holds none, on the host process's own, and the command then
    /// gets no `PATH`; and
    /// a command inherits it, less every variable that filled one of
    /// lablet's credentials and every variable holding a credential's
    /// value, with `tools.builtin.env` and then `tools.builtin.credentials`
    /// on top. A host that hands in `std::env::vars_os()` passes its own
    /// environment on, as the command line does, and lablet drops from it
    /// only what holds a credential the config names: a token the config
    /// never named reaches `bash`, and the fix is the host's, to hand in
    /// less. The servers a later phase starts begin from it too.
    #[must_use]
    pub fn with_process_environment<K: Into<OsString>, V: Into<OsString>>(
        mut self,
        vars: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.environment = Some(
            vars.into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        );
        self
    }

    /// Checks the config whole, as [`Builder::build`] does, and calls no
    /// provider: it builds the target of each adapter that exists, the
    /// Anthropic adapter's with its HTTP client, which loads the platform's
    /// trust roots and opens no connection, so it refuses what building
    /// the adapter refuses, and a provider this lablet has no adapter for
    /// yet passes it.
    ///
    /// It checks every setting, that `model.api_key` is set when the
    /// provider needs a key and that every credential is a `${VAR}` or a
    /// `Secret`, that each file the config names can be read, the fake
    /// provider's script among them, and the root holds none of lablet's
    /// own, and builds the tool set, so the tools a run is offered are
    /// listed and what `bash` inherits of the environment handed in is
    /// reported. It says nothing of the `telemetry` section: the host's
    /// [`Otel`] decides where a run's telemetry goes.
    ///
    /// # Errors
    ///
    /// Returns what [`Builder::build`] returns, but for
    /// [`BuildError::Unsupported`] of the provider.
    pub async fn check(&self) -> Result<Checked, BuildError> {
        let prepared = prepared(&self.config, self.environment.clone())?;
        // A check starts no command, so nothing is injected; the host's
        // propagator names the context variables a command would get.
        let propagator = Arc::clone(&self.otel.propagator);
        let Built { tools, environment } =
            lablet_tools_wiring::tools(&prepared, propagator).await?;
        Ok(Checked::new(
            &self.config,
            tools.specs().to_vec(),
            prepared.into_secrets(),
            environment,
        ))
    }

    /// The `Lablet`.
    ///
    /// # Errors
    ///
    /// What [`build`] returns.
    pub async fn build(self) -> Result<Lablet, BuildError> {
        let Self {
            config,
            otel,
            environment,
        } = self;
        let Otel {
            tracer,
            logger,
            propagator,
        } = otel;
        let prepared = prepared(&config, environment)?;
        let Built { tools, .. } =
            lablet_tools_wiring::tools(&prepared, Arc::clone(&propagator)).await?;
        let wiring = prepared.wiring()?;
        wire(wiring, tools, tracer, logger, propagator)
    }
}

/// The check of `config` with `environment` handed in, gone on with what a
/// library run reads of the telemetry: nothing, since the host's SDK
/// decides, and no file of lablet's own.
fn prepared(config: &Config, environment: Base) -> Result<Prepared, BuildError> {
    Ok(prepare(config, environment)?.finish(Telemetry::default()))
}

/// A `Lablet` that runs as `config` says on the host's OpenTelemetry,
/// `otel`: [`Lablet::builder`] with nothing else handed in, so a config
/// that enables `bash` is refused, since no process environment was handed
/// in.
///
/// Library mode configures no OpenTelemetry SDK and reads no environment
/// variable: the host's SDK samples, exports and flushes what lablet emits,
/// and the config's `telemetry.file`, `telemetry.otlp`,
/// `telemetry.propagators` and `telemetry.resource` do nothing, and none of
/// them is warned about.
/// `telemetry.capture_content` decides whether content is captured, and
/// lablet's credentials are still dropped from what a command inherits and
/// cut from what it prints: `model.api_key`, the values of
/// `telemetry.otlp.headers`, the user information of
/// `telemetry.otlp.endpoint` and each entry of `tools.builtin.credentials`,
/// which `bash` is given under its name.
///
/// # Errors
///
/// Returns [`BuildError::Config`] for a config that states what a config
/// may not: a setting a rule refuses, a `${VAR}` left in it that no fill
/// filled, a credential written out rather than as a `${VAR}` or a
/// `Secret`, a key the provider needs that isn't set, a relative path in a
/// config [`Config::relative_to`] wasn't called on, or a file the config
/// names that can't be read, a script that's refused, or a root that isn't
/// a directory, for `bash` enabled with no process environment handed in,
/// and for what building the provider's target refuses, a key a header
/// can't carry, a base URL with a query or an HTTP client that can't be
/// made. The config is checked whole before the provider's target is
/// built, so every other refusal doesn't depend on which adapters exist.
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

/// The `Lablet` a checked config comes to, which offers `tools`, whose
/// runs open their spans through `tracer` and emit their records through
/// `logger`, and whose provider injects each attempt's context through
/// `propagator`, the host's.
///
/// # Errors
///
/// Returns [`BuildError::Unsupported`] for a provider there's no adapter
/// for yet, which [`Prepared::wiring`] refused already.
pub(crate) fn wire(
    wiring: Wiring,
    tools: Arc<ToolSet>,
    tracer: Arc<BoxedTracer>,
    logger: Arc<dyn Logger>,
    propagator: Arc<dyn TextMapPropagator + Send + Sync>,
) -> Result<Lablet, BuildError> {
    let Wiring {
        config,
        settings,
        provider,
        system,
        capture_content,
        secrets,
    } = wiring;
    let (provider, played) = lablet_provider_wiring::build(
        provider,
        config.model.name.clone(),
        propagator,
        capture_content,
    )?;
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
    // The transcript's document names its path as the file wrote it, so
    // nothing a variable gave reaches what a run shows.
    let transcript = config.run.transcript_path.clone().map(|real| {
        let written = config
            .written_text("run.transcript_path")
            .map_or_else(|| real.clone(), Into::into);
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
    Ok(Lablet::new(runner, played, tools, config_digest))
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
