//! The loop: one run, from its first provider call to its outcome.

use std::future::{Future, poll_fn};
use std::num::NonZeroU32;
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lablet_model::{
    Answer, CacheScope, Cost, Effort, Endpoint, Final, FinishedRun, KeptOutput, Message, ModelRef,
    OutputCap, Pending, Progress, Prompts, ProviderErrorKind, ProviderResponse, Rates,
    RequestParams, Responded, Run, RunContext, RunId, RunSetup, Schedule, Secrets, StopReason,
    ToolCallEnd, ToolCallStatus, ToolInput, ToolSource, ToolUse, Turn, Usage, whole_ms,
};
use lablet_policy::{Pricing, RetryPolicy, StopPolicy};
use opentelemetry::Context;
use opentelemetry::global::{BoxedSpan, BoxedTracer};
use opentelemetry::trace::{
    FutureExt as _, Span as _, SpanKind, Status, TraceContextExt as _, Tracer as _,
};

use crate::shown::{OfferedSpecs, system_prompt_digest};
use crate::telemetry::conversation::{Conversation, Exchange, tool_definitions, tool_result};
use crate::telemetry::generated::{
    GenAiClientInferenceOperationDetails,
    GenAiClientInferenceOperationDetailsGenAiOperationName as Operation,
    GenAiClientOperationException, Join, LabletChat, LabletChatErrorType, LabletExecuteTool,
    LabletExecuteToolNetworkTransport, LabletRetry,
};
use crate::telemetry::spellings::tool_error_type;
use crate::telemetry::{Logger, count_of};
use crate::{
    Cancellation, Clock, McpCallMeta, ModelProvider, ProviderError, ProviderRequest, ToolCall,
    ToolSet, bounded,
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
    /// What every span of a run is opened through. Never the global tracer:
    /// one process may hold several loops, each with its own destinations,
    /// and tests run in parallel.
    tracer: BoxedTracer,
    /// What every record of a run goes through, for the same reason.
    logger: Box<dyn Logger>,
    clock: Arc<dyn Clock>,
    cancel: Arc<dyn Cancellation>,
    stop: StopPolicy,
    retry: RetryPolicy,
    request: RequestParams,
    pricing: Option<Pricing>,
    calls: CallLimits,
    /// The values of lablet's own secrets, handed to every executor with
    /// its call and cut out of every text the loop writes itself.
    secrets: Arc<Secrets>,
}

/// What the status of a chat span says when the run was cancelled while
/// the attempt was in flight. Nothing failed, so there's no error message
/// to carry.
const CANCELLED_IN_FLIGHT: &str = "the run was cancelled while the attempt was in flight";

/// What every signal of one run shares, fixed when the run starts.
struct Running {
    run_id: RunId,
    /// The reading of the clock the run started at, which every offset is
    /// measured from.
    started: Instant,
    /// When the run started on the wall clock, which every span and record
    /// is timed from.
    started_unix_ms: u64,
    /// Whether the run's content goes into the content records.
    capture: bool,
    /// The context `run` was called in: the parent of every span, and the
    /// context the record of the tools offered is written in. It's taken
    /// once and named wherever a span opens, so that no span's parent
    /// depends on what's current when its future is polled.
    parent: Context,
    /// The keys a consumer groups by, on every span and record of the run.
    join: Join,
    /// What every chat span says the run's provider calls asked for.
    asked: Asked,
}

