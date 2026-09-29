//! The run in progress: what the observer keeps of it between events, and
//! what each event becomes.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use lablet_model::{
    ContentBlock, Endpoint, ModelRef, RequestParams, RunContext, RunId, RunSummary, StopReason,
    ToolCallId, ToolCallStatus, ToolName, ToolResultContent, ToolSource, ToolSpec, TurnRecord,
    Usage,
};
use lablet_run::{McpCallMeta, ProviderError, TraceContext};
use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::enums::LabletChatPurpose;
use lablet_telemetry_registry::signals::{
    EVENT_GEN_AI_CLIENT_INFERENCE_OPERATION_DETAILS_NAME as CONTENT,
    EVENT_GEN_AI_CLIENT_OPERATION_EXCEPTION_NAME as EXCEPTION, EVENT_LABLET_RETRY_NAME as RETRY,
};
use opentelemetry::logs::Severity;
use opentelemetry::trace::{SpanId, SpanKind, TraceId};
use opentelemetry_sdk::trace::{IdGenerator as _, RandomIdGenerator};

use crate::attributes::Attributes;
use crate::content::{self, Conversation, Exchange};
use crate::signal::{Ended, Happened, Record, Signals, Span, instant, traceparent};

/// The name of the agent, which is `gen_ai.agent.name` and the root span's
/// name after its operation.
pub(crate) const AGENT: &str = "lablet";

/// The three operations a run's spans are of, as `gen_ai.operation.name`
/// spells them. Each is the first word of its span's name.
const INVOKE_AGENT: &str = "invoke_agent";
const CHAT: &str = "chat";
const EXECUTE_TOOL: &str = "execute_tool";

/// Whole milliseconds, as the loop's own are cut.
fn whole_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// What a run's first event says of it.
#[derive(Debug)]
pub(crate) struct Opening {
    pub(crate) context: RunContext,
    pub(crate) model: ModelRef,
    pub(crate) endpoint: Option<Endpoint>,
    pub(crate) request: RequestParams,
    pub(crate) tools: Vec<ToolSpec>,
    pub(crate) system_prompt: Option<String>,
    pub(crate) prompt: Option<String>,
}

/// A tool call that has started and not ended.
#[derive(Debug)]
struct OpenCall {
    /// The id of the call's span, fixed when the call starts because an
    /// executor propagates it while the call runs.
    span: SpanId,
    name: ToolName,
    source: Option<ToolSource>,
    input_bytes: u64,
    /// The call's arguments as JSON, when content is captured and they
    /// parsed.
    arguments: Option<String>,
}

/// What a tool call's last event says of it.
#[derive(Debug)]
pub(crate) struct CallEnd {
    pub(crate) turn: u32,
    pub(crate) status: ToolCallStatus,
    pub(crate) started_ms: u64,
    pub(crate) latency_ms: u64,
    pub(crate) output_bytes: u64,
    pub(crate) truncated_from_bytes: Option<u64>,
    pub(crate) mcp: Option<McpCallMeta>,
    pub(crate) output: Option<Vec<ToolResultContent>>,
}

/// One attempt of a provider call, as the event that ended it names it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Attempt {
    pub(crate) turn: u32,
    /// Which attempt of its call, counted from 1.
    pub(crate) number: u32,
    pub(crate) started_ms: u64,
    pub(crate) latency_ms: u64,
}

/// A run that has ended, waiting for its wide event, which is made once
/// the run's other records have been exported.
#[derive(Debug)]
pub(crate) struct Closed {
    pub(crate) trace: TraceId,
    pub(crate) root: SpanId,
    /// When the run ended.
    pub(crate) at: SystemTime,
    pub(crate) context: RunContext,
    pub(crate) summary: RunSummary,
}

