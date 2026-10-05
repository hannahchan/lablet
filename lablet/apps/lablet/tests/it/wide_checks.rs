//! What a run's exports are held to, whatever made them: the wide event
//! against the spans beside it, against what the run returned, and against
//! the registry; and two destinations against each other.
//!
//! The registry's key lists aren't in the generated modules, which hold a
//! key as a struct field, so the required set and the declared set of each
//! signal are held here as lists of the generated constants: a key the
//! registry adds fails the build at the struct's call site and, once it's
//! filled, these lists until they name it.

use std::collections::{BTreeMap, BTreeSet};

use lablet::telemetry::generated::{LabletInvokeAgent, LabletRun};
use lablet_conformance::otlp::{Attributes, Exported, LogRecord, Span};
use lablet_model::RunSummary;
use lablet_run::telemetry::generated::{LabletChat, LabletExecuteTool};
use serde_json::Value;

use crate::key;

/// How far a latency the wide event reports may be from the durations of
/// the spans it sums, in milliseconds. Every span is stamped at the offsets
/// the loop measured, so the two agree exactly today; the slack is for a
/// span timed on a clock of its own.
const LATENCY_SLACK_MS: u64 = 5;

/// The keys the registry requires of the root span.
pub const INVOKE_AGENT_REQUIRED: &[&str] = &[
    key::GEN_AI_AGENT_NAME,
    key::GEN_AI_AGENT_VERSION,
    key::GEN_AI_CONVERSATION_ID,
    key::GEN_AI_OPERATION_NAME,
    key::GEN_AI_REQUEST_MODEL,
    key::GEN_AI_USAGE_INPUT_TOKENS,
    key::GEN_AI_USAGE_OUTPUT_TOKENS,
    key::LABLET_CONFIG_DIGEST,
    key::LABLET_RUN_STOP_REASON,
    key::LABLET_RUN_TURNS,
    key::LABLET_TOOL_CALLS_TOTAL,
    key::SESSION_ID,
];

/// Every key the registry declares for the root span.
pub const INVOKE_AGENT_KEYS: &[&str] = &[
    key::ERROR_TYPE,
    key::GEN_AI_AGENT_NAME,
    key::GEN_AI_AGENT_VERSION,
    key::GEN_AI_CONVERSATION_ID,
    key::GEN_AI_OPERATION_NAME,
    key::GEN_AI_REQUEST_MODEL,
    key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
    key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
    key::GEN_AI_USAGE_INPUT_TOKENS,
    key::GEN_AI_USAGE_OUTPUT_TOKENS,
    key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
    key::LABLET_CONFIG_DIGEST,
    key::LABLET_EXPERIMENT_ID,
    key::LABLET_RUN_COST_USD,
    key::LABLET_RUN_STOP_REASON,
    key::LABLET_RUN_TURNS,
    key::LABLET_TASK_ID,
    key::LABLET_TOOL_CALLS_TOTAL,
    key::LABLET_TRIAL,
    key::SESSION_ID,
];

/// The keys the registry requires of a chat span.
pub const CHAT_REQUIRED: &[&str] = &[
    key::GEN_AI_CONVERSATION_ID,
    key::GEN_AI_OPERATION_NAME,
    key::GEN_AI_PROVIDER_NAME,
    key::GEN_AI_REQUEST_MAX_TOKENS,
    key::GEN_AI_REQUEST_MODEL,
    key::LABLET_ATTEMPT,
    key::LABLET_CHAT_PURPOSE,
    key::LABLET_CONFIG_DIGEST,
    key::LABLET_REQUEST_BYTES,
    key::LABLET_TURN,
    key::SESSION_ID,
];