impl Running {
    /// The instant `offset_ms` into the run.
    ///
    /// The sum is of milliseconds in a `u64`, and the clocks of the
    /// platforms lablet runs on count seconds in an `i64`, so every instant
    /// this can name is one the clock can say.
    fn at(&self, offset_ms: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.started_unix_ms.saturating_add(offset_ms))
    }

    /// How far into the run the reading `at` is.
    fn offset(&self, at: Instant) -> Duration {
        at.saturating_duration_since(self.started)
    }

    /// The chat span of the attempt `attempt` of the turn `turn`, as far as
    /// what it asked for says it; what it got comes when the attempt ends.
    fn chat(&self, turn: u32, attempt: u32, request_bytes: u64) -> LabletChat {
        let asked = &self.asked;
        LabletChat {
            join: self.join.clone(),
            gen_ai_provider_name: asked.provider_name.clone(),
            gen_ai_request_max_tokens: asked.max_tokens,
            gen_ai_request_model: asked.model.clone(),
            lablet_attempt: i64::from(attempt),
            lablet_request_bytes: count_of(request_bytes),
            lablet_turn: i64::from(turn),
            error_type: None,
            gen_ai_request_reasoning_level: asked.reasoning_level.clone(),
            gen_ai_request_seed: asked.seed,
            gen_ai_request_temperature: asked.temperature,
            gen_ai_response_finish_reasons: None,
            gen_ai_response_id: None,
            gen_ai_response_model: None,
            gen_ai_usage_cache_read_input_tokens: None,
            gen_ai_usage_cache_write_input_tokens: None,
            gen_ai_usage_input_tokens: None,
            gen_ai_usage_output_tokens: None,
            gen_ai_usage_reasoning_output_tokens: None,
            server_address: asked.server_address.clone(),
            server_port: asked.server_port,
        }
    }

    /// A content record that belongs to a span of the turn `turn`, or to
    /// the run when there's no turn yet, with nothing in it but the run's
    /// keys and its operation.
    fn content(
        &self,
        operation: Operation,
        turn: Option<u32>,
    ) -> GenAiClientInferenceOperationDetails {
        GenAiClientInferenceOperationDetails {
            join: self.join.clone(),
            gen_ai_operation_name: operation,
            gen_ai_input_messages: None,
            gen_ai_output_messages: None,
            gen_ai_system_instructions: None,
            gen_ai_tool_call_arguments: None,
            gen_ai_tool_call_id: None,
            gen_ai_tool_call_result: None,
            gen_ai_tool_definitions: None,
            gen_ai_tool_name: None,
            lablet_turn: turn.map(i64::from),
        }
    }
}

/// What every provider call of a run asks for, as its chat span says it.
struct Asked {
    provider_name: String,
    model: String,
    max_tokens: i64,
    temperature: Option<f64>,
    seed: Option<i64>,
    reasoning_level: Option<String>,
    server_address: Option<String>,
    server_port: Option<i64>,
}

impl Asked {
    /// What calls to `model` at `endpoint` ask for with `request`.
    fn of(model: &ModelRef, endpoint: Option<Endpoint>, request: &RequestParams) -> Self {
        let (server_address, server_port) = match endpoint {
            Some(Endpoint { host, port }) => (Some(host), Some(i64::from(port))),
            None => (None, None),
        };
        Self {
            provider_name: model.api.provider().as_str().to_owned(),
            model: model.name.clone(),
            max_tokens: i64::from(request.max_tokens),
            temperature: request.temperature,
            seed: request.seed,
            reasoning_level: request.effort.map(Effort::as_str).map(str::to_owned),
            server_address,
            server_port,
        }
    }
}

/// Sets what the provider said the attempt used on `chat`. A count the
/// provider didn't report stays off the span.
fn spent(chat: &mut LabletChat, usage: &Usage) {
    chat.gen_ai_usage_input_tokens = Some(count_of(usage.input_tokens));
    chat.gen_ai_usage_output_tokens = Some(count_of(usage.output_tokens));
    chat.gen_ai_usage_reasoning_output_tokens = usage.reasoning_output_tokens.map(count_of);
    chat.gen_ai_usage_cache_read_input_tokens = usage.cache_read_tokens.map(count_of);
    chat.gen_ai_usage_cache_write_input_tokens = usage.cache_write_tokens.map(count_of);
}

/// The `mcp.*`, `jsonrpc`, `rpc` and `network.transport` attributes of a
/// tool span, which a call over MCP carries back and any other call lacks.
struct OverMcp {
    method: Option<String>,
    session_id: Option<String>,
    protocol_version: Option<String>,
    jsonrpc_request_id: Option<String>,
    rpc_status_code: Option<String>,
    transport: Option<LabletExecuteToolNetworkTransport>,
}

impl OverMcp {
    fn of(mcp: Option<&McpCallMeta>) -> Self {
        match mcp {
            Some(mcp) => Self {
                method: Some(mcp.method.clone()),
                session_id: mcp.session_id.clone(),
                protocol_version: mcp.protocol_version.clone(),
                jsonrpc_request_id: mcp.jsonrpc_request_id.clone(),
                rpc_status_code: mcp.rpc_status_code.clone(),
                transport: Some(mcp.transport.into()),
            },
            None => Self {
                method: None,
                session_id: None,
                protocol_version: None,
                jsonrpc_request_id: None,
                rpc_status_code: None,
                transport: None,
            },
        }
    }
}

/// A call whose turn has come: the one reading of the clock it's timed
/// from, how far into the run that is, and the time the run has left at it,
/// which the check that admitted the call found to be some.
struct Began {
    at: Instant,
    offset: Duration,
    left: Duration,
}

