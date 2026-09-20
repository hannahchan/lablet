//! A run in progress: what the loop tells it as things happen, what the stop
//! policy reads from it, and the finished run it becomes.
//!
//! It keeps the transcript, and beside it only what a transcript doesn't
//! hold: the input waiting for the next turn, and the provider call attempts
//! that failed. Every total of the
//! summary is computed from the two when the run ends, so a total can't
//! disagree with the turns it sums.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::Duration;

use crate::outcome::{RawOutcome, whole_ms};
use crate::{
    CompletionMode, Cost, Endpoint, FinishedRun, Message, ModelRef, ProviderResponse, Rates,
    RequestParams, RunId, RunOutcome, RunSummary, StopReason, TaskResult, ToolCallOutcome,
    ToolName, Transcript, TranscriptError, Turn, Usage, UserContent,
};

/// What the loop knows about a run before its first provider call, the
/// prompts aside.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSetup {
    /// The run's id.
    pub run_id: RunId,
    /// The model the run calls.
    pub model: ModelRef,
    /// Where the provider's API is served; `None` for a provider that isn't
    /// reached over the network.
    pub endpoint: Option<Endpoint>,
    /// The tools offered to the model, after the allow and deny lists, and
    /// so the names the executor can resolve a call to.
    pub tools: Vec<ToolName>,
    /// How the run decides that the model has finished.
    pub completion: CompletionMode,
    /// The cap on turns.
    pub max_turns: NonZeroU32,
    /// The run timeout.
    pub timeout: Duration,
    /// The request parameters every provider call shares.
    pub request: RequestParams,
}

/// What the user gives a run to work from.
///
/// One value rather than two strings, because `start(setup, system, prompt)`
/// takes them adjacent and swapping them runs the task as the system prompt
/// and reports both prompt sizes inverted, with nothing to catch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompts {
    /// The system prompt, skills already appended.
    pub system: String,
    /// The task the run is given, which becomes the first turn's input.
    pub task: String,
}

/// How far a run has come, which is what its limits are held against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    /// Model responses received so far.
    pub turns: u32,
    /// The time since the run started.
    pub elapsed: Duration,
    /// Usage summed over every response so far.
    pub usage: Usage,
    /// Tool error results since the last successful tool call.
    pub consecutive_tool_errors: u32,
}

/// One run, from its first provider call to the outcome it becomes.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    setup: RunSetup,
    transcript: Transcript,
    /// What the user has supplied to the next turn: the task prompt until the
    /// first turn takes it, and nothing after that.
    input: Vec<UserContent>,
    /// Failed attempts of the provider call in progress.
    failed_attempts: u32,
    failed_latency_total_ms: u64,
    failed_latency_max_ms: u64,
}

impl Run {
    /// A run that has done nothing yet. The task prompt becomes the input of
    /// the first turn.
    #[must_use]
    pub fn start(setup: RunSetup, prompts: Prompts) -> Self {
        Self {
            setup,
            transcript: Transcript::new(prompts.system),
            input: vec![UserContent::Text(prompts.task)],
            failed_attempts: 0,
            failed_latency_total_ms: 0,
            failed_latency_max_ms: 0,
        }
    }

    /// The conversation so far.
    #[must_use]
    pub const fn transcript(&self) -> &Transcript {
        &self.transcript
    }

