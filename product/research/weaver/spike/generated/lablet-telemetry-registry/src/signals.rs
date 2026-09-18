//! Signal inventory: which attribute keys each lablet span and log record carries.
//!
//! DO NOT EDIT. Generated from the registry by `weaver registry generate`.
//! Used by tests to assert that an emitted signal carries no undeclared key.

/// Attribute keys declared on span `lablet.invoke_agent`.
pub const SPAN_LABLET_INVOKE_AGENT_ATTRIBUTES: &[&str] = &[
    "error.type",
    "gen_ai.agent.name",
    "gen_ai.operation.name",
    "gen_ai.usage.input_tokens",
    "gen_ai.usage.output_tokens",
    "lablet.run.stop_reason",
    "lablet.run.turns",
    "lablet.tool.calls",
];

/// Attribute keys declared on event (log record) `lablet.run`.
pub const EVENT_LABLET_RUN_ATTRIBUTES: &[&str] = &[
    "gen_ai.agent.name",
    "gen_ai.usage.input_tokens",
    "lablet.run.stop_reason",
    "lablet.run.turns",
    "lablet.tool.calls",
];