/// A chat span opened for an attempt, and what it's filled from when the
/// attempt ends: the struct that named it, which says what the attempt
/// asked for and gains what it got, which attempt of which turn it was, and
/// the reading it's timed from.
struct Opened {
    span: BoxedSpan,
    chat: LabletChat,
    turn: u32,
    attempt: u32,
    began: Began,
}

/// A provider call that answered, with how long the attempt that did took
/// and the span it answered in.
struct Answered {
    response: ProviderResponse,
    latency: Duration,
    opened: Opened,
}

/// A tool call that's run: the call, its turn, where its tool comes from
/// and its arguments when they parsed, which the span and the record of
/// the call are filled from.
struct Called<'a> {
    call: &'a ToolUse,
    turn: u32,
    source: Option<ToolSource>,
    input: Option<&'a serde_json::Value>,
}

/// A tool call that's never run: how far into the run its turn came, and
/// why nothing was started for it.
struct Unstarted {
    offset: Duration,
    why: &'static str,
}

/// What became of one tool call. The loop is the only hop between an executor
/// and the call's span, so the call's transport metadata travels with its
/// result or the `mcp.*` attributes have no way to be set.
struct Settled {
    status: ToolCallStatus,
    output: KeptOutput,
    mcp: Option<McpCallMeta>,
}