/// Every key the registry declares for a chat span.
pub const CHAT_KEYS: &[&str] = &[
    key::ERROR_TYPE,
    key::GEN_AI_CONVERSATION_ID,
    key::GEN_AI_OPERATION_NAME,
    key::GEN_AI_PROVIDER_NAME,
    key::GEN_AI_REQUEST_MAX_TOKENS,
    key::GEN_AI_REQUEST_MODEL,
    key::GEN_AI_REQUEST_REASONING_LEVEL,
    key::GEN_AI_REQUEST_SEED,
    key::GEN_AI_REQUEST_TEMPERATURE,
    key::GEN_AI_RESPONSE_FINISH_REASONS,
    key::GEN_AI_RESPONSE_ID,
    key::GEN_AI_RESPONSE_MODEL,
    key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
    key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
    key::GEN_AI_USAGE_INPUT_TOKENS,
    key::GEN_AI_USAGE_OUTPUT_TOKENS,
    key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
    key::LABLET_ATTEMPT,
    key::LABLET_CHAT_PURPOSE,
    key::LABLET_CONFIG_DIGEST,
    key::LABLET_EXPERIMENT_ID,
    key::LABLET_REQUEST_BYTES,
    key::LABLET_TASK_ID,
    key::LABLET_TRIAL,
    key::LABLET_TURN,
    key::SERVER_ADDRESS,
    key::SERVER_PORT,
    key::SESSION_ID,
];

/// The keys the registry requires of a tool span.
pub const EXECUTE_TOOL_REQUIRED: &[&str] = &[
    key::GEN_AI_CONVERSATION_ID,
    key::GEN_AI_OPERATION_NAME,
    key::GEN_AI_TOOL_CALL_ID,
    key::GEN_AI_TOOL_NAME,
    key::LABLET_CONFIG_DIGEST,
    key::LABLET_TOOL_INPUT_BYTES,
    key::LABLET_TOOL_IS_ERROR,
    key::LABLET_TOOL_OUTPUT_BYTES,
    key::LABLET_TOOL_OUTPUT_TRUNCATED,
    key::LABLET_TOOL_STATUS,
    key::LABLET_TURN,
    key::SESSION_ID,
];

/// Every key the registry declares for a tool span.
pub const EXECUTE_TOOL_KEYS: &[&str] = &[
    key::ERROR_TYPE,
    key::GEN_AI_CONVERSATION_ID,
    key::GEN_AI_OPERATION_NAME,
    key::GEN_AI_TOOL_CALL_ID,
    key::GEN_AI_TOOL_DESCRIPTION,
    key::GEN_AI_TOOL_NAME,
    key::GEN_AI_TOOL_TYPE,
    key::JSONRPC_REQUEST_ID,
    key::LABLET_CONFIG_DIGEST,
    key::LABLET_EXPERIMENT_ID,
    key::LABLET_TASK_ID,
    key::LABLET_TOOL_INPUT_BYTES,
    key::LABLET_TOOL_IS_ERROR,
    key::LABLET_TOOL_OUTPUT_BYTES,
    key::LABLET_TOOL_OUTPUT_ORIGINAL_BYTES,
    key::LABLET_TOOL_OUTPUT_TRUNCATED,
    key::LABLET_TOOL_SOURCE,
    key::LABLET_TOOL_STATUS,
    key::LABLET_TRIAL,
    key::LABLET_TURN,
    key::MCP_METHOD_NAME,
    key::MCP_PROTOCOL_VERSION,
    key::MCP_SESSION_ID,
    key::NETWORK_TRANSPORT,
    key::RPC_RESPONSE_STATUS_CODE,
    key::SESSION_ID,
];

