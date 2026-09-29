//! The loop: one run, from its first provider call to its outcome.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lablet_model::{
    Answer, Cost, Final, FinishedRun, KeptOutput, Message, OutputCap, Pending, Progress, Prompts,
    ProviderErrorKind, ProviderResponse, Rates, RequestParams, Responded, Run, RunContext, RunId,
    RunLabels, RunSetup, Schedule, StopReason, ToolCallStatus, ToolInput, ToolUse, Turn, Usage,
};
use lablet_policy::{Pricing, RetryPolicy, StopPolicy};

use crate::{
    Cancellation, Clock, EventKind, McpCallMeta, ModelProvider, ProviderError, ProviderRequest,
    RunEvent, RunObserver, ToolCall, ToolSet,
};

/// What bounds the calls, which is not a stop decision: the run's limits are
/// the stop policy's business and these are the adapters' and the tool
/// phase's.
///
/// No limit on a tool call is here. An executor has its own, and the loop
/// hands each call the time the run has left, which the stop policy knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallLimits {
    /// How long one provider attempt may take, when the run has that long
    /// left. An attempt's deadline is the shorter of the two.
    pub provider_timeout: Duration,
    /// The cap on what the model is sent of a tool's output, and how a
    /// longer one is cut; `None` for no cap. The loop applies it to every
    /// call's answer, and hands each executor what the cap can use of an
    /// output, so none holds more.
    pub output_cap: Option<OutputCap>,
    /// How many calls of one group may run at once; 1 runs every call alone.
    pub max_concurrent_tool_calls: NonZeroU32,
}

/// One instrumented agent loop.
///
/// `run` takes `&mut self` so that two runs on one service is a compile
/// error rather than a race: the spec asks for concurrent calls to be
/// rejected, and this ring has no runtime to reject them at.
pub struct RunService {
    provider: Arc<dyn ModelProvider>,
    tools: Arc<ToolSet>,
    observer: Arc<dyn RunObserver>,
    clock: Arc<dyn Clock>,
    cancel: Arc<dyn Cancellation>,
    stop: StopPolicy,
    retry: RetryPolicy,
    request: RequestParams,
    pricing: Option<Pricing>,
    calls: CallLimits,
}

/// A provider call that answered, with the timing of the attempt that did.
struct Answered {
    response: ProviderResponse,
    began: Duration,
    latency: Duration,
}

/// What became of one tool call. The loop is the only hop between an executor
/// and an observer, so the call's transport metadata travels with its result
/// or the `mcp.*` attributes have no way to be set.
struct Settled {
    status: ToolCallStatus,
    output: KeptOutput,
    mcp: Option<McpCallMeta>,
}

impl Settled {
    /// A call the loop answered itself, so no executor was reached.
    fn local(status: ToolCallStatus, message: &str) -> Self {
        Self {
            status,
            output: KeptOutput::whole(message),
            mcp: None,
        }
    }
}

/// Why the loop stopped, and what it has to say about it: the error that
/// ended it, and the argument of the `task_complete` call that completed it.
/// The outcome keeps only what the stop reason allows.
struct Stopped {
    reason: StopReason,
    error: Option<String>,
    structured: Option<serde_json::Value>,
}

/// The state a run stopped in. Each can finish; they differ in whether the
/// last turn is in the transcript yet.
enum Ending {
    /// Stopped before a response came back.
    Waiting(Run),
    /// Stopped by a response that called no tool.
    Final(Final),
    /// Stopped at a response's calls, or before running them.
    Pending(Pending),
}

impl Ending {
    /// What the run is priced on: the failed attempts that reported usage
    /// were billed too.
    fn spent(&self) -> Usage {
        match self {
            Self::Waiting(run) => run.spent(),
            Self::Final(done) => done.spent(),
            Self::Pending(pending) => pending.spent(),
        }
    }

    fn finish(
        self,
        stopped: Stopped,
        duration: Duration,
        rates: Option<Rates>,
        cost: Option<Cost>,
    ) -> FinishedRun {
        let Stopped {
            reason,
            error,
            structured,
        } = stopped;
        match self {
            Self::Waiting(run) => run.finish(reason, duration, structured, error, rates, cost),
            Self::Final(done) => done.finish(reason, duration, structured, error, rates, cost),
            Self::Pending(pending) => {
                pending.finish(reason, duration, structured, error, rates, cost)
            }
        }
    }
}

