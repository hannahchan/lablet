//! A run in progress: what the loop tells it as things happen, what the stop
//! policy reads from it, and the finished run it becomes.
//!
//! It keeps the transcript, and beside it only what a transcript doesn't
//! hold: the input waiting for the next turn, and what the provider call
//! attempts that failed cost. Every total of the summary is computed from the
//! two when the run ends, so a total can't disagree with the turns it sums.

use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroU32;
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;

use crate::totals::sum;
use crate::whole_ms;
use crate::{
    Answer, Calls, CompletionMode, Cost, Endpoint, FinishedRun, Latency, Message, ModelRef,
    OutcomeParts, PromptSizes, ProviderResponse, ProviderTotals, Rates, RequestParams, RunId,
    RunLabels, RunOutcome, RunSummary, StopReason, TaskResult, ToolCallOutcome, ToolCallTotals,
    ToolConcurrency, ToolInput, ToolName, ToolStats, ToolUse, Transcript, Turn, Usage, UserContent,
};

/// What the loop knows about a run before its first provider call, the
/// prompts aside.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSetup {
    /// The run's id.
    pub run_id: RunId,
    /// What the run request named the run's task, experiment and trial, which
    /// the outcome echoes.
    pub labels: RunLabels,
    /// The model the run calls.
    pub model: ModelRef,
    /// Where the provider's API is served; `None` for a provider that isn't
    /// reached over the network.
    pub endpoint: Option<Endpoint>,
    /// The tools offered to the model, after the allow and deny lists, and
    /// so the names the executor can resolve a call to.
    pub tools: Vec<ToolName>,
    /// Size of the tool specs in bytes, each as compact JSON.
    pub tools_bytes: u64,
    /// SHA-256 of those bytes in the order the specs are offered, in hex.
    pub tools_digest: String,
    /// SHA-256 of the system prompt, in hex.
    pub system_prompt_digest: String,
    /// How the run decides that the model has finished.
    pub completion: CompletionMode,
    /// The cap on turns; `None` when the run has none.
    pub max_turns: Option<NonZeroU32>,
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

    /// The same task under `system`, in place of the system prompt these
    /// prompts held.
    ///
    /// It's for whoever takes a task before the system prompt it runs under
    /// is at hand: the task was checked when these prompts were made, so
    /// nothing is left to refuse.
    #[must_use]
    pub fn with_system(self, system: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            ..self
        }
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
    /// Usage summed over every provider call attempt so far: every response,
    /// and every failed attempt that reported what it used.
    pub usage: Usage,
    /// The turns in a row, counted back from the last, of which every call
    /// was one the model got wrong ([`crate::ToolCallStatus::is_invalid`]). A
    /// turn in which any call reached a tool ends the count, whatever the
    /// tool returned, and so does one the run's timeout cut short, since a
    /// call that was never run isn't one the model got wrong.
    pub consecutive_invalid_turns: u32,
}