    /// The flat form the next provider call sends. Before each turn's
    /// response comes one message from the user, which holds the results of
    /// the tool calls of the turn before, in call order, and then the turn's
    /// input; the last message is the one the next turn will answer. Blocks
    /// are rendered as they're stored and in order, which a provider that
    /// verifies thinking blocks requires.
    #[must_use]
    pub fn messages(&self) -> Vec<Message<'_>> {
        self.transcript.messages(&self.input)
    }

    /// Records a provider call attempt that failed after `latency`.
    pub fn failed_attempt(&mut self, latency: Duration) {
        let latency_ms = whole_ms(latency);
        self.failed_attempts = self.failed_attempts.saturating_add(1);
        self.failed_latency_total_ms = self.failed_latency_total_ms.saturating_add(latency_ms);
        self.failed_latency_max_ms = self.failed_latency_max_ms.max(latency_ms);
    }

    /// Records the provider call attempt that returned `response` as the
    /// next turn, and returns it. The attempt started `started` into the run
    /// and took `latency`; the turn's record counts it and the failed
    /// attempts since the turn before. The turn's input is what was waiting
    /// for it, which is the task prompt for the first turn.
    ///
    /// The turn's response is the response's content without its text
    /// blocks that are empty or only whitespace, and this is the one place
    /// where the transcript differs from what the provider sent. Models emit
    /// such blocks, typically just before a tool call, and Anthropic refuses
    /// them when the response is replayed. They carry nothing, so refusing
    /// the response instead would spend a retry on nothing. Everything reads
    /// the response after the drop, so [`Transcript::final_text`] and every
    /// size measured from [`Run::messages`] describe what's replayed.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptError::UnansweredCalls`] when the turn before made
    /// tool calls and [`Run::tool_calls`] never answered them, and
    /// [`TranscriptError::NothingFromTheUser`] when it made none, so that
    /// this response would follow that one directly. The loop makes no
    /// provider call in either state, so it never sees them. The record is
    /// unchanged.
    pub fn responded(
        &mut self,
        response: ProviderResponse,
        started: Duration,
        latency: Duration,
    ) -> Result<&Turn, TranscriptError> {
        let attempts = self.failed_attempts.saturating_add(1);
        let turn = self
            .transcript
            .record(&mut self.input, response, started, latency, attempts)?;
        self.failed_attempts = 0;
        Ok(turn)
    }

    /// Records what happened to the tool calls of the last turn. The
    /// intercepted `task_complete` call isn't executed, so it has no outcome.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptError::OutcomesDontAnswerCalls`] unless `outcomes`
    /// answers the tool calls of the last response, each once and in call
    /// order, or both are empty; and [`TranscriptError::AlreadyAnswered`] when
    /// the turn's calls were answered before. The loop builds one outcome for
    /// each call, in order, so it never sees either.
    pub fn tool_calls(&mut self, outcomes: Vec<ToolCallOutcome>) -> Result<(), TranscriptError> {
        self.transcript.answer(outcomes)
    }

    /// How far the run has come, `elapsed` after it started.
    #[must_use]
    pub fn progress(&self, elapsed: Duration) -> Progress {
        Progress {
            turns: self.turns(),
            elapsed,
            usage: self.usage(),
            consecutive_tool_errors: self.transcript.consecutive_tool_errors(),
        }
    }

    /// Usage summed over every turn so far, which is what a run's cost is
    /// priced from.
    #[must_use]
    pub fn usage(&self) -> Usage {
        self.transcript.usage()
    }

    /// Closes the record of a run that stopped with `stop`, `duration` after
    /// it started.
    ///
    /// `structured` is the argument of a `task_complete` call in the last
    /// response, and `error` the text of the error that ended the run, when
    /// the loop has one. The outcome keeps only what the class of `stop`
    /// allows ([`StopReason::class`]): `structured` when the run completed,
    /// and an error when it failed, which is the reason's own message when
    /// `error` is `None`. So the loop passes what it has at hand, and the
    /// outcome is still one a run can have.
    ///
    /// A retry is an attempt made beyond the first of its call, so a call's
    /// last failed attempt, which nothing followed, isn't one. A tool call to
    /// a name the run didn't offer counts in the totals and as an unknown
    /// call, and gets no per-tool entry: the model can call any name, and the
    /// per-tool keys of the wide event must stay bounded. Which
    /// calls those are is read from each outcome's
    /// [`crate::ToolCallStatus`], the value the executor set when it looked
    /// the name up, rather than looked up a second time here against the tool
    /// list, where the two answers could differ.
    #[must_use]
    pub fn finish(
        self,
        stop: StopReason,
        duration: Duration,
        structured: Option<serde_json::Value>,
        error: Option<String>,
        rates: Option<Rates>,
        cost: Option<Cost>,
    ) -> FinishedRun {
        let Self {
            setup,
            transcript,
            failed_attempts,
            failed_latency_total_ms,
            failed_latency_max_ms,
            input,
        } = self;
        let prompt = transcript.turns().first().map_or(&*input, Turn::input);
        let tool_calls = transcript
            .turns()
            .iter()
            .map(|turn| turn.tool_calls().len() as u64)
            .sum();
        let mut summary = RunSummary {
            model: setup.model,
            endpoint: setup.endpoint,
            tools: setup.tools,
            completion: setup.completion,
            max_turns: setup.max_turns,
            timeout_ms: whole_ms(setup.timeout),
            request: setup.request,
            prompt_system_bytes: transcript.system().len() as u64,
            prompt_user_bytes: UserContent::bytes(prompt),
            provider_retries: u64::from(failed_attempts.saturating_sub(1)),
            provider_latency_total_ms: failed_latency_total_ms,
            provider_latency_max_ms: failed_latency_max_ms,
            finish_reasons: Vec::new(),
            tool_calls_errors: 0,
            tool_calls_unknown: 0,
            tool_latency_total_ms: 0,
            tool_input_bytes: 0,
            tool_output_bytes: 0,
            tool_calls_truncated: 0,
            per_tool: BTreeMap::new(),
            rates,
            cost,
            outcome: RunOutcome::closing(RawOutcome {
                run_id: setup.run_id,
                stop_reason: stop,
                turns: turns(&transcript),
                usage: transcript.usage(),
                tool_calls,
                duration_ms: whole_ms(duration),
                result: TaskResult {
                    text: transcript.final_text(),
                    structured,
                },
                error,
            }),
        };
        for turn in transcript.turns() {
            add_turn(&mut summary, turn);
        }
        FinishedRun {
            summary,
            transcript,
        }
    }

    fn turns(&self) -> u32 {
        turns(&self.transcript)
    }
}

