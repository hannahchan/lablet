//! The checks are what a case fails by, so each is held to refusing what
//! it's there to refuse, and each case to passing an observer that keeps
//! the contract and to refusing one that breaks it in the one way the case
//! is there to see.

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::json;

use super::*;
use crate::otlp::{Scope, SpanKind, Status};

mod sketch;

use sketch::{Fault, Sketch};

const TRACE: &str = "0af7651916cd43dd8448eb211c80319c";
const ROOT: &str = "b7ad6b7169203331";

fn attributes(of: Value) -> Attributes {
    let Value::Object(of) = of else {
        panic!("attributes are an object");
    };
    of.into_iter().collect()
}

fn scope() -> Scope {
    Scope {
        name: "lablet".to_owned(),
        version: "0.4.2".to_owned(),
        schema_url: lablet_telemetry_registry::SCHEMA_URL.to_owned(),
    }
}

/// A span of the run `RUN` that lasted `lasted_ms`, with `more` beside the
/// run's id.
fn span(name: &str, lasted_ms: u64, more: Value) -> Span {
    let mut attributes = attributes(more);
    attributes.insert(key::GEN_AI_CONVERSATION_ID.to_owned(), json!(RUN));
    let start_unix_nano = 1_790_000_000_000_000_000;
    Span {
        line: 1,
        resource: Attributes::new(),
        scope: scope(),
        trace_id: TRACE.to_owned(),
        span_id: if name.starts_with(INVOKE_AGENT) {
            ROOT.to_owned()
        } else {
            "00f067aa0ba902b7".to_owned()
        },
        parent_span_id: None,
        flags: 1,
        name: name.to_owned(),
        kind: SpanKind::Internal,
        start_unix_nano,
        end_unix_nano: start_unix_nano + lasted_ms * 1_000_000,
        attributes,
        events: Vec::new(),
        status: Status::Unset,
    }
}

fn chat(turn: u64, attempt: u64, lasted_ms: u64, more: Value) -> Span {
    let mut chat = span("chat scripted-1", lasted_ms, more);
    chat.attributes
        .insert(key::LABLET_TURN.to_owned(), json!(turn));
    chat.attributes
        .insert(key::LABLET_ATTEMPT.to_owned(), json!(attempt));
    chat
}

fn tool(name: &str, status: &str, lasted_ms: u64, sizes: (u64, u64), truncated: bool) -> Span {
    span(
        &format!("execute_tool {name}"),
        lasted_ms,
        json!({
            key::GEN_AI_TOOL_NAME: name,
            key::LABLET_TOOL_STATUS: status,
            key::LABLET_TOOL_IS_ERROR: status != "ok",
            key::LABLET_TOOL_INPUT_BYTES: sizes.0,
            key::LABLET_TOOL_OUTPUT_BYTES: sizes.1,
            key::LABLET_TOOL_OUTPUT_TRUNCATED: truncated,
        }),
    )
}

/// What the spans of [`exported`] sum to, as the wide event says it.
fn sums() -> Vec<(&'static str, Value)> {
    vec![
        (key::GEN_AI_CONVERSATION_ID, json!(RUN)),
        (
            key::LABLET_TOOLS_NAMES,
            json!(["bash", "read_file", "grep"]),
        ),
        (
            key::GEN_AI_RESPONSE_FINISH_REASONS,
            json!(["tool_use", "end_turn"]),
        ),
        (key::LABLET_TOOLS_COUNT, json!(3)),
        (key::LABLET_RUN_TURNS, json!(2)),
        (key::LABLET_RUN_DURATION_MS, json!(1_800)),
        (key::LABLET_PROVIDER_RETRIES, json!(1)),
        (key::LABLET_PROVIDER_LATENCY_MS_TOTAL, json!(390)),
        (key::LABLET_PROVIDER_LATENCY_MS_MAX, json!(250)),
        (key::GEN_AI_USAGE_INPUT_TOKENS, json!(2_100)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, json!(70)),
        (key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS, json!(5)),
        (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, json!(200)),
        (key::LABLET_PROVIDER_FAILED_INPUT_TOKENS, json!(800)),
        (key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS, json!(3)),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
            json!(50),
        ),
        (key::LABLET_TOOL_CALLS_TOTAL, json!(4)),
        (key::LABLET_TOOL_CALLS_ERRORS, json!(2)),
        (key::LABLET_TOOL_CALLS_UNKNOWN, json!(1)),
        (key::LABLET_TOOL_CALLS_TRUNCATED, json!(3)),
        (key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL, json!(1_420)),
        (key::LABLET_TOOL_CALLS_INPUT_BYTES_TOTAL, json!(106)),
        (key::LABLET_TOOL_CALLS_OUTPUT_BYTES_TOTAL, json!(361)),
        ("lablet.tool.calls.bash", json!(2)),
        ("lablet.tool.errors.bash", json!(0)),
        ("lablet.tool.latency_ms.bash", json!(1_100)),
        ("lablet.tool.calls.grep", json!(1)),
        ("lablet.tool.errors.grep", json!(1)),
        ("lablet.tool.latency_ms.grep", json!(320)),
    ]
}

