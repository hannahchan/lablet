//! The cases every [`RunObserver`] adapter is held to, whatever it exports
//! to.
//!
//! A case runs the loop itself, with a scripted provider, around the
//! observer it's handed, and reads back what the observer exported. An
//! adapter's test says how its observer is flushed and where its exports
//! are read from, as a [`Subject`], and calls each case from a test that
//! pauses tokio's clock.
//!
//! What a case asserts of a run's exports is here as a check of its own
//! too, for a test that made the run another way.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lablet_model::StopReason;
use lablet_run::RunObserver;
use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::signals::{
    EVENT_LABLET_RUN_NAME, EVENT_LABLET_RUN_REQUIRED, EventLabletRunKey, EventLabletRunTemplate,
};
use serde_json::Value;

use crate::must;
use crate::otlp::{Attributes, Exported, LogRecord, ReadError, Span};

mod harness;

use harness::{EVERYTHING, FAILS, OTHER_RUN, RUN, Unobserved, playing, run};

/// How far a latency the wide event reports may be from the durations of
/// the spans it sums, in milliseconds. An observer may time a span on a
/// clock of its own.
const LATENCY_SLACK_MS: u64 = 5;

/// The first word of the name of each kind of span.
const INVOKE_AGENT: &str = "invoke_agent";
const CHAT: &str = "chat";
const EXECUTE_TOOL: &str = "execute_tool";

/// An observer under test, as a case needs it.
#[async_trait::async_trait]
pub trait Subject: Send + Sync {
    /// The observer, as the loop is handed it. Every call gives the same
    /// one.
    fn observer(&self) -> Arc<dyn RunObserver>;

    /// Has the observer export what it has yet to export of the runs that
    /// have ended, as whoever runs the loop does once a run has returned.
    ///
    /// # Errors
    ///
    /// Returns what the observer says it couldn't export.
    async fn flush(&self) -> Result<(), String>;

    /// Everything the observer has exported, read back.
    ///
    /// # Errors
    ///
    /// Returns a [`ReadError`] when what was exported can't be read.
    fn exported(&self) -> Result<Exported, ReadError>;
}

async fn flushed(subject: &dyn Subject) {
    must(subject.flush().await, "flushing the observer");
}

fn read_back(subject: &dyn Subject) -> Exported {
    must(subject.exported(), "reading the exports back")
}

/// Every run has exactly one wide event, whatever its stop reason, in the
/// trace and the context of its root span, and a flush that no run came
/// before adds none.
///
/// # Panics
///
/// Panics when that doesn't hold of `subject`, which is how a case fails.
pub async fn a_run_has_exactly_one_wide_event(subject: &dyn Subject) {
    let mut to_the_end = playing(EVERYTHING, subject.observer()).await;
    let mut to_a_failure = playing(FAILS, subject.observer()).await;

    let completed = run(&mut to_the_end, RUN).await;
    flushed(subject).await;
    let failed = run(&mut to_a_failure, OTHER_RUN).await;
    flushed(subject).await;
    flushed(subject).await;

    let reasons = [
        completed.summary.outcome.stop_reason(),
        failed.summary.outcome.stop_reason(),
    ];
    assert_eq!(reasons, [StopReason::Completed, StopReason::ProviderError]);
    let exported = read_back(subject);
    let wide = exported.records_of(EVENT_LABLET_RUN_NAME);
    let of: Vec<_> = wide
        .iter()
        .map(|wide| text(&wide.attributes, key::GEN_AI_CONVERSATION_ID))
        .collect();
    assert_eq!(of, [RUN, OTHER_RUN], "one wide event for each run");
    for (wide, reason) in wide.into_iter().zip(reasons) {
        assert_eq!(
            text(&wide.attributes, key::LABLET_RUN_STOP_REASON),
            reason.as_str()
        );
        assert_the_wide_event_is_declared(wide);
    }
    assert_the_wide_event_sums_its_steps(&exported, RUN);
    assert_the_wide_event_sums_its_steps(&exported, OTHER_RUN);
}

/// The numbers of a run's wide event are the sums of what the run's spans
/// say of each step, and they're what the run returned.
///
/// # Panics
///
/// Panics when that doesn't hold of `subject`, which is how a case fails.
pub async fn the_numbers_of_the_wide_event_are_the_sums_of_the_steps(subject: &dyn Subject) {
    let mut service = playing(EVERYTHING, subject.observer()).await;

    let finished = run(&mut service, RUN).await;
    flushed(subject).await;

    let exported = read_back(subject);
    assert_the_wide_event_sums_its_steps(&exported, RUN);
    let wide = &the_wide_event(&exported, RUN).attributes;
    let summary = &finished.summary;
    // The script makes every total one that no other total of its kind
    // is, so a total in another's place is seen.
    for (key, measured) in [
        (key::LABLET_RUN_TURNS, 3),
        (key::LABLET_PROVIDER_RETRIES, 4),
        (key::LABLET_TOOL_CALLS_TOTAL, 7),
        (key::LABLET_TOOL_CALLS_ERRORS, 3),
        (key::LABLET_TOOL_CALLS_UNKNOWN, 1),
        (key::LABLET_TOOL_CALLS_TRUNCATED, 2),
    ] {
        assert_eq!(count(wide, key), measured, "{key}");
    }
    assert_eq!(u64::from(summary.outcome.turns), 3);
    assert_eq!(summary.provider.retries, 4);
    assert_eq!(summary.outcome.tool_calls, 7);
    assert_eq!(
        (
            summary.tool_calls.errors,
            summary.tool_calls.unknown,
            summary.tool_calls.truncated
        ),
        (3, 1, 2)
    );
    assert_eq!(
        count(wide, key::LABLET_RUN_DURATION_MS),
        summary.outcome.duration_ms
    );
}

