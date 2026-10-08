//! One `Lablet`, many runs: each is a run of its own, and all of them hear
//! the script from its first entry.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lablet::{BlankTask, OutcomeDocument, RunId, RunRequest, StopReason};
use lablet_run::telemetry::generated::GenAiClientInferenceOperationDetails;
use serde_json::{Value, json};

use crate::harness::{ENDS, Lab, MODEL, PROMPT, SYSTEM, Traced, request};
use crate::key;

const CALLS_THEN_ENDS: &str = "
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo one test fails } } }
    usage: { input_tokens: 100, output_tokens: 10 }
    finish: tool_use
- response:
    content:
      - text: One test fails.
    usage: { input_tokens: 150, output_tokens: 8 }
    finish: end_turn
";

fn unix_ms_now() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    u64::try_from(since.as_millis()).unwrap()
}

fn unix_nanos_now() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    u64::try_from(since.as_nanos()).unwrap()
}

/// The outcome as its document, without what names the run and what times
/// it: what two runs of one config and one script share.
fn shared(outcome: lablet::FinishedRun) -> Value {
    let mut document =
        serde_json::to_value(OutcomeDocument::from(outcome.summary.outcome)).unwrap();
    let keys = document.as_object_mut().unwrap();
    assert!(keys.remove("run_id").is_some());
    assert!(keys.remove("duration_ms").is_some());
    document
}

