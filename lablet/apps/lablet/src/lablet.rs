//! The `Lablet`: one loop and its adapters, which runs a task at a time.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lablet_documents::TranscriptDocument;
use lablet_model::{
    BlankTask, ConfigDigest, FinishedRun, Prompts, RunContext, RunId, RunLabels, ToolSpec,
};
use lablet_provider_fake::FakeProvider;
use lablet_run::telemetry::{count_of, span_attributes};
use lablet_run::{RunService, ToolSet};
use lablet_transcript_json::{TranscriptFile, TranscriptWriteError};
use opentelemetry::Context;
use opentelemetry::global::BoxedTracer;
use opentelemetry::trace::{FutureExt as _, TraceContextExt as _, Tracer as _};
use ulid::Ulid;

use crate::cancel::{CancelHandle, RunCancellation};
use crate::export::Telemetry;
use crate::telemetry::generated::{LabletInvokeAgent, LabletRun};
use crate::{root_span, wide};

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
    /// Where a transcript goes, when the config names a place for one.
    pub(crate) transcript: Option<TranscriptPath>,
}

/// Where a run's transcript goes, as it's written to and as the config
/// writes it.
pub(crate) struct TranscriptPath {
    /// The path, with `${VAR}` substituted, that's written to.
    pub(crate) real: PathBuf,
    /// The path before substitution, which is the one a message shows, so
    /// that nothing a variable holds reaches the diagnostic log.
    pub(crate) written: PathBuf,
}

/// One loop with its adapters, built from a config.
///
/// It runs many times, one run at a time, and every run has an id, a start
/// time and labels of its own.
pub struct Lablet {
    service: RunService,
    provider: Played,
    tools: Arc<ToolSet>,
    /// The tracer the root span is opened with, of the same scope the
    /// loop's is.
    tracer: BoxedTracer,
    telemetry: Telemetry,
    cancellation: Arc<RunCancellation>,
    fixed: Fixed,
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "the loop, the tracer and the provider hold trait objects with nothing to print, and the system prompt is content"
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
        tracer: BoxedTracer,
        telemetry: Telemetry,
        cancellation: Arc<RunCancellation>,
        fixed: Fixed,
    ) -> Self {
        Self {
            service,
            provider,
            tools,
            tracer,
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
    /// The run has the id the request names, or a fresh ULID. It's a trace
    /// of its own, whatever span the caller has open: the root span is
    /// opened from the empty context, and the loop runs in it. Once the loop
    /// has returned, the root span ends with the run's measured duration,
    /// the telemetry is flushed, which makes the run's wide event the last
    /// line of its file, and then the run's transcript is written, when the
    /// config names a place for it. So the telemetry file is whole when
    /// this returns.
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
            transcript_path: transcript.as_ref().map(|(file, _)| file.path().to_owned()),
            skills_count: 0,
            mcp: None,
            capture_content: self.fixed.capture_content,
        };

        self.telemetry.begin_run(&context.run_id);
        self.provider.begin_run();
        let task_prompt = task.task().to_owned();
        let prompts = task.with_system(self.fixed.system.as_str());
        // Every run sets its own, so a handle of an earlier run never
        // reaches this one.
        self.cancellation.set(cancellation);

        // From the empty context, never the caller's current one, so a run
        // is a trace of its own whatever span its caller is in. The span
        // goes into the context by value, as a local span: a child of a
        // remote span context would be marked as having a remote parent.
        let root = self
            .tracer
            .span_builder(root_span::name())
            .with_kind(LabletInvokeAgent::KIND)
            .with_start_time(at(context.started_unix_ms, 0))
            .start_with_context(&self.tracer, &Context::new());
        let within = Context::new().with_span(root);
        let finished = self
            .service
            .run(context.clone(), prompts)
            .with_context(within.clone())
            .await;

        let summary = &finished.summary;
        let end = at(context.started_unix_ms, summary.outcome.duration_ms);
        let root = within.span();
        root.set_attributes(span_attributes(
            root_span::invoke_agent(&context, summary).attributes(),
        ));
        root.set_status(root_span::status(&summary.outcome));
        root.end_with_timestamp(end);
        let root_context = root.span_context().clone();

        // Everything but each destination's own count of what it lost, which
        // the destination fills once it has flushed.
        let filled = wide::wide_event(&context, summary);
        let flushed = self
            .telemetry
            .flush(Box::new(move |lost| {
                LabletRun {
                    lablet_telemetry_dropped_records: count_of(lost),
                    ..filled.clone()
                }
                .record(end, &root_context)
            }))
            .await;
        if let Err(error) = flushed {
            tracing::warn!(
                run_id = %summary.outcome.run_id,
                failures = ?error.failures(),
                "the run's telemetry wasn't exported whole"
            );
        }

        if let Some((file, shown)) = transcript {
            self.write_transcript(file, &shown, context, task_prompt, &finished)
                .await;
        }
        finished
    }

    /// Flushes the telemetry and stops its exporters. A destination that
    /// doesn't answer is given seconds, and what couldn't be exported is
    /// reported on the diagnostic log.
    pub async fn shutdown(self) {
        if let Err(error) = self.telemetry.shutdown().await {
            tracing::warn!(
                failures = ?error.failures(),
                "the telemetry wasn't exported whole at shutdown"
            );
        }
    }

    /// The file of the run's transcript, when the config names a place for
    /// one and the run id can be part of it, and its path as the config
    /// writes it.
    fn transcript_file(&self, run_id: &RunId) -> Option<(TranscriptFile, PathBuf)> {
        let path = self.fixed.transcript.as_ref()?;
        TranscriptFile::for_run(&path.real, run_id)
            .inspect_err(|error| {
                tracing::warn!(%run_id, %error, "the run has no transcript file");
            })
            .ok()
            .map(|file| (file, path.written.clone()))
    }

    /// Writes the run's transcript to `file`, and warns of a write that
    /// failed with `shown`, the path as the config writes it.
    async fn write_transcript(
        &self,
        file: TranscriptFile,
        shown: &Path,
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
            .map_err(|error| format!("the transcript's write didn't run to its end: {error}"))
            .and_then(|written| {
                written.map_err(|error| match error {
                    TranscriptWriteError::Unwritable { reason, .. } => format!(
                        "the transcript couldn't be written to {}: {reason}",
                        shown.display()
                    ),
                    error @ TranscriptWriteError::RunIdNotOneComponent { .. } => error.to_string(),
                })
            });
        if let Err(error) = written {
            tracing::warn!(%run_id, "{error}");
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

/// The instant `offset_ms` into a run that started at `started_unix_ms`:
/// what every span and record of the run is timed by, so an exporter's own
/// clock reaches none of them.
fn at(started_unix_ms: u64, offset_ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(started_unix_ms.saturating_add(offset_ms))
}