/// A run that has started and not ended.
#[derive(Debug)]
pub(crate) struct OpenRun {
    run_id: RunId,
    started_unix_ms: u64,
    trace: TraceId,
    root: SpanId,
    agent_version: String,
    model: String,
    /// The keys a consumer groups by, which every signal of the run holds.
    join: Attributes,
    /// What every chat span of the run says, which is what every provider
    /// call of the run was asked.
    asked: Attributes,
    /// What the failure of any provider call of the run says of the call.
    called: Attributes,
    /// What the model was told each tool does.
    descriptions: HashMap<ToolName, String>,
    /// Whether the run's content may be recorded.
    captures: bool,
    /// The conversation so far, when its content came with the events.
    conversation: Option<Conversation>,
    /// The size of the request of each attempt that has begun and not
    /// ended, by turn and attempt.
    requests: HashMap<(u32, u32), u64>,
    calls: HashMap<ToolCallId, OpenCall>,
}

fn join(context: &RunContext) -> Attributes {
    let run_id = context.run_id.as_str();
    Attributes::default()
        .with(key::GEN_AI_CONVERSATION_ID, run_id)
        .with(key::SESSION_ID, run_id)
        .with(key::LABLET_CONFIG_DIGEST, context.config_digest.as_str())
        .with_any(key::LABLET_TASK_ID, context.labels.task.as_deref())
        .with_any(
            key::LABLET_EXPERIMENT_ID,
            context.labels.experiment.as_deref(),
        )
        .with_any(key::LABLET_TRIAL, context.labels.trial.as_deref())
}

/// The counts of `usage`, of which one the provider didn't report is left
/// out.
fn spent(usage: &Usage) -> Attributes {
    Attributes::default()
        .with(key::GEN_AI_USAGE_INPUT_TOKENS, usage.input_tokens)
        .with(key::GEN_AI_USAGE_OUTPUT_TOKENS, usage.output_tokens)
        .with_any(
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
            usage.reasoning_output_tokens,
        )
        .with_any(
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            usage.cache_read_tokens,
        )
        .with_any(
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            usage.cache_write_tokens,
        )
}

fn over_mcp(mcp: McpCallMeta) -> Attributes {
    Attributes::default()
        .with(key::MCP_METHOD_NAME, mcp.method)
        .with_any(key::MCP_SESSION_ID, mcp.session_id)
        .with_any(key::MCP_PROTOCOL_VERSION, mcp.protocol_version)
        .with_any(key::JSONRPC_REQUEST_ID, mcp.jsonrpc_request_id)
        .with_any(key::RPC_RESPONSE_STATUS_CODE, mcp.rpc_status_code)
        .with(key::NETWORK_TRANSPORT, mcp.transport.as_str())
}

impl OpenRun {
    /// The run `opening` describes, and the record of the tools it offers
    /// when it captures content.
    pub(crate) fn open(opening: Opening) -> (Self, Signals) {
        let Opening {
            context,
            model,
            endpoint,
            request,
            tools,
            system_prompt,
            prompt,
        } = opening;
        let ids = RandomIdGenerator::default();
        let join = join(&context);
        let called = Attributes::default()
            .with(key::GEN_AI_OPERATION_NAME, CHAT)
            .with(key::GEN_AI_PROVIDER_NAME, model.api.provider().as_str())
            .with(key::GEN_AI_REQUEST_MODEL, model.name.as_str());
        let (address, port) = endpoint
            .map(|endpoint| (endpoint.host, endpoint.port))
            .unzip();
        let asked = called
            .clone()
            .with(key::GEN_AI_REQUEST_MAX_TOKENS, request.max_tokens)
            .with_any(key::GEN_AI_REQUEST_TEMPERATURE, request.temperature)
            .with_any(key::GEN_AI_REQUEST_SEED, request.seed)
            .with_any(
                key::GEN_AI_REQUEST_REASONING_LEVEL,
                request.effort.map(lablet_model::Effort::as_str),
            )
            .with_any(key::SERVER_ADDRESS, address)
            .with_any(key::SERVER_PORT, port)
            .with(key::LABLET_CHAT_PURPOSE, LabletChatPurpose::Turn.as_str());
        let captures = context.capture_content;
        let conversation = system_prompt
            .zip(prompt)
            .filter(|_| captures)
            .map(|(system, prompt)| Conversation::opening(&system, &prompt));

        let run = Self {
            run_id: context.run_id,
            started_unix_ms: context.started_unix_ms,
            trace: ids.new_trace_id(),
            root: ids.new_span_id(),
            agent_version: context.agent_version,
            model: model.name,
            join,
            asked,
            called,
            descriptions: tools
                .iter()
                .map(|spec| (spec.name.clone(), spec.description.clone()))
                .collect(),
            captures,
            conversation,
            requests: HashMap::new(),
            calls: HashMap::new(),
        };
        let offered = captures.then(|| Record {
            name: CONTENT,
            severity: Severity::Info,
            at: run.at(0),
            span: run.root,
            attributes: run
                .join
                .clone()
                .with(key::GEN_AI_OPERATION_NAME, INVOKE_AGENT)
                .with(
                    key::GEN_AI_TOOL_DEFINITIONS,
                    content::tool_definitions(&tools),
                ),
        });
        (run, Signals::default().record(offered))
    }