#[tokio::test]
async fn two_runs_on_one_lablet_are_two_runs_with_one_outcome() {
    let scratch = Lab::new("twice");
    let config = scratch.config(
        CALLS_THEN_ENDS,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    let digest = config.digest().to_string();
    let mut lablet = scratch.build(config).await.unwrap();
    let named = RunId::new("the-second-run").unwrap();

    let before = unix_ms_now();
    let first = lablet.run(request()).await;
    let second = lablet.run(request().run_id(named.clone()).unwrap()).await;
    let after = unix_ms_now();
    lablet.shutdown().await;

    let fresh = first.summary.outcome.run_id.clone();
    assert_ne!(fresh, named);
    assert_eq!(second.summary.outcome.run_id, named);
    for finished in [&first, &second] {
        let outcome = &finished.summary.outcome;
        assert_eq!(outcome.stop_reason(), StopReason::Completed);
        assert_eq!(outcome.result().text, "One test fails.");
        assert_eq!((outcome.turns, outcome.tool_calls), (2, 1));
        assert_eq!(outcome.usage.total(), 268);
        assert_eq!(finished.transcript.system(), SYSTEM);
    }
    assert_eq!(shared(first), shared(second));

    // One root span each, under the run's own id, and the same spans under
    // it: the second run made the first run's provider calls again.
    let exported = scratch.exported();
    assert_eq!(exported.spans_of(key::INVOKE_AGENT).len(), 2);
    assert_eq!(exported.records_of(key::WIDE_EVENT).len(), 2);
    for run_id in [&fresh, &named] {
        let traced = Traced::of(&exported, run_id.as_str());
        assert_eq!(traced.spans.len(), 4, "a root, two attempts and a call");
        let root = traced.root();
        assert_eq!(root.attributes[key::LABLET_CONFIG_DIGEST], json!(digest));
        assert_eq!(
            root.attributes[key::GEN_AI_AGENT_VERSION],
            json!(lablet::VERSION)
        );
        assert_eq!(root.attributes[key::GEN_AI_REQUEST_MODEL], json!(MODEL));
        let started_ms = root.start_unix_nano / 1_000_000;
        assert!(
            (before..=after).contains(&started_ms),
            "{before} <= {started_ms} <= {after}"
        );
        assert_eq!(
            traced.wide().attributes[key::LABLET_RUN_STOP_REASON],
            json!("completed")
        );
        assert_eq!(traced.wide().trace_id, root.trace_id);
    }
    assert_ne!(
        Traced::of(&exported, fresh.as_str()).root().trace_id,
        Traced::of(&exported, named.as_str()).root().trace_id
    );
}

/// The value of a ULID's first ten digits, which is when it was made, in
/// milliseconds since the Unix epoch.
fn ulid_time(id: &str) -> u64 {
    const DIGITS: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    assert_eq!(id.len(), 26, "{id}");
    id.chars().take(10).fold(0, |time, digit| {
        time * 32 + u64::try_from(DIGITS.find(digit).unwrap()).unwrap()
    })
}

#[tokio::test]
async fn a_run_without_an_id_gets_a_fresh_ulid_that_holds_when_it_started() {
    let scratch = Lab::new("fresh");
    let mut lablet = scratch
        .build(scratch.config(ENDS, json!({})))
        .await
        .unwrap();

    let first = lablet.run(request()).await.summary.outcome.run_id;
    let second = lablet.run(request()).await.summary.outcome.run_id;
    lablet.shutdown().await;

    assert_ne!(first, second);
    let exported = scratch.exported();
    for run_id in [&first, &second] {
        let traced = Traced::of(&exported, run_id.as_str());
        let root = traced.root();
        assert_eq!(
            ulid_time(run_id.as_str()),
            root.start_unix_nano / 1_000_000,
            "the root span starts when the id says the run did"
        );
        for span in &traced.spans {
            for label in [
                key::LABLET_TASK_ID,
                key::LABLET_EXPERIMENT_ID,
                key::LABLET_TRIAL,
            ] {
                assert!(!span.attributes.contains_key(label), "{label}");
            }
        }
        let wide = traced.wide();
        assert!(
            !wide
                .attributes
                .contains_key(key::LABLET_RUN_TRANSCRIPT_PATH)
        );
        assert!(
            traced
                .records
                .iter()
                .all(|record| record.event_name != GenAiClientInferenceOperationDetails::NAME),
            "no content is captured"
        );
    }
}

#[tokio::test]
async fn what_a_lablet_prints_of_itself_names_its_config_and_holds_no_prompt() {
    let scratch = Lab::new("debug");
    let config = scratch.config(
        ENDS,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    let digest = config.digest().to_string();

    let lablet = scratch.build(config).await.unwrap();

    let printed = format!("{lablet:?}");
    assert!(printed.starts_with("Lablet {"), "{printed}");
    assert!(printed.contains(&digest), "{printed}");
    assert!(printed.contains("bash"), "{printed}");
    assert!(!printed.contains(SYSTEM), "{printed}");
    lablet.shutdown().await;
}

#[test]
fn a_blank_prompt_is_refused_when_the_request_is_made() {
    for prompt in ["", " ", "\n\t "] {
        assert_eq!(RunRequest::new(prompt), Err(BlankTask), "{prompt:?}");
    }
    RunRequest::new(PROMPT).unwrap();
}

#[tokio::test]
async fn the_run_is_given_the_prompt_the_request_holds_under_the_system_prompt_of_the_config() {
    let scratch = Lab::new("prompts");
    let config = scratch.config(ENDS, json!({ "telemetry": { "capture_content": true } }));
    let mut lablet = scratch.build(config).await.unwrap();

    let finished = lablet
        .run(RunRequest::new(" Rename the parser. ").unwrap())
        .await;
    lablet.shutdown().await;

    assert_eq!(finished.transcript.system(), SYSTEM);
    assert_eq!(finished.summary.prompt.system_bytes, SYSTEM.len() as u64);
    assert_eq!(finished.summary.prompt.user_bytes, 20);
    // The first provider call's content record holds what the model was
    // sent: the config's system prompt over the request's task.
    let exported = scratch.exported();
    let traced = Traced::of(&exported, finished.summary.outcome.run_id.as_str());
    let chat = traced
        .records
        .iter()
        .find(|record| {
            record.event_name == GenAiClientInferenceOperationDetails::NAME
                && record.attributes.contains_key(key::GEN_AI_INPUT_MESSAGES)
        })
        .expect("the run captures content, so its one call has a record");
    let system = chat.attributes[key::GEN_AI_SYSTEM_INSTRUCTIONS]
        .as_str()
        .unwrap();
    assert!(system.contains(SYSTEM), "{system}");
    let input = chat.attributes[key::GEN_AI_INPUT_MESSAGES]
        .as_str()
        .unwrap();
    assert!(input.contains(" Rename the parser. "), "{input}");
}

/// A response that takes a millisecond, which a test cuts short.
const TAKES_A_MILLISECOND: &str = "
- response:
    content:
      - text: Nothing to fix.
    usage: { input_tokens: 100, output_tokens: 10 }
    finish: end_turn
    latency: 1ms
";

/// Tokio's paused clock moves on by itself only to a whole millisecond of
/// its timer, so the clock is moved by hand, a millisecond and a half, while
/// the attempt waits: the attempt and the run then last exactly that. The
/// run's start is bounded by readings of the wall clock to the nanosecond, so
/// a start cut to the millisecond falls before the first, unless that reading
/// lands on a whole millisecond.
#[tokio::test(start_paused = true)]
async fn the_run_s_own_span_and_its_wide_event_are_timed_to_the_nanosecond_from_its_start() {
    let scratch = Lab::new("nanoseconds");
    let mut lablet = scratch
        .build(scratch.config(TAKES_A_MILLISECOND, json!({})))
        .await
        .unwrap();
    let lasting = Duration::from_micros(1_500);

    let before = unix_nanos_now();
    let (finished, ()) = tokio::join!(lablet.run(request()), tokio::time::advance(lasting));
    let after = unix_nanos_now();
    lablet.shutdown().await;

    assert_eq!(finished.duration, lasting);
    let exported = scratch.exported();
    let traced = Traced::of(&exported, finished.summary.outcome.run_id.as_str());
    let (root, wide) = (traced.root(), traced.wide());
    let [chat] = traced.chats().try_into().unwrap();
    assert!(
        (before..=after).contains(&root.start_unix_nano),
        "{before} <= {} <= {after}",
        root.start_unix_nano
    );
    assert_eq!(
        (root.start_unix_nano, root.end_unix_nano),
        (chat.start_unix_nano, chat.end_unix_nano),
        "the run began and ended with its one attempt, timed by the loop"
    );
    assert_eq!(root.end_unix_nano - root.start_unix_nano, 1_500_000);
    assert_eq!(
        (wide.time_unix_nano, wide.observed_time_unix_nano),
        (root.end_unix_nano, root.end_unix_nano)
    );
    assert_eq!(wide.attributes[key::LABLET_RUN_DURATION_MS], json!(1));
}
