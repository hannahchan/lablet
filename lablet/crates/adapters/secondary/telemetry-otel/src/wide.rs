//! The wide event: the one record of a run that says everything worth
//! knowing about it.
//!
//! It's the context and the summary that the run's last event brought,
//! flattened to the keys the registry declares for `lablet.run`, and the
//! observer's own count of what it lost. The registry crate has those keys as
//! an enum, and what each holds is one match over it with no wildcard arm.
//! So a key the registry gains doesn't compile until it has a source here,
//! one it loses doesn't compile while it still has one, and each key is
//! filled once, under the name its own variant gives.

use lablet_model::{FinishReason, StopReason, Thinking, ToolName, ToolStats};
use lablet_telemetry_registry::signals::{
    EVENT_LABLET_RUN_NAME, EventLabletRunKey as Key, EventLabletRunTemplate as Template,
};
use opentelemetry::logs::Severity;

use crate::attributes::{Attributes, Held};
use crate::run::{AGENT, Closed};
use crate::signal::Record;

/// The wide event of the run `closed`, of whose spans and other records
/// the exporters lost `dropped`.
///
/// It's in the context of the run's root span and is timed at the run's
/// end. A key whose condition doesn't hold of the run is left out, and
/// never written as a zero or as nothing.
///
/// The per-tool keys are made for the tools the run offered, each when it
/// was called. So the names the record lists as its tools bound them,
/// whatever shares a summary holds.
pub(crate) fn wide_event(closed: &Closed, dropped: u64) -> Record {
    let summary = &closed.summary;
    let of_the_run = Key::ALL
        .iter()
        .fold(Attributes::default(), |attributes, key| {
            attributes.with_any(key.name(), held(*key, closed, dropped))
        });
    let called = summary
        .tools
        .iter()
        .filter_map(|tool| summary.per_tool.get(tool).map(|share| (tool, share)));
    Record {
        name: EVENT_LABLET_RUN_NAME,
        severity: Severity::Info,
        at: closed.at,
        span: closed.root,
        attributes: called.fold(of_the_run, per_tool),
    }
}

/// `attributes`, and every template's key for `tool`.
fn per_tool(attributes: Attributes, (tool, share): (&ToolName, &ToolStats)) -> Attributes {
    Template::ALL
        .iter()
        .fold(attributes, |attributes, template| {
            let held = match template {
                Template::LabletToolCalls => share.calls,
                Template::LabletToolErrors => share.errors,
                Template::LabletToolLatencyMs => share.latency_ms,
            };
            attributes.with_under(template.prefix(), tool.as_str(), held)
        })
}

fn texts<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<String> {
    texts.map(str::to_owned).collect()
}

/// How a record spells the way the model was asked to reason: the mode,
/// and for a budget its tokens after a colon.
fn thinking(thinking: Thinking) -> String {
    match thinking {
        Thinking::ProviderDefault => "provider_default".to_owned(),
        Thinking::Adaptive => "adaptive".to_owned(),
        Thinking::Budget(tokens) => format!("budget:{tokens}"),
        Thinking::Disabled => "disabled".to_owned(),
    }
}