impl Settled {
    /// A call the loop answered itself, so no executor was reached.
    const fn local(status: ToolCallStatus, output: KeptOutput) -> Self {
        Self {
            status,
            output,
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
    /// A loop built from its ports, its telemetry and its policies.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "a composition root assembles this once; every argument is a distinct port or policy, so none can be given in another's place"
    )]
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        tools: Arc<ToolSet>,
        tracer: BoxedTracer,
        logger: Box<dyn Logger>,
        clock: Arc<dyn Clock>,
        cancel: Arc<dyn Cancellation>,
        stop: StopPolicy,
        retry: RetryPolicy,
        request: RequestParams,
        pricing: Option<Pricing>,
        calls: CallLimits,
        secrets: Arc<Secrets>,
    ) -> Self {
        Self {
            provider,
            tools,
            tracer,
            logger,
            clock,
            cancel,
            stop,
            retry,
            request,
            pricing,
            calls,
            secrets,
        }
    }

    /// Runs one task to its outcome.
    ///
    /// Never fails: every way a run can go wrong is a stop reason on the
    /// outcome, so a caller always gets a `FinishedRun` and telemetry always
    /// agrees with what came back.
    ///
    /// Every span of the run is a child of the context this is called in,
    /// which is the root span's when the composition root runs the loop.
    pub async fn run(&mut self, context: RunContext, prompts: Prompts) -> FinishedRun {
        let started = self.clock.now();
        let parent = Context::current();
        let capture = context.capture_content;
        let run_id = context.run_id.clone();

        let specs = OfferedSpecs::measure(self.tools.specs());
        let bytes = RequestBytes::new(prompts.system().len() as u64 + specs.bytes);
        let setup = RunSetup {
            run_id: run_id.clone(),
            labels: context.labels.clone(),
            model: self.provider.model().clone(),
            endpoint: self.provider.endpoint(),
            tools: self.tools.specs().iter().map(|s| s.name.clone()).collect(),
            tools_bytes: specs.bytes,
            tools_digest: specs.digest,
            system_prompt_digest: system_prompt_digest(prompts.system()),
            completion: self.tools.completion(),
            max_turns: self.stop.max_turns,
            timeout: self.stop.timeout,
            request: self.request.clone(),
        };
        let running = Running {
            run_id: run_id.clone(),
            started,
            started_unix_ms: context.started_unix_ms,
            capture,
            parent,
            join: Join::from(&context),
            asked: Asked::of(&setup.model, setup.endpoint.clone(), &self.request),
        };

        // The tools offered are content, so their record is written only
        // when the run captures it, in the context the run was called in.
        let mut conversation = if capture {
            let offered = GenAiClientInferenceOperationDetails {
                gen_ai_tool_definitions: Some(tool_definitions(self.tools.specs())),
                ..running.content(Operation::InvokeAgent, None)
            };
            self.logger
                .emit(offered.record(running.at(0), running.parent.span().span_context()));
            Some(Conversation::opening(prompts.system(), prompts.task()))
        } else {
            None
        };

        let run = Run::start(setup, prompts);
        let (ending, stopped) = self.drive(&running, run, bytes, &mut conversation).await;

        let rates = self.pricing.as_ref().map(Pricing::rates);
        let cost = self.pricing.as_ref().and_then(|p| p.cost(&ending.spent()));
        ending.finish(stopped, self.elapsed(&running), rates, cost)
    }

    /// The loop proper, split out so `run` can close the record whatever
    /// happens here.
    async fn drive(
        &self,
        running: &Running,
        mut run: Run,
        mut bytes: RequestBytes,
        conversation: &mut Option<Conversation>,
    ) -> (Ending, Stopped) {
        let mode = self.tools.completion();
        loop {
            let began = match self.admit(&run, running) {
                Ok(began) => began,
                Err(stopped) => return (Ending::Waiting(run), stopped),
            };

            let turn = u32::try_from(run.transcript().turns().len())
                .unwrap_or(u32::MAX)
                .saturating_add(1);
            let answered = match self
                .call(running, &mut run, &mut bytes, turn, began, conversation)
                .await
            {
                Ok(answered) => answered,
                Err(stopped) => return (Ending::Waiting(run), stopped),
            };
            let Answered {
                response,
                latency,
                opened,
            } = answered;

            let pending = match run.responded(response, opened.began.offset, latency) {
                Responded::Final(done) => {
                    let reason = self.stop.after_final(&done.turn().record().finish, mode);
                    self.response_came(running, done.turn(), opened, conversation);
                    return (Ending::Final(done), Stopped::just(reason));
                }
                Responded::Pending(pending) => pending,
            };
            let reason = self.stop.after_response(
                &pending.turn().record().finish,
                mode,
                pending.calls(mode),
            );
            self.response_came(running, pending.turn(), opened, conversation);
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
            if self.stop.time_left(self.elapsed(running)).is_none() {
                return (Ending::Pending(pending), Stopped::just(StopReason::Timeout));
            }

            run = self.run_tools(running, pending, turn).await;
            // The calls of a group end in any order, so the conversation
            // learns their results here, in call order, from the turn that
            // holds every outcome. A call that was never run is in the
            // transcript and nowhere else, so the conversation isn't told of
            // it.
            if let Some(conversation) = conversation.as_mut() {
                let outcomes = run.transcript().turns().last().map(Turn::tool_calls);
                for outcome in outcomes.unwrap_or_default() {
                    if outcome.status.was_started() {
                        conversation.answered(
                            &outcome.call_id,
                            &outcome.content,
                            outcome.status.is_error(),
                        );
                    }
                }
            }

            if self.cancel.is_cancelled() {
                return (Ending::Waiting(run), Stopped::just(StopReason::Cancelled));
            }
            if let Some(reason) = self.stop.after_tools(&self.progress(&run, running)) {
                return (Ending::Waiting(run), Stopped::just(reason));
            }
        }
    }

    /// The provider call behind `recorded` returned: fills and ends the
    /// attempt's span from the turn's record, which is
    /// the one place that holds the attempt's timing, its usage and its
    /// finish together.
    fn response_came(
        &self,
        running: &Running,
        recorded: &Turn,
        opened: Opened,
        conversation: &mut Option<Conversation>,
    ) {
        let record = recorded.record();
        let mut opened = opened;
        let chat = &mut opened.chat;
        chat.gen_ai_response_finish_reasons = Some(vec![record.finish.as_str().to_owned()]);
        chat.gen_ai_response_id.clone_from(&record.response_id);
        chat.gen_ai_response_model
            .clone_from(&record.response_model);
        spent(chat, &record.usage);
        let end = running.at(record.started_ms.saturating_add(record.latency_ms));
        let exchange = conversation
            .as_mut()
            .map(|conversation| conversation.responded(recorded.response()));
        self.end_chat(running, opened, Status::Unset, exchange, end);
    }

    /// Point A: whether the run makes a provider call now, and what the
    /// call is timed from when it does.
    ///
    /// One reading of the clock serves the check and the call alike: the
    /// call's start and its latency are measured from it, and its deadline
    /// is the time the run has left at it. Cancellation is polled first,
    /// because the policy never sees it, and the time left is read before
    /// the policy is asked, so that a call the policy admits is one the run
    /// has time for and the deadline is that time.
    fn admit(&self, run: &Run, running: &Running) -> Result<Began, Stopped> {
        let at = self.clock.now();
        let offset = running.offset(at);
        if self.cancel.is_cancelled() {
            return Err(Stopped::just(StopReason::Cancelled));
        }
        let Some(left) = self.stop.time_left(offset) else {
            return Err(Stopped::just(StopReason::Timeout));
        };
        match self.stop.before_call(&run.progress(offset)) {
            Some(reason) => Err(Stopped::just(reason)),
            None => Ok(Began { at, offset, left }),
        }
    }

    /// One provider call, with its retries, the first attempt of which point
    /// A admitted at `began`.
    ///
    /// The attempt's own timing comes back with the response, because the
    /// turn records when the attempt began and how long it took, and only
    /// this function is in a position to know either.
    ///
    /// A retry is a provider call, so point A is asked before each one as it
    /// was before the first attempt, once the wait is over, and the reading
    /// that admits it is the reading the retry is timed from.
    async fn call(
        &self,
        running: &Running,
        run: &mut Run,
        bytes: &mut RequestBytes,
        turn: u32,
        mut began: Began,
        conversation: &mut Option<Conversation>,
    ) -> Result<Answered, Stopped> {
        let mut attempt = 1;
        loop {
            let (opened, result) = {
                let messages = run.messages();
                let request_bytes = bytes.measure(&messages);
                // The struct names the span as the registry does, and is kept
                // to be filled with what the attempt got when it ends.
                let chat = running.chat(turn, attempt, request_bytes);
                let span = self.open(running, chat.name(), LabletChat::KIND, &began);
                let result = self
                    .unless_cancelled(self.provider.complete(ProviderRequest {
                        system: run.transcript().system(),
                        messages: &messages,
                        tools: self.tools.specs(),
                        max_tokens: self.request.max_tokens,
                        temperature: self.request.temperature,
                        thinking: self.request.thinking,
                        effort: self.request.effort,
                        seed: self.request.seed,
                        cache_key: self.cache_key(&running.run_id),
                        deadline: began.left.min(self.calls.provider_timeout),
                    }))
                    .await;
                let opened = Opened {
                    span,
                    chat,
                    turn,
                    attempt,
                    began,
                };
                (opened, result)
            };
            let latency = self.clock.now().saturating_duration_since(opened.began.at);

            let error = match result {
                Some(Ok(response)) => {
                    return Ok(Answered {
                        response,
                        latency,
                        opened,
                    });
                }
                Some(Err(error)) => error,
                None => {
                    self.attempt_dropped(running, run, opened, latency, conversation);
                    return Err(Stopped::just(StopReason::Cancelled));
                }
            };
            let next = self.attempt_failed(running, run, opened, latency, &error, conversation);

            // A wait the run was cancelled during is over then: a retry is a
            // provider call, which a cancelled run never makes.
            if self
                .unless_cancelled(self.clock.sleep(next?))
                .await
                .is_none()
            {
                return Err(Stopped::just(StopReason::Cancelled));
            }
            began = self.admit(run, running)?;
            attempt += 1;
        }
    }

    /// The attempt in `opened` failed with `error` after `latency`: the run
    /// counts it, point A and the retry policy say what follows, the span
    /// and the records say how it failed and what was decided. What follows
    /// is the wait before the next attempt, or why there's none.
    fn attempt_failed(
        &self,
        running: &Running,
        run: &mut Run,
        opened: Opened,
        latency: Duration,
        error: &ProviderError,
        conversation: &mut Option<Conversation>,
    ) -> Result<Duration, Stopped> {
        let mut opened = opened;
        let (turn, attempt) = (opened.turn, opened.attempt);
        let failed = run.failed_attempt(opened.began.offset, latency, error.usage);
        let next = self.after_failure(
            run,
            running,
            error,
            running.run_id.salt(turn, attempt),
            attempt,
        );

        opened.chat.error_type = Some(error.kind.into());
        if let Some(usage) = &error.usage {
            spent(&mut opened.chat, usage);
        }
        let end = running.at(failed.started_ms.saturating_add(failed.latency_ms));
        // The retry event and the exception record carry the attempt's
        // number and the span's end time: they say what was decided when
        // the attempt ended.
        LabletRetry {
            lablet_attempt: i64::from(attempt),
            lablet_retry_will_retry: next.is_ok(),
            lablet_retry_backoff_ms: next.as_ref().ok().copied().map(whole_ms).map(count_of),
        }
        .add_to(&mut opened.span, end);
        let exception = GenAiClientOperationException {
            join: running.join.clone(),
            exception_message: error.message().to_owned(),
            exception_type: error.kind.into(),
            gen_ai_provider_name: running.asked.provider_name.clone(),
            gen_ai_request_model: running.asked.model.clone(),
            lablet_attempt: i64::from(attempt),
            lablet_turn: i64::from(turn),
        };
        self.logger
            .emit(exception.record(end, opened.span.span_context()));
        let exchange = conversation.as_mut().map(Conversation::unanswered);
        self.end_chat(
            running,
            opened,
            Status::error(error.message().to_owned()),
            exchange,
            end,
        );
        next
    }

    /// The attempt in `opened` was dropped where it was after `latency`,
    /// because the run was cancelled. It took its time and it was an
    /// attempt, so the run counts it as one that failed, and its span ends
    /// saying why; nothing failed, so there's no retry and no exception.
    fn attempt_dropped(
        &self,
        running: &Running,
        run: &mut Run,
        opened: Opened,
        latency: Duration,
        conversation: &mut Option<Conversation>,
    ) {
        let dropped = run.failed_attempt(opened.began.offset, latency, None);
        let mut opened = opened;
        opened.chat.error_type = Some(LabletChatErrorType::Cancelled);
        let end = running.at(dropped.started_ms.saturating_add(dropped.latency_ms));
        let exchange = conversation.as_mut().map(Conversation::unanswered);
        self.end_chat(
            running,
            opened,
            Status::error(CANCELLED_IN_FLIGHT),
            exchange,
            end,
        );
    }

    /// A span of the run, opened through the tracer under the context the
    /// run was called in, at the instant the call it's for began.
    fn open(&self, running: &Running, name: String, kind: SpanKind, began: &Began) -> BoxedSpan {
        self.tracer
            .span_builder(name)
            .with_kind(kind)
            .with_start_time(running.at(whole_ms(began.offset)))
            .start_with_context(&self.tracer, &running.parent)
    }

    /// Fills the span of `opened` from its struct, gives it `status`, writes
    /// the record of `exchange` in its context when the run captured one,
    /// and ends it at `end`. The one way a chat span ends, so every way an
    /// attempt can end leaves the same shape behind.
    fn end_chat(
        &self,
        running: &Running,
        opened: Opened,
        status: Status,
        exchange: Option<Exchange>,
        end: SystemTime,
    ) {
        let Opened {
            mut span,
            chat,
            turn,
            ..
        } = opened;
        chat.record(&mut span);
        span.set_status(status);
        if let Some(exchange) = exchange {
            let content = GenAiClientInferenceOperationDetails {
                gen_ai_input_messages: Some(exchange.input),
                gen_ai_output_messages: exchange.output,
                gen_ai_system_instructions: Some(exchange.system),
                ..running.content(Operation::Chat, Some(turn))
            };
            self.logger.emit(content.record(end, span.span_context()));
        }
        span.end_with_timestamp(end);
    }

    /// `work` to its end, or `None` when the run is cancelled first, and
    /// then the work is dropped where it is.
    ///
    /// Dropping is the whole of stopping: nothing waits for the work to
    /// wind down, so a cancelled run takes no longer to stop than its
    /// futures take to drop, whatever call was in flight. The work is
    /// polled first, so work that has ended counts as ended even when the
    /// run was cancelled as it ended.
    async fn unless_cancelled<T>(&self, work: impl Future<Output = T>) -> Option<T> {
        let mut work = pin!(work);
        let mut cancelled = self.cancel.cancelled();
        poll_fn(|context| match work.as_mut().poll(context) {
            Poll::Ready(done) => Poll::Ready(Some(done)),
            Poll::Pending => cancelled.as_mut().poll(context).map(|()| None),
        })
        .await
    }

    /// What every request of the run `run_id` carries for the adapter to
    /// keep the run's cache apart by: the id, which no other run has, and
    /// nothing when runs share a cache.
    fn cache_key<'a>(&self, run_id: &'a RunId) -> Option<&'a str> {
        match self.request.cache_scope {
            CacheScope::Shared => None,
            CacheScope::Run => Some(run_id.as_str()),
        }
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
        running: &Running,
        error: &ProviderError,
        salt: u64,
        attempt: u32,
    ) -> Result<Duration, Stopped> {
        if error.kind.is_retryable()
            && let Err(stopped) = self.admit(run, running)
        {
            return Err(stopped);
        }
        match self
            .retry
            .next(attempt, error.kind, error.retry_after, salt)
        {
            Some(wait) if self.stop.allows_wait(self.elapsed(running), wait) => Ok(wait),
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
    async fn run_tools(&self, running: &Running, pending: Pending, turn: u32) -> Run {
        let concurrency = |call: &ToolUse| self.tools.concurrency(call);
        let schedule = Schedule {
            max_concurrent: self.calls.max_concurrent_tool_calls,
            concurrency: &concurrency,
        };
        pending
            .answer(schedule, |call| self.answer(running, call, turn))
            .await
    }

    /// One tool call, from the span that opens it to the span's end, or a
    /// call that's never run because its turn came when the run had been
    /// cancelled or had no time left.
    ///
    /// A call the run is cancelled during is dropped where it is, and its
    /// span still ends. The calls whose turn comes after that are never
    /// run, so a cancelled tool phase starts no later group, and the turn
    /// has an outcome for every call.
    ///
    /// Nothing is started for a call that's never run, so it has no span
    /// and the output cap, which cuts what a call wrote, has nothing of it
    /// to cut.
    async fn answer(&self, running: &Running, call: ToolUse, turn: u32) -> Answer {
        let began = match self.tool_turn(running) {
            Ok(began) => began,
            Err(Unstarted { offset, why }) => {
                return Answer::measured(
                    ToolCallStatus::NotRun,
                    self.whole(why),
                    None,
                    offset,
                    Duration::ZERO,
                );
            }
        };

        let called = Called {
            call: &call,
            turn,
            source: self.tools.source(&call.name).cloned(),
            input: match &call.input {
                ToolInput::Json(value) => Some(value),
                ToolInput::Unparsed(_) => None,
            },
        };
        let span = self.open(
            running,
            format!("{} {}", LabletExecuteTool::GEN_AI_OPERATION_NAME, call.name),
            LabletExecuteTool::KIND,
            &began,
        );
        // The span stays here, where it's filled and ended with what the
        // loop measured, since the generated struct records onto an owned
        // span and a context's `SpanRef` can't take it. The call runs under
        // a context that holds the span's context alone, so `Context::current()`
        // inside the executor names this call's span, which is what a
        // propagator needs of it.
        let within = running
            .parent
            .with_remote_span_context(span.span_context().clone());
        let settled = self
            .settle(&call, called.source.clone(), began.left, &within)
            .await;
        let latency = self.clock.now().saturating_duration_since(began.at);
        let Settled {
            status,
            output,
            mcp,
        } = settled;

        let answer = Answer::measured(status, output, self.calls.output_cap, began.offset, latency);
        self.tool_span(running, span, &called, &answer, mcp.as_ref());
        answer
    }

    /// A tool call's turn has come: the one reading of the clock the call
    /// is timed from, when the call is run, or why it isn't. The clock is
    /// read once, which for a call of a later group is after the groups
    /// ahead of it, and that reading decides whether the call is run and
    /// gives the call its start, its latency and its deadline.
    ///
    /// Cancellation comes first, as it does at points A and B.
    fn tool_turn(&self, running: &Running) -> Result<Began, Unstarted> {
        let at = self.clock.now();
        let offset = running.offset(at);
        let left = if self.cancel.is_cancelled() {
            Err("the run was cancelled before this call started")
        } else {
            self.stop
                .time_left(offset)
                .ok_or("the run reached its timeout before this call started")
        };
        match left {
            Ok(left) => Ok(Began { at, offset, left }),
            Err(why) => Err(Unstarted { offset, why }),
        }
    }

    /// Fills `span` from what `answer` says of `called`, and from what the
    /// call carried back over MCP when it did, writes the record of the
    /// call's content when the run captures it, and ends the span when the
    /// call ended.
    fn tool_span(
        &self,
        running: &Running,
        mut span: BoxedSpan,
        called: &Called<'_>,
        answer: &Answer,
        mcp: Option<&McpCallMeta>,
    ) {
        let Called {
            call,
            turn,
            source,
            input,
        } = called;
        let end = running.at(answer.started_ms().saturating_add(answer.latency_ms()));
        let failed = answer.status().is_error();
        let over_mcp = OverMcp::of(mcp);
        LabletExecuteTool {
            join: running.join.clone(),
            gen_ai_tool_call_id: call.id.as_str().to_owned(),
            gen_ai_tool_name: call.name.as_str().to_owned(),
            lablet_tool_input_bytes: count_of(call.input_bytes()),
            lablet_tool_is_error: failed,
            lablet_tool_output_bytes: count_of(answer.output_bytes()),
            lablet_tool_output_truncated: answer.truncated_from_bytes().is_some(),
            lablet_tool_status: answer.status().into(),
            lablet_turn: i64::from(*turn),
            error_type: tool_error_type(answer.status()),
            gen_ai_tool_description: self.description(&call.name),
            gen_ai_tool_type: source.as_ref().map(Into::into),
            jsonrpc_request_id: over_mcp.jsonrpc_request_id,
            lablet_tool_output_original_bytes: answer.truncated_from_bytes().map(count_of),
            lablet_tool_source: source.as_ref().map(Into::into),
            mcp_method_name: over_mcp.method,
            mcp_protocol_version: over_mcp.protocol_version,
            mcp_session_id: over_mcp.session_id,
            network_transport: over_mcp.transport,
            rpc_response_status_code: over_mcp.rpc_status_code,
        }
        .record(&mut span);
        // What a tool said of its failure is content, so the span says that
        // the call failed and its record, if there is one, says how.
        if failed {
            span.set_status(Status::error(""));
        }
        if running.capture {
            let content = GenAiClientInferenceOperationDetails {
                gen_ai_tool_call_arguments: input.map(ToString::to_string),
                gen_ai_tool_call_id: Some(call.id.as_str().to_owned()),
                gen_ai_tool_call_result: Some(tool_result(answer.content(), failed).to_string()),
                gen_ai_tool_name: Some(call.name.as_str().to_owned()),
                ..running.content(Operation::ExecuteTool, Some(*turn))
            };
            self.logger.emit(content.record(end, span.span_context()));
        }
        span.end_with_timestamp(end);
    }

    /// What the model was told the tool `name` does, for a tool the run
    /// offered; a name no tool has was described to no one.
    fn description(&self, name: &lablet_model::ToolName) -> Option<String> {
        self.tools
            .specs()
            .iter()
            .find(|spec| spec.name == *name)
            .map(|spec| spec.description.clone())
    }

    /// A result the loop wrote itself, held whole, with the run's secrets cut
    /// out of it as an executor cuts them out of a tool's text. The words
    /// are the loop's own, but the text may carry the arguments the model
    /// sent, which may not show a value.
    fn whole(&self, text: &str) -> KeptOutput {
        KeptOutput::whole(&self.secrets.redacted(text))
    }

    /// The error result the model is sent of an executor's `message`: the
    /// run's secrets cut out of it, as out of any text the loop writes, and
    /// then no more than
    /// [`ERROR_MESSAGE_MAX_BYTES`](crate::ERROR_MESSAGE_MAX_BYTES) of what's
    /// left. The bound comes after the cut, since a value the bound chopped
    /// would be no value to the cut, and every byte of it before the bound
    /// would be sent; what the bound takes instead is the end of a marker.
    fn error_result(&self, message: &str) -> KeptOutput {
        KeptOutput::whole(&bounded(self.secrets.redacted(message)))
    }

    /// What became of one call that has `left` to run in, and what the model
    /// is sent back. The executor runs under `within`, the call's span's
    /// context.
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
        source: Option<ToolSource>,
        left: Duration,
        within: &Context,
    ) -> Settled {
        let Some(source) = source else {
            return Settled::local(
                ToolCallStatus::Unknown,
                self.whole(&format!(
                    "no tool named {} is offered by this run",
                    call.name
                )),
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
                    self.whole(&format!(
                        "the arguments weren't valid JSON, so {} wasn't called: {text}",
                        call.name
                    )),
                );
            }
            ToolInput::Json(_) if self.tools.completion().intercepts(&call.name) => {
                return Settled::local(
                    ToolCallStatus::Rejected,
                    self.whole(&format!(
                        "call {} on its own, once your other calls have returned",
                        call.name
                    )),
                );
            }
            ToolInput::Json(value) => value.clone(),
        };

        let executed = self
            .unless_cancelled(
                self.tools
                    .execute(ToolCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        input,
                        deadline: left,
                        keep: self.calls.output_cap.map(OutputCap::keeps),
                        secrets: Arc::clone(&self.secrets),
                    })
                    .with_context(within.clone()),
            )
            .await;
        let Some(executed) = executed else {
            // Whatever the call had written went with it, and the transport
            // it went over is the executor's to say, which it didn't.
            return Settled::local(
                ToolCallStatus::ran(source, ToolCallEnd::Cancelled),
                self.whole("the run was cancelled while this call ran, and the call was stopped"),
            );
        };
        match executed {
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
                    output: self.error_result(error.message()),
                    mcp: error.mcp,
                }
            }
        }
    }

    fn progress(&self, run: &Run, running: &Running) -> Progress {
        run.progress(self.elapsed(running))
    }

    fn elapsed(&self, running: &Running) -> Duration {
        running.offset(self.clock.now())
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
    /// The measure of a run whose system prompt and tool specs come to
    /// `fixed` bytes together.
    const fn new(fixed: u64) -> Self {
        Self {
            fixed,
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
