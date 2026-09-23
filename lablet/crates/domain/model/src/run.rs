//! A run in progress: what the loop tells it as things happen, what the stop
//! policy reads from it, and the finished run it becomes.
//!
//! It keeps the transcript, and beside it only what a transcript doesn't
//! hold: the input waiting for the next turn, and the provider call attempts
//! that failed. Every total of the summary is computed from the two when the
//! run ends, so a total can't disagree with the turns it sums.

use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroU32;
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::stream::FuturesOrdered;

use crate::outcome::RawOutcome;
use crate::whole_ms;
use crate::{
    Answer, Calls, CompletionMode, Cost, Endpoint, FinishedRun, Message, ModelRef,
    ProviderResponse, Rates, RequestParams, RunId, RunOutcome, RunSummary, StopReason, TaskResult,
    ToolConcurrency, ToolInput, ToolName, ToolUse, Transcript, Turn, Usage, UserContent,
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
    system: String,
    task: String,
}

/// A run was given nothing to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the task prompt is blank")]
pub struct BlankTask;

impl Prompts {
    /// The prompts of a run, with skills already appended to `system`.
    ///
    /// # Errors
    ///
    /// Returns [`BlankTask`] when `task` is empty or only whitespace. The
    /// first response needs something from the user before it, and the task
    /// is all a run has, so a blank one is refused here, before a run can
    /// spend a provider call on it. An empty `system` is a run with no system
    /// prompt, which is allowed.
    pub fn new(system: impl Into<String>, task: impl Into<String>) -> Result<Self, BlankTask> {
        let task: String = task.into();
        if task.trim().is_empty() {
            return Err(BlankTask);
        }
        Ok(Self {
            system: system.into(),
            task,
        })
    }

    /// The system prompt.
    #[must_use]
    pub fn system(&self) -> &str {
        &self.system
    }

    /// The task the run is given, which becomes the first turn's input.
    #[must_use]
    pub fn task(&self) -> &str {
        &self.task
    }
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

/// One run, from its first provider call to the outcome it becomes, while it
/// waits for a provider response.
///
/// A run moves through three states, and each offers only what may happen
/// next. A `Run` waits for a response; recording one consumes it and gives a
/// [`Final`] when the response called no tool, which can only finish, or a
/// [`Pending`] when it called at least one, which finishes or is answered and
/// becomes a `Run` again. So something from the user comes before every
/// response, every call of a turn that goes on is answered once and in call
/// order, and only the last turn can have calls with no outcomes, because no
/// method exists for anything else. A `Run` is made by [`Run::start`], whose
/// task [`Prompts::new`] has refused to let be blank, and by
/// [`Pending::answer`], which leaves at least one outcome.
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
    /// next turn. The attempt started `started` into the run and took
    /// `latency`; the record counts it and the failed attempts since the turn
    /// before. The turn's input is what was waiting for it.
    ///
    /// The turn's response is the response's content without its text
    /// blocks that are empty or only whitespace, and this is the one place
    /// where the transcript differs from what the provider sent. Models emit
    /// such blocks, typically just before a tool call, and Anthropic refuses
    /// them when the response is replayed. They carry nothing, so refusing
    /// the response instead would spend a retry on nothing. Everything reads
    /// the response after the drop, so [`Transcript::final_text`] and every
    /// size measured from [`Run::messages`] describe what's replayed.
    #[must_use]
    pub fn responded(
        mut self,
        response: ProviderResponse,
        started: Duration,
        latency: Duration,
    ) -> Responded {
        let attempts = self.failed_attempts.saturating_add(1);
        self.failed_attempts = 0;
        let input = std::mem::take(&mut self.input);
        let turn = Turn::recorded(input, response, started, latency, attempts);
        let responded = Recorded { run: self, turn };
        if responded.turn.tool_uses().next().is_none() {
            Responded::Final(Final(responded))
        } else {
            Responded::Pending(Pending(responded))
        }
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
            summary.add_turn(turn);
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

/// What recording a response gives: which of the two states the run is in.
#[derive(Debug, Clone, PartialEq)]
pub enum Responded {
    /// The response called no tool.
    Final(Final),
    /// The response called at least one tool.
    Pending(Pending),
}

/// A run whose last response called no tool. The stop policy stops every
/// such run, so the only way on is to finish.
#[derive(Debug, Clone, PartialEq)]
pub struct Final(Recorded);

/// A run whose last response called at least one tool. It finishes, which
/// leaves those calls unanswered, or is answered and waits for the next
/// response.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending(Recorded);

/// A run and the turn it has just recorded, held apart until the run knows
/// what becomes of the turn's calls.
#[derive(Debug, Clone, PartialEq)]
struct Recorded {
    run: Run,
    turn: Turn,
}

impl Recorded {
    fn usage(&self) -> Usage {
        self.run.usage() + self.turn.record().usage
    }

    fn finish(
        self,
        stop: StopReason,
        duration: Duration,
        structured: Option<serde_json::Value>,
        error: Option<String>,
        rates: Option<Rates>,
        cost: Option<Cost>,
    ) -> FinishedRun {
        let Self { mut run, turn } = self;
        run.transcript.push(turn);
        run.finish(stop, duration, structured, error, rates, cost)
    }
}

impl Final {
    /// The turn the response made.
    #[must_use]
    pub const fn turn(&self) -> &Turn {
        &self.0.turn
    }

    /// Usage summed over every turn, this one included.
    #[must_use]
    pub fn usage(&self) -> Usage {
        self.0.usage()
    }

    /// Closes the record of the run; see [`Run::finish`].
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
        self.0
            .finish(stop, duration, structured, error, rates, cost)
    }
}

/// How [`Pending::answer`] groups a turn's calls.
#[derive(Clone, Copy)]
pub struct Schedule<'a> {
    /// How many calls of one group may run at once.
    pub max_concurrent: NonZeroU32,
    /// Whether a call may run beside others, read from the tool it names.
    pub concurrency: &'a (dyn Fn(&ToolUse) -> ToolConcurrency + Sync),
}

