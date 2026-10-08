//! What a caller asks of a run and how it stops one: the [`RunRequest`]
//! both roots build, the library from a host's call and the command line
//! from its arguments, and the [`CancelHandle`] that fires a stop.

mod cancel;

use std::sync::Arc;

use lablet_model::{BlankTask, Prompts, RunId, RunLabels};
use lablet_run::{Cancellation, RunStart};

pub use cancel::CancelHandle;

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
    #[must_use]
    pub fn into_start(self) -> RunStart {
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
