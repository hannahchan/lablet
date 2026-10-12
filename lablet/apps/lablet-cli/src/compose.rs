//! The command line's composition: a filled config and the process's
//! environment in, a composed run or a checked config out. It checks the
//! config through `lablet-prepare`, settles the telemetry's settings from
//! the config and the SDK's environment between the check's two halves,
//! builds the tools and the provider through the wiring kernels, the tools
//! with the propagators the environment names, and connects them to the
//! loop and the runner itself, with the tracer and the logger of the SDK it
//! builds.

use std::ffi::OsString;
use std::sync::Arc;

use lablet_clock_tokio::TokioClock;
use lablet_config::Config;
use lablet_model::{ConfigDigest, FinishedRun};
use lablet_otel_env::Env;
use lablet_prepare::{BuildError, Checked, Wiring, prepare};
use lablet_provider_wiring::Played;
use lablet_run::{
    Clock, Ran, RunCancellation, RunService, Runner, Shared, ToolSet, TranscriptError,
    TranscriptWriter,
};
use lablet_run_request::RunRequest;
use lablet_tools_wiring::Built;
use lablet_transcript_json::JsonTranscripts;
use opentelemetry::trace::FutureExt as _;

use crate::export::Telemetry;
use crate::exports::{otlp_refused, settle};
use crate::input;
use crate::propagation::Inbound;

pub use crate::exports::telemetry_on_stderr;

/// The version of lablet, the workspace's, which a run's record names as
/// `gen_ai.agent.version`.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The target of this module's warnings, the module they were written in
/// before the command line had a package of its own, so a line of the
/// diagnostic log, and a `RUST_LOG` directive that names it, are as they
/// were.
const TARGET: &str = "lablet::lablet";

/// Checks `config` whole, as [`build`] does, and stops before the provider
/// is selected, so the answer is the same for every provider: one this
/// lablet has no adapter for yet passes, and no call reaches a provider.
/// `environment` is the base every process a run starts begins from, the
/// command's own as the input module read it.
///
/// # Errors
///
/// Returns what [`build`] returns, but for [`BuildError::Unsupported`] of
/// the provider, and for what only making the network exporter finds, which
/// a check passes.
pub async fn check(
    config: &Config,
    environment: Vec<(OsString, OsString)>,
) -> Result<Checked, BuildError> {
    check_in(config, &input::variable, environment).await
}

/// [`check`], where `env` is what the seam reads of the environment.
pub(crate) async fn check_in(
    config: &Config,
    env: Env<'_>,
    environment: Vec<(OsString, OsString)>,
) -> Result<Checked, BuildError> {
    let (prepared, _, inbound) = settle(prepare(config, Some(environment))?, env)?;
    let Built { tools, environment } =
        lablet_tools_wiring::tools(&prepared, inbound.propagator).await?;
    Ok(Checked::new(
        config,
        tools.specs().to_vec(),
        prepared.into_secrets(),
        environment,
    ))
}

/// The run `config` says, composed, with `environment` the base every
/// process it starts begins from: the seam reads the process's environment
/// once, here, for the SDK's settings and the inbound context.
///
/// # Errors
///
/// Returns the [`BuildError`] of the first thing the config's check
/// refuses, a provider this lablet has no adapter for yet, or what only
/// making the network exporter finds.
pub async fn build(
    config: Config,
    environment: Vec<(OsString, OsString)>,
) -> Result<Composed, BuildError> {
    build_in(config, &input::variable, environment).await
}

pub(crate) async fn build_in(
    config: Config,
    env: Env<'_>,
    environment: Vec<(OsString, OsString)>,
) -> Result<Composed, BuildError> {
    let (prepared, exports, inbound) = settle(prepare(&config, Some(environment))?, env)?;
    let Built { tools, .. } =
        lablet_tools_wiring::tools(&prepared, Arc::clone(&inbound.propagator)).await?;
    let wiring = prepared.wiring()?;
    let variables = exports.endpoint_variables();
    let telemetry = exports
        .telemetry()
        .map_err(|error| otlp_refused(&config, variables, error))?;
    Ok(wire(wiring, tools, telemetry, inbound))
}

/// What a checked config comes to, which offers `tools`, whose run goes
/// through `telemetry`, and whose root span is opened in `inbound`.
pub(crate) fn wire(
    wiring: Wiring,
    tools: Arc<ToolSet>,
    telemetry: Telemetry,
    inbound: Inbound,
) -> Composed {
    let Wiring {
        config,
        settings,
        script,
        system,
        capture_content,
        secrets,
    } = wiring;
    let (provider, played) = lablet_provider_wiring::fake(config.model.name.clone(), script);
    let clock: Arc<dyn Clock> = Arc::new(TokioClock);

    let cancellation = Arc::new(RunCancellation::default());
    let service = RunService::new(
        provider,
        Arc::clone(&tools),
        telemetry.tracer(),
        telemetry.logger(),
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
        telemetry.tracer(),
        telemetry.logger(),
        clock,
        cancellation,
        Shared {
            system,
            config_digest: config_digest.clone(),
            capture_content,
            transcript,
            agent_version: VERSION.to_owned(),
        },
    );
    Composed {
        runner,
        provider: played,
        tools,
        telemetry,
        inbound,
        config_digest,
    }
}

/// The loop and its adapters as the command line composes them, for the
/// one run it makes.
pub struct Composed {
    runner: Runner,
    provider: Played,
    tools: Arc<ToolSet>,
    telemetry: Telemetry,
    /// The context the environment named, which is current around the run.
    inbound: Inbound,
    config_digest: ConfigDigest,
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "the runner and the provider hold trait objects with nothing to print, and the system prompt is content"
)]
impl core::fmt::Debug for Composed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Composed")
            .field("tools", &self.tools)
            .field("telemetry", &self.telemetry)
            .field("config_digest", &self.config_digest)
            .finish()
    }
}

impl Composed {
    /// Runs one task to its outcome, with the inbound context the
    /// environment named made current around it, so the run's root span is
    /// its child, and flushes the telemetry once it has returned. A
    /// transcript that can't be written and telemetry that can't be
    /// exported are reported on the diagnostic log.
    pub async fn run(&mut self, request: RunRequest) -> FinishedRun {
        self.telemetry.begin_run();
        self.provider.begin_run();

        let Ran {
            finished,
            transcript,
        } = self
            .runner
            .run(request.into_start())
            .with_context(self.inbound.parent.clone())
            .await;
        let run_id = &finished.summary.outcome.run_id;
        match transcript {
            Some(TranscriptError::NoPlace(error)) => {
                tracing::warn!(target: TARGET, %run_id, %error, "the run has no transcript file");
            }
            Some(TranscriptError::Unwritten(error)) => {
                tracing::warn!(target: TARGET, %run_id, "{error}");
            }
            None => {}
        }
        if let Err(error) = self.telemetry.flush_providers().await {
            tracing::warn!(
                target: TARGET,
                %run_id,
                failures = ?error.failures(),
                "the run's telemetry wasn't exported whole"
            );
        }
        finished
    }

    /// Flushes the telemetry and stops its exporters. A destination that
    /// doesn't answer is given about five seconds, and what couldn't be
    /// exported is reported on the diagnostic log.
    pub async fn shutdown(self) {
        if let Err(error) = self.telemetry.shutdown().await {
            tracing::warn!(
                target: TARGET,
                failures = ?error.failures(),
                "the telemetry wasn't exported whole at shutdown"
            );
        }
    }
}

#[cfg(test)]
mod tests;