    /// Whether this is the run `run_id`.
    pub(crate) fn is(&self, run_id: &RunId) -> bool {
        self.run_id == *run_id
    }

    /// The instant `offset_ms` into the run.
    fn at(&self, offset_ms: u64) -> SystemTime {
        instant(self.started_unix_ms, offset_ms)
    }

    /// An attempt of a provider call began, with a request of
    /// `request_bytes`.
    pub(crate) fn attempt_began(&mut self, turn: u32, attempt: u32, request_bytes: u64) {
        self.requests.insert((turn, attempt), request_bytes);
    }

    /// The span of an attempt that ended, as far as the two ways it can
    /// end agree; `None` for an attempt nothing said had begun.
    fn chat(&mut self, attempt: Attempt, more: Attributes, ended: Ended) -> Option<Span> {
        let request_bytes = self.requests.remove(&(attempt.turn, attempt.number))?;
        let start = self.at(attempt.started_ms);
        let end = self.at(attempt.started_ms.saturating_add(attempt.latency_ms));
        Some(Span {
            name: format!("{CHAT} {}", self.model),
            kind: SpanKind::Client,
            id: RandomIdGenerator::default().new_span_id(),
            parent: Some(self.root),
            start,
            end,
            attributes: self
                .join
                .clone()
                .and(self.asked.clone())
                .with(key::LABLET_TURN, attempt.turn)
                .with(key::LABLET_ATTEMPT, attempt.number)
                .with(key::LABLET_REQUEST_BYTES, request_bytes)
                .and(more),
            events: Vec::new(),
            ended,
        })
    }

    /// The record of what a provider call was sent and what it answered,
    /// in the context of the call's span.
    fn exchanged(&self, span: &Span, turn: u32, exchange: Exchange) -> Record {
        Record {
            name: CONTENT,
            severity: Severity::Info,
            at: span.end,
            span: span.id,
            attributes: self
                .join
                .clone()
                .with(key::GEN_AI_OPERATION_NAME, CHAT)
                .with(key::LABLET_TURN, turn)
                .with(key::GEN_AI_SYSTEM_INSTRUCTIONS, exchange.system)
                .with(key::GEN_AI_INPUT_MESSAGES, exchange.input)
                .with_any(key::GEN_AI_OUTPUT_MESSAGES, exchange.output),
        }
    }

    /// An attempt returned a response.
    pub(crate) fn attempt_answered(
        &mut self,
        turn: u32,
        attempt: u32,
        record: TurnRecord,
        response: Option<&[ContentBlock]>,
    ) -> Signals {
        let TurnRecord {
            usage,
            finish,
            response_id,
            response_model,
            started_ms,
            latency_ms,
            attempts: _,
        } = record;
        let answered = Attributes::default()
            .with_any(key::GEN_AI_RESPONSE_ID, response_id)
            .with_any(key::GEN_AI_RESPONSE_MODEL, response_model)
            .with(
                key::GEN_AI_RESPONSE_FINISH_REASONS,
                vec![finish.as_str().to_owned()],
            )
            .and(spent(&usage));
        let attempt = Attempt {
            turn,
            number: attempt,
            started_ms,
            latency_ms,
        };
        let Some(span) = self.chat(attempt, answered, Ended::Well) else {
            return Signals::default();
        };
        let exchange = self
            .conversation
            .as_mut()
            .zip(response)
            .map(|(conversation, response)| conversation.responded(response));
        let exchanged = exchange.map(|exchange| self.exchanged(&span, turn, exchange));
        Signals::default().record(exchanged).span(span)
    }