/// What `key` holds of the run `closed`; `None` for a key whose condition
/// doesn't hold of it.
fn held(key: Key, closed: &Closed, dropped: u64) -> Option<Held> {
    let Closed {
        context, summary, ..
    } = closed;
    let (model, request) = (&summary.model, &summary.request);
    let (outcome, failed) = (&summary.outcome, summary.failed_usage);
    let (usage, result) = (outcome.usage, outcome.result());
    let reason = outcome.stop_reason();
    let (endpoint, mcp) = (summary.endpoint.as_ref(), context.mcp.as_ref());
    let (rates, labels) = (summary.rates, &context.labels);
    let captured = context.capture_content.then_some(result);

    match key {
        // Identity
        Key::GenAiConversationId | Key::SessionId => Some(context.run_id.as_str().into()),
        Key::GenAiAgentName => Some(AGENT.into()),
        Key::GenAiAgentVersion => Some(context.agent_version.as_str().into()),
        Key::LabletConfigDigest => Some(context.config_digest.as_str().into()),
        Key::LabletToolsDigest => Some(summary.tools_digest.as_str().into()),
        Key::LabletPromptSystemDigest => Some(summary.system_prompt_digest.as_str().into()),
        Key::LabletTaskId => labels.task.as_deref().map(Held::from),
        Key::LabletExperimentId => labels.experiment.as_deref().map(Held::from),
        Key::LabletTrial => labels.trial.as_deref().map(Held::from),

        // Setup
        Key::GenAiProviderName => Some(model.api.provider().as_str().into()),
        Key::GenAiRequestModel => Some(model.name.as_str().into()),
        Key::ServerAddress => endpoint.map(|endpoint| endpoint.host.as_str().into()),
        Key::ServerPort => endpoint.map(|endpoint| endpoint.port.into()),
        Key::GenAiRequestMaxTokens => Some(request.max_tokens.into()),
        Key::GenAiRequestSeed => request.seed.map(Held::from),
        Key::GenAiRequestReasoningLevel => request.effort.map(|effort| effort.as_str().into()),
        Key::GenAiRequestTemperature => request.temperature.map(Held::from),
        Key::LabletRequestThinking => Some(thinking(request.thinking).into()),
        Key::LabletRequestApi => Some(model.api.as_str().into()),
        Key::LabletRequestReasoningReplayed => Some(model.replays_reasoning.into()),
        Key::LabletRequestCacheScope => Some(request.cache_scope.as_str().into()),
        Key::LabletRunCompletionMode => Some(summary.completion.as_str().into()),
        Key::LabletRunMaxTurns => summary.max_turns.map(|cap| cap.get().into()),
        Key::LabletRunTimeoutMs => Some(summary.timeout_ms.into()),
        Key::LabletToolsNames => Some(texts(summary.tools.iter().map(ToolName::as_str)).into()),
        Key::LabletToolsCount => Some((summary.tools.len() as u64).into()),
        Key::LabletMcpServers => mcp.map(|mcp| texts(mcp.names()).into()),
        Key::LabletMcpServerVersions => mcp.map(|mcp| texts(mcp.versions()).into()),
        Key::LabletMcpLifetime => mcp.map(|mcp| mcp.lifetime().as_str().into()),
        Key::LabletPromptSystemBytes => Some(summary.prompt.system_bytes.into()),
        Key::LabletPromptUserBytes => Some(summary.prompt.user_bytes.into()),
        Key::LabletPromptToolsBytes => Some(summary.prompt.tools_bytes.into()),
        Key::LabletSkillsCount => Some(context.skills_count.into()),

        // Outcome
        Key::LabletRunStopReason => Some(reason.as_str().into()),
        Key::ErrorType => (reason != StopReason::Completed).then(|| reason.as_str().into()),
        Key::LabletRunError => outcome.error().map(Held::from),
        Key::LabletRunDurationMs => Some(outcome.duration_ms.into()),
        Key::LabletRunTurns => Some(outcome.turns.into()),
        Key::LabletResultTextBytes => Some((result.text.len() as u64).into()),
        Key::LabletResultHasStructured => Some(result.structured.is_some().into()),
        Key::LabletRunTranscriptPath => context
            .transcript_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned().into()),
        Key::LabletTelemetryDroppedRecords => Some(dropped.into()),

        // Provider
        Key::LabletProviderRetries => Some(summary.provider.retries.into()),
        Key::LabletProviderLatencyMsTotal => Some(summary.provider.latency.total_ms().into()),
        Key::LabletProviderLatencyMsMax => Some(summary.provider.latency.max_ms().into()),
        Key::GenAiUsageInputTokens => Some(usage.input_tokens.into()),
        Key::GenAiUsageOutputTokens => Some(usage.output_tokens.into()),
        Key::GenAiUsageReasoningOutputTokens => usage.reasoning_output_tokens.map(Held::from),
        Key::GenAiUsageCacheReadInputTokens => usage.cache_read_tokens.map(Held::from),
        Key::GenAiUsageCacheWriteInputTokens => usage.cache_write_tokens.map(Held::from),
        Key::LabletProviderFailedInputTokens => failed.map(|failed| failed.input_tokens.into()),
        Key::LabletProviderFailedOutputTokens => failed.map(|failed| failed.output_tokens.into()),
        Key::LabletProviderFailedCacheReadInputTokens => failed
            .and_then(|failed| failed.cache_read_tokens)
            .map(Held::from),
        Key::LabletProviderFailedCacheWriteInputTokens => failed
            .and_then(|failed| failed.cache_write_tokens)
            .map(Held::from),
        Key::GenAiResponseFinishReasons => {
            Some(texts(summary.finish_reasons.iter().map(FinishReason::as_str)).into())
        }
        Key::LabletRunCostUsd => summary.cost.map(|cost| cost.usd().into()),
        Key::LabletPricingInputUsdPerMtok => rates.map(|rates| rates.input.into()),
        Key::LabletPricingOutputUsdPerMtok => rates.map(|rates| rates.output.into()),
        Key::LabletPricingCacheReadUsdPerMtok => rates.map(|rates| rates.cache_read.into()),
        Key::LabletPricingCacheWriteUsdPerMtok => rates.map(|rates| rates.cache_write.into()),

        // Tools
        Key::LabletToolCallsTotal => Some(outcome.tool_calls.into()),
        Key::LabletToolCallsErrors => Some(summary.tool_calls.errors.into()),
        Key::LabletToolCallsUnknown => Some(summary.tool_calls.unknown.into()),
        Key::LabletToolCallsTruncated => Some(summary.tool_calls.truncated.into()),
        Key::LabletToolCallsLatencyMsTotal => Some(summary.tool_calls.latency_ms.into()),
        Key::LabletToolCallsInputBytesTotal => Some(summary.tool_calls.input_bytes.into()),
        Key::LabletToolCallsOutputBytesTotal => Some(summary.tool_calls.output_bytes.into()),

        // Content
        Key::LabletResultText => captured.map(|result| result.text.as_str().into()),
        Key::LabletResultStructured => captured
            .and_then(|result| result.structured.as_ref())
            .map(|structured| structured.to_string().into()),
    }
}

#[cfg(test)]
mod tests;