impl RunService {
    /// A loop built from its ports and its policies.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "a composition root assembles this once; every argument is a distinct port or policy, so none can be given in another's place"
    )]
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        tools: Arc<ToolSet>,
        observer: Arc<dyn RunObserver>,
        clock: Arc<dyn Clock>,
        cancel: Arc<dyn Cancellation>,
        stop: StopPolicy,
        retry: RetryPolicy,
        request: RequestParams,
        pricing: Option<Pricing>,
        calls: CallLimits,
    ) -> Self {
        Self {
            provider,
            tools,
            observer,
            clock,
            cancel,
            stop,
            retry,
            request,
            pricing,
            calls,
        }
    }

    /// Runs one task to its outcome.
    ///
    /// Never fails: every way a run can go wrong is a stop reason on the
    /// outcome, so a caller always gets a `FinishedRun` and telemetry always
    /// agrees with what came back.
    pub async fn run(&mut self, context: RunContext, prompts: Prompts) -> FinishedRun {
        let started = self.clock.now();
        let capture = context.capture_content;
        let run_id = context.run_id.clone();

        let setup = RunSetup {
            run_id: run_id.clone(),
            labels: RunLabels::default(),
            model: self.provider.model().clone(),
            endpoint: self.provider.endpoint(),
            tools: self.tools.specs().iter().map(|s| s.name.clone()).collect(),
            completion: self.tools.completion(),
            max_turns: self.stop.max_turns,
            timeout: self.stop.timeout,
            request: self.request.clone(),
        };

        self.emit(
            &run_id,
            EventKind::RunStarted {
                context: Box::new(context.clone()),
                model: setup.model.clone(),
                endpoint: setup.endpoint.clone(),
                tools: self.tools.specs().to_vec(),
                system_prompt: capture.then(|| prompts.system().to_owned()),
                prompt: capture.then(|| prompts.task().to_owned()),
            },
        )
        .await;

        let run = Run::start(setup, prompts);
        let (ending, stopped) = self.drive(&run_id, run, started, capture).await;

        let rates = self.pricing.as_ref().map(Pricing::rates);
        let cost = self.pricing.as_ref().and_then(|p| p.cost(&ending.spent()));
        let finished = ending.finish(stopped, self.elapsed(started), rates, cost);

        self.emit(
            &run_id,
            EventKind::RunFinished {
                context: Box::new(context),
                summary: Box::new(finished.summary.clone()),
            },
        )
        .await;
        finished
    }

    /// The loop proper, split out so `run` can close the record whatever
    /// happens here.
    async fn drive(
        &self,
        run_id: &RunId,
        mut run: Run,
        started: Instant,
        capture: bool,
    ) -> (Ending, Stopped) {
        let mode = self.tools.completion();
        let mut bytes = RequestBytes::new(run.transcript().system(), self.tools.specs());
        loop {
            if let Some(stopped) = self.before_call(&run, started) {
                return (Ending::Waiting(run), stopped);
            }

            let turn = u32::try_from(run.transcript().turns().len())
                .unwrap_or(u32::MAX)
                .saturating_add(1);
            self.emit(run_id, EventKind::TurnStarted { turn }).await;

            let answered = match self.call(run_id, &mut run, &mut bytes, turn, started).await {
                Ok(answered) => answered,
                Err(stopped) => return (Ending::Waiting(run), stopped),
            };

            let pending = match run.responded(answered.response, answered.began, answered.latency) {
                Responded::Final(done) => {
                    let reason = self.stop.after_final(&done.turn().record().finish, mode);
                    self.report_response(run_id, turn, done.turn(), capture)
                        .await;
                    return (Ending::Final(done), Stopped::just(reason));
                }
                Responded::Pending(pending) => pending,
            };
            let reason = self.stop.after_response(
                &pending.turn().record().finish,
                mode,
                pending.calls(mode),
            );
            self.report_response(run_id, turn, pending.turn(), capture)
                .await;
            if let Some(reason) = reason {
                let stopped = Stopped {
                    reason,
                    error: None,
                    structured: pending.completed_with(mode).cloned(),
                };
                return (Ending::Pending(pending), stopped);
            }
            // Point R reads only the response, so it lets a run go on whose
            // time the response used up. No tool starts in such a run: its
            // calls stay unanswered, as at any stop ahead of a tool phase.
            if self.stop.time_left(self.elapsed(started)).is_none() {
                return (Ending::Pending(pending), Stopped::just(StopReason::Timeout));
            }

            run = self
                .run_tools(run_id, pending, turn, started, capture)
                .await;

            if self.cancel.is_cancelled() {
                return (Ending::Waiting(run), Stopped::just(StopReason::Cancelled));
            }
            if let Some(reason) = self.stop.after_tools(&self.progress(&run, started)) {
                return (Ending::Waiting(run), Stopped::just(reason));
            }
        }
    }

    /// Tells the observer what the provider call behind `recorded` returned.
    async fn report_response(&self, run_id: &RunId, turn: u32, recorded: &Turn, capture: bool) {
        let record = recorded.record();
        self.emit(
            run_id,
            EventKind::ProviderCallFinished {
                turn,
                attempt: record.attempts,
                record: Box::new(record.clone()),
                response: capture.then(|| recorded.response().to_vec()),
            },
        )
        .await;
    }

    /// Point A: why the run makes no provider call now, if it makes none.
    ///
    /// Cancellation is polled first, because the policy never sees it.
    fn before_call(&self, run: &Run, started: Instant) -> Option<Stopped> {
        if self.cancel.is_cancelled() {
            return Some(Stopped::just(StopReason::Cancelled));
        }
        self.stop
            .before_call(&self.progress(run, started))
            .map(Stopped::just)
    }

    /// One provider call, with its retries.
    ///
    /// The attempt's own timing comes back with the response, because the
    /// turn records when the attempt began and how long it took, and only
    /// this function is in a position to know either.
    ///
    /// A retry is a provider call, so point A is asked before each one as it
    /// was before the first attempt, once the wait is over.
    async fn call(
        &self,
        run_id: &RunId,
        run: &mut Run,
        bytes: &mut RequestBytes,
        turn: u32,
        started: Instant,
    ) -> Result<Answered, Stopped> {
        let mut attempt = 1;
        loop {
            let began = self.clock.now();
            let began_at = began.saturating_duration_since(started);
            let result = {
                let messages = run.messages();
                let request_bytes = bytes.measure(&messages);
                self.emit(
                    run_id,
                    EventKind::ProviderCallStarted {
                        turn,
                        attempt,
                        request_bytes,
                    },
                )
                .await;
                self.provider
                    .complete(ProviderRequest {
                        system: run.transcript().system(),
                        messages: &messages,
                        tools: self.tools.specs(),
                        max_tokens: self.request.max_tokens,
                        temperature: self.request.temperature,
                        thinking: self.request.thinking,
                        effort: self.request.effort,
                        seed: self.request.seed,
                        deadline: self.provider_deadline(began_at),
                    })
                    .await
            };
            let latency = self.clock.now().saturating_duration_since(began);

            let error = match result {
                Ok(response) => {
                    return Ok(Answered {
                        response,
                        began: began_at,
                        latency,
                    });
                }
                Err(error) => error,
            };
            run.failed_attempt(latency, error.usage);

            let next =
                self.after_failure(run, &error, run_id.salt(turn, attempt), attempt, started);
            self.emit(
                run_id,
                EventKind::ProviderCallFailed {
                    turn,
                    attempt,
                    error,
                    retry: next.as_ref().ok().copied(),
                },
            )
            .await;

            self.clock.sleep(next?).await;
            if let Some(stopped) = self.before_call(run, started) {
                return Err(stopped);
            }
            attempt += 1;
        }
    }

    /// The deadline of a provider attempt that begins `elapsed` into the
    /// run: the shorter of the provider timeout and the time the run has
    /// left, so that no attempt is given longer than the run has.
    ///
    /// Point A found time left before the attempt, on a reading of the clock
    /// taken ahead of this one. Should the last of it go between the two,
    /// the attempt has no time at all, and the adapter reports the deadline
    /// as reached.
    fn provider_deadline(&self, elapsed: Duration) -> Duration {
        self.stop
            .time_left(elapsed)
            .map_or(Duration::ZERO, |left| left.min(self.calls.provider_timeout))
    }

    /// What follows a failed attempt: the wait before the next one, or why
    /// there won't be one.
    ///
    /// A failure another attempt could answer is held to point A before the
    /// retry policy is asked. The attempt may have used up the run's time or
    /// its token budget, and that's why the run stops then, whether or not
    /// the call has a retry left: one state gives one stop reason. A failure
    /// no attempt could answer ends the run as itself.
    fn after_failure(
        &self,
        run: &Run,
        error: &ProviderError,
        salt: u64,
        attempt: u32,
        started: Instant,
    ) -> Result<Duration, Stopped> {
        if error.kind.is_retryable()
            && let Some(stopped) = self.before_call(run, started)
        {
            return Err(stopped);
        }
        match self
            .retry
            .next(attempt, error.kind, error.retry_after, salt)
        {
            Some(wait) if self.stop.allows_wait(self.elapsed(started), wait) => Ok(wait),
            // A wait that would carry the run past its own timeout isn't
            // taken: the run stops now rather than sleeping up to a limit
            // it's known to reach.
            Some(_) => Err(Stopped::just(StopReason::Timeout)),
            None => Err(Stopped::with(
                stop_reason_for(error.kind),
                error.message().to_owned(),
            )),
        }
    }

    /// The tool phase of one turn: every call answered, in the groups the
    /// tools' concurrency sets, and the run back waiting for its next
    /// response.
    async fn run_tools(
        &self,
        run_id: &RunId,
        pending: Pending,
        turn: u32,
        started: Instant,
        capture: bool,
    ) -> Run {
        let concurrency = |call: &ToolUse| self.tools.concurrency(call);
        let schedule = Schedule {
            max_concurrent: self.calls.max_concurrent_tool_calls,
            concurrency: &concurrency,
        };
        pending
            .answer(schedule, |call| {
                self.answer(run_id, call, turn, started, capture)
            })
            .await
    }

    /// One tool call, from the event that opens it to the event that closes
    /// it, or a call that's never run because its turn came when the run
    /// had no time left.
    ///
    /// The clock is read once, when the call's turn comes, which for a call
    /// of a later group is after the groups ahead of it. The call's start and
    /// its deadline are both taken from that reading, so the deadline is the
    /// time the run had left when the call is said to have started.
    ///
    /// Nothing is started for a call that's never run, so an observer isn't
    /// told of it and the output cap, which cuts what a call wrote, has
    /// nothing of it to cut.
    async fn answer(
        &self,
        run_id: &RunId,
        call: ToolUse,
        turn: u32,
        started: Instant,
        capture: bool,
    ) -> Answer {
        let began = self.clock.now();
        let began_at = began.saturating_duration_since(started);
        let Some(left) = self.stop.time_left(began_at) else {
            return Answer::measured(
                ToolCallStatus::NotRun,
                KeptOutput::whole("the run reached its timeout before this call started"),
                None,
                began_at,
                Duration::ZERO,
            );
        };

        let source = self.tools.source(&call.name).cloned();
        let input = match &call.input {
            ToolInput::Json(value) => Some(value),
            ToolInput::Unparsed(_) => None,
        };
        self.emit(
            run_id,
            EventKind::ToolCallStarted {
                turn,
                call_id: call.id.clone(),
                name: call.name.clone(),
                source: source.clone(),
                input_bytes: call.input_bytes(),
                input: capture.then(|| input.cloned()).flatten(),
            },
        )
        .await;

        let trace_context = self.observer.trace_context(&call.id);
        let settled = self.settle(&call, source, left, trace_context).await;
        let latency = self.clock.now().saturating_duration_since(began);

        let answer = Answer::measured(
            settled.status,
            settled.output,
            self.calls.output_cap,
            began_at,
            latency,
        );
        self.emit(
            run_id,
            EventKind::ToolCallFinished {
                turn,
                call_id: call.id,
                status: answer.status().clone(),
                latency_ms: answer.latency_ms(),
                output_bytes: answer.output_bytes(),
                truncated_from_bytes: answer.truncated_from_bytes(),
                mcp: settled.mcp,
                output: capture.then(|| answer.content().to_vec()),
            },
        )
        .await;
        answer
    }

    /// What became of one call that has `left` to run in, and what the model
    /// is sent back.
    ///
    /// The name is resolved before the arguments are read, so a call to a
    /// name this run doesn't offer is `Unknown` whether or not its arguments
    /// parsed.
    ///
    /// The completion call gets here only when it didn't complete the run:
    /// its arguments didn't parse, or it wasn't the response's only call.
    /// Which call that is is the mode's to say, as it was at point R, so a
    /// tool an executor serves under the name in natural mode runs like any
    /// other.
    async fn settle(
        &self,
        call: &ToolUse,
        source: Option<lablet_model::ToolSource>,
        left: Duration,
        trace_context: Option<crate::TraceContext>,
    ) -> Settled {
        use lablet_model::ToolCallEnd;

        let Some(source) = source else {
            return Settled::local(
                ToolCallStatus::Unknown,
                &format!("no tool named {} is offered by this run", call.name),
            );
        };
        // The arguments are read here rather than handed in already unwrapped.
        // A caller that matched them into an `Option` leaves this function to
        // unwrap it again, and the arm that can't then happen is a region no
        // test can reach.
        let input = match &call.input {
            ToolInput::Unparsed(text) => {
                return Settled::local(
                    ToolCallStatus::MalformedInput,
                    &format!(
                        "the arguments weren't valid JSON, so {} wasn't called: {text}",
                        call.name
                    ),
                );
            }
            ToolInput::Json(_) if self.tools.completion().intercepts(&call.name) => {
                return Settled::local(
                    ToolCallStatus::Rejected,
                    &format!(
                        "call {} on its own, once your other calls have returned",
                        call.name
                    ),
                );
            }
            ToolInput::Json(value) => value.clone(),
        };

        match self
            .tools
            .execute(ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                input,
                deadline: left,
                keep: self.calls.output_cap.map(OutputCap::keeps),
                trace_context,
            })
            .await
        {
            Ok(output) => {
                let ended = if output.is_error {
                    ToolCallEnd::ToolError
                } else {
                    ToolCallEnd::Ok
                };
                Settled {
                    status: ToolCallStatus::ran(source, ended),
                    output: output.output,
                    mcp: output.mcp,
                }
            }
            Err(error) => {
                let status = error.kind.ended().map_or(ToolCallStatus::Unknown, |ended| {
                    ToolCallStatus::ran(source, ended)
                });
                Settled {
                    status,
                    output: KeptOutput::whole(error.message()),
                    mcp: error.mcp,
                }
            }
        }
    }

    fn progress(&self, run: &Run, started: Instant) -> Progress {
        run.progress(self.elapsed(started))
    }

    fn elapsed(&self, started: Instant) -> Duration {
        self.clock.now().saturating_duration_since(started)
    }

    async fn emit(&self, run_id: &RunId, kind: EventKind) {
        self.observer
            .on(RunEvent {
                run_id: run_id.clone(),
                kind,
            })
            .await;
    }
}

