//! The `Lablet`: one loop and its adapters, which runs a task at a time.

use std::sync::Arc;

use lablet_model::{ConfigDigest, FinishedRun, ToolSpec};
use lablet_provider_wiring::Played;
use lablet_run::{Ran, Runner, ToolSet, TranscriptError};
use lablet_run_request::RunRequest;

/// One loop with its adapters, built from a config.
///
/// It runs many times, one run at a time, and every run has an id, a start
/// time and labels of its own. Its spans go to the tracer provider it was
/// built with, or the global one as it was when it was built, and its
/// records to the logger provider it was built with.
pub struct Lablet {
    runner: Runner,
    provider: Played,
    tools: Arc<ToolSet>,
    config_digest: ConfigDigest,
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "the runner and the provider hold trait objects with nothing to print, and the system prompt is content"
)]
impl core::fmt::Debug for Lablet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lablet")
            .field("tools", &self.tools)
            .field("config_digest", &self.config_digest)
            .finish()
    }
}

impl Lablet {
    pub(crate) fn new(
        runner: Runner,
        provider: Played,
        tools: Arc<ToolSet>,
        config_digest: ConfigDigest,
    ) -> Self {
        Self {
            runner,
            provider,
            tools,
            config_digest,
        }
    }

    /// The tools every run of this `Lablet` is offered, in the order
    /// they're offered, after `tools.allow` and `tools.deny`.
    #[must_use]
    pub fn tools(&self) -> &[ToolSpec] {
        self.tools.specs()
    }

    /// Runs one task to its outcome.
    ///
    /// The run has the id the request names, or a fresh ULID. Its root span
    /// is a child of the context that's current where this is awaited, so a
    /// run under a span the host has open is in the host's trace, and a run
    /// under none starts a trace of its own. Its spans go to the tracer
    /// provider this `Lablet` was built with, or OpenTelemetry's global one
    /// as it was when it was built, and its records to the logger provider
    /// it was built with. Once the loop has returned, the root span ends
    /// with the run's measured duration, the run's transcript is written,
    /// when the config names a place for it, and then the run's wide event
    /// is emitted, so the transcript it names is whole, or its failure
    /// logged, when it arrives. Nothing is flushed: the host's SDK decides
    /// when what the run emitted is exported.
    ///
    /// Library mode leaves alone what configured an OpenTelemetry SDK, and
    /// says nothing of it: the config's `telemetry.file`, `telemetry.otlp`
    /// and `telemetry.resource`, the `OTEL_*` variables the SDK reads,
    /// `OTEL_SDK_DISABLED`, and `TRACEPARENT`, `TRACESTATE` and `BAGGAGE`.
    /// Content capture still applies, and the secrets the config's
    /// telemetry section names are still withheld from commands and cut
    /// from what they print.
    ///
    /// A run is stopped through its [`CancelHandle`](crate::CancelHandle). Dropping the future
    /// abandons it with no outcome. Dropped before the loop returns, it leaves
    /// no transcript or wide event either. Dropped during the transcript write,
    /// it still leaves the transcript, since the write doesn't stop part-way,
    /// and no wide event, which is emitted only once the write has returned.
    ///
    /// Never fails: every way a run can go wrong is a stop reason of its
    /// outcome. A transcript that can't be written is reported on the
    /// diagnostic log, and the run measured what it measured either way.
    pub async fn run(&mut self, request: RunRequest) -> FinishedRun {
        self.provider.begin_run();

        let Ran {
            finished,
            transcript,
        } = self.runner.run(request.into_start()).await;
        let run_id = &finished.summary.outcome.run_id;
        match transcript {
            Some(TranscriptError::NoPlace(error)) => {
                tracing::warn!(%run_id, %error, "the run has no transcript file");
            }
            Some(TranscriptError::Unwritten(error)) => tracing::warn!(%run_id, "{error}"),
            None => {}
        }
        finished
    }

    /// Ends this `Lablet`. It does no telemetry work, since the host's SDK
    /// owns the providers, and is where the servers a `Lablet` starts are
    /// stopped once it starts any.
    #[expect(
        clippy::unused_async,
        reason = "the MCP servers phase 8 starts are stopped here, which is awaited"
    )]
    pub async fn shutdown(self) {}
}

#[cfg(test)]
mod tests;
