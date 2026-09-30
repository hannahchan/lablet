//! The `Lablet`: one loop and its adapters, which runs a task at a time.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use lablet_documents::TranscriptDocument;
use lablet_model::{
    BlankTask, ConfigDigest, FinishedRun, Prompts, RunContext, RunId, RunLabels, ToolSpec,
};
use lablet_provider_fake::FakeProvider;
use lablet_run::{RunService, ToolSet};
use lablet_telemetry_otel::OtelObserver;
use lablet_transcript_json::TranscriptFile;
use ulid::Ulid;

use crate::cancel::{CancelHandle, RunCancellation};

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
    /// no file at all.
    pub fn run_id(self, id: RunId) -> Result<Self, RunIdRefused> {
        let reason = match id.as_str() {
            "." => "it names the directory itself",
            ".." => "it names the directory above",
            named if named.contains('/') => "it holds a `/`",
            named if named.contains('\0') => "it holds a NUL",
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

/// What every run of one `Lablet` shares, beside the loop.
pub(crate) struct Fixed {
    pub(crate) system: String,
    pub(crate) config_digest: ConfigDigest,
    pub(crate) capture_content: bool,
    /// Where a transcript goes, as the config states it.
    pub(crate) transcript_path: Option<PathBuf>,
}

/// One loop with its adapters, built from a config.
///
/// It runs many times, one run at a time, and every run has an id, a start
/// time and labels of its own.
pub struct Lablet {
    service: RunService,
    provider: Played,
    tools: Arc<ToolSet>,
    telemetry: OtelObserver,
    cancellation: Arc<RunCancellation>,
    fixed: Fixed,
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "the loop and the provider hold trait objects with nothing to print, and the system prompt is content"
)]
impl core::fmt::Debug for Lablet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lablet")
            .field("tools", &self.tools)
            .field("telemetry", &self.telemetry)
            .field("config_digest", &self.fixed.config_digest)
            .finish()
    }
}

impl Lablet {
    pub(crate) fn new(
        service: RunService,
        provider: Played,
        tools: Arc<ToolSet>,
        telemetry: OtelObserver,
        cancellation: Arc<RunCancellation>,
        fixed: Fixed,
    ) -> Self {
        Self {
            service,
            provider,
            tools,
            telemetry,
            cancellation,
            fixed,
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
    /// The run has the id the request names, or a fresh ULID. Once the loop
    /// has returned, the run's transcript is written, when the config names
    /// a place for it, and the telemetry is flushed. So the telemetry file
    /// is whole when this returns, and the run's wide event is its last
    /// line.
    ///
    /// Never fails: every way a run can go wrong is a stop reason of its
    /// outcome. A transcript that can't be written and telemetry that can't
    /// be exported are reported on the diagnostic log, and the run measured
    /// what it measured either way.
    pub async fn run(&mut self, request: RunRequest) -> FinishedRun {
        let RunRequest {
            task,
            run_id,
            labels,
            cancellation,
        } = request;
        // One reading of the clock, so a fresh id holds the time its run
        // started.
        let started = SystemTime::now();
        let run_id = run_id.unwrap_or_else(|| RunId::ulid(Ulid::from_datetime(started).0));
        let transcript = self.transcript_file(&run_id);
        let context = RunContext {
            run_id,
            labels,
            started_unix_ms: unix_ms(started),
            config_digest: self.fixed.config_digest.clone(),
            agent_version: crate::VERSION.to_owned(),
            transcript_path: transcript.as_ref().map(|file| file.path().to_owned()),
            skills_count: 0,
            mcp: None,
            capture_content: self.fixed.capture_content,
        };

        self.provider.begin_run();
        let task_prompt = task.task().to_owned();
        let prompts = task.with_system(self.fixed.system.as_str());
        // Every run sets its own, so a handle of an earlier run never
        // reaches this one.
        self.cancellation.set(cancellation);
        let finished = self.service.run(context.clone(), prompts).await;

        if let Some(file) = transcript {
            self.write_transcript(file, context, task_prompt, &finished)
                .await;
        }
        if let Err(error) = self.telemetry.flush().await {
            tracing::warn!(
                run_id = %finished.summary.outcome.run_id,
                %error,
                "the run's telemetry wasn't exported whole"
            );
        }
        finished
    }

    /// Flushes the telemetry and stops its exporters. A destination that
    /// doesn't answer is given seconds, and what couldn't be exported is
    /// reported on the diagnostic log.
    pub async fn shutdown(self) {
        if let Err(error) = self.telemetry.shutdown().await {
            tracing::warn!(%error, "the telemetry didn't shut down clean");
        }
    }

    /// The file of the run's transcript, when the config names a place for
    /// one and the run id can be part of it.
    fn transcript_file(&self, run_id: &RunId) -> Option<TranscriptFile> {
        let configured = self.fixed.transcript_path.as_deref()?;
        TranscriptFile::for_run(configured, run_id)
            .inspect_err(|error| {
                tracing::warn!(%run_id, %error, "the run has no transcript file");
            })
            .ok()
    }

    async fn write_transcript(
        &self,
        file: TranscriptFile,
        context: RunContext,
        task_prompt: String,
        run: &FinishedRun,
    ) {
        let run_id = context.run_id.clone();
        let document = TranscriptDocument::new(
            context,
            run.summary.model.clone(),
            self.tools.specs().to_vec(),
            task_prompt,
            run.transcript.clone(),
        );
        // A file is written where waiting is allowed, as the telemetry's
        // files are.
        let written = tokio::task::spawn_blocking(move || file.write(&document))
            .await
            .map_err(|error| error.to_string())
            .and_then(|written| written.map_err(|error| error.to_string()));
        if let Err(error) = written {
            tracing::warn!(%run_id, %error, "the run's transcript wasn't written");
        }
    }
}

/// `time` in milliseconds since the Unix epoch; 0 for a time before it,
/// which is what a machine whose clock was never set gives.
fn unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    })
}