/// One run: a failed attempt and two responses, and four calls, of which
/// one named a tool the run doesn't have.
fn exported() -> Exported {
    let root = span(
        "invoke_agent lablet",
        1_800,
        json!({
            key::LABLET_RUN_TURNS: 2,
            key::LABLET_TOOL_CALLS_TOTAL: 4,
            key::GEN_AI_USAGE_INPUT_TOKENS: 2_100,
            key::GEN_AI_USAGE_OUTPUT_TOKENS: 70,
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS: 5,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS: 200,
        }),
    );
    let failed = chat(
        1,
        1,
        40,
        json!({
            key::ERROR_TYPE: "retryable",
            key::GEN_AI_USAGE_INPUT_TOKENS: 800,
            key::GEN_AI_USAGE_OUTPUT_TOKENS: 3,
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS: 50,
        }),
    );
    let first = chat(
        1,
        2,
        250,
        json!({
            key::GEN_AI_USAGE_INPUT_TOKENS: 1_000,
            key::GEN_AI_USAGE_OUTPUT_TOKENS: 50,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS: 200,
            key::GEN_AI_RESPONSE_FINISH_REASONS: ["tool_use"],
        }),
    );
    let second = chat(
        2,
        1,
        100,
        json!({
            key::GEN_AI_USAGE_INPUT_TOKENS: 1_100,
            key::GEN_AI_USAGE_OUTPUT_TOKENS: 20,
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS: 5,
            key::GEN_AI_RESPONSE_FINISH_REASONS: ["end_turn"],
        }),
    );
    let wide = LogRecord {
        line: 2,
        resource: Attributes::new(),
        scope: scope(),
        event_name: EVENT_LABLET_RUN_NAME.to_owned(),
        severity_number: 9,
        severity_text: "INFO".to_owned(),
        time_unix_nano: root.end_unix_nano,
        observed_time_unix_nano: root.end_unix_nano,
        trace_id: TRACE.to_owned(),
        span_id: ROOT.to_owned(),
        flags: 1,
        attributes: sums()
            .into_iter()
            .map(|(key, held)| (key.to_owned(), held))
            .collect(),
        body: None,
    };
    Exported {
        lines: 2,
        spans: vec![
            failed,
            first,
            tool("bash", "ok", 1_000, (32, 25), true),
            tool("grep", "tool_error", 320, (40, 88), true),
            tool("bash", "ok", 100, (32, 200), true),
            tool("no_such_tool", "unknown", 0, (2, 48), false),
            second,
            root,
        ],
        records: vec![wide],
    }
}

/// [`exported`], with `key` of its wide event holding `held`, or gone.
fn with(key: &str, held: Option<Value>) -> Exported {
    let mut exported = exported();
    let wide = &mut exported.records[0].attributes;
    match held {
        Some(held) => wide.insert(key.to_owned(), held),
        None => wide.remove(key),
    };
    exported
}

fn is_refused(exported: &Exported) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        assert_the_wide_event_sums_its_steps(exported, RUN);
    }))
    .is_err()
}

#[test]
fn a_wide_event_that_holds_the_sums_of_its_run_s_spans_passes() {
    assert_the_wide_event_sums_its_steps(&exported(), RUN);
}

#[test]
fn a_wide_event_that_is_off_in_any_one_number_is_refused() {
    let sums = sums();
    let numbers: Vec<_> = sums
        .iter()
        .filter_map(|(key, held)| held.as_u64().map(|number| (*key, number)))
        .collect();
    assert_eq!(numbers.len(), sums.len() - 3);

    for (key, number) in numbers {
        let off = with(key, Some(json!(number + LATENCY_SLACK_MS + 1)));

        assert!(is_refused(&off), "{key}");
    }
}

#[test]
fn a_latency_within_the_slack_of_what_the_spans_lasted_passes() {
    for key in [
        key::LABLET_PROVIDER_LATENCY_MS_TOTAL,
        key::LABLET_PROVIDER_LATENCY_MS_MAX,
        key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL,
        key::LABLET_RUN_DURATION_MS,
        "lablet.tool.latency_ms.bash",
    ] {
        let held = count(&exported().records[0].attributes, key);
        let near = with(key, Some(json!(held - LATENCY_SLACK_MS)));

        assert!(!is_refused(&near), "{key}");
    }
}