/// The keys the registry requires of the wide event: the ones the generated
/// struct has as fields of their own type, beside the join keys and the
/// fixed agent name.
pub const RUN_REQUIRED: &[&str] = &[
    key::GEN_AI_AGENT_NAME,
    key::GEN_AI_AGENT_VERSION,
    key::GEN_AI_CONVERSATION_ID,
    key::GEN_AI_PROVIDER_NAME,
    key::GEN_AI_REQUEST_MAX_TOKENS,
    key::GEN_AI_REQUEST_MODEL,
    key::GEN_AI_RESPONSE_FINISH_REASONS,
    key::GEN_AI_USAGE_INPUT_TOKENS,
    key::GEN_AI_USAGE_OUTPUT_TOKENS,
    key::LABLET_CONFIG_DIGEST,
    key::LABLET_PROMPT_SYSTEM_BYTES,
    key::LABLET_PROMPT_SYSTEM_DIGEST,
    key::LABLET_PROMPT_TOOLS_BYTES,
    key::LABLET_PROMPT_USER_BYTES,
    key::LABLET_PROVIDER_LATENCY_MS_MAX,
    key::LABLET_PROVIDER_LATENCY_MS_TOTAL,
    key::LABLET_PROVIDER_RETRIES,
    key::LABLET_REQUEST_API,
    key::LABLET_REQUEST_CACHE_SCOPE,
    key::LABLET_REQUEST_REASONING_REPLAYED,
    key::LABLET_REQUEST_THINKING,
    key::LABLET_RESULT_HAS_STRUCTURED,
    key::LABLET_RESULT_TEXT_BYTES,
    key::LABLET_RUN_COMPLETION_MODE,
    key::LABLET_RUN_DURATION_MS,
    key::LABLET_RUN_STOP_REASON,
    key::LABLET_RUN_TIMEOUT_MS,
    key::LABLET_RUN_TURNS,
    key::LABLET_SKILLS_COUNT,
    key::LABLET_TELEMETRY_DROPPED_RECORDS,
    key::LABLET_TOOL_CALLS_ERRORS,
    key::LABLET_TOOL_CALLS_INPUT_BYTES_TOTAL,
    key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL,
    key::LABLET_TOOL_CALLS_OUTPUT_BYTES_TOTAL,
    key::LABLET_TOOL_CALLS_TOTAL,
    key::LABLET_TOOL_CALLS_TRUNCATED,
    key::LABLET_TOOL_CALLS_UNKNOWN,
    key::LABLET_TOOLS_COUNT,
    key::LABLET_TOOLS_DIGEST,
    key::LABLET_TOOLS_NAMES,
    key::SESSION_ID,
];

