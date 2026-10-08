//! The `Lablet`: one loop and its adapters, which runs a task at a time.

use std::sync::Arc;

use lablet_model::{ConfigDigest, FinishedRun, ToolSpec};
use lablet_otel_sdk::export::Telemetry;
use lablet_otel_sdk::propagation::Inbound;
use lablet_provider_wiring::Played;
use lablet_run::{Ran, Runner, ToolSet, TranscriptError};
use lablet_run_request::RunRequest;
use opentelemetry::trace::FutureExt as _;

/// One loop with its adapters, built from a config.
///
/// It runs many times, one run at a time, and every run has an id, a start
/// time and labels of its own. Where its telemetry goes, and the parent
/// every run's root span has, are fixed when it's built, by the config and
/// the `OTEL_*`, `TRACEPARENT`, `TRACESTATE` and `BAGGAGE` variables of the
/// environment it's built in.
pub struct Lablet {
    runner: Runner,
    provider: Played,
    tools: Arc<ToolSet>,
    telemetry: Telemetry,
    /// The context every run's root span is opened in.
    inbound: Inbound,
    config_digest: ConfigDigest,
    /// Whether a run began and its telemetry wasn't flushed: its future was
    /// dropped before it ended, and the spans it had open were queued,
    /// unfilled, as they were dropped.
    unflushed: bool,
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "the runner and the provider hold trait objects with nothing to print, and the system prompt is content"
)]
impl core::fmt::Debug for Lablet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lablet")
            .field("tools", &self.tools)
            .field("telemetry", &self.telemetry)
            .field("config_digest", &self.config_digest)
            .finish()
    }
}

impl Lablet {
    pub(crate) fn new(
        runner: Runner,
        provider: Played,
        tools: Arc<ToolSet>,
        telemetry: Telemetry,
        inbound: Inbound,
        config_digest: ConfigDigest,
    ) -> Self {
        Self {
            runner,
            provider,
            tools,
            telemetry,
            inbound,
            config_digest,
            unflushed: false,
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
    /// The run has the id the request names, or a fresh ULID. Its root span is
    /// a child of the parent `TRACEPARENT` named when this `Lablet` was built,
    /// and starts a trace of its own when that named none; never a child of a
    /// span the caller has open. The loop runs in the root span's context. Once
    /// the loop has returned, the root span ends with the run's measured
    /// duration and the run's wide event is filled; the run's transcript is
    /// written, when the config names a place for it, and then the wide event
    /// is emitted and the telemetry flushed once, so the transcript the wide
    /// event names is whole, or its failure logged, when it arrives. Both are
    /// whole when this returns, and the file holds the run's one wide event.
    ///
    /// A run is stopped through its [`CancelHandle`](crate::CancelHandle). Dropping the future
    /// abandons it with no outcome. Dropped before the loop returns, it leaves
    /// no transcript or wide event either. Dropped during the transcript write,
    /// it still leaves the transcript, since the write doesn't stop part-way,
    /// and no wide event, which is emitted only once the write has returned;
    /// dropped during the flush, it leaves both. The spans it had open are
    /// exported unfilled, with the providers' next export, and before this
    /// `Lablet`'s next run reads the clock, which waits up to one flush for
    /// them.
    ///
    /// Never fails: every way a run can go wrong is a stop reason of its
    /// outcome. A transcript that can't be written and telemetry that can't be
    /// exported are reported on the diagnostic log, and the run measured what
    /// it measured either way.
    pub async fn run(&mut self, request: RunRequest) -> FinishedRun {
        if self.unflushed {
            // What a dropped run left is queued, and goes before the next
            // run's first line, so the two runs' lines don't interleave. It
            // goes before the clock is read, since the loop's offsets count
            // from its own reading after this, and the run's times would
            // otherwise be early by the wait.
            if let Err(error) = self.telemetry.flush_leftovers().await {
                tracing::warn!(
                    failures = ?error.failures(),
                    "an abandoned run's telemetry wasn't exported whole"
                );
            }
        }
        self.telemetry.begin_run();
        self.unflushed = true;
        self.provider.begin_run();

        // In the inbound context, never the caller's current one, so the
        // span its caller is in has no part in the run's trace.
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
                tracing::warn!(%run_id, %error, "the run has no transcript file");
            }
            Some(TranscriptError::Unwritten(error)) => tracing::warn!(%run_id, "{error}"),
            None => {}
        }
        if let Err(error) = self.telemetry.flush_leftovers().await {
            tracing::warn!(
                %run_id,
                failures = ?error.failures(),
                "the run's telemetry wasn't exported whole"
            );
        }
        self.unflushed = false;
        finished
    }

    /// Flushes the telemetry and stops its exporters. A destination that
    /// doesn't answer is given about five seconds, and what couldn't be
    /// exported is reported on the diagnostic log: by the SDK, as an export
    /// that failed, or by a warning that the telemetry wasn't exported whole
    /// when an exporter didn't stop in time.
    pub async fn shutdown(self) {
        if let Err(error) = self.telemetry.shutdown().await {
            tracing::warn!(
                failures = ?error.failures(),
                "the telemetry wasn't exported whole at shutdown"
            );
        }
    }
}

#[cfg(test)]
mod tests;