/// A run's turns are the model responses it received, so a run whose first
/// provider call failed took none.
fn turns(transcript: &Transcript) -> u32 {
    u32::try_from(transcript.turns().len()).unwrap_or(u32::MAX)
}

/// Raises a total by `amount`, the way every total in the model grows: a
/// count that can't be raised any further stops there rather than wrapping to
/// nonsense or panicking. One function, so no total is added up differently
/// from the rest.
fn add(total: &mut u64, amount: u64) {
    *total = total.saturating_add(amount);
}

/// Adds what `turn` holds to the totals of `summary`.
fn add_turn(summary: &mut RunSummary, turn: &Turn) {
    let record = turn.record();
    add(
        &mut summary.provider_retries,
        u64::from(record.attempts.saturating_sub(1)),
    );
    add(&mut summary.provider_latency_total_ms, record.latency_ms);
    summary.provider_latency_max_ms = summary.provider_latency_max_ms.max(record.latency_ms);
    summary.finish_reasons.push(record.finish.clone());
    for (call, outcome) in turn.tool_uses().zip(turn.tool_calls()) {
        let errors = u64::from(outcome.status.is_error());
        add(&mut summary.tool_calls_errors, errors);
        add(
            &mut summary.tool_calls_truncated,
            u64::from(outcome.truncated_from_bytes.is_some()),
        );
        add(&mut summary.tool_latency_total_ms, outcome.latency_ms);
        add(&mut summary.tool_input_bytes, call.input_bytes());
        add(&mut summary.tool_output_bytes, outcome.output_bytes());
        if outcome.status.source().is_some() {
            let stats = summary.per_tool.entry(call.name.clone()).or_default();
            add(&mut stats.calls, 1);
            add(&mut stats.errors, errors);
            add(&mut stats.latency_ms, outcome.latency_ms);
        } else {
            add(&mut summary.tool_calls_unknown, 1);
        }
    }
}

#[cfg(test)]
mod tests;