/// Every plain key the registry declares for the wide event; its three
/// templates are [`RUN_TEMPLATES`].
pub const RUN_KEYS: &[&str] = &[
    key::ERROR_TYPE,
    key::GEN_AI_AGENT_NAME,
    key::GEN_AI_AGENT_VERSION,
    key::GEN_AI_CONVERSATION_ID,
    key::GEN_AI_PROVIDER_NAME,
    key::GEN_AI_REQUEST_MAX_TOKENS,
    key::GEN_AI_REQUEST_MODEL,
    key::GEN_AI_REQUEST_REASONING_LEVEL,
    key::GEN_AI_REQUEST_SEED,
    key::GEN_AI_REQUEST_TEMPERATURE,
    key::GEN_AI_RESPONSE_FINISH_REASONS,
    key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
    key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
    key::GEN_AI_USAGE_INPUT_TOKENS,
    key::GEN_AI_USAGE_OUTPUT_TOKENS,
    key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
    key::LABLET_CONFIG_DIGEST,
    key::LABLET_EXPERIMENT_ID,
    key::LABLET_MCP_LIFETIME,
    key::LABLET_MCP_SERVER_VERSIONS,
    key::LABLET_MCP_SERVERS,
    key::LABLET_PRICING_CACHE_READ_USD_PER_MTOK,
    key::LABLET_PRICING_CACHE_WRITE_USD_PER_MTOK,
    key::LABLET_PRICING_INPUT_USD_PER_MTOK,
    key::LABLET_PRICING_OUTPUT_USD_PER_MTOK,
    key::LABLET_PROMPT_SYSTEM_BYTES,
    key::LABLET_PROMPT_SYSTEM_DIGEST,
    key::LABLET_PROMPT_TOOLS_BYTES,
    key::LABLET_PROMPT_USER_BYTES,
    key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
    key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
    key::LABLET_PROVIDER_FAILED_INPUT_TOKENS,
    key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS,
    key::LABLET_PROVIDER_LATENCY_MS_MAX,
    key::LABLET_PROVIDER_LATENCY_MS_TOTAL,
    key::LABLET_PROVIDER_RETRIES,
    key::LABLET_REQUEST_API,
    key::LABLET_REQUEST_CACHE_SCOPE,
    key::LABLET_REQUEST_REASONING_REPLAYED,
    key::LABLET_REQUEST_THINKING,
    key::LABLET_RESULT_HAS_STRUCTURED,
    key::LABLET_RESULT_STRUCTURED,
    key::LABLET_RESULT_TEXT,
    key::LABLET_RESULT_TEXT_BYTES,
    key::LABLET_RUN_COMPLETION_MODE,
    key::LABLET_RUN_COST_USD,
    key::LABLET_RUN_DURATION_MS,
    key::LABLET_RUN_ERROR,
    key::LABLET_RUN_MAX_TURNS,
    key::LABLET_RUN_STOP_REASON,
    key::LABLET_RUN_TIMEOUT_MS,
    key::LABLET_RUN_TRANSCRIPT_PATH,
    key::LABLET_RUN_TURNS,
    key::LABLET_SKILLS_COUNT,
    key::LABLET_TASK_ID,
    key::LABLET_TELEMETRY_DROPPED_RECORDS,
    key::LABLET_TOOL_CALLS_ERRORS,
    key::LABLET_TOOL_CALLS_INPUT_BYTES_TOTAL,
    key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL,
    key::LABLET_TOOL_CALLS_OUTPUT_BYTES_TOTAL,
    key::LABLET_TOOL_CALLS_TOTAL,
    key::LABLET_TOOL_CALLS_TRUNCATED,
    key::LABLET_TOOL_CALLS_UNKNOWN,
    key::LABLET_TOOLS_COUNT,
    key::LABLET_TOOLS_DIGEST,
    key::LABLET_TOOLS_NAMES,
    key::LABLET_TRIAL,
    key::SERVER_ADDRESS,
    key::SERVER_PORT,
    key::SESSION_ID,
];

/// The wide event's per-tool templates, each a prefix a tool's name follows
/// after a dot.
pub const RUN_TEMPLATES: &[&str] = &[
    key::LABLET_TOOL_CALLS,
    key::LABLET_TOOL_ERRORS,
    key::LABLET_TOOL_LATENCY_MS,
];

/// Holds `attributes` to the registry's lists for their signal: every key
/// the signal always carries is there, and none is there that the signal
/// doesn't declare, by name or as one of `templates` with a tool's name
/// after it.
pub fn assert_declared(
    signal: &str,
    attributes: &Attributes,
    required: &[&str],
    declared: &[&str],
    templates: &[&str],
) {
    for key in required {
        assert!(
            attributes.contains_key(*key),
            "{signal} lacks `{key}`, which the registry requires of it"
        );
    }
    for key in attributes.keys() {
        let of_a_tool = templates.iter().any(|template| {
            key.strip_prefix(*template)
                .is_some_and(|tool| tool.starts_with('.'))
        });
        assert!(
            declared.contains(&key.as_str()) || of_a_tool,
            "{signal} holds `{key}`, which the registry doesn't declare for it"
        );
    }
}

/// Holds a wide event to the registry: every key the registry requires of
/// `lablet.run` is there, and every key that's there is one the registry
/// declares for it. A template's key is declared for a tool the event
/// lists among the tools the run offered, and for nothing else, which is
/// what bounds the keys.
pub fn assert_the_wide_event_is_declared(wide: &LogRecord) {
    let attributes = &wide.attributes;
    assert_eq!(wide.event_name, LabletRun::NAME);
    let missing: Vec<_> = RUN_REQUIRED
        .iter()
        .filter(|key| !attributes.contains_key(**key))
        .collect();
    assert!(
        missing.is_empty(),
        "the wide event lacks {missing:?}, which the registry requires of it"
    );

    let offered: Vec<&str> = attributes
        .get(key::LABLET_TOOLS_NAMES)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let declared = |key: &str| {
        RUN_KEYS.contains(&key) || template_of(key).is_some_and(|(_, tool)| offered.contains(&tool))
    };
    let undeclared: Vec<_> = attributes.keys().filter(|key| !declared(key)).collect();
    assert!(
        undeclared.is_empty(),
        "the wide event holds {undeclared:?}, which the registry doesn't declare for it, or \
         declares for a tool the run offered and these aren't of one"
    );
}