#[test]
fn a_wide_event_that_lacks_a_sum_or_holds_one_of_nothing_is_refused() {
    for key in [
        key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
        key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
        key::LABLET_TOOL_CALLS_UNKNOWN,
        key::GEN_AI_RESPONSE_FINISH_REASONS,
        "lablet.tool.errors.grep",
    ] {
        assert!(is_refused(&with(key, None)), "{key}");
    }
    for key in [
        key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
        key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
        "lablet.tool.calls.no_such_tool",
        "lablet.tool.calls.read_file",
    ] {
        assert!(is_refused(&with(key, Some(json!(0)))), "{key}");
    }
}

#[test]
fn finish_reasons_in_another_order_are_refused() {
    let reordered = with(
        key::GEN_AI_RESPONSE_FINISH_REASONS,
        Some(json!(["end_turn", "tool_use"])),
    );

    assert!(is_refused(&reordered));
}

#[test]
fn a_share_of_a_tool_the_run_did_not_offer_is_refused() {
    let unlisted = with(key::LABLET_TOOLS_NAMES, Some(json!(["bash", "read_file"])));
    let unlisted = {
        let mut exported = unlisted;
        exported.records[0]
            .attributes
            .insert(key::LABLET_TOOLS_COUNT.to_owned(), json!(2));
        exported
    };

    assert!(is_refused(&unlisted));
}

#[test]
fn a_wide_event_outside_the_context_of_its_root_span_is_refused() {
    let mut elsewhere = exported();
    elsewhere.records[0].span_id = "00f067aa0ba902b7".to_owned();

    assert!(is_refused(&elsewhere));
}

#[test]
fn a_run_with_two_wide_events_or_none_is_refused() {
    let mut twice = exported();
    twice.records.push(twice.records[0].clone());
    let mut never = exported();
    never.records.clear();

    assert!(is_refused(&twice));
    assert!(is_refused(&never));
    assert!(
        catch_unwind(|| assert_the_wide_event_sums_its_steps(&exported(), OTHER_RUN)).is_err(),
        "the run has no wide event among the exports of another"
    );
}

#[test]
fn a_run_that_ended_on_a_call_that_failed_has_a_chat_span_more() {
    let mut ended = exported();
    // The failed attempt becomes the run's last, of a call of its own.
    let mut failed = ended.spans.remove(0);
    failed
        .attributes
        .insert(key::LABLET_TURN.to_owned(), json!(3));
    ended.spans.insert(6, failed);
    ended.records[0]
        .attributes
        .insert(key::LABLET_PROVIDER_RETRIES.to_owned(), json!(0));

    assert_the_wide_event_sums_its_steps(&ended, RUN);
}

/// A run that reports the tokens [`exported`] tells of: an attempt that
/// failed and two responses.
const TOKENS: &str = r"
- error:
    kind: retryable
    message: 529 overloaded
    usage: { input_tokens: 800, output_tokens: 3, cache_write_tokens: 50 }
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: cargo test } } }
    usage: { input_tokens: 1000, output_tokens: 50, cache_read_tokens: 200 }
    finish: tool_use
- response:
    content:
      - text: The parser test passes now.
    usage: { input_tokens: 1100, output_tokens: 20, reasoning_output_tokens: 5 }
    finish: end_turn
";

/// What the loop returns of a run of `script` that nothing observes.
async fn returned(script: &str) -> RunSummary {
    let mut service = playing(script, Arc::new(Unobserved)).await;
    run(&mut service, RUN).await.summary
}

fn miscounts(exported: &Exported, summary: &RunSummary) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        assert_the_wide_event_counts_the_tokens_the_run_returned(exported, RUN, summary);
    }))
    .is_err()
}

#[tokio::test(start_paused = true)]
async fn a_wide_event_that_counts_the_tokens_the_run_returned_passes() {
    let summary = returned(TOKENS).await;

    assert_the_wide_event_counts_the_tokens_the_run_returned(&exported(), RUN, &summary);
    assert_eq!(
        (
            summary.outcome.usage.input_tokens,
            summary.failed_usage.unwrap().input_tokens
        ),
        (2_100, 800),
        "the run reported tokens, so the check held the wide event to some"
    );
}

