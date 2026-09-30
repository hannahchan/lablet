//! One `Lablet`, many runs: each is a run of its own, and all of them hear
//! the script from its first entry.

use std::time::{SystemTime, UNIX_EPOCH};

use lablet::{BlankTask, EventKind, OutcomeDocument, RunId, RunRequest, StopReason};
use lablet_telemetry_registry::attribute as key;
use serde_json::{Value, json};

use crate::harness::{ENDS, Lab, MODEL, PROMPT, SYSTEM, Traced, observed, request};

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
    let (mut lablet, recorder) = observed(config).await;
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

    // One `RunStarted` each, under the run's own id, and the same events
    // after it: the second run made the first run's provider calls again.
    let events = recorder.events();
    let of_one_run = [
        "RunStarted",
        "TurnStarted",
        "ProviderCallStarted",
        "ProviderCallFinished",
        "ToolCallStarted",
        "ToolCallFinished",
        "TurnStarted",
        "ProviderCallStarted",
        "ProviderCallFinished",
        "RunFinished",
    ];
    assert_eq!(recorder.names(), [of_one_run, of_one_run].concat());
    for (run_id, events) in [(&fresh, &events[..10]), (&named, &events[10..])] {
        for event in events {
            assert_eq!(&event.run_id, run_id);
        }
        let EventKind::RunStarted { context, model, .. } = &events[0].kind else {
            panic!("{:?}", events[0]);
        };
        assert_eq!(&context.run_id, run_id);
        assert_eq!(context.config_digest.as_str(), digest);
        assert_eq!(context.agent_version, lablet::VERSION);
        assert!(
            (before..=after).contains(&context.started_unix_ms),
            "{before} <= {} <= {after}",
            context.started_unix_ms
        );
        assert_eq!(model.name, MODEL);
    }

    let exported = scratch.exported();
    assert_eq!(exported.spans_of("invoke_agent").len(), 2);
    assert_eq!(exported.records_of("lablet.run").len(), 2);
    for run_id in [&fresh, &named] {
        let traced = Traced::of(&exported, run_id.as_str());
        assert_eq!(traced.spans.len(), 4, "a root, two attempts and a call");
        assert_eq!(
            traced.wide().attributes[key::LABLET_RUN_STOP_REASON],
            json!("completed")
        );
        assert_eq!(traced.wide().trace_id, traced.root().trace_id);
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
    let (mut lablet, recorder) = observed(scratch.config(ENDS, json!({}))).await;

    let first = lablet.run(request()).await.summary.outcome.run_id;
    let second = lablet.run(request()).await.summary.outcome.run_id;
    lablet.shutdown().await;

    assert_ne!(first, second);
    let started: Vec<_> = recorder
        .events()
        .into_iter()
        .filter_map(|event| match event.kind {
            EventKind::RunStarted { context, .. } => Some(*context),
            _ => None,
        })
        .collect();
    assert_eq!(started.len(), 2);
    for (context, run_id) in started.iter().zip([&first, &second]) {
        assert_eq!(&context.run_id, run_id);
        assert_eq!(ulid_time(run_id.as_str()), context.started_unix_ms);
        assert_eq!(context.labels, lablet::RunLabels::default());
        assert_eq!(context.transcript_path, None);
        assert!(!context.capture_content);
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

    let lablet = lablet::build(config).await.unwrap();

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
    let (mut lablet, recorder) = observed(config).await;

    let finished = lablet
        .run(RunRequest::new(" Rename the parser. ").unwrap())
        .await;
    lablet.shutdown().await;

    assert_eq!(finished.transcript.system(), SYSTEM);
    assert_eq!(finished.summary.prompt.system_bytes, SYSTEM.len() as u64);
    assert_eq!(finished.summary.prompt.user_bytes, 20);
    let EventKind::RunStarted {
        context,
        system_prompt,
        prompt,
        ..
    } = &recorder.events()[0].kind
    else {
        panic!("the first event is the start of the run");
    };
    assert!(context.capture_content);
    assert_eq!(system_prompt.as_deref(), Some(SYSTEM));
    assert_eq!(prompt.as_deref(), Some(" Rename the parser. "));
}

#[tokio::test]
async fn telemetry_that_cannot_be_written_leaves_the_outcome_alone() {
    let scratch = Lab::new("unwritable");
    let nowhere = scratch.at("no-such-directory/telemetry.otlp.jsonl");
    let config = scratch.config(
        ENDS,
        json!({ "telemetry": { "file": { "path": nowhere } } }),
    );
    let mut lablet = lablet::build(config).await.unwrap();
    let diagnostics = crate::harness::Diagnostics::capture();

    let finished = lablet.run(request()).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.result().text, "Nothing to fix.");
    assert!(!nowhere.exists());
    let lines = diagnostics.lines();
    let warned: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("WARN") && line.contains(outcome.run_id.as_str()))
        .collect();
    assert_eq!(warned.len(), 1, "{lines:?}");
    assert!(
        warned[0].contains("the run's telemetry wasn't exported whole"),
        "{lines:?}"
    );
    assert_eq!(warned[0].matches("exported whole").count(), 1, "{lines:?}");
    lablet.shutdown().await;
}
