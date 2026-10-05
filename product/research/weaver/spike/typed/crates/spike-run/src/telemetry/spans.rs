//! This crate's spans, one struct each. A required attribute is a field, and
//! any other is an `Option`, so a span can't be recorded without what the
//! registry requires of it.

use opentelemetry::trace::{Span, SpanKind};
use opentelemetry::{Array, KeyValue, StringValue, Value};

#[allow(unused_imports)]
use super::enums::*;

/// One attempt of a provider call, a child of the root span. Follows the GenAI inference span.
#[derive(Debug, Clone, PartialEq)]
pub struct LabletChat {
    /// The run id.
    pub gen_ai_conversation_id: String,
    /// Always `chat`.
    pub gen_ai_operation_name: GenAiOperationName,
    /// The Generative AI provider as identified by the client or server instrumentation.
    pub gen_ai_provider_name: GenAiProviderName,
    /// The maximum number of tokens the model generates for a request.
    pub gen_ai_request_max_tokens: i64,
    /// The name of the GenAI model a request is being made to.
    pub gen_ai_request_model: String,
    /// One-based attempt number of a provider call within its turn.
    pub lablet_attempt: i64,
    /// What a provider call is for.
    pub lablet_chat_purpose: LabletChatPurpose,
    /// SHA-256 of the resolved config, in hex.
    pub lablet_config_digest: String,
    /// Size in bytes of the system prompt, messages, and tool specs sent in a provider call.
    pub lablet_request_bytes: i64,
    /// One-based index of the turn a provider call or tool call belongs to.
    pub lablet_turn: i64,
    /// The run id, for backends that group by session.
    pub session_id: String,
    /// The class of provider error: `retryable`, `context_exhausted`, `auth`, `fatal`, or `malformed`; or `cancelled` when the run was cancelled while the attempt was in flight.
    pub error_type: Option<ErrorType>,
    /// The reasoning or thinking effort level requested for a GenAI model.
    pub gen_ai_request_reasoning_level: Option<String>,
    /// Requests with same seed value more likely to return same result.
    pub gen_ai_request_seed: Option<i64>,
    /// The temperature setting for the GenAI request.
    pub gen_ai_request_temperature: Option<f64>,
    /// Array of reasons the model stopped generating tokens, corresponding to each generation received.
    pub gen_ai_response_finish_reasons: Option<Vec<String>>,
    /// The unique identifier for the completion.
    pub gen_ai_response_id: Option<String>,
    /// The name of the model that generated the response.
    pub gen_ai_response_model: Option<String>,
    /// The number of input tokens served from a provider-managed cache.
    pub gen_ai_usage_cache_read_input_tokens: Option<i64>,
    /// The number of input tokens written to a provider-managed cache.
    pub gen_ai_usage_cache_write_input_tokens: Option<i64>,
    /// The number of tokens used in the GenAI input (prompt).
    pub gen_ai_usage_input_tokens: Option<i64>,
    /// The number of tokens used in the GenAI response (completion).
    pub gen_ai_usage_output_tokens: Option<i64>,
    /// The number of output tokens used for reasoning (e.g. chain-of-thought, extended thinking).
    pub gen_ai_usage_reasoning_output_tokens: Option<i64>,
    /// The experiment the run is part of, as the run request named it.
    pub lablet_experiment_id: Option<String>,
    /// The task the run attempts, as the run request named it.
    pub lablet_task_id: Option<String>,
    /// Which repetition of the task the run is, as the run request named it.
    pub lablet_trial: Option<String>,
    /// Server domain name if available without reverse DNS lookup; otherwise, IP address or UNIX domain socket name.
    pub server_address: Option<String>,
    /// Server port number.
    pub server_port: Option<i64>,
}

impl LabletChat {
    /// The registry's type for this span.
    pub const TYPE: &'static str = "lablet.chat";
    /// The span's kind.
    pub const KIND: SpanKind = SpanKind::Client;

