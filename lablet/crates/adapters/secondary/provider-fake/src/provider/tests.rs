use std::future::Future as _;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use lablet_model::{ContentBlock, FinishReason, ProviderKind, Thinking, TokenCounts, Usage};
use lablet_run::ERROR_MESSAGE_MAX_BYTES;
use tokio::time::Instant;

use super::*;
use crate::script::{ScriptFormat, ScriptSource};

const NAME: &str = "scripts/smoke.yaml";

/// A provider that plays the YAML script `text`.
fn playing(text: &str) -> FakeProvider {
    let script = Script::read(ScriptSource {
        name: NAME,
        text,
        format: ScriptFormat::Yaml,
    })
    .unwrap();
    FakeProvider::new("scripted-1", script)
}

/// An attempt that may take `deadline`.
fn within(deadline: Duration) -> ProviderRequest<'static> {
    ProviderRequest {
        system: "You fix tests.",
        messages: &[],
        tools: &[],
        max_tokens: 4_096,
        temperature: None,
        thinking: Thinking::ProviderDefault,
        effort: None,
        seed: None,
        cache_key: None,
        deadline,
    }
}

/// An attempt with more time than any script here takes.
fn unhurried() -> ProviderRequest<'static> {
    within(Duration::from_secs(600))
}

const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// The text of a response that says one thing.
fn said(response: &ProviderResponse) -> &str {
    match response.content() {
        [ContentBlock::Text(text)] => text,
        other => panic!("the response holds one text block, not {other:?}"),
    }
}

const THREE: &str = r"
- response: { content: [{ text: first }], finish: end_turn }
- error: { kind: retryable, message: second }
- response: { content: [{ text: third }], finish: end_turn }
";

#[test]
fn it_reports_the_script_as_its_api_and_the_name_it_was_given() {
    let provider = playing(THREE);

    assert_eq!(
        provider.model(),
        &ModelRef {
            api: ProviderApi::Script,
            name: "scripted-1".to_owned(),
            replays_reasoning: false,
        }
    );
    assert_eq!(provider.model().api.provider(), ProviderKind::Fake);
}

#[test]
fn it_is_reached_over_no_network() {
    assert_eq!(playing(THREE).endpoint(), None);
}

#[test]
fn it_is_a_provider_the_loop_can_hold() {
    let provider: Arc<dyn ModelProvider> = Arc::new(playing(THREE));

    assert_eq!(provider.model().name, "scripted-1");
}

#[tokio::test(start_paused = true)]
async fn each_attempt_is_answered_by_the_next_entry_whatever_came_of_the_one_before() {
    let provider = playing(THREE);

    let first = provider.complete(unhurried()).await.unwrap();
    let second = provider.complete(unhurried()).await.unwrap_err();
    let third = provider.complete(unhurried()).await.unwrap();

    assert_eq!(said(&first), "first");
    assert_eq!(
        second,
        ProviderError::new(ProviderErrorKind::Retryable, "second")
    );
    assert_eq!(said(&third), "third");
}

#[tokio::test(start_paused = true)]
async fn a_response_arrives_as_the_script_states_it() {
    let provider = playing(
        r"
- response:
    content:
      - text: Listing.
      - tool_use: { id: call_1, name: bash, input: { json: { command: ls } } }
    usage: { input_tokens: 1200, output_tokens: 80, reasoning_output_tokens: 30, cache_read_tokens: 1000, cache_write_tokens: 0 }
    finish: tool_calls
    response_id: msg_01
    response_model: scripted-2026-09
",
    );

    let response = provider.complete(unhurried()).await.unwrap();

    assert_eq!(response.content().len(), 2);
    assert_eq!(
        response.usage,
        Usage::from_inclusive(TokenCounts {
            input: 1_200,
            output: 80,
            reasoning: Some(30),
            cache_read: Some(1_000),
            cache_write: Some(0),
        })
    );
    assert_eq!(response.finish, FinishReason::ToolUse);
    assert_eq!(response.response_id.as_deref(), Some("msg_01"));
    assert_eq!(response.response_model.as_deref(), Some("scripted-2026-09"));
}

/// O14, the provider's half: the counts a script leaves out reach the loop
/// as counts nobody reported.
#[tokio::test(start_paused = true)]
async fn a_usage_without_cache_or_reasoning_counts_reports_none_of_the_three() {
    let provider = playing(
        "- response: { content: [], finish: end_turn, usage: { input_tokens: 12, output_tokens: 3 } }",
    );

    let response = provider.complete(unhurried()).await.unwrap();

    assert_eq!(
        response.usage,
        Usage {
            input_tokens: 12,
            output_tokens: 3,
            reasoning_output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        }
    );
}