/// The observer this is there for: it misreads a count where it reads the
/// loop's events, so its spans and its wide event say the same wrong
/// number, and the sums of its spans are the numbers of its wide event.
#[tokio::test(start_paused = true)]
async fn an_observer_that_is_off_in_its_spans_and_its_wide_event_alike_is_refused() {
    /// Where [`exported`] holds the span of the attempt that failed, of the
    /// first response, of the second, and of the run.
    const FAILED: usize = 0;
    const FIRST: usize = 1;
    const SECOND: usize = 6;
    const ROOT_SPAN: usize = 7;
    let summary = returned(TOKENS).await;
    let chat_of = |exported: &mut Exported, chat: usize, key: &str, held: u64| {
        exported.spans[chat]
            .attributes
            .insert(key.to_owned(), json!(held));
    };

    for (key, in_the_chat, chat, off_by_one) in [
        (
            key::GEN_AI_USAGE_INPUT_TOKENS,
            key::GEN_AI_USAGE_INPUT_TOKENS,
            FIRST,
            (1_001, 2_101),
        ),
        (
            key::GEN_AI_USAGE_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_OUTPUT_TOKENS,
            FIRST,
            (51, 71),
        ),
        (
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            FIRST,
            (201, 201),
        ),
        (
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            FIRST,
            (1, 1),
        ),
        (
            key::LABLET_PROVIDER_FAILED_INPUT_TOKENS,
            key::GEN_AI_USAGE_INPUT_TOKENS,
            FAILED,
            (801, 801),
        ),
        (
            key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_OUTPUT_TOKENS,
            FAILED,
            (4, 4),
        ),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            FAILED,
            (1, 1),
        ),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            FAILED,
            (51, 51),
        ),
    ] {
        let (of_the_chat, of_the_run) = off_by_one;
        let mut off = with(key, Some(json!(of_the_run)));
        chat_of(&mut off, chat, in_the_chat, of_the_chat);
        if chat == FIRST {
            chat_of(&mut off, ROOT_SPAN, key, of_the_run);
        }

        assert!(!is_refused(&off), "{key}: the spans sum to the wide event");
        assert!(miscounts(&off, &summary), "{key}");
    }

    let mut reasoning = with(key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS, None);
    for span in [SECOND, ROOT_SPAN] {
        reasoning.spans[span]
            .attributes
            .remove(key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS);
    }
    assert!(!is_refused(&reasoning));
    assert!(
        miscounts(&reasoning, &summary),
        "a count the run returned and the wide event lacks"
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_none_of_whose_attempts_failed_has_none_of_their_tokens_counted() {
    let summary = returned(
        r"
- response:
    content:
      - text: Nothing to fix.
    usage: { input_tokens: 2100, output_tokens: 70, reasoning_output_tokens: 5, cache_read_tokens: 200 }
    finish: end_turn
",
    )
    .await;
    let mut none_failed = exported();
    for failed in [
        key::LABLET_PROVIDER_FAILED_INPUT_TOKENS,
        key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS,
        key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
    ] {
        none_failed.records[0].attributes.remove(failed);
    }

    assert_eq!(summary.failed_usage, None);
    assert!(miscounts(&exported(), &summary));
    assert_the_wide_event_counts_the_tokens_the_run_returned(&none_failed, RUN, &summary);
}

/// A wide event that holds every key the registry requires, and the
/// per-tool keys of [`sums`].
fn declared() -> LogRecord {
    let mut wide = exported().records.remove(0);
    for required in EVENT_LABLET_RUN_REQUIRED {
        wide.attributes
            .entry((*required).to_owned())
            .or_insert(json!(0));
    }
    wide
}

fn is_undeclared(wide: &LogRecord) -> bool {
    catch_unwind(AssertUnwindSafe(|| assert_the_wide_event_is_declared(wide))).is_err()
}

#[test]
fn a_wide_event_of_the_keys_the_registry_declares_passes() {
    assert_the_wide_event_is_declared(&declared());
}

#[test]
fn a_wide_event_that_lacks_a_key_the_registry_requires_is_refused() {
    for required in EVENT_LABLET_RUN_REQUIRED {
        let mut wide = declared();
        wide.attributes.remove(*required);

        assert!(is_undeclared(&wide), "{required}");
    }
}

#[test]
fn a_wide_event_with_a_key_the_registry_does_not_declare_for_it_is_refused() {
    for undeclared in [
        key::LABLET_TURN,
        "team",
        "lablet.tool.calls",
        "lablet.tool.calls.",
        "lablet.tool.calls.write_file",
        "lablet.tool.bytes.bash",
    ] {
        let mut wide = declared();
        wide.attributes.insert(undeclared.to_owned(), json!(1));

        assert!(is_undeclared(&wide), "{undeclared}");
    }
}

#[test]
fn a_record_of_another_event_is_no_wide_event() {
    let mut wide = declared();
    wide.event_name = "lablet.retry".to_owned();

    assert!(is_undeclared(&wide));
}

// The cases, against an observer with one fault

/// What the case said as it refused the observer; `None` when it passed.
async fn refusal<Case>(case: Case) -> Option<String>
where
    Case: Future<Output = ()> + Send + 'static,
{
    let panic = tokio::spawn(case).await.err()?.into_panic();
    let said = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|said| (*said).to_owned()));
    Some(said.expect("a case refuses in words"))
}

