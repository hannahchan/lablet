//! This crate's events, one struct each, written as `tracing` events that the
//! OpenTelemetry appender turns into log records, or as span events when the
//! registry annotates them so. The generator writes each
//! field's name where the macro needs it, which a hand-written call can't
//! take from a constant.

#[allow(unused_imports)]
use super::enums::*;

/// Captured content. A log record in the context of the span whose content it carries, emitted only when `telemetry.capture_content` is on.
#[derive(Debug, Clone, PartialEq)]
pub struct GenAiClientInferenceOperationDetails {
    /// The run id.
    pub gen_ai_conversation_id: String,
    /// The operation of the span the content belongs to, `invoke_agent`, `chat`, or `execute_tool`.
    pub gen_ai_operation_name: GenAiOperationName,
    /// SHA-256 of the resolved config, in hex.
    pub lablet_config_digest: String,
    /// The run id, for backends that group by session.
    pub session_id: String,
    /// The chat history provided to the model as an input.
    pub gen_ai_input_messages: Option<String>,
    /// Messages returned by the model where each message represents a specific model response (choice, candidate).
    pub gen_ai_output_messages: Option<String>,
    /// The system message or instructions provided to the GenAI model separately from the chat history.
    pub gen_ai_system_instructions: Option<String>,
    /// Parameters passed to the tool call.
    pub gen_ai_tool_call_arguments: Option<String>,
    /// The tool call identifier.
    pub gen_ai_tool_call_id: Option<String>,
    /// The result returned by the tool call (if any and if execution was successful).
    pub gen_ai_tool_call_result: Option<String>,
    /// The list of tool definitions available to the GenAI agent or model.
    pub gen_ai_tool_definitions: Option<String>,
    /// Name of the tool utilized by the agent.
    pub gen_ai_tool_name: Option<String>,
    /// The experiment the run is part of, as the run request named it.
    pub lablet_experiment_id: Option<String>,
    /// The task the run attempts, as the run request named it.
    pub lablet_task_id: Option<String>,
    /// Which repetition of the task the run is, as the run request named it.
    pub lablet_trial: Option<String>,
    /// One-based index of the turn a provider call or tool call belongs to.
    pub lablet_turn: Option<i64>,
}

impl GenAiClientInferenceOperationDetails {
    /// The record's event name.
    pub const NAME: &'static str = "gen_ai.client.inference.operation.details";

    /// Writes the record, in the context of the current span.
    pub fn emit(&self) {
        tracing::event!(
            name: "gen_ai.client.inference.operation.details",
            target: "lablet",
            tracing::Level::INFO,
            "gen_ai.conversation.id" = self.gen_ai_conversation_id.as_str(),
            "gen_ai.operation.name" = self.gen_ai_operation_name.as_str(),
            "lablet.config.digest" = self.lablet_config_digest.as_str(),
            "session.id" = self.session_id.as_str(),
            "gen_ai.input.messages" = self.gen_ai_input_messages,
            "gen_ai.output.messages" = self.gen_ai_output_messages,
            "gen_ai.system_instructions" = self.gen_ai_system_instructions,
            "gen_ai.tool.call.arguments" = self.gen_ai_tool_call_arguments,
            "gen_ai.tool.call.id" = self.gen_ai_tool_call_id.as_deref(),
            "gen_ai.tool.call.result" = self.gen_ai_tool_call_result,
            "gen_ai.tool.definitions" = self.gen_ai_tool_definitions,
            "gen_ai.tool.name" = self.gen_ai_tool_name.as_deref(),
            "lablet.experiment.id" = self.lablet_experiment_id.as_deref(),
            "lablet.task.id" = self.lablet_task_id.as_deref(),
            "lablet.trial" = self.lablet_trial.as_deref(),
            "lablet.turn" = self.lablet_turn,
        );
    }
}