#[tokio::test(start_paused = true)]
async fn an_error_of_each_kind_arrives_with_the_usage_and_the_hint_the_script_gave_it() {
    for kind in [
        ProviderErrorKind::Retryable,
        ProviderErrorKind::ContextExhausted,
        ProviderErrorKind::Auth,
        ProviderErrorKind::Fatal,
        ProviderErrorKind::Malformed,
    ] {
        let provider = playing(&format!(
            "- error: {{ kind: {kind}, message: it failed, usage: {{ input_tokens: 900 }}, retry_after: 7s }}"
        ));

        let failure = provider.complete(unhurried()).await.unwrap_err();

        assert_eq!(
            failure,
            ProviderError::new(kind, "it failed")
                .with_usage(Usage {
                    input_tokens: 900,
                    ..Usage::default()
                })
                .with_retry_after(Duration::from_secs(7)),
            "{kind}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn an_error_that_states_neither_reports_no_usage_and_asks_for_no_wait() {
    let provider = playing("- error: { kind: malformed, message: garbled }");

    let failure = provider.complete(unhurried()).await.unwrap_err();

    assert_eq!(failure.usage, None);
    assert_eq!(failure.retry_after, None);
}

#[tokio::test(start_paused = true)]
async fn an_injected_message_reaches_the_loop_within_the_ports_bound() {
    let provider = playing(&format!(
        "- error: {{ kind: fatal, message: {} }}",
        "é".repeat(ERROR_MESSAGE_MAX_BYTES)
    ));

    let failure = provider.complete(unhurried()).await.unwrap_err();

    assert_eq!(failure.message().len(), ERROR_MESSAGE_MAX_BYTES);
    assert_eq!(failure.message(), "é".repeat(ERROR_MESSAGE_MAX_BYTES / 2));
}

#[tokio::test(start_paused = true)]
async fn an_attempt_takes_the_latency_its_entry_states_whether_it_answers_or_fails() {
    let provider = playing(
        r"
- response: { content: [{ text: slow }], finish: end_turn, latency: 250ms }
- error: { kind: retryable, latency: 40ms }
- response: { content: [{ text: at once }], finish: end_turn }
",
    );
    let began = Instant::now();

    let slow = provider.complete(unhurried()).await;
    let after_response = began.elapsed();
    let failed = provider.complete(unhurried()).await;
    let after_failure = began.elapsed();
    let at_once = provider.complete(unhurried()).await;
    let after_all = began.elapsed();

    assert_eq!(said(&slow.unwrap()), "slow");
    assert_eq!(after_response, ms(250));
    assert_eq!(failed.unwrap_err().kind, ProviderErrorKind::Retryable);
    assert_eq!(after_failure, ms(290));
    assert_eq!(said(&at_once.unwrap()), "at once");
    assert_eq!(after_all, ms(290));
}

#[tokio::test(start_paused = true)]
async fn an_attempt_whose_latency_outlives_its_deadline_fails_when_the_deadline_is_reached() {
    let provider = playing(
        r"
- response:
    content: [{ text: too late }]
    usage: { input_tokens: 1200, output_tokens: 80 }
    finish: end_turn
    latency: 10s
",
    );
    let began = Instant::now();

    let failure = provider.complete(within(ms(1_500))).await.unwrap_err();

    assert_eq!(began.elapsed(), ms(1_500));
    assert_eq!(
        failure,
        ProviderError::new(
            ProviderErrorKind::Retryable,
            r#"entry 1 of script "scripts/smoke.yaml" answers after 10s, and the attempt's deadline was 1s 500ms"#
        ),
        "what the entry held is never answered, so neither is its usage"
    );
}

#[tokio::test(start_paused = true)]
async fn an_injected_error_that_outlives_the_deadline_is_the_deadlines_failure_and_not_its_own() {
    let provider = playing(
        "- error: { kind: auth, message: rejected, usage: { input_tokens: 9 }, retry_after: 30s, latency: 2s }",
    );
    let began = Instant::now();

    let failure = provider.complete(within(ms(500))).await.unwrap_err();

    assert_eq!(began.elapsed(), ms(500));
    assert_eq!(failure.kind, ProviderErrorKind::Retryable);
    assert_eq!(failure.usage, None);
    assert_eq!(failure.retry_after, None);
}

#[tokio::test(start_paused = true)]
async fn a_deadline_is_met_when_the_latency_reaches_it_and_not_a_moment_before() {
    let script = r"
- response: { content: [{ text: reached }], finish: end_turn, latency: 100ms }
- response: { content: [{ text: in time }], finish: end_turn, latency: 100ms }
";
    let provider = playing(script);
    let began = Instant::now();

    let reached = provider.complete(within(ms(100))).await;
    let after_reached = began.elapsed();
    let in_time = provider.complete(within(ms(101))).await;

    assert_eq!(reached.unwrap_err().kind, ProviderErrorKind::Retryable);
    assert_eq!(after_reached, ms(100));
    assert_eq!(said(&in_time.unwrap()), "in time");
    assert_eq!(began.elapsed(), ms(200));
}

#[tokio::test(start_paused = true)]
async fn a_deadline_of_zero_is_reached_at_once_by_an_entry_that_would_answer_at_once() {
    let provider = playing("- response: { content: [{ text: at once }], finish: end_turn }");
    let began = Instant::now();

    let failure = provider.complete(within(Duration::ZERO)).await.unwrap_err();

    assert_eq!(began.elapsed(), Duration::ZERO);
    assert_eq!(
        failure,
        ProviderError::new(
            ProviderErrorKind::Retryable,
            r#"entry 1 of script "scripts/smoke.yaml" answers after 0s, and the attempt's deadline was 0s"#
        )
    );
}

#[tokio::test(start_paused = true)]
async fn an_attempt_the_deadline_cut_short_has_played_its_entry() {
    let provider = playing(
        r"
- response: { content: [{ text: too late }], finish: end_turn, latency: 10s }
- response: { content: [{ text: the retry }], finish: end_turn }
",
    );

    let cut_short = provider.complete(within(ms(10))).await;
    let retry = provider.complete(within(ms(10))).await;

    assert_eq!(cut_short.unwrap_err().kind, ProviderErrorKind::Retryable);
    assert_eq!(said(&retry.unwrap()), "the retry");
}

#[tokio::test(start_paused = true)]
async fn an_attempt_after_the_last_entry_fails_fatally_and_says_what_to_do() {
    let provider = playing(THREE);
    for _ in 0..3 {
        let _played = provider.complete(unhurried()).await;
    }
    let began = Instant::now();

    let failure = provider.complete(unhurried()).await.unwrap_err();

    assert_eq!(began.elapsed(), Duration::ZERO);
    assert_eq!(
        failure,
        ProviderError::new(
            ProviderErrorKind::Fatal,
            "script \"scripts/smoke.yaml\" ran out: entry 3 was its last, and the run made \
             another attempt of a provider call. Add an entry to answer it, or have the last \
             response end the run."
        )
    );
    assert!(!failure.kind.is_retryable());
}

#[tokio::test(start_paused = true)]
async fn a_script_that_ran_out_fails_every_later_attempt_the_same_way() {
    let provider = playing("- response: { content: [], finish: end_turn }");
    let _played = provider.complete(unhurried()).await;

    let once = provider.complete(unhurried()).await;
    let again = provider.complete(unhurried()).await;

    assert_eq!(once.as_ref().unwrap_err().kind, ProviderErrorKind::Fatal);
    assert_eq!(once, again);
}

#[tokio::test(start_paused = true)]
async fn a_script_that_ran_out_says_so_whatever_the_deadline() {
    let provider = playing("- response: { content: [], finish: end_turn }");
    let _played = provider.complete(unhurried()).await;

    let failure = provider.complete(within(Duration::ZERO)).await.unwrap_err();

    assert_eq!(failure.kind, ProviderErrorKind::Fatal);
}

#[tokio::test(start_paused = true)]
async fn a_rewound_script_is_played_from_its_first_entry() {
    let provider = playing(THREE);
    let _first = provider.complete(unhurried()).await;
    let _second = provider.complete(unhurried()).await;

    provider.rewind();
    let first = provider.complete(unhurried()).await.unwrap();
    let second = provider.complete(unhurried()).await.unwrap_err();

    assert_eq!(said(&first), "first");
    assert_eq!(second.message(), "second");
}

#[tokio::test(start_paused = true)]
async fn a_script_that_ran_out_can_be_rewound() {
    let provider = playing("- response: { content: [{ text: only }], finish: end_turn }");
    let _played = provider.complete(unhurried()).await;
    let ran_out = provider.complete(unhurried()).await;

    provider.rewind();
    let replayed = provider.complete(unhurried()).await.unwrap();

    assert_eq!(ran_out.unwrap_err().kind, ProviderErrorKind::Fatal);
    assert_eq!(said(&replayed), "only");
}

#[tokio::test(start_paused = true)]
async fn attempts_made_together_take_an_entry_each_and_wait_together() {
    let provider = playing(
        r"
- response: { content: [{ text: first }], finish: end_turn, latency: 300ms }
- response: { content: [{ text: second }], finish: end_turn, latency: 100ms }
",
    );
    let began = Instant::now();

    let (first, second) = tokio::join!(
        provider.complete(unhurried()),
        provider.complete(unhurried())
    );

    assert_eq!(said(&first.unwrap()), "first");
    assert_eq!(said(&second.unwrap()), "second");
    assert_eq!(began.elapsed(), ms(300));
}

/// A script with no latency in it asks nothing of a runtime, so a caller
/// that has none, or one without a timer, can play it.
#[test]
fn a_script_that_waits_for_nothing_is_played_without_a_runtime() {
    let provider = playing(THREE);
    let mut context = Context::from_waker(Waker::noop());

    let attempt = pin!(provider.complete(unhurried()));

    let Poll::Ready(Ok(response)) = attempt.poll(&mut context) else {
        panic!("an entry with no latency answers the first time it's asked");
    };
    assert_eq!(said(&response), "first");
}