/// Holds the wide event of the run `run` to the spans `exported` holds of
/// that run: every count, size and token count is the sum of what the spans
/// of the run's steps say, every latency is within 5 ms of how long those
/// spans lasted, and the per-tool keys are those of the tools that were
/// called, among the tools the run offered.
pub fn assert_the_wide_event_sums_its_steps(exported: &Exported, run: &str) {
    let told = Told::of(exported, run);
    told.assert_sums_the_provider_calls();
    told.assert_sums_the_tool_calls();
    told.assert_agrees_with_the_root_span();
}

/// Holds the token counts of the wide event of the run `run` to the ones
/// the run returned, as `summary`: what the provider calls that were
/// answered reported, and what the attempts that failed did. A count that
/// no call reported is one the wide event doesn't hold.
///
/// The spans are the loop's word as the wide event is, so a count misread
/// on the way is said alike in both, and only what the loop returned can
/// tell.
pub fn assert_the_wide_event_counts_the_tokens_the_run_returned(
    exported: &Exported,
    run: &str,
    summary: &RunSummary,
) {
    let wide = &the_wide_event(exported, run).attributes;
    let (usage, failed) = (summary.outcome.usage, summary.failed_usage);
    for (key, returned) in [
        (key::GEN_AI_USAGE_INPUT_TOKENS, Some(usage.input_tokens)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, Some(usage.output_tokens)),
        (
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
            usage.reasoning_output_tokens,
        ),
        (
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            usage.cache_read_tokens,
        ),
        (
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            usage.cache_write_tokens,
        ),
        (
            key::LABLET_PROVIDER_FAILED_INPUT_TOKENS,
            failed.map(|failed| failed.input_tokens),
        ),
        (
            key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS,
            failed.map(|failed| failed.output_tokens),
        ),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
            failed.and_then(|failed| failed.cache_read_tokens),
        ),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
            failed.and_then(|failed| failed.cache_write_tokens),
        ),
    ] {
        assert_eq!(
            counted(wide, key),
            returned,
            "`{key}` of the wide event, and the count the run returned"
        );
    }
}

/// Holds `first` and `second` to the same spans and the same log records,
/// as multisets, and to one wide event of the run `run` each, whose count
/// of dropped records is nothing.
pub fn assert_hold_the_same_run(first: &Exported, second: &Exported, run: &str) {
    for exported in [first, second] {
        let wide = the_wide_event(exported, run);
        assert_eq!(
            counted(&wide.attributes, key::LABLET_TELEMETRY_DROPPED_RECORDS),
            Some(0)
        );
    }
    let (first, second) = (first.ungrouped(), second.ungrouped());
    assert!(!first.spans.is_empty(), "the run has spans");
    assert_eq!(first.spans.len(), second.spans.len(), "as many spans");
    assert_eq!(first.records.len(), second.records.len(), "as many records");
    assert_eq!(
        first, second,
        "the same spans and records, whatever the batches"
    );
}

fn of_the_run(attributes: &Attributes, run: &str) -> bool {
    attributes
        .get(key::GEN_AI_CONVERSATION_ID)
        .and_then(Value::as_str)
        == Some(run)
}