/// The timing of a provider call attempt that failed, as the run counted it.
///
/// [`Run::failed_attempt`] gives it back, so what the loop puts on the
/// attempt's span is what the summary's provider latencies hold of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FailedAttempt {
    /// When the attempt began, in whole milliseconds since the run started.
    pub started_ms: u64,
    /// How long the attempt took, in whole milliseconds.
    pub latency_ms: u64,
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
    /// How long the failed attempts of every call took.
    failed_latency: Latency,
    /// What the failed attempts of every call reported using, and `None`
    /// when none reported anything.
    failed_usage: Option<Usage>,
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
            failed_latency: Latency::default(),
            failed_usage: None,
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

    /// Records a provider call attempt that started `started` into the run
    /// and failed after `latency`, and the `usage` it reported, when the
    /// provider billed for it and said so. Gives back the attempt's timing
    /// as it was counted, which is the latency the summary's totals hold.
    ///
    /// An attempt that was dropped because the run was cancelled while it
    /// was in flight is one that failed too, having answered nothing and
    /// reported nothing: it took its time and it was an attempt, so the
    /// provider latencies and the retries count it as they count any other.
    ///
    /// That usage is in no turn, because a failed attempt made none, so the
    /// outcome's usage leaves it out. It counts toward the token budget and
    /// the cost, or a run that fails more often would look cheaper than it
    /// was.
    pub fn failed_attempt(
        &mut self,
        started: Duration,
        latency: Duration,
        usage: Option<Usage>,
    ) -> FailedAttempt {
        let attempt = FailedAttempt {
            started_ms: whole_ms(started),
            latency_ms: whole_ms(latency),
        };
        self.failed_attempts = self.failed_attempts.saturating_add(1);
        self.failed_latency = self.failed_latency + Latency::of(attempt.latency_ms);
        if let Some(usage) = usage {
            self.failed_usage = Some(self.failed() + usage);
        }
        attempt
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
            usage: self.spent(),
            consecutive_invalid_turns: self.consecutive_invalid_turns(),
        }
    }

    /// The invalid turns the run ends with.
    ///
    /// Every turn of a run that waits for a response made at least one call
    /// and has an outcome for each, because [`Pending::answer`] is the only
    /// way a turn gets here. So each turn is read whole, once all its calls
    /// have their outcomes: the order of a turn's calls never decides the
    /// count, and one response adds one to it at most.
    fn consecutive_invalid_turns(&self) -> u32 {
        let invalid = self
            .transcript
            .turns()
            .iter()
            .rev()
            .take_while(|turn| {
                turn.tool_calls()
                    .iter()
                    .all(|outcome| outcome.status.is_invalid())
            })
            .count();
        u32::try_from(invalid).unwrap_or(u32::MAX)
    }

    /// Usage summed over every turn so far, which is every provider call
    /// that succeeded.
    #[must_use]
    pub fn usage(&self) -> Usage {
        self.transcript.usage()
    }

    /// [`Run::usage`] and what the failed attempts reported, which is what a
    /// run's token budget is held against and its cost priced from.
    #[must_use]
    pub fn spent(&self) -> Usage {
        self.usage() + self.failed()
    }

    /// What the failed attempts reported, read as a sum: nothing when none
    /// reported anything.
    fn failed(&self) -> Usage {
        self.failed_usage.unwrap_or_default()
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
    /// list, where the two answers could differ. A call that was never run
    /// counts nowhere: it's in the transcript and in no total.
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
            failed_latency,
            failed_usage,
            input,
        } = self;
        let recorded = transcript.turns();
        let prompt = recorded.first().map_or(&*input, Turn::input);
        let counted = || recorded.iter().flat_map(Turn::counted);
        // What the attempts that failed add: the latency of every one, and
        // the retries of the call that no response followed. The attempts
        // of a call that was answered are counted by its turn's record.
        let failed = ProviderTotals {
            retries: u64::from(failed_attempts.saturating_sub(1)),
            latency: failed_latency,
        };
        let answered = recorded
            .iter()
            .map(|turn| ProviderTotals::of(turn.record()));
        let summary = RunSummary {
            model: setup.model,
            endpoint: setup.endpoint,
            tools: setup.tools,
            completion: setup.completion,
            max_turns: setup.max_turns,
            timeout_ms: whole_ms(setup.timeout),
            request: setup.request,
            prompt: PromptSizes {
                system_bytes: transcript.system().len() as u64,
                user_bytes: UserContent::bytes(prompt),
                tools_bytes: setup.tools_bytes,
            },
            tools_digest: setup.tools_digest,
            system_prompt_digest: setup.system_prompt_digest,
            failed_usage,
            provider: failed + sum(answered),
            finish_reasons: recorded
                .iter()
                .map(|turn| turn.record().finish.clone())
                .collect(),
            tool_calls: sum(counted().map(|(call, outcome)| ToolCallTotals::of(call, outcome))),
            per_tool: per_tool(counted()),
            rates,
            cost,
            outcome: RunOutcome::closing(OutcomeParts {
                run_id: setup.run_id,
                labels: setup.labels,
                stop_reason: stop,
                turns: turns(&transcript),
                usage: transcript.usage(),
                tool_calls: counted().count() as u64,
                duration_ms: whole_ms(duration),
                result: TaskResult {
                    text: transcript.final_text(),
                    structured,
                },
                error,
            }),
        };
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

    fn spent(&self) -> Usage {
        self.run.spent() + self.turn.record().usage
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

    /// That usage and what the failed attempts reported; see [`Run::spent`].
    #[must_use]
    pub fn spent(&self) -> Usage {
        self.0.spent()
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

    /// That usage and what the failed attempts reported; see [`Run::spent`].
    #[must_use]
    pub fn spent(&self) -> Usage {
        self.0.spent()
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

    /// The argument of the call that completes the run, which is the run's
    /// structured result: the response's only call, when it's one `mode`
    /// intercepts ([`CompletionMode::intercepts`]) and its arguments parsed.
    ///
    /// A completion call made beside other calls completes nothing, a second
    /// completion call among them: the run would report work as done that
    /// the response only asked for. Nor does one whose arguments didn't
    /// parse. The loop answers each, so the model can make the call again.
    #[must_use]
    pub fn completed_with(&self, mode: CompletionMode) -> Option<&serde_json::Value> {
        let mut calls = self.0.turn.tool_uses();
        let (Some(only), None) = (calls.next(), calls.next()) else {
            return None;
        };
        match &only.input {
            ToolInput::Json(value) if mode.intercepts(&only.name) => Some(value),
            ToolInput::Json(_) | ToolInput::Unparsed(_) => None,
        }
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
    /// It runs no future of its own beyond wrapping each one `answer` makes
    /// with the id of its call, and names no runtime. It takes each call by
    /// value because the future an async closure returns for a borrowed call
    /// can't be shown `Send` while several are held.
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
        while let Some((first, others)) = rest.split_first() {
            let shared = |call: &ToolUse| (schedule.concurrency)(call) == ToolConcurrency::Shared;
            // A group is the call it starts with and the calls that run
            // beside it, which are cut from the calls after it. So a group
            // always holds a call and the loop always moves on.
            let more = if shared(first) {
                others.iter().take_while(|call| shared(call)).count()
            } else {
                0
            };
            let (beside, after) = others.split_at(more);
            rest = after;

            // A pool rather than an ordered queue: a call starts as soon as
            // any call of its group ends, not only when the earliest one
            // does, and each answer carries its place so the outcomes can be
            // put back in call order.
            let mut waiting = std::iter::once(first).chain(beside).enumerate();
            let mut running = FuturesUnordered::new();
            let mut answered = Vec::new();
            loop {
                while running.len() < max {
                    let Some((at, call)) = waiting.next() else {
                        break;
                    };
                    let id = call.id.clone();
                    let answering = answer(call.clone());
                    running.push(async move { (at, answering.await.answering(id)) });
                }
                let Some(done) = running.next().await else {
                    break;
                };
                answered.push(done);
            }
            answered.sort_unstable_by_key(|(at, _)| *at);
            outcomes.extend(answered.into_iter().map(|(_, outcome)| outcome));
        }
        turn.answered(outcomes);
        run.transcript.push(turn);
        run
    }
}

/// Each tool's share of `calls`, by the name it was called by. A call that
/// named a tool the run didn't offer adds to no share: the model can call any
/// name, and the shares are what the per-tool keys of the wide event are made
/// from.
fn per_tool<'a>(
    calls: impl Iterator<Item = (&'a ToolUse, &'a ToolCallOutcome)>,
) -> BTreeMap<ToolName, ToolStats> {
    calls
        .filter(|(_, outcome)| outcome.status.names_an_offered_tool())
        .fold(BTreeMap::new(), |mut shares, (call, outcome)| {
            let share = shares.entry(call.name.clone()).or_default();
            *share = *share + ToolStats::of(outcome);
            shares
        })
}

/// A run's turns are the model responses it received, so a run whose first
/// provider call failed took none.
fn turns(transcript: &Transcript) -> u32 {
    u32::try_from(transcript.turns().len()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