/// A provider call failed. A log record of severity WARN in the context of the chat span of the failed attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct GenAiClientOperationException {
    /// The exception message.
    pub exception_message: String,
    /// The class of provider error: `retryable`, `context_exhausted`, `auth`, `fatal`, or `malformed`.
    pub exception_type: String,
    /// The run id.
    pub gen_ai_conversation_id: String,
    /// Always `chat`.
    pub gen_ai_operation_name: GenAiOperationName,
    /// The Generative AI provider as identified by the client or server instrumentation.
    pub gen_ai_provider_name: GenAiProviderName,
    /// The name of the GenAI model a request is being made to.
    pub gen_ai_request_model: String,
    /// One-based attempt number of a provider call within its turn.
    pub lablet_attempt: i64,
    /// SHA-256 of the resolved config, in hex.
    pub lablet_config_digest: String,
    /// One-based index of the turn a provider call or tool call belongs to.
    pub lablet_turn: i64,
    /// The run id, for backends that group by session.
    pub session_id: String,
    /// The experiment the run is part of, as the run request named it.
    pub lablet_experiment_id: Option<String>,
    /// The task the run attempts, as the run request named it.
    pub lablet_task_id: Option<String>,
    /// Which repetition of the task the run is, as the run request named it.
    pub lablet_trial: Option<String>,
}

impl GenAiClientOperationException {
    /// The record's event name.
    pub const NAME: &'static str = "gen_ai.client.operation.exception";

    /// Writes the record, in the context of the current span.
    pub fn emit(&self) {
        tracing::event!(
            name: "gen_ai.client.operation.exception",
            target: "lablet",
            tracing::Level::WARN,
            "exception.message" = self.exception_message.as_str(),
            "exception.type" = self.exception_type.as_str(),
            "gen_ai.conversation.id" = self.gen_ai_conversation_id.as_str(),
            "gen_ai.operation.name" = self.gen_ai_operation_name.as_str(),
            "gen_ai.provider.name" = self.gen_ai_provider_name.as_str(),
            "gen_ai.request.model" = self.gen_ai_request_model.as_str(),
            "lablet.attempt" = self.lablet_attempt,
            "lablet.config.digest" = self.lablet_config_digest.as_str(),
            "lablet.turn" = self.lablet_turn,
            "session.id" = self.session_id.as_str(),
            "lablet.experiment.id" = self.lablet_experiment_id.as_deref(),
            "lablet.task.id" = self.lablet_task_id.as_deref(),
            "lablet.trial" = self.lablet_trial.as_deref(),
        );
    }
}

/// A provider call failed. Recorded as a span event on the chat span of the failed attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct LabletRetry {
    /// One-based attempt number of a provider call within its turn.
    pub lablet_attempt: i64,
    /// Whether the loop decided to retry the failed provider call.
    pub lablet_retry_will_retry: bool,
    /// How long the loop waits before the next attempt, in milliseconds.
    pub lablet_retry_backoff_ms: Option<i64>,
}

impl LabletRetry {
    /// The record's event name.
    pub const NAME: &'static str = "lablet.retry";

    /// Adds the event to `span`, at `timestamp`.
    pub fn add_to(
        &self,
        span: &mut impl opentelemetry::trace::Span,
        timestamp: std::time::SystemTime,
    ) {
        let mut attributes: Vec<opentelemetry::KeyValue> = Vec::with_capacity(3);
        attributes.push(opentelemetry::KeyValue::new(
            "lablet.attempt",
            self.lablet_attempt,
        ));
        attributes.push(opentelemetry::KeyValue::new(
            "lablet.retry.will_retry",
            self.lablet_retry_will_retry,
        ));
        if let Some(value) = &self.lablet_retry_backoff_ms {
            attributes.push(opentelemetry::KeyValue::new(
                "lablet.retry.backoff_ms",
                *value,
            ));
        }
        span.add_event_with_timestamp(Self::NAME, timestamp, attributes);
    }
}