async fn of_the_wide_events(fault: Fault) -> Option<String> {
    let sketch = Sketch::writing(fault);
    refusal(async move { a_run_has_exactly_one_wide_event(&sketch).await }).await
}

async fn of_the_sums(fault: Fault) -> Option<String> {
    let sketch = Sketch::writing(fault);
    refusal(async move { the_numbers_of_the_wide_event_are_the_sums_of_the_steps(&sketch).await })
        .await
}

async fn of_a_destination_that_cannot_be_written(fault: Fault) -> Option<String> {
    let sketch = Sketch::unwritable(fault);
    refusal(async move {
        a_destination_that_cannot_be_written_changes_nothing_about_the_run(&sketch).await;
    })
    .await
}

/// Holds `refusal` to being one, in words that hold `words`.
fn assert_refused(refusal: Option<String>, words: &str) {
    let said = refusal.unwrap_or_else(|| panic!("the case passed, and was to say: {words}"));
    assert!(said.contains(words), "the case said: {said}");
}

#[tokio::test(start_paused = true)]
async fn an_observer_that_keeps_the_contract_passes_every_case() {
    assert_eq!(of_the_wide_events(Fault::None).await, None);
    assert_eq!(of_the_sums(Fault::None).await, None);
    assert_eq!(
        of_a_destination_that_cannot_be_written(Fault::None).await,
        None
    );
}

#[tokio::test(start_paused = true)]
async fn an_observer_with_no_wide_event_for_a_run_that_failed_is_refused() {
    assert_refused(
        of_the_wide_events(Fault::ForgetsARunThatFailed).await,
        "one wide event for each run",
    );
}

#[tokio::test(start_paused = true)]
async fn an_observer_whose_flush_adds_a_wide_event_when_no_run_came_before_is_refused() {
    assert_refused(
        of_the_wide_events(Fault::ExportsTheLastWideEventAtEveryFlush).await,
        "one wide event for each run",
    );
}

#[tokio::test(start_paused = true)]
async fn a_wide_event_outside_the_context_of_its_root_span_is_refused_by_the_case() {
    assert_refused(
        of_the_wide_events(Fault::PutsTheWideEventUnderAChatSpan).await,
        "the wide event is in the context of the root span",
    );
}

#[tokio::test(start_paused = true)]
async fn a_wide_event_that_is_not_the_sum_of_its_spans_is_refused_by_the_case() {
    assert_refused(
        of_the_sums(Fault::LeavesTheUnknownCallOutOfTheTotal).await,
        &format!(
            "{}\n  left: Some(6)\n right: Some(7)",
            key::LABLET_TOOL_CALLS_TOTAL
        ),
    );
}

#[tokio::test(start_paused = true)]
async fn spans_and_a_wide_event_that_miscount_the_tokens_alike_are_refused() {
    assert_refused(
        of_the_sums(Fault::CountsATokenTooMany).await,
        "`gen_ai.usage.input_tokens` of the wide event, and the count the run returned",
    );
}

#[tokio::test(start_paused = true)]
async fn spans_and_a_wide_event_that_miscount_a_step_alike_are_refused() {
    assert_refused(
        of_the_sums(Fault::SeesNothingCut).await,
        &format!("{}\n  left: 0\n right: 2", key::LABLET_TOOL_CALLS_TRUNCATED),
    );
}

#[tokio::test(start_paused = true)]
async fn an_observer_that_slows_the_run_is_refused() {
    assert_refused(
        of_a_destination_that_cannot_be_written(Fault::SlowsTheRun).await,
        "the run ended as it does when nothing observes it",
    );
}

#[tokio::test(start_paused = true)]
async fn an_observer_that_says_it_wrote_to_a_destination_that_cannot_be_written_is_refused() {
    assert_refused(
        of_a_destination_that_cannot_be_written(Fault::SaysItWrote).await,
        "the destination was written, so the case held nothing to the observer",
    );
}
