//! The transcript a run leaves, when the config names a place for it.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use lablet::{FinishedRun, RunId, RunLabels, StopReason};
use lablet_telemetry_registry::attribute as key;
use serde_json::{Value, json};

use crate::harness::{Diagnostics, ENDS, MODEL, PROMPT, SYSTEM, Scratch, Traced, json_of, request};

const CALLS_THEN_ENDS: &str = "
- response:
    content:
      - text: I'll run the tests.
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo one test fails } } }
    usage: { input_tokens: 100, output_tokens: 10 }
    finish: tool_use
    response_id: msg_01
    response_model: scripted-2026-09
- response:
    content:
      - text: One test fails.
    usage: { input_tokens: 150, output_tokens: 8, cache_read_tokens: 100 }
    finish: end_turn
";

fn unix_ms_now() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    u64::try_from(since.as_millis()).unwrap()
}

fn keys(document: &Value) -> Vec<&str> {
    document
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

/// Two runs on one `Lablet` whose transcripts go to `transcript-{run_id}`:
/// the first under the id `run-a` and with labels, the second under a
/// fresh id and with none.
struct Written {
    scratch: Scratch,
    config_digest: String,
    /// When the first run was asked for and when the second returned, in
    /// milliseconds since the Unix epoch.
    between: (u64, u64),
    first: FinishedRun,
    second: FinishedRun,
}

impl Written {
    async fn by_two_runs(test: &str) -> Self {
        let scratch = Scratch::new(test);
        let config = scratch.config(
            CALLS_THEN_ENDS,
            json!({
                "run": { "transcript_path": scratch.at("transcript-{run_id}.json") },
                "tools": { "builtin": scratch.builtin(&["bash", "read_file"]) },
            }),
        );
        let config_digest = config.digest();
        let mut lablet = lablet::build(config).await.unwrap();
        let labels = RunLabels {
            task: Some("fix-failing-test".to_owned()),
            experiment: None,
            trial: Some("3".to_owned()),
        };

        let before = unix_ms_now();
        let first = lablet
            .run(
                request()
                    .run_id(RunId::new("run-a").unwrap())
                    .labels(labels),
            )
            .await;
        let second = lablet.run(request()).await;
        let after = unix_ms_now();
        lablet.shutdown().await;

        Self {
            scratch,
            config_digest,
            between: (before, after),
            first,
            second,
        }
    }

    /// The id of the second run.
    fn fresh(&self) -> &str {
        self.second.summary.outcome.run_id.as_str()
    }

    fn file_of(&self, run_id: &str) -> PathBuf {
        self.scratch.at(&format!("transcript-{run_id}.json"))
    }

    fn of_first(&self) -> Value {
        json_of(&self.file_of("run-a"))
    }

    fn of_second(&self) -> Value {
        json_of(&self.file_of(self.fresh()))
    }
}

#[tokio::test]
async fn each_run_writes_a_transcript_to_a_file_of_its_own_which_its_record_names() {
    let written = Written::by_two_runs("transcripts").await;

    let files: Vec<_> = std::fs::read_dir(written.scratch.at(""))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
    assert_ne!(written.fresh(), "run-a");
    for run_id in ["run-a", written.fresh()] {
        let file = written.file_of(run_id);
        assert!(files.contains(&file), "{files:?}");
        assert_eq!(json_of(&file)["run_id"], json!(run_id));
        assert_eq!(
            Traced::of(&written.scratch.exported(), run_id)
                .wide()
                .attributes[key::LABLET_RUN_TRANSCRIPT_PATH],
            json!(file)
        );
    }
}

#[tokio::test]
async fn a_transcript_opens_with_its_version_and_what_names_its_run() {
    let written = Written::by_two_runs("transcript-names").await;

    let document = written.of_first();
    assert_eq!(
        keys(&document),
        [
            "config_digest",
            "labels",
            "lablet_version",
            "model",
            "run_id",
            "schema_version",
            "started_unix_ms",
            "system",
            "tools",
            "turns",
        ]
    );
    assert_eq!(document["schema_version"], json!(1));
    assert_eq!(document["run_id"], json!("run-a"));
    assert_eq!(
        document["labels"],
        json!({ "task": "fix-failing-test", "experiment": null, "trial": "3" })
    );
    assert_eq!(document["config_digest"], json!(written.config_digest));
    assert_eq!(document["lablet_version"], json!(lablet::VERSION));
    assert_eq!(
        document["model"],
        json!({ "provider": "fake", "api": "script", "name": MODEL, "replays_reasoning": false })
    );
    assert_eq!(document["system"], json!(SYSTEM));

    let other = written.of_second();
    assert_eq!(other["run_id"], json!(written.fresh()));
    assert_eq!(
        other["labels"],
        json!({ "task": null, "experiment": null, "trial": null })
    );
    assert_eq!(other["config_digest"], document["config_digest"]);

    let (before, after) = written.between;
    let started = [&document, &other].map(|document| document["started_unix_ms"].as_u64().unwrap());
    assert!(
        before <= started[0] && started[0] <= started[1] && started[1] <= after,
        "{before} <= {started:?} <= {after}"
    );
}

#[tokio::test]
async fn a_transcript_holds_the_tool_specs_the_run_offered() {
    let written = Written::by_two_runs("transcript-tools").await;

    let document = written.of_first();
    let tools = document["tools"].as_array().unwrap();
    assert_eq!(
        tools.iter().map(|tool| &tool["name"]).collect::<Vec<_>>(),
        ["bash", "read_file"]
    );
    for tool in tools {
        assert_eq!(
            keys(tool),
            [
                "concurrency",
                "description",
                "input_schema",
                "name",
                "source"
            ]
        );
        assert_eq!(tool["source"], json!("builtin"));
        assert_eq!(tool["input_schema"]["type"], json!("object"));
    }
    assert_eq!(tools[0]["concurrency"], json!("exclusive"));
    assert_eq!(tools[1]["concurrency"], json!("shared"));
    assert!(tools[0]["description"].as_str().unwrap().contains("120s"));
    assert_eq!(written.of_second()["tools"], document["tools"]);
}

#[tokio::test]
async fn a_transcript_holds_every_turn_with_its_input_its_response_its_record_and_its_calls() {
    let written = Written::by_two_runs("transcript-turns").await;

    let document = written.of_first();
    let turns = document["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2);
    for turn in turns {
        assert_eq!(keys(turn), ["input", "record", "response", "tool_calls"]);
        assert_eq!(
            keys(&turn["record"]),
            [
                "attempts",
                "finish",
                "latency_ms",
                "response_id",
                "response_model",
                "started_ms",
                "usage",
            ]
        );
    }
    assert_eq!(turns[0]["input"], json!([{ "text": PROMPT }]));
    assert_eq!(
        turns[0]["response"],
        json!([
            { "text": "I'll run the tests." },
            { "tool_use": {
                "id": "call_1",
                "name": "bash",
                "input": { "json": { "command": "echo one test fails" } },
            } },
        ])
    );
    let of_the_run = written.first.transcript.turns()[0].record();
    assert_eq!(
        turns[0]["record"],
        json!({
            "usage": {
                "input_tokens": 100,
                "output_tokens": 10,
                "reasoning_output_tokens": null,
                "cache_read_tokens": null,
                "cache_write_tokens": null,
            },
            "finish": "tool_use",
            "response_id": "msg_01",
            "response_model": "scripted-2026-09",
            "started_ms": of_the_run.started_ms,
            "latency_ms": of_the_run.latency_ms,
            "attempts": 1,
        })
    );
    let of_the_run = &written.first.transcript.turns()[0].tool_calls()[0];
    assert_eq!(
        turns[0]["tool_calls"],
        json!([{
            "call_id": "call_1",
            "status": { "ran": { "source": "builtin", "ended": "ok" } },
            "started_ms": of_the_run.started_ms,
            "latency_ms": of_the_run.latency_ms,
            "truncated_from_bytes": null,
            "content": [{ "text": "one test fails\nexit code: 0" }],
        }])
    );

    assert_eq!(turns[1]["input"], json!([]));
    assert_eq!(turns[1]["response"], json!([{ "text": "One test fails." }]));
    assert_eq!(turns[1]["tool_calls"], json!([]));
    assert_eq!(turns[1]["record"]["finish"], json!("end_turn"));
    assert_eq!(turns[1]["record"]["usage"]["cache_read_tokens"], json!(100));

    // The second run heard the same script, so it said the same.
    let other = written.of_second();
    for (turn, of_other) in turns.iter().zip(other["turns"].as_array().unwrap()) {
        for part in ["input", "response"] {
            assert_eq!(turn[part], of_other[part]);
        }
        assert_eq!(turn["record"]["usage"], of_other["record"]["usage"]);
    }
}

#[tokio::test]
async fn a_run_writes_its_transcript_whatever_stopped_it() {
    for (test, script, run, stopped, turns) in [
        (
            "provider-error",
            "- error: { kind: fatal, message: unknown model }",
            json!({}),
            StopReason::ProviderError,
            0,
        ),
        (
            "max-turns",
            CALLS_THEN_ENDS,
            json!({ "max_turns": 1 }),
            StopReason::MaxTurns,
            1,
        ),
        (
            "refused",
            "- response: { content: [{ text: I can't help with that. }], finish: refusal }",
            json!({}),
            StopReason::Refused,
            1,
        ),
        (
            "truncated",
            "
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo never run } } }
    finish: max_tokens
",
            json!({}),
            StopReason::OutputTruncated,
            1,
        ),
    ] {
        let scratch = Scratch::new(test);
        let transcript = scratch.at("transcript.json");
        let mut config = scratch.tree(
            script,
            json!({
                "run": { "transcript_path": transcript },
                "tools": { "builtin": scratch.builtin(&["bash"]) },
            }),
        );
        for (key, value) in run.as_object().unwrap() {
            config["run"][key] = value.clone();
        }
        let mut lablet = lablet::build(crate::harness::read(&config)).await.unwrap();

        let finished = lablet.run(request()).await;
        lablet.shutdown().await;

        let outcome = &finished.summary.outcome;
        assert_eq!(outcome.stop_reason(), stopped, "{test}");
        let document = json_of(&transcript);
        assert_eq!(document["run_id"], json!(outcome.run_id.as_str()), "{test}");
        assert_eq!(document["system"], json!(SYSTEM), "{test}");
        assert_eq!(document["turns"].as_array().unwrap().len(), turns, "{test}");
        if stopped == StopReason::OutputTruncated {
            assert_eq!(
                document["turns"][0]["tool_calls"],
                json!([]),
                "the call of a response that was cut short is never run"
            );
            assert_eq!(
                document["turns"][0]["record"]["finish"],
                json!("max_tokens")
            );
        }
    }
}