/// A destination that can't be written changes nothing about the run: what
/// the loop returns is what it returns when nothing observes it. The
/// failure is the observer's to report, to whoever flushes it.
///
/// `unwritable` is a subject whose observer exports to a destination that
/// can't be written.
///
/// # Panics
///
/// Panics when that doesn't hold of `unwritable`, which is how a case
/// fails.
pub async fn a_destination_that_cannot_be_written_changes_nothing_about_the_run(
    unwritable: &dyn Subject,
) {
    let mut unobserved = playing(EVERYTHING, Arc::new(Unobserved)).await;
    let mut observed = playing(EVERYTHING, unwritable.observer()).await;

    let expected = run(&mut unobserved, RUN).await;
    let finished = run(&mut observed, RUN).await;
    let flushed = unwritable.flush().await;

    assert_eq!(finished, expected);
    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
    assert!(
        flushed.is_err(),
        "the destination was written, so the case held nothing to the observer"
    );
}

fn of_the_run(attributes: &Attributes, run: &str) -> bool {
    attributes
        .get(key::GEN_AI_CONVERSATION_ID)
        .and_then(Value::as_str)
        == Some(run)
}

fn the_wide_event<'a>(exported: &'a Exported, run: &str) -> &'a LogRecord {
    let wide: Vec<_> = exported
        .records_of(EVENT_LABLET_RUN_NAME)
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
        let roots = spans_of(exported, INVOKE_AGENT, run);
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
            chats: spans_of(exported, CHAT, run),
            tools: spans_of(exported, EXECUTE_TOOL, run),
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
    }
}

/// Holds the wide event of the run `run` to the spans `exported` holds of
/// that run: every count, size and token count is the sum of what the spans
/// of the run's steps say, every latency is within 5 ms of how long those
/// spans lasted, and the per-tool keys are those of the tools that were
/// called, among the tools the run offered.
///
/// # Panics
///
/// Panics when the run hasn't one wide event and one root span among
/// `exported`, and when a number of the wide event isn't what the spans sum
/// to.
pub fn assert_the_wide_event_sums_its_steps(exported: &Exported, run: &str) {
    let told = Told::of(exported, run);
    told.assert_sums_the_provider_calls();
    told.assert_sums_the_tool_calls();
    told.assert_agrees_with_the_root_span();
}

/// Whether the call a tool span is of named a tool the run offered.
fn names_an_offered_tool(tool: &Attributes) -> bool {
    text(tool, key::LABLET_TOOL_STATUS) != "unknown"
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
        for template in EventLabletRunTemplate::ALL {
            let key = format!("{}.{name}", template.prefix());
            match template {
                EventLabletRunTemplate::LabletToolCalls => {
                    assert_eq!(count(wide, &key), calls.len() as u64, "{key}");
                }
                EventLabletRunTemplate::LabletToolErrors => {
                    let errors = how_many(calls, |tool| is(tool, key::LABLET_TOOL_IS_ERROR));
                    assert_eq!(count(wide, &key), errors, "{key}");
                }
                EventLabletRunTemplate::LabletToolLatencyMs => {
                    assert_near(wide, &key, lasted(calls));
                }
            }
            expected.insert(key);
        }
    }
    let per_tool: BTreeSet<String> = wide
        .keys()
        .filter(|key| template_of(key).is_some())
        .cloned()
        .collect();
    assert_eq!(per_tool, expected, "the per-tool keys of the wide event");
}

/// The template `key` is a key of, and the suffix it has.
fn template_of(key: &str) -> Option<(EventLabletRunTemplate, &str)> {
    EventLabletRunTemplate::ALL.iter().find_map(|template| {
        key.strip_prefix(template.prefix())
            .and_then(|rest| rest.strip_prefix('.'))
            .map(|suffix| (*template, suffix))
    })
}

/// Holds a wide event to the registry: every key the registry requires of
/// `lablet.run` is there, and every key that's there is one the registry
/// declares for it. A template's key is declared for a tool the event
/// lists among the tools the run offered, and for nothing else, which is
/// what bounds the keys.
///
/// # Panics
///
/// Panics when `wide` lacks a key or holds one it shouldn't.
pub fn assert_the_wide_event_is_declared(wide: &LogRecord) {
    let attributes = &wide.attributes;
    assert_eq!(wide.event_name, EVENT_LABLET_RUN_NAME);
    let missing: Vec<_> = EVENT_LABLET_RUN_REQUIRED
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
        EventLabletRunKey::ALL
            .iter()
            .any(|plain| plain.name() == key)
            || template_of(key).is_some_and(|(_, tool)| offered.contains(&tool))
    };
    let undeclared: Vec<_> = attributes.keys().filter(|key| !declared(key)).collect();
    assert!(
        undeclared.is_empty(),
        "the wide event holds {undeclared:?}, which the registry doesn't declare for it, or \
         declares for a tool the run offered and these aren't of one"
    );
}

#[cfg(test)]
mod tests;