impl Stopped {
    const fn just(reason: StopReason) -> Self {
        Self {
            reason,
            error: None,
            structured: None,
        }
    }

    const fn with(reason: StopReason, error: String) -> Self {
        Self {
            reason,
            error: Some(error),
            structured: None,
        }
    }
}

/// The running measure of how much the conversation has grown.
///
/// Every message is measured once, the first time it's rendered whole, so a
/// provider call costs the serialisation of what the last turn added rather
/// than of the whole conversation. The system prompt and the tool specs are
/// measured once, when the run starts.
struct RequestBytes {
    fixed: u64,
    settled: u64,
    measured: usize,
}

impl RequestBytes {
    fn new(system: &str, specs: &[lablet_model::ToolSpec]) -> Self {
        let specs: u64 = specs
            .iter()
            .map(|spec| serde_json::to_string(spec).map_or(0, |json| json.len() as u64))
            .sum();
        Self {
            fixed: system.len() as u64 + specs,
            settled: 0,
            measured: 0,
        }
    }

    /// The size of this request, measuring only what hasn't settled. The last
    /// message can still grow — a user message gains the results of the tool
    /// calls it answers — so it's measured afresh every time and never
    /// settled.
    fn measure(&mut self, messages: &[Message<'_>]) -> u64 {
        let settled_count = messages.len().saturating_sub(1);
        for message in messages.iter().take(settled_count).skip(self.measured) {
            self.settled += size_of_message(message);
        }
        self.measured = settled_count;
        let last = messages.last().map_or(0, size_of_message);
        self.fixed + self.settled + last
    }
}

fn size_of_message(message: &Message<'_>) -> u64 {
    serde_json::to_string(message).map_or(0, |json| json.len() as u64)
}

/// What a failure the loop has stopped trying on means for the run.
///
/// It's here rather than on [`ProviderErrorKind`] because it depends on the
/// loop's context: a retryable failure becomes `retries_exhausted` only once
/// the budget is spent, where a fatal one is `provider_error` the first time.
const fn stop_reason_for(kind: ProviderErrorKind) -> StopReason {
    match kind {
        // Both are failures another attempt could have answered, and none did.
        ProviderErrorKind::Retryable | ProviderErrorKind::Malformed => StopReason::RetriesExhausted,
        ProviderErrorKind::ContextExhausted => StopReason::ContextExhausted,
        // A rejected key is a run that failed, like any other request the
        // provider won't take.
        ProviderErrorKind::Auth | ProviderErrorKind::Fatal => StopReason::ProviderError,
    }
}