#[tokio::test]
async fn a_path_that_holds_no_run_id_is_every_runs_file() {
    let scratch = Scratch::new("one-file");
    let transcript = scratch.at("transcript.json");
    let config = scratch.config(ENDS, json!({ "run": { "transcript_path": transcript } }));
    let mut lablet = lablet::build(config).await.unwrap();

    let first = lablet.run(request()).await.summary.outcome.run_id;
    assert_eq!(json_of(&transcript)["run_id"], json!(first.as_str()));
    let second = lablet.run(request()).await.summary.outcome.run_id;
    lablet.shutdown().await;

    assert_eq!(json_of(&transcript)["run_id"], json!(second.as_str()));
}

#[tokio::test]
async fn a_transcript_that_cannot_be_written_is_reported_and_the_outcome_is_as_it_was() {
    let scratch = Scratch::new("no-directory");
    let nowhere = scratch.at("no-such-directory/{run_id}.json");
    let config = scratch.config(ENDS, json!({ "run": { "transcript_path": nowhere } }));
    let mut lablet = lablet::build(config).await.unwrap();
    let diagnostics = Diagnostics::capture();

    let finished = lablet
        .run(request().run_id(RunId::new("run-a").unwrap()))
        .await;
    lablet.shutdown().await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.error(), None);
    assert_eq!(outcome.result().text, "Nothing to fix.");
    assert!(!scratch.at("no-such-directory").exists());
    let lines = diagnostics.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("WARN")
            && lines[0].contains("the run's transcript wasn't written")
            && lines[0].contains("run-a")
            && lines[0].contains("no-such-directory/run-a.json"),
        "{lines:?}"
    );
    // The record names where the transcript was to go, whether or not it
    // got there.
    let exported = scratch.exported();
    assert_eq!(
        Traced::of(&exported, "run-a").wide().attributes[key::LABLET_RUN_TRANSCRIPT_PATH],
        json!(scratch.at("no-such-directory/run-a.json"))
    );
}

