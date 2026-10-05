//! The root span `invoke_agent lablet`, as the composition root fills it from
//! the run it covers. Not to be confused with `root`, the directory the
//! built-in tools work under.

use lablet_model::{RunContext, RunOutcome, RunSummary, StopReason};
use lablet_run::telemetry::count_of;
use lablet_run::telemetry::generated::Join;
use opentelemetry::trace::Status;

use crate::telemetry::generated::LabletInvokeAgent;
use crate::telemetry::spellings::invoke_agent_error_type;

/// The span's name, as the registry builds it from the operation and the
/// agent's name, which are fixed before the run has a struct to ask.
pub(crate) fn name() -> String {
    format!(
        "{} {}",
        LabletInvokeAgent::GEN_AI_OPERATION_NAME,
        LabletInvokeAgent::GEN_AI_AGENT_NAME
    )
}

/// The root span of a run of `context` that came to `summary`: what names
/// the run, how it stopped, and the totals a trace is read by.
pub(crate) fn invoke_agent(context: &RunContext, summary: &RunSummary) -> LabletInvokeAgent {
    let outcome = &summary.outcome;
    let usage = outcome.usage;
    LabletInvokeAgent {
        join: Join::from(context),
        gen_ai_agent_version: context.agent_version.clone(),
        gen_ai_request_model: summary.model.name.clone(),
        gen_ai_usage_input_tokens: count_of(usage.input_tokens),
        gen_ai_usage_output_tokens: count_of(usage.output_tokens),
        lablet_run_stop_reason: outcome.stop_reason().into(),
        lablet_run_turns: i64::from(outcome.turns),
        lablet_tool_calls_total: count_of(outcome.tool_calls),
        error_type: invoke_agent_error_type(outcome.stop_reason()),
        gen_ai_usage_cache_read_input_tokens: usage.cache_read_tokens.map(count_of),
        gen_ai_usage_cache_write_input_tokens: usage.cache_write_tokens.map(count_of),
        gen_ai_usage_reasoning_output_tokens: usage.reasoning_output_tokens.map(count_of),
        lablet_run_cost_usd: summary.cost.map(lablet_model::Cost::usd),
    }
}

/// The span's status: unset for a run that completed, and an error that
/// carries the run's error, when it has one, for any other stop reason.
pub(crate) fn status(outcome: &RunOutcome) -> Status {
    if outcome.stop_reason() == StopReason::Completed {
        Status::Unset
    } else {
        Status::error(outcome.error().unwrap_or_default().to_owned())
    }
}

#[cfg(test)]
mod tests;
