//! The `run` section: how a run completes, what bounds it, and where its
//! transcript goes.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::written::{duration, path};

/// The `run` section.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Run {
    /// How the run decides that the model has finished.
    pub completion: Completion,
    /// The cap on turns; `None` is no cap, and the timeout and the token
    /// budget bound the run.
    pub max_turns: Option<NonZeroU32>,
    /// How long the run may take.
    #[serde(with = "duration")]
    #[schemars(with = "String")]
    pub timeout: Duration,
    /// The budget of input plus output tokens, summed over every provider
    /// call; `None` is no budget.
    pub max_total_tokens: Option<u64>,
    /// How many times a failed provider call is tried again; `0` never
    /// retries.
    pub max_retries: u32,
    /// The wait after the first failed attempt, which doubles each time.
    #[serde(with = "duration")]
    #[schemars(with = "String")]
    pub retry_backoff_base: Duration,
    /// The longest backoff.
    #[serde(with = "duration")]
    #[schemars(with = "String")]
    pub retry_backoff_max: Duration,
    /// The largest share of a wait that's added to it, from 0 to 1.
    pub retry_jitter: f64,
    /// The longest wait a server may ask for; a longer one ends the retries.
    #[serde(with = "duration")]
    #[schemars(with = "String")]
    pub retry_hint_max: Duration,
    /// How many turns in a row may make no call that reached a tool; `None`
    /// is no cap.
    pub max_consecutive_invalid_turns: Option<NonZeroU32>,
    /// How long one provider call may take, when the run has that long left.
    #[serde(with = "duration")]
    #[schemars(with = "String")]
    pub provider_timeout: Duration,
    /// What's sent of the conversation on each provider call.
    pub context: Context,
    /// Where the transcript is written when the run ends; it may hold
    /// `{run_id}`. `None` writes none.
    #[serde(serialize_with = "path::optional")]
    pub transcript_path: Option<PathBuf>,
    /// The form the transcript is written in.
    pub transcript_format: TranscriptFormat,
    /// The JSON schema of the `task_complete` argument in explicit mode;
    /// `None` takes any object.
    pub completion_schema: Option<serde_json::Value>,
}

impl Default for Run {
    fn default() -> Self {
        Self {
            completion: Completion::Natural,
            max_turns: None,
            timeout: Duration::from_secs(600),
            max_total_tokens: None,
            max_retries: 10,
            retry_backoff_base: Duration::from_millis(500),
            retry_backoff_max: Duration::from_secs(32),
            retry_jitter: 0.25,
            retry_hint_max: Duration::from_secs(60),
            max_consecutive_invalid_turns: NonZeroU32::new(3),
            provider_timeout: Duration::from_secs(600),
            context: Context::Full,
            transcript_path: None,
            transcript_format: TranscriptFormat::Json,
            completion_schema: None,
        }
    }
}

/// How a run decides that the model has finished.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    /// A response that calls no tool completes the run.
    #[default]
    Natural,
    /// A `task_complete` call made on its own completes the run.
    Explicit,
}

/// What's sent of the conversation on each provider call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Context {
    /// All of it.
    #[default]
    Full,
    /// All of it, with a placeholder in place of each older tool result.
    Mask {
        /// The size of a request, in tokens, from which results are masked.
        trigger_tokens: u64,
        /// How many of the latest tool results are sent whole.
        keep_last: u32,
    },
}

/// The form a transcript is written in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptFormat {
    /// The transcript document.
    #[default]
    Json,
    /// An ATIF trajectory.
    Atif,
}