#[tokio::test]
async fn a_run_id_that_cannot_be_part_of_a_path_leaves_the_run_without_a_transcript() {
    let scratch = Scratch::new("no-component");
    let config = scratch.config(
        ENDS,
        json!({ "run": { "transcript_path": scratch.at("{run_id}.json") } }),
    );
    let mut lablet = lablet::build(config).await.unwrap();
    let diagnostics = Diagnostics::capture();

    let finished = lablet
        .run(request().run_id(RunId::new("../run-a").unwrap()))
        .await;
    lablet.shutdown().await;

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
    assert!(!scratch.at("../run-a.json").exists());
    let lines = diagnostics.lines();
    assert!(
        lines.iter().any(|line| line.contains("WARN")
            && line.contains("the run has no transcript file")
            && line.contains("../run-a")),
        "{lines:?}"
    );
    let exported = scratch.exported();
    assert!(
        !Traced::of(&exported, "../run-a")
            .wide()
            .attributes
            .contains_key(key::LABLET_RUN_TRANSCRIPT_PATH)
    );
}

#[tokio::test]
async fn a_config_that_names_no_place_writes_no_transcript() {
    let scratch = Scratch::new("no-transcript");
    let mut lablet = lablet::build(scratch.config(ENDS, json!({})))
        .await
        .unwrap();

    lablet.run(request()).await;
    lablet.shutdown().await;

    let left: Vec<_> = std::fs::read_dir(scratch.at(""))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left.len(), 3, "{left:?}");
    for name in ["script.yaml", "telemetry.otlp.jsonl", "work"] {
        assert!(left.iter().any(|left| left == name), "{left:?}");
    }
}
