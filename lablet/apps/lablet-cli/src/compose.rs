//! The command line's composition: a filled config and the process's
//! environment in, a composed run or a checked config out. It checks the
//! config through `lablet-prepare`, settles the telemetry's settings from
//! the config's `telemetry` section between the check's two halves, builds
//! the tools and the provider through the wiring kernels, the tools with the
//! propagators the config names, and connects them to the loop and the
//! runner itself, with the tracer and the logger of the SDK it builds.

use std::ffi::OsString;
use std::sync::Arc;

use lablet_clock_tokio::TokioClock;
use lablet_config::Config;
use lablet_model::{ConfigDigest, FinishedRun};
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
use crate::input::{self, Retired, UNREAD};
use crate::propagation::{self, Inbound};

pub use crate::exports::telemetry_on_stderr;

/// The version of lablet, the workspace's, which a run's record names as
/// `gen_ai.agent.version`.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The target of this module's warnings, the module they were written in
/// before the command line had a package of its own, so a line of the
/// diagnostic log, and a `RUST_LOG` directive that names it, are as they
/// were.
const TARGET: &str = "lablet::lablet";

/// What `check` reports beside what the kernels check: where a run's
/// telemetry goes, and which retired variables are set and read no more.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    checked: Checked,
    telemetry: Vec<String>,
    unread: Vec<Retired>,
}

impl Report {
    /// What the config passed its check with.
    #[must_use]
    pub fn checked(&self) -> &Checked {
        &self.checked
    }

    /// Where a run sends and writes its telemetry, a form each, and empty
    /// when nothing is named.
    #[must_use]
    pub fn telemetry(&self) -> &[String] {
        &self.telemetry
    }

    /// The variables of the retired roster that are set, and read only
    /// where the file writes `${NAME}`, which none of these does.
    #[must_use]
    pub fn unread(&self) -> &[Retired] {
        &self.unread
    }
}

/// Checks `config` whole, as [`build`] does, and calls no provider: it
/// builds the target of each adapter that exists, the Anthropic adapter's
/// with its HTTP client, and the network exporters the `telemetry` section
/// names, each of which loads the platform's trust roots and opens no
/// connection, so it refuses what building them refuses, and a provider
/// this lablet has no adapter for yet passes it. `environment` is the base
/// every process a run starts begins from, the command's own as the input
/// module read it.
///
/// # Errors
///
/// Returns what [`build`] returns, but for [`BuildError::Unsupported`] of
/// the provider.
pub async fn check(
    config: &Config,
    environment: Vec<(OsString, OsString)>,
) -> Result<Report, BuildError> {
    check_in(config, environment, &input::is_set).await
}

/// [`check`], where `set` says whether a variable of the process's
/// environment is set, for the names the report asks about: what a test
/// states in place of the process's environment.
///
/// # Errors
///
/// Returns what [`check`] returns.
pub async fn check_in(
    config: &Config,
    environment: Vec<(OsString, OsString)>,
    set: &(dyn Fn(&str) -> bool + Sync),
) -> Result<Report, BuildError> {
    let (prepared, exports) = settle(prepare(config, Some(environment))?)?;
    // A check starts no run, so no context is read; the propagator is for
    // the fields a command is given.
    let propagator = Arc::new(propagation::propagator(
        config.telemetry.propagators.as_ref(),
    ));
    let Built { tools, environment } = lablet_tools_wiring::tools(&prepared, propagator).await?;
    let telemetry = exports.report(config, set);
    let unread = UNREAD
        .iter()
        .copied()
        .filter(|retired| set(retired.name))
        // A name the file writes `${NAME}` for is read there, so it isn't
        // unread.
        .filter(|retired| {
            !config
                .filled()
                .iter()
                .any(|(_, fill)| fill.names().any(|name| name == retired.name))
        })
        .collect();
    Ok(Report {
        checked: Checked::new(
            config,
            tools.specs().to_vec(),
            prepared.into_secrets(),
            environment,
        ),
        telemetry,
        unread,
    })
}

/// The run `config` says, composed, with `environment` the base every
/// process it starts begins from: the context variables are read once,
/// here, through the input module, for the inbound context.
///
/// # Errors
///
/// Returns the [`BuildError`] of the first thing the config's check
/// refuses, or a provider this lablet has no adapter for yet.
pub async fn build(
    config: Config,
    environment: Vec<(OsString, OsString)>,
) -> Result<Composed, BuildError> {
    let (prepared, exports) = settle(prepare(&config, Some(environment))?)?;
    let inbound = propagation::inbound(config.telemetry.propagators.as_ref());
    let Built { tools, .. } =
        lablet_tools_wiring::tools(&prepared, Arc::clone(&inbound.propagator)).await?;
    let wiring = prepared.wiring()?;
    let telemetry = exports
        .telemetry()
        .map_err(|error| otlp_refused(&config, error))?;
    wire(wiring, tools, telemetry, inbound)
}

/// What a checked config comes to, which offers `tools`, whose run goes
/// through `telemetry`, whose root span is opened in `inbound` and whose
/// provider injects each attempt's context through `inbound`'s propagator.
///
/// # Errors
///
/// Returns [`BuildError::Unsupported`] for a provider there's no adapter
/// for yet, which [`lablet_prepare::Prepared::wiring`] refused already.
pub(crate) fn wire(
    wiring: Wiring,
    tools: Arc<ToolSet>,
    telemetry: Telemetry,
    inbound: Inbound,
) -> Result<Composed, BuildError> {
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
        Arc::clone(&inbound.propagator),
        capture_content,
    )?;
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
    Ok(Composed {
        runner,
        provider: played,
        tools,
        telemetry,
        inbound,
        config_digest,
    })
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
