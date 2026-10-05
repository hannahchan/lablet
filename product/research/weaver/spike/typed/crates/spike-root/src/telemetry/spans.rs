//! This crate's spans, one struct each. A required attribute is a field, and
//! any other is an `Option`, so a span can't be recorded without what the
//! registry requires of it.

use opentelemetry::trace::{Span, SpanKind};
use opentelemetry::{Array, KeyValue, StringValue, Value};

#[allow(unused_imports)]
use super::enums::*;

/// The root span of a run. Follows the GenAI `invoke_agent` span for an agent in the same process.
#[derive(Debug, Clone, PartialEq)]
pub struct LabletInvokeAgent {
    /// Always `lablet`.
    pub gen_ai_agent_name: String,
    /// The lablet version.
    pub gen_ai_agent_version: String,
    /// The run id.
    pub gen_ai_conversation_id: String,
    /// Always `invoke_agent`.
    pub gen_ai_operation_name: GenAiOperationName,
    /// The name of the GenAI model a request is being made to.
    pub gen_ai_request_model: String,
    /// The number of tokens used in the GenAI input (prompt).
    pub gen_ai_usage_input_tokens: i64,
    /// The number of tokens used in the GenAI response (completion).
    pub gen_ai_usage_output_tokens: i64,
    /// SHA-256 of the resolved config, in hex.
    pub lablet_config_digest: String,
    /// Why the run ended.
    pub lablet_run_stop_reason: LabletRunStopReason,
    /// Number of turns the run took.
    pub lablet_run_turns: i64,
    /// Number of tool calls executed. The intercepted `task_complete` call isn't one, and neither is a call that was never run.
    pub lablet_tool_calls_total: i64,
    /// The run id, for backends that group by session.
    pub session_id: String,
    /// The stop reason, when the run didn't complete.
    pub error_type: Option<ErrorType>,
    /// The number of input tokens served from a provider-managed cache.
    pub gen_ai_usage_cache_read_input_tokens: Option<i64>,
    /// The number of input tokens written to a provider-managed cache.
    pub gen_ai_usage_cache_write_input_tokens: Option<i64>,
    /// The number of output tokens used for reasoning (e.g. chain-of-thought, extended thinking).
    pub gen_ai_usage_reasoning_output_tokens: Option<i64>,
    /// The experiment the run is part of, as the run request named it.
    pub lablet_experiment_id: Option<String>,
    /// Cost of the run in US dollars, from the configured pricing.
    pub lablet_run_cost_usd: Option<f64>,
    /// The task the run attempts, as the run request named it.
    pub lablet_task_id: Option<String>,
    /// Which repetition of the task the run is, as the run request named it.
    pub lablet_trial: Option<String>,
}

impl LabletInvokeAgent {
    /// The registry's type for this span.
    pub const TYPE: &'static str = "lablet.invoke_agent";
    /// The span's kind.
    pub const KIND: SpanKind = SpanKind::Internal;

    /// The span's name, as the registry builds it.
    pub fn name(&self) -> String {
        format!("invoke_agent {}", self.gen_ai_agent_name)
    }

    /// Every attribute that has a value.
    pub fn attributes(&self) -> Vec<KeyValue> {
        let mut attributes = Vec::with_capacity(20);
        attributes.push(KeyValue::new(
            "gen_ai.agent.name",
            (&self.gen_ai_agent_name).clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.agent.version",
            (&self.gen_ai_agent_version).clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.conversation.id",
            (&self.gen_ai_conversation_id).clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.operation.name",
            (&self.gen_ai_operation_name).as_str().to_owned(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.request.model",
            (&self.gen_ai_request_model).clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.input_tokens",
            *(&self.gen_ai_usage_input_tokens),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.output_tokens",
            *(&self.gen_ai_usage_output_tokens),
        ));
        attributes.push(KeyValue::new(
            "lablet.config.digest",
            (&self.lablet_config_digest).clone(),
        ));
        attributes.push(KeyValue::new(
            "lablet.run.stop_reason",
            (&self.lablet_run_stop_reason).as_str().to_owned(),
        ));
        attributes.push(KeyValue::new("lablet.run.turns", *(&self.lablet_run_turns)));
        attributes.push(KeyValue::new(
            "lablet.tool_calls.total",
            *(&self.lablet_tool_calls_total),
        ));
        attributes.push(KeyValue::new("session.id", (&self.session_id).clone()));
        if let Some(value) = &self.error_type {
            attributes.push(KeyValue::new("error.type", (value).as_str().to_owned()));
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
        if let Some(value) = &self.gen_ai_usage_reasoning_output_tokens {
            attributes.push(KeyValue::new(
                "gen_ai.usage.reasoning.output_tokens",
                *(value),
            ));
        }
        if let Some(value) = &self.lablet_experiment_id {
            attributes.push(KeyValue::new("lablet.experiment.id", (value).clone()));
        }
        if let Some(value) = &self.lablet_run_cost_usd {
            attributes.push(KeyValue::new("lablet.run.cost_usd", *(value)));
        }
        if let Some(value) = &self.lablet_task_id {
            attributes.push(KeyValue::new("lablet.task.id", (value).clone()));
        }
        if let Some(value) = &self.lablet_trial {
            attributes.push(KeyValue::new("lablet.trial", (value).clone()));
        }
        attributes
    }

    /// Sets every attribute that has a value on `span`.
    pub fn record(&self, span: &mut impl Span) {
        span.set_attributes(self.attributes());
    }
}
