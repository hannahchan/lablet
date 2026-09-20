//! The loop: one run, from its first provider call to its outcome.

use std::sync::Arc;
use std::time::{Duration, Instant};

use lablet_model::{
    FinishedRun, Message, Progress, Prompts, ProviderErrorKind, ProviderResponse, RequestParams,
    Run, RunContext, RunSetup, StopReason, ToolCallOutcome, ToolCallStatus, ToolInput, ToolUse,
};
use lablet_policy::{Pricing, RetryPolicy, StopPolicy};

use crate::{
    Cancellation, Clock, EventKind, ModelProvider, ProviderRequest, RunEvent, RunObserver,
    ToolCall, ToolSet,
};

/// What bounds one call, which is not a stop decision: the run's limits are
/// the stop policy's business and these are the adapters'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallLimits {
    /// How long one provider attempt may take.
    pub provider_timeout: Duration,
    /// How long one tool call may take.
    pub tool_timeout: Duration,
    /// The cap on what a tool's output may be before the model sees it.
    pub max_tool_output_bytes: Option<u64>,
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

/// Why the loop stopped, and what it has to say about it.
struct Stopped {
    reason: StopReason,
    error: Option<String>,
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
            model: self.provider.model().clone(),
            endpoint: self.provider.endpoint(),
            tools: self.tools.specs().iter().map(|s| s.name.clone()).collect(),
            completion: self.stop.completion,
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
                system_prompt: capture.then(|| prompts.system.clone()),
                prompt: capture.then(|| prompts.task.clone()),
            },
        )
        .await;

        let mut run = Run::start(setup, prompts);
        let mut bytes = RequestBytes::new(run.transcript().system(), self.tools.specs());
        let mut structured = None;

        let stopped = self
            .drive(
                &run_id,
                &mut run,
                &mut bytes,
                &mut structured,
                started,
                capture,
            )
            .await;

        let rates = self.pricing.as_ref().map(Pricing::rates);
        let cost = self.pricing.as_ref().and_then(|p| p.cost(&run.usage()));
        let finished = run.finish(
            stopped.reason,
            self.elapsed(started),
            structured,
            stopped.error,
            rates,
            cost,
        );

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
        run_id: &lablet_model::RunId,
        run: &mut Run,
        bytes: &mut RequestBytes,
        structured: &mut Option<serde_json::Value>,
        started: Instant,
        capture: bool,
    ) -> Stopped {
        loop {
            if self.cancel.is_cancelled() {
                return Stopped::just(StopReason::Cancelled);
            }
            if let Some(reason) = self.stop.before_call(&self.progress(run, started)) {
                return Stopped::just(reason);
            }

            let turn_index = u32::try_from(run.transcript().turns().len())
                .unwrap_or(u32::MAX)
                .saturating_add(1);
            self.emit(run_id, EventKind::TurnStarted { turn: turn_index })
                .await;

            let response = match self.call(run_id, run, bytes, turn_index, started).await {
                Ok(response) => response,
                Err(stopped) => return stopped,
            };

            let call_started = self.elapsed(started);
            let turn = match run.responded(response, call_started, Duration::ZERO) {
                Ok(turn) => turn,
                Err(refusal) => return Stopped::defect(&refusal),
            };
            let record = turn.record().clone();
            let calls: Vec<ToolUse> = turn.tool_uses().cloned().collect();
            let reason = self
                .stop
                .after_response(&record.finish, turn.calls(self.stop.completion));
            let response_blocks = capture.then(|| turn.response().to_vec());

            self.emit(
                run_id,
                EventKind::ProviderCallFinished {
                    turn: turn_index,
                    attempt: record.attempts,
                    record: Box::new(record.clone()),
                    response: response_blocks,
                },
            )
            .await;

            *structured = task_complete_argument(&self.tools, &calls).or(structured.take());

            if let Some(reason) = reason {
                return Stopped::just(reason);
            }

            let outcomes = self
                .run_tools(run_id, &calls, turn_index, started, capture)
                .await;
            if let Err(refusal) = run.tool_calls(outcomes) {
                return Stopped::defect(&refusal);
            }

            if self.cancel.is_cancelled() {
                return Stopped::just(StopReason::Cancelled);
            }
            if let Some(reason) = self.stop.after_tools(&self.progress(run, started)) {
                return Stopped::just(reason);
            }
        }
    }

    /// One provider call, with its retries.
    async fn call(
        &self,
        run_id: &lablet_model::RunId,
        run: &mut Run,
        bytes: &mut RequestBytes,
        turn: u32,
        started: Instant,
    ) -> Result<ProviderResponse, Stopped> {
        let mut attempt = 1;
        loop {
            let began = self.clock.now();
            let request_bytes = {
                let messages = run.messages();
                bytes.measure(&messages)
            };
            self.emit(
                run_id,
                EventKind::ProviderCallStarted {
                    turn,
                    attempt,
                    request_bytes,
                },
            )
            .await;

            let result = {
                let messages = run.messages();
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
                        deadline: self.calls.provider_timeout,
                    })
                    .await
            };
            let latency = self.clock.now().saturating_duration_since(began);

            let error = match result {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            run.failed_attempt(latency);

            let wait = self.retry.next(attempt, error.kind);
            let elapsed = self.elapsed(started);
            let will_retry = wait.is_some_and(|wait| self.stop.allows_wait(elapsed, wait));
            self.emit(
                run_id,
                EventKind::ProviderCallFailed {
                    turn,
                    attempt,
                    error: error.clone(),
                    will_retry,
                    backoff: will_retry.then(|| wait.unwrap_or_default()),
                },
            )
            .await;

            match wait {
                Some(wait) if will_retry => {
                    self.clock.sleep(wait).await;
                    attempt += 1;
                }
                // A wait that would carry the run past its own timeout isn't
                // taken: the run stops now rather than sleeping up to a limit
                // it's known to reach.
                Some(_) => return Err(Stopped::just(StopReason::Timeout)),
                None => return Err(Stopped::with(stop_reason_for(error.kind), error.message)),
            }
        }
    }

    /// The tool phase of one turn, in call order.
    async fn run_tools(
        &self,
        run_id: &lablet_model::RunId,
        calls: &[ToolUse],
        turn: u32,
        started: Instant,
        capture: bool,
    ) -> Vec<ToolCallOutcome> {
        let mut outcomes = Vec::with_capacity(calls.len());
        for call in calls {
            let source = self.tools.source(&call.name).cloned();
            let input = match &call.input {
                ToolInput::Json(value) => Some(value.clone()),
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
                    input: capture.then(|| input.clone()).flatten(),
                },
            )
            .await;

            let began = self.clock.now();
            let trace_context = self.observer.trace_context(&call.id);
            let (status, content) = self.settle(call, source, input, trace_context).await;
            let latency = self.clock.now().saturating_duration_since(began);

            let outcome = ToolCallOutcome::measured(
                call.id.clone(),
                status,
                content,
                self.calls.max_tool_output_bytes,
                self.elapsed(started).saturating_sub(latency),
                latency,
            );
            self.emit(
                run_id,
                EventKind::ToolCallFinished {
                    turn,
                    call_id: outcome.call_id.clone(),
                    status: outcome.status.clone(),
                    latency_ms: outcome.latency_ms,
                    output_bytes: outcome.output_bytes(),
                    truncated_from_bytes: outcome.truncated_from_bytes,
                    mcp: None,
                    output: capture.then(|| outcome.content.clone()),
                },
            )
            .await;
            outcomes.push(outcome);
        }
        outcomes
    }

    /// What became of one call, and what the model is sent back.
    ///
    /// The name is resolved before the arguments are read, so a call to a
    /// name this run doesn't offer is `Unknown` whether or not its arguments
    /// parsed.
    async fn settle(
        &self,
        call: &ToolUse,
        source: Option<lablet_model::ToolSource>,
        input: Option<serde_json::Value>,
        trace_context: Option<crate::TraceContext>,
    ) -> (ToolCallStatus, Vec<lablet_model::ToolResultContent>) {
        use lablet_model::{ToolCallEnd, ToolResultContent};

        let Some(source) = source else {
            return (
                ToolCallStatus::Unknown,
                vec![ToolResultContent::Text(format!(
                    "no tool named {} is offered by this run",
                    call.name
                ))],
            );
        };
        let Some(input) = input else {
            let ToolInput::Unparsed(text) = &call.input else {
                unreachable!("input is None only for arguments that didn't parse")
            };
            return (
                ToolCallStatus::MalformedInput,
                vec![ToolResultContent::Text(format!(
                    "the arguments weren't valid JSON, so {} wasn't called: {text}",
                    call.name
                ))],
            );
        };

        match self
            .tools
            .execute(ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                input,
                deadline: self.calls.tool_timeout,
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
                (ToolCallStatus::ran(source, ended), output.content)
            }
            Err(error) => {
                let status = error.kind.ended().map_or(ToolCallStatus::Unknown, |ended| {
                    ToolCallStatus::ran(source, ended)
                });
                (status, vec![ToolResultContent::Text(error.message)])
            }
        }
    }

    fn progress(&self, run: &Run, started: Instant) -> Progress {
        run.progress(self.elapsed(started))
    }

    fn elapsed(&self, started: Instant) -> Duration {
        self.clock.now().saturating_duration_since(started)
    }

    async fn emit(&self, run_id: &lablet_model::RunId, kind: EventKind) {
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
        }
    }

    const fn with(reason: StopReason, error: String) -> Self {
        Self {
            reason,
            error: Some(error),
        }
    }

    /// The transcript refused a sequence this loop can't produce. The run
    /// still has to come back with an outcome, so the refusal becomes the
    /// error of a `provider_error`.
    fn defect(refusal: &lablet_model::TranscriptError) -> Self {
        Self::with(
            StopReason::ProviderError,
            format!("lablet defect: the transcript refused the loop's own sequence: {refusal}"),
        )
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
        ProviderErrorKind::Fatal => StopReason::ProviderError,
    }
}

/// The argument of a `task_complete` call in this response, if it made one.
fn task_complete_argument(tools: &ToolSet, calls: &[ToolUse]) -> Option<serde_json::Value> {
    calls
        .iter()
        .find(|call| tools.is_task_complete(&call.name))
        .and_then(|call| match &call.input {
            ToolInput::Json(value) => Some(value.clone()),
            ToolInput::Unparsed(_) => None,
        })
}

#[cfg(test)]
mod tests;