/// The one wide event of the run `run` among `exported`.
pub fn the_wide_event<'a>(exported: &'a Exported, run: &str) -> &'a LogRecord {
    let wide: Vec<_> = exported
        .records_of(LabletRun::NAME)
        .into_iter()
        .filter(|wide| of_the_run(&wide.attributes, run))
        .collect();
    assert_eq!(wide.len(), 1, "the run {run} has one wide event");
    wide[0]
}

fn spans_of<'a>(exported: &'a Exported, operation: &str, run: &str) -> Vec<&'a Span> {
    exported
        .spans_of(operation)
        .into_iter()
        .filter(|span| of_the_run(&span.attributes, run))
        .collect()
}

/// The text `key` holds.
fn text<'a>(attributes: &'a Attributes, key: &str) -> &'a str {
    attributes
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("`{key}` holds no text among {attributes:?}"))
}

/// The count `key` holds, when it's there.
fn counted(attributes: &Attributes, key: &str) -> Option<u64> {
    attributes.get(key).map(|held| {
        held.as_u64()
            .unwrap_or_else(|| panic!("`{key}` holds {held}, which isn't a count"))
    })
}

/// The count `key` holds.
fn count(attributes: &Attributes, key: &str) -> u64 {
    counted(attributes, key).unwrap_or_else(|| panic!("`{key}` isn't among {attributes:?}"))
}

/// Whether `key` holds `true`.
fn is(attributes: &Attributes, key: &str) -> bool {
    attributes.get(key) == Some(&Value::Bool(true))
}

/// The sum of what `key` holds over `spans`, where a span that doesn't hold
/// it adds nothing; `None` when none of them holds it.
fn sum(spans: &[&Span], key: &str) -> Option<u64> {
    spans
        .iter()
        .filter_map(|span| counted(&span.attributes, key))
        .reduce(u64::saturating_add)
}

fn lasted(spans: &[&Span]) -> u64 {
    spans.iter().map(|span| span.duration_ms()).sum()
}

fn how_many(spans: &[&Span], holds: impl Fn(&Attributes) -> bool) -> u64 {
    spans.iter().filter(|span| holds(&span.attributes)).count() as u64
}

fn assert_near(wide: &Attributes, key: &str, measured: u64) {
    let reported = count(wide, key);
    assert!(
        reported.abs_diff(measured) <= LATENCY_SLACK_MS,
        "`{key}` is {reported}, and the spans it sums lasted {measured} ms"
    );
}

/// Whether the call a tool span is of named a tool the run offered.
fn names_an_offered_tool(tool: &Attributes) -> bool {
    text(tool, key::LABLET_TOOL_STATUS) != "unknown"
}

/// The template `key` is a key of, and the suffix it has.
fn template_of(key: &str) -> Option<(&'static str, &str)> {
    RUN_TEMPLATES.iter().find_map(|template| {
        key.strip_prefix(template)
            .and_then(|rest| rest.strip_prefix('.'))
            .map(|suffix| (*template, suffix))
    })
}

/// Holds the per-tool keys of the wide event to the tool spans: a tool has
/// its three keys when a call named it and the run offered it, and no
/// other per-tool key is there.
fn assert_each_tool_has_its_share(wide: &Attributes, tools: &[&Span], names: &[&str]) {
    let mut called: BTreeMap<&str, Vec<&Span>> = BTreeMap::new();
    for tool in tools
        .iter()
        .filter(|tool| names_an_offered_tool(&tool.attributes))
    {
        called
            .entry(text(&tool.attributes, key::GEN_AI_TOOL_NAME))
            .or_default()
            .push(tool);
    }

    let mut expected = BTreeSet::new();
    for (name, calls) in &called {
        assert!(
            names.contains(name),
            "{name} has a share and isn't among the tools the run offered, {names:?}"
        );
        let calls_key = format!("{}.{name}", key::LABLET_TOOL_CALLS);
        assert_eq!(count(wide, &calls_key), calls.len() as u64, "{calls_key}");
        let errors_key = format!("{}.{name}", key::LABLET_TOOL_ERRORS);
        let errors = how_many(calls, |tool| is(tool, key::LABLET_TOOL_IS_ERROR));
        assert_eq!(count(wide, &errors_key), errors, "{errors_key}");
        let latency_key = format!("{}.{name}", key::LABLET_TOOL_LATENCY_MS);
        assert_near(wide, &latency_key, lasted(calls));
        expected.extend([calls_key, errors_key, latency_key]);
    }
    let per_tool: BTreeSet<String> = wide
        .keys()
        .filter(|key| template_of(key).is_some())
        .cloned()
        .collect();
    assert_eq!(per_tool, expected, "the per-tool keys of the wide event");
}

