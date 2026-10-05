//! The wide event: the one record of a run that says everything worth
//! knowing about it, filled from the run's context and its summary.
//!
//! The record's struct is generated from the registry, so a key the registry
//! adds to `lablet.run` is a field this module must fill before the crate
//! builds again, and a key it drops is a field that no longer compiles. What
//! only the export crate knows, each destination's count of lost records, is
//! filled there, from the struct this module hands over.

use std::collections::BTreeMap;

use lablet_model::{FinishReason, RunContext, RunSummary, Thinking, ToolName, ToolStats};
use lablet_run::telemetry::count_of;
use lablet_run::telemetry::generated::Join;

use crate::telemetry::generated::LabletRun;
use crate::telemetry::spellings::run_error_type;

/// The wide event of a run of `context` that came to `summary`, with every
/// key but `lablet.telemetry.dropped_records`, which each destination fills
/// with its own count once it has flushed. A key whose condition doesn't
/// hold of the run is left out, and never written as a zero or as nothing.
///
/// The per-tool keys are made for the tools the run offered, each when it
/// was called. So the names the record lists as its tools bound them,
/// whatever shares a summary holds.
pub(crate) fn wide_event(context: &RunContext, summary: &RunSummary) -> LabletRun {
    let outcome = &summary.outcome;
    let (model, request) = (&summary.model, &summary.request);
    let (usage, failed) = (outcome.usage, summary.failed_usage);
    let result = outcome.result();
    let reason = outcome.stop_reason();
    let (endpoint, mcp) = (summary.endpoint.as_ref(), context.mcp.as_ref());
    let rates = summary.rates;
    let captured = context.capture_content.then_some(result);
    let called: Vec<(&ToolName, &ToolStats)> = summary
        .tools
        .iter()
        .filter_map(|tool| summary.per_tool.get(tool).map(|share| (tool, share)))
        .collect();
    let per_tool = |held: fn(&ToolStats) -> u64| -> BTreeMap<String, i64> {
        called
            .iter()
            .map(|(tool, share)| (tool.as_str().to_owned(), count_of(held(share))))
            .collect()
    };

    LabletRun {
        join: Join::from(context),
        gen_ai_agent_version: context.agent_version.clone(),
        gen_ai_provider_name: model.api.provider().as_str().to_owned(),
        gen_ai_request_max_tokens: i64::from(request.max_tokens),
        gen_ai_request_model: model.name.clone(),
        gen_ai_response_finish_reasons: texts(
            summary.finish_reasons.iter().map(FinishReason::as_str),
        ),
        gen_ai_usage_input_tokens: count_of(usage.input_tokens),
        gen_ai_usage_output_tokens: count_of(usage.output_tokens),
        lablet_prompt_system_bytes: count_of(summary.prompt.system_bytes),
        lablet_prompt_system_digest: summary.system_prompt_digest.clone(),
        lablet_prompt_tools_bytes: count_of(summary.prompt.tools_bytes),
        lablet_prompt_user_bytes: count_of(summary.prompt.user_bytes),
        lablet_provider_latency_ms_max: count_of(summary.provider.latency.max_ms()),
        lablet_provider_latency_ms_total: count_of(summary.provider.latency.total_ms()),
        lablet_provider_retries: count_of(summary.provider.retries),
        lablet_request_api: model.api.into(),
        lablet_request_cache_scope: request.cache_scope.into(),
        lablet_request_reasoning_replayed: model.replays_reasoning,
        lablet_request_thinking: thinking(request.thinking),
        lablet_result_has_structured: result.structured.is_some(),
        lablet_result_text_bytes: size(result.text.len()),
        lablet_run_completion_mode: summary.completion.into(),
        lablet_run_duration_ms: count_of(outcome.duration_ms),
        lablet_run_stop_reason: reason.into(),
        lablet_run_timeout_ms: count_of(summary.timeout_ms),
        lablet_run_turns: i64::from(outcome.turns),
        lablet_skills_count: i64::from(context.skills_count),
        lablet_telemetry_dropped_records: 0,
        lablet_tool_calls_errors: count_of(summary.tool_calls.errors),
        lablet_tool_calls_input_bytes_total: count_of(summary.tool_calls.input_bytes),
        lablet_tool_calls_latency_ms_total: count_of(summary.tool_calls.latency_ms),
        lablet_tool_calls_output_bytes_total: count_of(summary.tool_calls.output_bytes),
        lablet_tool_calls_total: count_of(outcome.tool_calls),
        lablet_tool_calls_truncated: count_of(summary.tool_calls.truncated),
        lablet_tool_calls_unknown: count_of(summary.tool_calls.unknown),
        lablet_tools_count: size(summary.tools.len()),
        lablet_tools_digest: summary.tools_digest.clone(),
        lablet_tools_names: texts(summary.tools.iter().map(ToolName::as_str)),
        error_type: run_error_type(reason),
        gen_ai_request_reasoning_level: request.effort.map(|effort| effort.as_str().to_owned()),
        gen_ai_request_seed: request.seed,
        gen_ai_request_temperature: request.temperature,
        gen_ai_usage_cache_read_input_tokens: usage.cache_read_tokens.map(count_of),
        gen_ai_usage_cache_write_input_tokens: usage.cache_write_tokens.map(count_of),
        gen_ai_usage_reasoning_output_tokens: usage.reasoning_output_tokens.map(count_of),
        lablet_mcp_lifetime: mcp.map(|mcp| mcp.lifetime().into()),
        lablet_mcp_server_versions: mcp.map(|mcp| texts(mcp.versions())),
        lablet_mcp_servers: mcp.map(|mcp| texts(mcp.names())),
        lablet_pricing_cache_read_usd_per_mtok: rates.map(|rates| rates.cache_read),
        lablet_pricing_cache_write_usd_per_mtok: rates.map(|rates| rates.cache_write),
        lablet_pricing_input_usd_per_mtok: rates.map(|rates| rates.input),
        lablet_pricing_output_usd_per_mtok: rates.map(|rates| rates.output),
        lablet_provider_failed_cache_read_input_tokens: failed
            .and_then(|failed| failed.cache_read_tokens)
            .map(count_of),
        lablet_provider_failed_cache_write_input_tokens: failed
            .and_then(|failed| failed.cache_write_tokens)
            .map(count_of),
        lablet_provider_failed_input_tokens: failed.map(|failed| count_of(failed.input_tokens)),
        lablet_provider_failed_output_tokens: failed.map(|failed| count_of(failed.output_tokens)),
        lablet_result_structured: captured
            .and_then(|result| result.structured.as_ref())
            .map(ToString::to_string),
        lablet_result_text: captured.map(|result| result.text.clone()),
        lablet_run_cost_usd: summary.cost.map(lablet_model::Cost::usd),
        lablet_run_error: outcome.error().map(str::to_owned),
        lablet_run_max_turns: summary.max_turns.map(|cap| i64::from(cap.get())),
        lablet_run_transcript_path: context
            .transcript_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        lablet_tool_calls: per_tool(|share| share.calls),
        lablet_tool_errors: per_tool(|share| share.errors),
        lablet_tool_latency_ms: per_tool(|share| share.latency_ms),
        server_address: endpoint.map(|endpoint| endpoint.host.clone()),
        server_port: endpoint.map(|endpoint| i64::from(endpoint.port)),
    }
}

/// How the record spells the way the model was asked to reason: the mode,
/// and for a budget its tokens after a colon.
fn thinking(thinking: Thinking) -> String {
    match thinking {
        Thinking::ProviderDefault => "provider_default".to_owned(),
        Thinking::Adaptive => "adaptive".to_owned(),
        Thinking::Budget(tokens) => format!("budget:{tokens}"),
        Thinking::Disabled => "disabled".to_owned(),
    }
}

fn texts<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<String> {
    texts.map(str::to_owned).collect()
}

/// A size as the wire carries it; one too large for it is the largest the
/// wire can say.
fn size(bytes: usize) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests;
