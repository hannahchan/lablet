//! The `Lablet`: one loop and its adapters, which runs a task at a time.

use std::sync::Arc;

use lablet_model::{BlankTask, ConfigDigest, FinishedRun, Prompts, RunId, RunLabels, ToolSpec};
use lablet_provider_fake::FakeProvider;
use lablet_run::{Cancellation, Ran, RunStart, Runner, ToolSet, TranscriptError};
use opentelemetry::trace::FutureExt as _;

use crate::cancel::CancelHandle;
use crate::export::Telemetry;
use crate::propagation::Inbound;

/// What a run is asked to do, what it's known by, and what may stop it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    /// The task, under no system prompt: the `Lablet` that runs the
    /// request has the system prompt, and the task is checked here.
    task: Prompts,
    run_id: Option<RunId>,
    labels: RunLabels,
    cancellation: Option<CancelHandle>,
}

/// The most bytes a run id may hold. A file's name is at most 255 bytes
/// where lablet runs, and a run's files put text of their own beside its id
/// in one name: `lablet-` and `.otlp.jsonl` for its telemetry, and whatever
/// a transcript's path writes around `{run_id}`, which this leaves 127
/// bytes. It holds a ULID, a UUID, or a run's labels joined with a digest.
/// The transcript writer and the telemetry file hold a run id to the same
/// number.
const RUN_ID_MAX_BYTES: usize = 128;

/// Why a run id was refused for a run: a run's files are named with its
/// id, and one that isn't one component of a path would name another
/// place than the one the config names, or none.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the run id {run_id:?} is refused: {reason}, and a run's files are named with its id")]
pub struct RunIdRefused {
    /// The refused id.
    pub run_id: String,
    /// Which rule of a path component it breaks.
    pub reason: &'static str,
}

impl RunRequest {
    /// A request to run the task `prompt`, under a fresh run id and no
    /// labels.
    ///
    /// # Errors
    ///
    /// Returns [`BlankTask`] when `prompt` is empty or only whitespace, so
    /// a run has nothing left to refuse.
    pub fn new(prompt: impl Into<String>) -> Result<Self, BlankTask> {
        Ok(Self {
            task: Prompts::new(String::new(), prompt)?,
            run_id: None,
            labels: RunLabels::default(),
            cancellation: None,
        })
    }

    /// The same request under the run id `id`, for a caller that has to
    /// know the id before the run starts.
    ///
    /// # Errors
    ///
    /// Returns [`RunIdRefused`] when `id` isn't one component of a path:
    /// when it's `.` or `..`, or holds a `/` or a NUL. The run's telemetry
    /// file and its transcript may be named with its id, and such an id
    /// would put them in another directory than the config names, or name
    /// no file at all. So does an id longer than 128 bytes, which leaves a
    /// file's name no room for what's written beside it.
    pub fn run_id(self, id: RunId) -> Result<Self, RunIdRefused> {
        let reason = match id.as_str() {
            "." => "it names the directory itself",
            ".." => "it names the directory above",
            named if named.contains('/') => "it holds a `/`",
            named if named.contains('\0') => "it holds a NUL",
            named if named.len() > RUN_ID_MAX_BYTES => "it's longer than 128 bytes",
            _ => {
                return Ok(Self {
                    run_id: Some(id),
                    ..self
                });
            }
        };
        Err(RunIdRefused {
            run_id: id.into(),
            reason,
        })
    }

    /// The same request, stopped when `handle` is fired: the run ends with
    /// `cancelled`, and its outcome, its transcript and its wide event are
    /// written.
    #[must_use]
    pub fn cancellation(self, handle: CancelHandle) -> Self {
        Self {
            cancellation: Some(handle),
            ..self
        }
    }

    /// The same request with the labels `labels`, which name the run's
    /// task, its experiment and its trial.
    #[must_use]
    pub fn labels(self, labels: RunLabels) -> Self {
        Self { labels, ..self }
    }

    /// The run's start, as the runner takes it.
    fn into_start(self) -> RunStart {
        RunStart {
            task: self.task,
            run_id: self.run_id,
            labels: self.labels,
            cancellation: self
                .cancellation
                .map(|handle| Arc::new(handle) as Arc<dyn Cancellation>),
        }
    }
}

/// The provider, as whoever starts a run knows it.
pub(crate) enum Played {
    /// A script, which every run hears from its first entry.
    Script(Arc<FakeProvider>),
}

impl Played {
    /// Readies the provider for a run that's about to start. The port
    /// names no run, so a provider can't tell where one begins.
    fn begin_run(&self) {
        match self {
            Self::Script(provider) => provider.rewind(),
        }
    }
}

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
    /// A run is stopped through its [`CancelHandle`]. Dropping the future
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
