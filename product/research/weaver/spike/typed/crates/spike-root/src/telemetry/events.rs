//! This crate's events, one struct each, written as `tracing` events that the
//! OpenTelemetry appender turns into log records, or as span events when the
//! registry annotates them so. The generator writes each
//! field's name where the macro needs it, which a hand-written call can't
//! take from a constant.

#[allow(unused_imports)]
use super::enums::*;

/// The wide event: one log record for each run, carrying everything worth knowing about it.
#[derive(Debug, Clone, PartialEq)]
pub struct LabletRun {
    /// Always `lablet`.
    pub gen_ai_agent_name: String,
    /// The lablet version.
    pub gen_ai_agent_version: String,
    /// The run id.
    pub gen_ai_conversation_id: String,
    /// The Generative AI provider as identified by the client or server instrumentation.
    pub gen_ai_provider_name: GenAiProviderName,
    /// The maximum number of tokens the model generates for a request.
    pub gen_ai_request_max_tokens: i64,
    /// The name of the GenAI model a request is being made to.
    pub gen_ai_request_model: String,
    /// The finish reason of each successful provider call, in order.
    pub gen_ai_response_finish_reasons: Vec<String>,
    /// Input tokens summed over every successful provider call, cached tokens included.
    pub gen_ai_usage_input_tokens: i64,
    /// Output tokens summed over every successful provider call.
    pub gen_ai_usage_output_tokens: i64,
    /// SHA-256 of the resolved config, in hex.
    pub lablet_config_digest: String,
    /// Size of the system prompt in bytes, skills included.
    pub lablet_prompt_system_bytes: i64,
    /// SHA-256 of the system prompt as it was sent, in hex.
    pub lablet_prompt_system_digest: String,
    /// Size of the tool specs in bytes, as the run offered them.
    pub lablet_prompt_tools_bytes: i64,
    /// Size of the task prompt in bytes.
    pub lablet_prompt_user_bytes: i64,
    /// Latency of the slowest provider call attempt, in milliseconds.
    pub lablet_provider_latency_ms_max: i64,
    /// Sum of the latencies of every provider call attempt, in milliseconds.
    pub lablet_provider_latency_ms_total: i64,
    /// Number of provider call attempts made beyond the first of their call.
    pub lablet_provider_retries: i64,
    /// The API the run reached its model through.
    pub lablet_request_api: LabletRequestApi,
    /// Which runs share what the provider caches of the run's requests.
    pub lablet_request_cache_scope: LabletRequestCacheScope,
    /// Whether the reasoning of earlier responses was sent back on later calls.
    pub lablet_request_reasoning_replayed: bool,
    /// How the model was asked to reason, as the config spells it.
    pub lablet_request_thinking: String,
    /// Whether the run produced a structured result, the `task_complete` argument.
    pub lablet_result_has_structured: bool,
    /// Size of the final assistant text in bytes.
    pub lablet_result_text_bytes: i64,
    /// How the run decides that the model has finished.
    pub lablet_run_completion_mode: LabletRunCompletionMode,
    /// Wall-clock duration of the run, in milliseconds.
    pub lablet_run_duration_ms: i64,
    /// Why the run ended.
    pub lablet_run_stop_reason: LabletRunStopReason,
    /// The configured run timeout, in milliseconds.
    pub lablet_run_timeout_ms: i64,
    /// Number of turns the run took.
    pub lablet_run_turns: i64,
    /// Number of skill files appended to the system prompt.
    pub lablet_skills_count: i64,
    /// Number of spans and log records of the run that the exporters to this destination refused or dropped before the wide event was made.
    pub lablet_telemetry_dropped_records: i64,
    /// Number of tool calls that returned an error result.
    pub lablet_tool_calls_errors: i64,
    /// Sum of the sizes of every tool call's input, in bytes.
    pub lablet_tool_calls_input_bytes_total: i64,
    /// Sum of the latencies of every tool call, in milliseconds.
    pub lablet_tool_calls_latency_ms_total: i64,
    /// Sum of the sizes of every tool call's output, in bytes.
    pub lablet_tool_calls_output_bytes_total: i64,
    /// Number of tool calls executed. The intercepted `task_complete` call isn't one, and neither is a call that was never run.
    pub lablet_tool_calls_total: i64,
    /// Number of tool calls whose output the output cap cut short.
    pub lablet_tool_calls_truncated: i64,
    /// Number of tool calls that named a tool the run didn't offer.
    pub lablet_tool_calls_unknown: i64,
    /// Number of tools offered to the model.
    pub lablet_tools_count: i64,
    /// SHA-256 of the tool specs as the run offered them, in hex.
    pub lablet_tools_digest: String,
    /// Names of the tools offered to the model, after the allow and deny lists.
    pub lablet_tools_names: Vec<String>,
    /// The run id, for backends that group by session.
    pub session_id: String,
    /// The stop reason, when the run didn't complete.
    pub error_type: Option<ErrorType>,
    /// The reasoning or thinking effort level requested for a GenAI model.
    pub gen_ai_request_reasoning_level: Option<String>,
    /// Requests with same seed value more likely to return same result.
    pub gen_ai_request_seed: Option<i64>,
    /// The temperature setting for the GenAI request.
    pub gen_ai_request_temperature: Option<f64>,
    /// The number of input tokens served from a provider-managed cache.
    pub gen_ai_usage_cache_read_input_tokens: Option<i64>,
    /// The number of input tokens written to a provider-managed cache.
    pub gen_ai_usage_cache_write_input_tokens: Option<i64>,
    /// The part of the output tokens spent on reasoning, summed over every successful provider call. A part of `gen_ai.usage.output_tokens`, never added to it.
    pub gen_ai_usage_reasoning_output_tokens: Option<i64>,
    /// The experiment the run is part of, as the run request named it.
    pub lablet_experiment_id: Option<String>,
    /// How long the run's MCP servers live.
    pub lablet_mcp_lifetime: Option<LabletMcpLifetime>,
    /// The version each MCP server gave of itself when it started, in the order of `lablet.mcp.servers`.
    pub lablet_mcp_server_versions: Option<Vec<String>>,
    /// Names of the configured MCP servers.
    pub lablet_mcp_servers: Option<Vec<String>>,
    /// The price of a million input tokens served from the prompt cache, in US dollars.
    pub lablet_pricing_cache_read_usd_per_mtok: Option<f64>,
    /// The price of a million input tokens written to the prompt cache, in US dollars.
    pub lablet_pricing_cache_write_usd_per_mtok: Option<f64>,
    /// The price of a million uncached input tokens, in US dollars.
    pub lablet_pricing_input_usd_per_mtok: Option<f64>,
    /// The price of a million output tokens, in US dollars. Reasoning tokens bill at this rate.
    pub lablet_pricing_output_usd_per_mtok: Option<f64>,
    /// The part of `lablet.provider.failed.input_tokens` served from the provider's prompt cache.
    pub lablet_provider_failed_cache_read_input_tokens: Option<i64>,
    /// The part of `lablet.provider.failed.input_tokens` written to the provider's prompt cache.
    pub lablet_provider_failed_cache_write_input_tokens: Option<i64>,
    /// Input tokens that failed provider call attempts reported, summed, cached tokens included.
    pub lablet_provider_failed_input_tokens: Option<i64>,
    /// Output tokens that failed provider call attempts reported, summed.
    pub lablet_provider_failed_output_tokens: Option<i64>,
    /// The structured result, the `task_complete` argument, as a JSON string.
    pub lablet_result_structured: Option<String>,
    /// The final assistant text.
    pub lablet_result_text: Option<String>,
    /// Cost of the run in US dollars, from the configured pricing.
    pub lablet_run_cost_usd: Option<f64>,
    /// The message of the error that ended the run.
    pub lablet_run_error: Option<String>,
    /// The configured cap on turns.
    pub lablet_run_max_turns: Option<i64>,
    /// Where the run's transcript is written.
    pub lablet_run_transcript_path: Option<String>,
    /// The task the run attempts, as the run request named it.
    pub lablet_task_id: Option<String>,
    /// Number of calls to one tool, `<key>` being the tool name.
    pub lablet_tool_calls: std::collections::BTreeMap<String, i64>,
    /// Number of error results from one tool, `<key>` being the tool name.
    pub lablet_tool_errors: std::collections::BTreeMap<String, i64>,
    /// Sum of the latencies of the calls to one tool, in milliseconds, `<key>` being the tool name.
    pub lablet_tool_latency_ms: std::collections::BTreeMap<String, i64>,
    /// Which repetition of the task the run is, as the run request named it.
    pub lablet_trial: Option<String>,
    /// Server domain name if available without reverse DNS lookup; otherwise, IP address or UNIX domain socket name.
    pub server_address: Option<String>,
    /// Server port number.
    pub server_port: Option<i64>,
}

impl LabletRun {
    /// The record's event name.
    pub const NAME: &'static str = "lablet.run";

    compile_error!(
        "lablet.run has 73 attributes, more than a `tracing` event can carry; emit it through the Logs Bridge API"
    );
}
