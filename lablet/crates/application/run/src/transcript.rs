//! Where a run's transcript goes, and its writing, as a port.

use std::path::PathBuf;

use lablet_model::{ModelRef, RunContext, RunId, ToolSpec, Transcript};

/// What a run's transcript is made from, as the run left it.
#[derive(Debug, Clone)]
pub struct RunTranscript {
    /// The run's context, which names the run and when it started.
    pub context: RunContext,
    /// The model the run was asked of.
    pub model: ModelRef,
    /// The tools the run was offered, in the order they were offered.
    pub tools: Vec<ToolSpec>,
    /// The task, as the run's request gave it.
    pub task: String,
    /// The conversation.
    pub transcript: Transcript,
}

/// Why a run's transcript has no place, or wasn't written there. Neither
/// fails a run: a transcript is a record of one, and the run measured what
/// it measured. Each carries the text the composition root reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranscriptError {
    /// The place the config names can't take the run's id.
    #[error("{0}")]
    NoPlace(String),
    /// The transcript wasn't written whole.
    #[error("{0}")]
    Unwritten(String),
}

/// Writes a run's transcript where the config says it goes.
///
/// [`crate::Runner`] asks for the place before the run starts, since the
/// run's context and its wide event name it, and writes there once the
/// loop has returned, before the wide event is emitted, so a wide event
/// names a transcript that's whole.
#[async_trait::async_trait]
pub trait TranscriptWriter: Send + Sync {
    /// Where the transcript of the run `run_id` goes.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptError::NoPlace`] when the place the config names
    /// can't take `run_id`; the run then has no transcript.
    fn place(&self, run_id: &RunId) -> Result<PathBuf, TranscriptError>;

    /// Writes `transcript` at `place`, which [`TranscriptWriter::place`]
    /// gave for its run.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptError::Unwritten`] when it couldn't be written
    /// whole.
    async fn write(&self, place: PathBuf, transcript: RunTranscript)
    -> Result<(), TranscriptError>;
}