    /// An attempt failed, and the loop waits `retry` before the next, or
    /// makes no other.
    pub(crate) fn attempt_failed(
        &mut self,
        attempt: Attempt,
        error: &ProviderError,
        retry: Option<Duration>,
    ) -> Signals {
        let class = error.kind.as_str();
        let failed = Attributes::default()
            .with(key::ERROR_TYPE, class)
            .and(error.usage.as_ref().map(spent).unwrap_or_default());
        let Some(mut span) = self.chat(attempt, failed, Ended::Badly(error.message().to_owned()))
        else {
            return Signals::default();
        };
        span.events.push(Happened {
            name: RETRY,
            at: span.end,
            attributes: Attributes::default()
                .with(key::LABLET_ATTEMPT, attempt.number)
                .with(key::LABLET_RETRY_WILL_RETRY, retry.is_some())
                .with_any(key::LABLET_RETRY_BACKOFF_MS, retry.map(whole_ms)),
        });
        let exception = Record {
            name: EXCEPTION,
            severity: Severity::Warn,
            at: span.end,
            span: span.id,
            attributes: self
                .join
                .clone()
                .with(key::EXCEPTION_TYPE, class)
                .with(key::EXCEPTION_MESSAGE, error.message())
                .and(self.called.clone())
                .with(key::LABLET_TURN, attempt.turn)
                .with(key::LABLET_ATTEMPT, attempt.number),
        };
        let exchange = self.conversation.as_mut().map(Conversation::unanswered);
        let exchanged = exchange.map(|exchange| self.exchanged(&span, attempt.turn, exchange));
        Signals::default()
            .record(Some(exception))
            .record(exchanged)
            .span(span)
    }

    /// A tool call began.
    pub(crate) fn call_began(
        &mut self,
        call: ToolCallId,
        name: ToolName,
        source: Option<ToolSource>,
        input_bytes: u64,
        input: Option<&serde_json::Value>,
    ) {
        self.calls.insert(
            call,
            OpenCall {
                span: RandomIdGenerator::default().new_span_id(),
                name,
                source,
                input_bytes,
                arguments: input
                    .filter(|_| self.captures)
                    .map(serde_json::Value::to_string),
            },
        );
    }

    /// The span of the call `call`, while the call runs.
    pub(crate) fn propagated(&self, call: &ToolCallId) -> Option<TraceContext> {
        self.calls.get(call).map(|open| TraceContext {
            traceparent: traceparent(self.trace, open.span),
            tracestate: None,
        })
    }