/// A run as its exports tell it.
struct Told<'a> {
    /// The attributes of the run's wide event.
    wide: &'a Attributes,
    root: &'a Span,
    /// The spans of the run's provider call attempts, in order.
    chats: Vec<&'a Span>,
    /// The spans of the run's tool calls.
    tools: Vec<&'a Span>,
}

impl<'a> Told<'a> {
    /// The run `run` among `exported`, which has one wide event, in the
    /// context of its one root span.
    fn of(exported: &'a Exported, run: &str) -> Self {
        let record = the_wide_event(exported, run);
        let roots = spans_of(exported, LabletInvokeAgent::GEN_AI_OPERATION_NAME, run);
        assert_eq!(roots.len(), 1, "the run {run} has one root span");
        let root = roots[0];
        assert_eq!(
            (&record.trace_id, &record.span_id),
            (&root.trace_id, &root.span_id),
            "the wide event is in the context of the root span"
        );
        Self {
            wide: &record.attributes,
            root,
            chats: spans_of(exported, LabletChat::GEN_AI_OPERATION_NAME, run),
            tools: spans_of(exported, LabletExecuteTool::GEN_AI_OPERATION_NAME, run),
        }
    }

    /// Holds each of `sums` to what the wide event holds for its key, which
    /// is nothing for a sum of nothing.
    fn assert_holds(&self, sums: &[(&str, Option<u64>)]) {
        for (key, summed) in sums {
            assert_eq!(counted(self.wide, key), *summed, "{key}");
        }
    }

    /// What the wide event says of the provider is what the chat spans sum
    /// to: the attempts that were answered are the turns, and the rest are
    /// the attempts that failed.
    fn assert_sums_the_provider_calls(&self) {
        let (failed, answered): (Vec<&Span>, Vec<&Span>) = self
            .chats
            .iter()
            .partition(|chat| chat.attributes.contains_key(key::ERROR_TYPE));
        let calls: BTreeSet<u64> = self
            .chats
            .iter()
            .map(|chat| count(&chat.attributes, key::LABLET_TURN))
            .collect();
        let ended_on_a_failure = self
            .chats
            .last()
            .is_some_and(|last| last.attributes.contains_key(key::ERROR_TYPE));

        self.assert_holds(&[
            (key::LABLET_RUN_TURNS, Some(answered.len() as u64)),
            (
                key::LABLET_PROVIDER_RETRIES,
                Some((self.chats.len() - calls.len()) as u64),
            ),
            (
                key::GEN_AI_USAGE_INPUT_TOKENS,
                sum(&answered, key::GEN_AI_USAGE_INPUT_TOKENS).or(Some(0)),
            ),
            (
                key::GEN_AI_USAGE_OUTPUT_TOKENS,
                sum(&answered, key::GEN_AI_USAGE_OUTPUT_TOKENS).or(Some(0)),
            ),
            (
                key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
                sum(&answered, key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS),
            ),
            (
                key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
                sum(&answered, key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS),
            ),
            (
                key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
                sum(&answered, key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS),
            ),
            (
                key::LABLET_PROVIDER_FAILED_INPUT_TOKENS,
                sum(&failed, key::GEN_AI_USAGE_INPUT_TOKENS),
            ),
            (
                key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS,
                sum(&failed, key::GEN_AI_USAGE_OUTPUT_TOKENS),
            ),
            (
                key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
                sum(&failed, key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS),
            ),
            (
                key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
                sum(&failed, key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS),
            ),
        ]);
        assert_eq!(
            self.chats.len() as u64,
            count(self.wide, key::LABLET_RUN_TURNS)
                + count(self.wide, key::LABLET_PROVIDER_RETRIES)
                + u64::from(ended_on_a_failure),
            "the chat spans number the turns and the retries, and one more when the run ended \
             on a call that failed"
        );

        let longest = self.chats.iter().map(|chat| chat.duration_ms()).max();
        assert_near(
            self.wide,
            key::LABLET_PROVIDER_LATENCY_MS_TOTAL,
            lasted(&self.chats),
        );
        assert_near(
            self.wide,
            key::LABLET_PROVIDER_LATENCY_MS_MAX,
            longest.unwrap_or(0),
        );

        let reasons: Vec<Value> = answered
            .iter()
            .filter_map(|chat| chat.attributes.get(key::GEN_AI_RESPONSE_FINISH_REASONS))
            .filter_map(Value::as_array)
            .flatten()
            .cloned()
            .collect();
        assert_eq!(
            self.wide.get(key::GEN_AI_RESPONSE_FINISH_REASONS),
            Some(&Value::Array(reasons)),
            "one finish reason for each response, in order"
        );
    }

