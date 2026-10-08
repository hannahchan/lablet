//! `build` and `check`: a config in, a `Lablet` or a checked config out.
//! The library's composition: it checks the config through
//! `lablet-prepare`, builds the tools and the provider through the wiring
//! kernels, and connects them to the loop and the runner itself.

use std::sync::Arc;

use lablet_config::{Config, Env, environment};
use lablet_otel_sdk::export::Telemetry;
use lablet_prepare::{BuildError, Checked, Wiring, otlp_refused, prepare};
use lablet_run::{Clock, RunCancellation, RunService, Runner, Shared, ToolSet, TranscriptWriter};
use lablet_transcript_json::JsonTranscripts;

use crate::clock::TokioClock;
use crate::lablet::Lablet;

/// Checks `config` whole, as [`build`] does, and stops before the provider
/// is selected, so the answer is the same for every provider: one this
/// lablet has no adapter for yet passes, and no call reaches a provider.
///
/// It substitutes `${VAR}`, checks every setting, that the provider's key
/// variable is set when the provider needs a key, that each file the config
/// names can be read, the fake provider's script among them, and the root
/// holds none of lablet's own, and builds the tool set, so the tools a run
/// is offered are listed.
///
/// # Errors
///
/// Returns what [`build`] returns, but for [`BuildError::Unsupported`] of
/// the provider, and for what only making the network exporter finds, which
/// a check passes: trust roots that can't be loaded, or other TLS to the
/// collector that can't be set up, and an HTTP client that can't be made.
pub async fn check(config: &Config) -> Result<Checked, BuildError> {
    check_in(config, &environment).await
}

/// [`check`], where `env` is lablet's environment.
pub(crate) async fn check_in(config: &Config, env: Env<'_>) -> Result<Checked, BuildError> {
    let prepared = prepare(config, env)?;
    let tools = lablet_tools_wiring::tools(config, &prepared).await?;
    Ok(Checked::new(
        config.resolved(),
        tools.specs().to_vec(),
        prepared.into_secrets(),
    ))
}

/// A `Lablet` that runs as `config` says.
///
/// The process's environment is read once, here: the `OTEL_*` variables
/// decide where the telemetry goes, and how, wherever the config's
/// `telemetry` section states nothing, while `OTEL_SDK_DISABLED=true` turns
/// it all off whatever the config states; `TRACEPARENT` and `TRACESTATE`
/// name the parent of every run the `Lablet` makes, and `BAGGAGE` is in
/// each run's context. An endpoint or a TLS file a variable names is
/// refused as one the config states is.
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
/// config, the system prompt's file, the task prompt's file, the
/// transcript or the telemetry file.
///
/// Returns [`BuildError::UnknownTool`] when `tools.allow` or `tools.deny`
/// names a tool the run doesn't have.
pub async fn build(config: Config) -> Result<Lablet, BuildError> {
    build_in(config, &environment).await
}

async fn build_in(config: Config, env: Env<'_>) -> Result<Lablet, BuildError> {
    let prepared = prepare(&config, env)?;
    let tools = lablet_tools_wiring::tools(&config, &prepared).await?;
    let (exports, wiring) = prepared.split()?;
    let variables = exports.endpoint_variables();
    let telemetry = exports
        .telemetry()
        .map_err(|error| otlp_refused(&config, variables, error))?;
    Ok(wire(&config, wiring, tools, telemetry))
}

/// The `Lablet` a checked config comes to, which offers `tools` and whose
/// runs go through `telemetry`.
pub(crate) fn wire(
    config: &Config,
    wiring: Wiring,
    tools: Arc<ToolSet>,
    telemetry: Telemetry,
) -> Lablet {
    let Wiring {
        real,
        settings,
        script,
        system,
        capture_content,
        inbound,
        secrets,
    } = wiring;
    let (provider, played) = lablet_provider_wiring::fake(real.model.name.clone(), script);
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
        telemetry.tracer(),
        telemetry.logger(),
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
    Lablet::new(runner, played, tools, telemetry, inbound, config_digest)
}

#[cfg(test)]
mod tests;
