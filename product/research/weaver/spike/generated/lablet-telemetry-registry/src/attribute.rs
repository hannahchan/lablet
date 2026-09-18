//! Attribute key constants: every attribute the lablet registry defines or references.
//!
//! DO NOT EDIT. Generated from the registry by `weaver registry generate`.

// ---- namespace `error` ----

/// Describes a class of error the operation ended with.
///
/// ## Notes
///
/// The `error.type` SHOULD be predictable, and SHOULD have low cardinality.
///
/// When `error.type` is set to a type (e.g., an exception type), its
/// canonical class name identifying the type within the artifact SHOULD be used.
///
/// If the recorded error type is a wrapper that is not meaningful for
/// failure classification, instrumentation MAY use the type of the inner
/// error instead. For example, in Go, errors created with `fmt.Errorf`
/// using `%w` MAY be unwrapped when the wrapper type does not help
/// classify the failure.
///
/// Instrumentations SHOULD document the list of errors they report.
///
/// The cardinality of `error.type` within one instrumentation library SHOULD be low.
/// Telemetry consumers that aggregate data from multiple instrumentation libraries and applications
/// should be prepared for `error.type` to have high cardinality at query time when no
/// additional filters are applied.
///
/// If the operation has completed successfully, instrumentations SHOULD NOT set `error.type`.
///
/// If a specific domain defines its own set of error identifiers (such as HTTP or RPC status codes),
/// it's RECOMMENDED to:
///
/// - Use a domain-specific attribute
/// - Set `error.type` to capture all errors, regardless of whether they are defined within the domain-specific set or not
pub const ERROR_TYPE: &str = "error.type";

// ---- namespace `gen_ai` ----

/// Human-readable name of the GenAI agent provided by the application
pub const GEN_AI_AGENT_NAME: &str = "gen_ai.agent.name";

/// The name of the operation being performed.
///
/// ## Notes
///
/// If one of the predefined values applies, but specific system uses a different name it's RECOMMENDED to document it in the semantic conventions for specific GenAI system and use system-specific name in the instrumentation. If a different name is not documented, instrumentation libraries SHOULD use applicable predefined value
pub const GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";

/// The number of tokens used in the GenAI input (prompt).
///
/// ## Notes
///
/// This value SHOULD include all types of input tokens, including cached tokens.
/// Instrumentations SHOULD make a best effort to populate this value, using a total
/// provided by the provider when available or, depending on the provider API,
/// by summing different token types parsed from the provider output.
///
/// When the provider reports both billed token counts and model-consumed
/// token counts (for example, Cohere exposes both `usage.billed_units` and
/// `usage.tokens`), instrumentations SHOULD report the billed count so the
/// value matches the units the customer is charged for.
///
/// Detailed usage attributes are subsets of total and aggregate counts. For example,
/// if a request has 100 text tokens (40 cached) and 200 image tokens:
///
/// - `gen_ai.usage.input_tokens`: 300
/// - `gen_ai.usage.cache_read.input_tokens`: 40
/// - `gen_ai.usage.text.input_tokens`: 100
/// - `gen_ai.usage.text.cache_read.input_tokens`: 40
/// - `gen_ai.usage.image.input_tokens`: 200
pub const GEN_AI_USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";

/// The number of tokens used in the GenAI response (completion).
///
/// ## Notes
///
/// When the provider reports both billed token counts and model-consumed
/// token counts (for example, Cohere exposes both `usage.billed_units` and
/// `usage.tokens`), instrumentations SHOULD report the billed count so the
/// value matches the units the customer is charged for
pub const GEN_AI_USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";

// ---- namespace `lablet` ----

/// Why the run ended.
///
/// ## Notes
///
/// Justification: the GenAI conventions have no per-run outcome; `gen_ai.response.finish_reasons` is per inference call and cannot express budget or timeout stops
pub const LABLET_RUN_STOP_REASON: &str = "lablet.run.stop_reason";

/// Number of loop turns the run took.
///
/// ## Notes
///
/// Justification: the conventions count inference calls (`gen_ai.invoke_agent.inference_calls` metric) but a lablet turn may hold several tool calls; the turn count is the loop's own unit
pub const LABLET_RUN_TURNS: &str = "lablet.run.turns";

/// Number of calls to a tool during the run, `<key>` being the tool name.
///
/// ## Notes
///
/// Justification: per-tool counts on the wide event so an analyst can answer "which tool dominated" from one row. Weaver template attributes allow a dynamic suffix and nothing else.
///
/// Template attribute (`template[int]`): use `lablet_tool_calls(suffix)` to build the full key
pub const LABLET_TOOL_CALLS: &str = "lablet.tool.calls";

/// Builds the full key `lablet.tool.calls.<suffix>` for the template attribute `lablet.tool.calls`.
pub fn lablet_tool_calls(suffix: &str) -> String {
    format!("{}.{}", LABLET_TOOL_CALLS, suffix)
}