    /// A tool call ended; nothing comes of one that nothing said had begun.
    pub(crate) fn call_ended(&mut self, call: &ToolCallId, end: CallEnd) -> Signals {
        let Some(open) = self.calls.remove(call) else {
            return Signals::default();
        };
        let CallEnd {
            turn,
            status,
            started_ms,
            latency_ms,
            output_bytes,
            truncated_from_bytes,
            mcp,
            output,
        } = end;
        let failed = status.is_error();
        let offered = open.source.as_ref().map(|source| {
            Attributes::default()
                .with(key::GEN_AI_TOOL_TYPE, content::tool_type(source))
                .with(key::LABLET_TOOL_SOURCE, source.as_str())
                .with_any(
                    key::GEN_AI_TOOL_DESCRIPTION,
                    self.descriptions.get(&open.name).map(String::as_str),
                )
        });
        let named = Attributes::default()
            .with(key::GEN_AI_OPERATION_NAME, EXECUTE_TOOL)
            .with(key::GEN_AI_TOOL_NAME, open.name.as_str())
            .with(key::GEN_AI_TOOL_CALL_ID, call.as_str())
            .with(key::LABLET_TURN, turn);
        let span = Span {
            name: format!("{EXECUTE_TOOL} {}", open.name),
            kind: SpanKind::Internal,
            id: open.span,
            parent: Some(self.root),
            start: self.at(started_ms),
            end: self.at(started_ms.saturating_add(latency_ms)),
            attributes: self
                .join
                .clone()
                .and(named.clone())
                .and(offered.unwrap_or_default())
                .with_any(key::ERROR_TYPE, failed.then(|| status.as_str()))
                .with(key::LABLET_TOOL_STATUS, status.as_str())
                .with(key::LABLET_TOOL_INPUT_BYTES, open.input_bytes)
                .with(key::LABLET_TOOL_OUTPUT_BYTES, output_bytes)
                .with(
                    key::LABLET_TOOL_OUTPUT_TRUNCATED,
                    truncated_from_bytes.is_some(),
                )
                .with_any(key::LABLET_TOOL_OUTPUT_ORIGINAL_BYTES, truncated_from_bytes)
                .with(key::LABLET_TOOL_IS_ERROR, failed)
                .and(mcp.map(over_mcp).unwrap_or_default()),
            events: Vec::new(),
            // What a tool said of its failure is content, so the span says
            // that the call failed and its record, if there is one, says how.
            ended: if failed {
                Ended::Badly(String::new())
            } else {
                Ended::Well
            },
        };
        if let Some((conversation, output)) = self.conversation.as_mut().zip(output.as_deref()) {
            conversation.answered(call, output, failed);
        }
        let returned = self.captures.then(|| Record {
            name: CONTENT,
            severity: Severity::Info,
            at: span.end,
            span: span.id,
            attributes: self
                .join
                .clone()
                .and(named)
                .with_any(key::GEN_AI_TOOL_CALL_ARGUMENTS, open.arguments)
                .with_any(
                    key::GEN_AI_TOOL_CALL_RESULT,
                    output.map(|output| content::tool_result(&output, failed).to_string()),
                ),
        });
        Signals::default().record(returned).span(span)
    }

    /// The run ended: its root span, and what its wide event is made from.
    pub(crate) fn close(self, context: RunContext, summary: RunSummary) -> (Signals, Closed) {
        let outcome = &summary.outcome;
        let reason = outcome.stop_reason();
        let stopped = reason != StopReason::Completed;
        let end = self.at(outcome.duration_ms);
        let root = Span {
            name: format!("{INVOKE_AGENT} {AGENT}"),
            kind: SpanKind::Internal,
            id: self.root,
            parent: None,
            start: self.at(0),
            end,
            attributes: self
                .join
                .clone()
                .with(key::GEN_AI_OPERATION_NAME, INVOKE_AGENT)
                .with(key::GEN_AI_AGENT_NAME, AGENT)
                .with(key::GEN_AI_AGENT_VERSION, self.agent_version)
                .with(key::GEN_AI_REQUEST_MODEL, self.model)
                .with(key::LABLET_RUN_STOP_REASON, reason.as_str())
                .with_any(key::ERROR_TYPE, stopped.then(|| reason.as_str()))
                .with(key::LABLET_RUN_TURNS, outcome.turns)
                .with(key::LABLET_TOOL_CALLS_TOTAL, outcome.tool_calls)
                .and(spent(&outcome.usage))
                .with_any(
                    key::LABLET_RUN_COST_USD,
                    summary.cost.map(lablet_model::Cost::usd),
                ),
            events: Vec::new(),
            ended: if stopped {
                Ended::Badly(outcome.error().unwrap_or_default().to_owned())
            } else {
                Ended::Well
            },
        };
        let closed = Closed {
            trace: self.trace,
            root: self.root,
            at: end,
            context,
            summary,
        };
        (Signals::default().span(root), closed)
    }

    /// The trace the run's signals belong to.
    pub(crate) const fn trace(&self) -> TraceId {
        self.trace
    }
}

#[cfg(test)]
mod tests;