    /// The span's name, as the registry builds it.
    pub fn name(&self) -> String {
        format!("chat {}", self.gen_ai_request_model)
    }

    /// Every attribute that has a value.
    pub fn attributes(&self) -> Vec<KeyValue> {
        let mut attributes = Vec::with_capacity(28);
        attributes.push(KeyValue::new(
            "gen_ai.conversation.id",
            (&self.gen_ai_conversation_id).clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.operation.name",
            (&self.gen_ai_operation_name).as_str().to_owned(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.provider.name",
            (&self.gen_ai_provider_name).as_str().to_owned(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.request.max_tokens",
            *(&self.gen_ai_request_max_tokens),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.request.model",
            (&self.gen_ai_request_model).clone(),
        ));
        attributes.push(KeyValue::new("lablet.attempt", *(&self.lablet_attempt)));
        attributes.push(KeyValue::new(
            "lablet.chat.purpose",
            (&self.lablet_chat_purpose).as_str().to_owned(),
        ));
        attributes.push(KeyValue::new(
            "lablet.config.digest",
            (&self.lablet_config_digest).clone(),
        ));
        attributes.push(KeyValue::new(
            "lablet.request.bytes",
            *(&self.lablet_request_bytes),
        ));
        attributes.push(KeyValue::new("lablet.turn", *(&self.lablet_turn)));
        attributes.push(KeyValue::new("session.id", (&self.session_id).clone()));
        if let Some(value) = &self.error_type {
            attributes.push(KeyValue::new("error.type", (value).as_str().to_owned()));
        }
        if let Some(value) = &self.gen_ai_request_reasoning_level {
            attributes.push(KeyValue::new(
                "gen_ai.request.reasoning.level",
                (value).clone(),
            ));
        }
        if let Some(value) = &self.gen_ai_request_seed {
            attributes.push(KeyValue::new("gen_ai.request.seed", *(value)));
        }
        if let Some(value) = &self.gen_ai_request_temperature {
            attributes.push(KeyValue::new("gen_ai.request.temperature", *(value)));
        }
        if let Some(value) = &self.gen_ai_response_finish_reasons {
            attributes.push(KeyValue::new(
                "gen_ai.response.finish_reasons",
                Value::Array(Array::String(
                    (value).iter().cloned().map(StringValue::from).collect(),
                )),
            ));
        }
        if let Some(value) = &self.gen_ai_response_id {
            attributes.push(KeyValue::new("gen_ai.response.id", (value).clone()));
        }
        if let Some(value) = &self.gen_ai_response_model {
            attributes.push(KeyValue::new("gen_ai.response.model", (value).clone()));
        }
        if let Some(value) = &self.gen_ai_usage_cache_read_input_tokens {
            attributes.push(KeyValue::new(
                "gen_ai.usage.cache_read.input_tokens",
                *(value),
            ));
        }
        if let Some(value) = &self.gen_ai_usage_cache_write_input_tokens {
            attributes.push(KeyValue::new(
                "gen_ai.usage.cache_write.input_tokens",
                *(value),
            ));
        }
        if let Some(value) = &self.gen_ai_usage_input_tokens {
            attributes.push(KeyValue::new("gen_ai.usage.input_tokens", *(value)));
        }
        if let Some(value) = &self.gen_ai_usage_output_tokens {
            attributes.push(KeyValue::new("gen_ai.usage.output_tokens", *(value)));
        }
        if let Some(value) = &self.gen_ai_usage_reasoning_output_tokens {
            attributes.push(KeyValue::new(
                "gen_ai.usage.reasoning.output_tokens",
                *(value),
            ));
        }
        if let Some(value) = &self.lablet_experiment_id {
            attributes.push(KeyValue::new("lablet.experiment.id", (value).clone()));
        }
        if let Some(value) = &self.lablet_task_id {
            attributes.push(KeyValue::new("lablet.task.id", (value).clone()));
        }
        if let Some(value) = &self.lablet_trial {
            attributes.push(KeyValue::new("lablet.trial", (value).clone()));
        }
        if let Some(value) = &self.server_address {
            attributes.push(KeyValue::new("server.address", (value).clone()));
        }
        if let Some(value) = &self.server_port {
            attributes.push(KeyValue::new("server.port", *(value)));
        }
        attributes
    }

    /// Sets every attribute that has a value on `span`.
    pub fn record(&self, span: &mut impl Span) {
        span.set_attributes(self.attributes());
    }
}

/// One tool call, a child of the root span. Follows the GenAI `execute_tool` span.
#[derive(Debug, Clone, PartialEq)]
pub struct LabletExecuteTool {
    /// The run id.
    pub gen_ai_conversation_id: String,
    /// Always `execute_tool`.
    pub gen_ai_operation_name: GenAiOperationName,
    /// The tool call identifier.
    pub gen_ai_tool_call_id: String,
    /// Name of the tool utilized by the agent.
    pub gen_ai_tool_name: String,
    /// SHA-256 of the resolved config, in hex.
    pub lablet_config_digest: String,
    /// Size of a tool call's input in bytes.
    pub lablet_tool_input_bytes: i64,
    /// Whether the tool call returned an error result to the model.
    pub lablet_tool_is_error: bool,
    /// Size in bytes of a tool call's output as the model was sent it, after the output cap.
    pub lablet_tool_output_bytes: i64,
    /// Whether the output cap cut the tool call's output short.
    pub lablet_tool_output_truncated: bool,
    /// How a tool call ended.
    pub lablet_tool_status: LabletToolStatus,
    /// One-based index of the turn a provider call or tool call belongs to.
    pub lablet_turn: i64,
    /// The run id, for backends that group by session.
    pub session_id: String,
    /// How the call failed, which is its `lablet.tool.status`: `tool_error` when the tool ran and reported an error, `timeout` or `failed` when the executor did, `cancelled` when the run was cancelled while the call ran, and `unknown`, `malformed_input`, or `rejected` when the loop answered the call itself.
    pub error_type: Option<ErrorType>,
    /// The tool description.
    pub gen_ai_tool_description: Option<String>,
    /// `function` for a built-in tool, `extension` for an MCP tool.
    pub gen_ai_tool_type: Option<String>,
    /// A string representation of the `id` property of the request and its corresponding response.
    pub jsonrpc_request_id: Option<String>,
    /// The experiment the run is part of, as the run request named it.
    pub lablet_experiment_id: Option<String>,
    /// The task the run attempts, as the run request named it.
    pub lablet_task_id: Option<String>,
    /// Size in bytes of a tool call's output before the output cap cut it.
    pub lablet_tool_output_original_bytes: Option<i64>,
    /// Where a tool comes from.
    pub lablet_tool_source: Option<LabletToolSource>,
    /// Which repetition of the task the run is, as the run request named it.
    pub lablet_trial: Option<String>,
    /// The name of the request or notification method.
    pub mcp_method_name: Option<McpMethodName>,
    /// The [version](https://modelcontextprotocol.io/specification/versioning) of the Model Context Protocol used.
    pub mcp_protocol_version: Option<String>,
    /// Identifies [MCP session](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#session-management).
    pub mcp_session_id: Option<String>,
    /// `pipe` for a stdio MCP server, `tcp` for an HTTP one.
    pub network_transport: Option<NetworkTransport>,
    /// The error code of the JSON-RPC response.
    pub rpc_response_status_code: Option<String>,
}

impl LabletExecuteTool {
    /// The registry's type for this span.
    pub const TYPE: &'static str = "lablet.execute_tool";
    /// The span's kind.
    pub const KIND: SpanKind = SpanKind::Internal;

    /// The span's name, as the registry builds it.
    pub fn name(&self) -> String {
        format!("execute_tool {}", self.gen_ai_tool_name)
    }

    /// Every attribute that has a value.
    pub fn attributes(&self) -> Vec<KeyValue> {
        let mut attributes = Vec::with_capacity(26);
        attributes.push(KeyValue::new(
            "gen_ai.conversation.id",
            (&self.gen_ai_conversation_id).clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.operation.name",
            (&self.gen_ai_operation_name).as_str().to_owned(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.tool.call.id",
            (&self.gen_ai_tool_call_id).clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.tool.name",
            (&self.gen_ai_tool_name).clone(),
        ));
        attributes.push(KeyValue::new(
            "lablet.config.digest",
            (&self.lablet_config_digest).clone(),
        ));
        attributes.push(KeyValue::new(
            "lablet.tool.input.bytes",
            *(&self.lablet_tool_input_bytes),
        ));
        attributes.push(KeyValue::new(
            "lablet.tool.is_error",
            *(&self.lablet_tool_is_error),
        ));
        attributes.push(KeyValue::new(
            "lablet.tool.output.bytes",
            *(&self.lablet_tool_output_bytes),
        ));
        attributes.push(KeyValue::new(
            "lablet.tool.output.truncated",
            *(&self.lablet_tool_output_truncated),
        ));
        attributes.push(KeyValue::new(
            "lablet.tool.status",
            (&self.lablet_tool_status).as_str().to_owned(),
        ));
        attributes.push(KeyValue::new("lablet.turn", *(&self.lablet_turn)));
        attributes.push(KeyValue::new("session.id", (&self.session_id).clone()));
        if let Some(value) = &self.error_type {
            attributes.push(KeyValue::new("error.type", (value).as_str().to_owned()));
        }
        if let Some(value) = &self.gen_ai_tool_description {
            attributes.push(KeyValue::new("gen_ai.tool.description", (value).clone()));
        }
        if let Some(value) = &self.gen_ai_tool_type {
            attributes.push(KeyValue::new("gen_ai.tool.type", (value).clone()));
        }
        if let Some(value) = &self.jsonrpc_request_id {
            attributes.push(KeyValue::new("jsonrpc.request.id", (value).clone()));
        }
        if let Some(value) = &self.lablet_experiment_id {
            attributes.push(KeyValue::new("lablet.experiment.id", (value).clone()));
        }
        if let Some(value) = &self.lablet_task_id {
            attributes.push(KeyValue::new("lablet.task.id", (value).clone()));
        }
        if let Some(value) = &self.lablet_tool_output_original_bytes {
            attributes.push(KeyValue::new("lablet.tool.output.original_bytes", *(value)));
        }
        if let Some(value) = &self.lablet_tool_source {
            attributes.push(KeyValue::new(
                "lablet.tool.source",
                (value).as_str().to_owned(),
            ));
        }
        if let Some(value) = &self.lablet_trial {
            attributes.push(KeyValue::new("lablet.trial", (value).clone()));
        }
        if let Some(value) = &self.mcp_method_name {
            attributes.push(KeyValue::new(
                "mcp.method.name",
                (value).as_str().to_owned(),
            ));
        }
        if let Some(value) = &self.mcp_protocol_version {
            attributes.push(KeyValue::new("mcp.protocol.version", (value).clone()));
        }
        if let Some(value) = &self.mcp_session_id {
            attributes.push(KeyValue::new("mcp.session.id", (value).clone()));
        }
        if let Some(value) = &self.network_transport {
            attributes.push(KeyValue::new(
                "network.transport",
                (value).as_str().to_owned(),
            ));
        }
        if let Some(value) = &self.rpc_response_status_code {
            attributes.push(KeyValue::new("rpc.response.status_code", (value).clone()));
        }
        attributes
    }

    /// Sets every attribute that has a value on `span`.
    pub fn record(&self, span: &mut impl Span) {
        span.set_attributes(self.attributes());
    }
}