impl core::fmt::Debug for Schedule<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Schedule")
            .field("max_concurrent", &self.max_concurrent)
            .finish_non_exhaustive()
    }
}

impl Pending {
    /// The turn the response made.
    #[must_use]
    pub const fn turn(&self) -> &Turn {
        &self.0.turn
    }

    /// Usage summed over every turn, this one included.
    #[must_use]
    pub fn usage(&self) -> Usage {
        self.0.usage()
    }

    /// How `mode` reads the response's calls: [`Calls::TaskComplete`] when
    /// [`Pending::completed_with`] has an argument, and [`Calls::Tools`]
    /// otherwise.
    #[must_use]
    pub fn calls(&self, mode: CompletionMode) -> Calls {
        if self.completed_with(mode).is_some() {
            Calls::TaskComplete
        } else {
            Calls::Tools
        }
    }

    /// The argument of the first `task_complete` call in the response whose
    /// arguments parsed, in explicit mode, which is the run's structured
    /// result. A call whose arguments didn't parse completes nothing: it's
    /// answered like any other call with bad arguments, so the model can try
    /// again.
    #[must_use]
    pub fn completed_with(&self, mode: CompletionMode) -> Option<&serde_json::Value> {
        if mode != CompletionMode::Explicit {
            return None;
        }
        self.0
            .turn
            .tool_uses()
            .filter(|call| call.name.as_str() == CompletionMode::TASK_COMPLETE)
            .find_map(|call| match &call.input {
                ToolInput::Json(value) => Some(value),
                ToolInput::Unparsed(_) => None,
            })
    }

    /// Closes the record of the run, the turn's calls unanswered; see
    /// [`Run::finish`].
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
        self.0
            .finish(stop, duration, structured, error, rates, cost)
    }

    /// Answers every call of the turn with what `answer` returns for it, and
    /// gives back the run, waiting for its next response.
    ///
    /// The calls run in groups, in call order: consecutive calls that
    /// `schedule` reads as [`ToolConcurrency::Shared`] form one group, whose
    /// calls run concurrently, at most `schedule.max_concurrent` at a time;
    /// any other call is a group of its own. Outcomes are recorded in call
    /// order whichever call finished first, each with the id of the call it
    /// answers, so the loop never holds a list whose length or order could
    /// be wrong.
    ///
    /// This composes the futures `answer` makes and makes none of its own. It
    /// takes each call by value because the future an async closure returns
    /// for a borrowed call can't be shown `Send` while several are held.
    pub async fn answer<F, Fut>(self, schedule: Schedule<'_>, answer: F) -> Run
    where
        F: Fn(ToolUse) -> Fut,
        Fut: Future<Output = Answer>,
    {
        let Recorded { mut run, mut turn } = self.0;
        let calls: Vec<ToolUse> = turn.tool_uses().cloned().collect();
        let max = usize::try_from(schedule.max_concurrent.get()).unwrap_or(usize::MAX);
        let mut outcomes = Vec::with_capacity(calls.len());
        let mut rest = calls.as_slice();
        while let Some(first) = rest.first() {
            let shared = |call: &ToolUse| (schedule.concurrency)(call) == ToolConcurrency::Shared;
            let len = if shared(first) {
                rest.iter().take_while(|call| shared(call)).count()
            } else {
                1
            };
            let (group, after) = rest.split_at(len);
            rest = after;

            let mut waiting = group.iter();
            let mut running = FuturesOrdered::new();
            loop {
                while running.len() < max {
                    let Some(call) = waiting.next() else { break };
                    let id = call.id.clone();
                    let answered = answer(call.clone());
                    running.push_back(async move { answered.await.answering(id) });
                }
                let Some(outcome) = running.next().await else {
                    break;
                };
                outcomes.push(outcome);
            }
        }
        turn.answered(outcomes);
        run.transcript.push(turn);
        run
    }
}

/// A run's turns are the model responses it received, so a run whose first
/// provider call failed took none.
fn turns(transcript: &Transcript) -> u32 {
    u32::try_from(transcript.turns().len()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