    /// What the wide event says of the tool calls is what the tool spans
    /// sum to, and each tool that was called has its share.
    fn assert_sums_the_tool_calls(&self) {
        let tools = &self.tools;
        let names: Vec<&str> = self
            .wide
            .get(key::LABLET_TOOLS_NAMES)
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("the wide event lists no tools: {:?}", self.wide))
            .iter()
            .filter_map(Value::as_str)
            .collect();

        self.assert_holds(&[
            (key::LABLET_TOOL_CALLS_TOTAL, Some(tools.len() as u64)),
            (
                key::LABLET_TOOL_CALLS_ERRORS,
                Some(how_many(tools, |tool| is(tool, key::LABLET_TOOL_IS_ERROR))),
            ),
            (
                key::LABLET_TOOL_CALLS_UNKNOWN,
                Some(how_many(tools, |tool| !names_an_offered_tool(tool))),
            ),
            (
                key::LABLET_TOOL_CALLS_TRUNCATED,
                Some(how_many(tools, |tool| {
                    is(tool, key::LABLET_TOOL_OUTPUT_TRUNCATED)
                })),
            ),
            (
                key::LABLET_TOOL_CALLS_INPUT_BYTES_TOTAL,
                sum(tools, key::LABLET_TOOL_INPUT_BYTES).or(Some(0)),
            ),
            (
                key::LABLET_TOOL_CALLS_OUTPUT_BYTES_TOTAL,
                sum(tools, key::LABLET_TOOL_OUTPUT_BYTES).or(Some(0)),
            ),
            (key::LABLET_TOOLS_COUNT, Some(names.len() as u64)),
        ]);
        assert_near(
            self.wide,
            key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL,
            lasted(tools),
        );
        assert_each_tool_has_its_share(self.wide, tools, &names);
    }

    /// What the wide event and the root span both say of the run, they say
    /// alike, and the run lasted as long as its root span.
    fn assert_agrees_with_the_root_span(&self) {
        for key in [
            key::GEN_AI_USAGE_INPUT_TOKENS,
            key::GEN_AI_USAGE_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            key::LABLET_RUN_TURNS,
            key::LABLET_TOOL_CALLS_TOTAL,
        ] {
            assert_eq!(
                counted(self.wide, key),
                counted(&self.root.attributes, key),
                "{key} of the wide event and of the root span"
            );
        }
        assert_near(
            self.wide,
            key::LABLET_RUN_DURATION_MS,
            self.root.duration_ms(),
        );
        assert_eq!(
            text(self.wide, key::LABLET_RUN_STOP_REASON),
            text(&self.root.attributes, key::LABLET_RUN_STOP_REASON)
        );
    }
}
