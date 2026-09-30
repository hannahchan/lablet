//! The built-in tools, as a run built from a config has them.

use std::os::unix::fs::symlink;

use lablet::{EventKind, FinishedRun, StopReason};
use lablet_model::{ToolCallOutcome, ToolResultContent};
use lablet_telemetry_registry::attribute as key;
use serde_json::{Value, json};

use crate::harness::{Scratch, Traced, observed, request};

const SECRET: &str = "what the model is not to read";

/// A variable cargo sets for every test, as a key is set for lablet. Its
/// value is a long path, as long as a key.
const KEY_VARIABLE: &str = "CARGO_MANIFEST_DIR";

/// A variable cargo sets for every test, which holds no secret of lablet's.
const NOT_A_SECRET: &str = "CARGO_PKG_NAME";

/// What the model was sent of a call: its status, and its text.
fn sent(outcome: &ToolCallOutcome) -> (&'static str, String) {
    let text = outcome
        .content
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect();
    (outcome.status.as_str(), text)
}

/// A script in which each of `calls` is a turn of its own, and a last turn
/// ends the run.
fn calling(calls: &[(&str, Value)]) -> String {
    let mut script: Vec<Value> = calls
        .iter()
        .zip(1..)
        .map(|((tool, input), number)| {
            json!({ "response": {
                "content": [{ "tool_use": {
                    "id": format!("call_{number}"),
                    "name": tool,
                    "input": { "json": input },
                } }],
                "finish": "tool_use",
            } })
        })
        .collect();
    script.push(json!({ "response": { "content": [{ "text": "Done." }], "finish": "end_turn" } }));
    Value::Array(script).to_string()
}

/// One run of the config `tree`.
async fn run_tree(tree: &Value) -> FinishedRun {
    let mut lablet = lablet::build(crate::harness::read(tree)).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;
    finished
}

/// One run of `script`, with `tools` stated, and what it exported.
async fn run(scratch: &Scratch, script: &str, tools: Value) -> FinishedRun {
    let config = scratch.config(script, json!({ "tools": tools }));
    let mut lablet = lablet::build(config).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;
    finished
}

#[tokio::test]
async fn an_allow_list_offers_the_tools_it_names_and_no_other() {
    let scratch = Scratch::new("allow");
    let config = scratch.config(
        &calling(&[
            ("bash", json!({ "command": "echo allowed" })),
            ("read_file", json!({ "path": "notes.md" })),
        ]),
        json!({ "tools": {
            "builtin": scratch.builtin(&["bash", "read_file", "write_file"]),
            "allow": ["bash"],
        } }),
    );
    scratch.write("work/notes.md", SECRET);
    let (mut lablet, recorder) = observed(config).await;

    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    let EventKind::RunStarted { tools, .. } = &recorder.events()[0].kind else {
        panic!("the first event is the start of the run");
    };
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["bash"]
    );
    assert_eq!(
        finished
            .summary
            .tools
            .iter()
            .map(lablet_model::ToolName::as_str)
            .collect::<Vec<_>>(),
        ["bash"]
    );
    let exported = scratch.exported();
    let traced = Traced::of(&exported, finished.summary.outcome.run_id.as_str());
    assert_eq!(
        traced.wide().attributes[key::LABLET_TOOLS_NAMES],
        json!(["bash"])
    );
    assert_eq!(traced.wide().attributes[key::LABLET_TOOLS_COUNT], json!(1));

    // A tool that isn't offered isn't there to be called either.
    let turns = finished.transcript.turns();
    assert_eq!(
        sent(&turns[0].tool_calls()[0]),
        ("ok", "allowed\nexit code: 0".to_owned())
    );
    let (status, text) = sent(&turns[1].tool_calls()[0]);
    assert_eq!(status, "unknown");
    assert!(!text.contains(SECRET), "{text}");
    assert_eq!(
        traced.wide().attributes[key::LABLET_TOOL_CALLS_UNKNOWN],
        json!(1)
    );
}

#[tokio::test]
async fn a_deny_list_takes_a_tool_out_of_what_is_offered() {
    let scratch = Scratch::new("deny");
    let tools = json!({
        "builtin": scratch.builtin(&["bash", "read_file", "write_file"]),
        "allow": ["bash", "write_file"],
        "deny": ["bash"],
    });

    let finished = run(&scratch, crate::harness::ENDS, tools).await;

    assert_eq!(
        finished
            .summary
            .tools
            .iter()
            .map(lablet_model::ToolName::as_str)
            .collect::<Vec<_>>(),
        ["write_file"]
    );
}

#[tokio::test]
async fn explicit_completion_offers_task_complete_beside_the_tools_that_are_enabled() {
    let scratch = Scratch::new("explicit");
    let config = scratch.config(
        r"
- response:
    content:
      - tool_use: { id: call_1, name: task_complete, input: { json: { fixed: true } } }
    finish: tool_use
",
        json!({
            "run": { "completion": "explicit" },
            "tools": { "builtin": scratch.builtin(&["bash"]), "allow": ["bash"] },
        }),
    );
    let mut lablet = lablet::build(config).await.unwrap();

    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    let names: Vec<_> = finished
        .summary
        .tools
        .iter()
        .map(lablet_model::ToolName::as_str)
        .collect();
    assert_eq!(names, ["bash", "task_complete"]);
    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.result().structured, Some(json!({ "fixed": true })));
}

#[tokio::test]
async fn a_path_outside_the_root_is_an_error_result_and_the_file_is_not_read() {
    let scratch = Scratch::new("outside");
    let outside = scratch.write("secret.txt", SECRET);
    scratch.write("work/inside.txt", "what the model may read");
    let script = calling(&[
        ("read_file", json!({ "path": outside })),
        ("read_file", json!({ "path": "../secret.txt" })),
        ("read_file", json!({ "path": "inside.txt" })),
    ]);

    let finished = run(&scratch, &script, scratch.builtin_tools(&["read_file"])).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!((outcome.turns, outcome.tool_calls), (4, 3));
    let turns = finished.transcript.turns();
    for refused in &turns[..2] {
        let call = &refused.tool_calls()[0];
        let (status, text) = sent(call);
        assert_eq!(status, "tool_error");
        assert!(call.status.is_error());
        assert!(text.contains("outside the run's root directory"), "{text}");
        assert!(!text.contains(SECRET), "{text}");
    }
    assert_eq!(
        sent(&turns[2].tool_calls()[0]),
        ("ok", "what the model may read".to_owned())
    );
    assert_eq!(finished.summary.tool_calls.errors, 2);

    let exported = scratch.exported();
    let traced = Traced::of(&exported, outcome.run_id.as_str());
    let statuses: Vec<_> = traced
        .tools()
        .iter()
        .map(|tool| {
            (
                &tool.attributes[key::LABLET_TOOL_STATUS],
                tool.attributes.get(key::ERROR_TYPE),
            )
        })
        .collect();
    let refused = (&json!("tool_error"), Some(&json!("tool_error")));
    assert_eq!(statuses, [refused, refused, (&json!("ok"), None)]);
    assert!(
        !std::fs::read_to_string(scratch.telemetry())
            .unwrap()
            .contains(SECRET)
    );
}

#[tokio::test]
async fn a_command_past_the_timeout_is_an_error_result_of_kind_timeout_and_the_run_goes_on() {
    let scratch = Scratch::new("timeout");
    let script = calling(&[
        ("bash", json!({ "command": "sleep 60" })),
        ("bash", json!({ "command": "echo on it goes" })),
    ]);
    let mut tools = scratch.builtin_tools(&["bash"]);
    tools["builtin"]["timeout"] = json!("50ms");

    let finished = run(&scratch, &script, tools).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.result().text, "Done.");
    assert_eq!((outcome.turns, outcome.tool_calls), (3, 2));
    let turns = finished.transcript.turns();
    let timed_out = &turns[0].tool_calls()[0];
    assert!(timed_out.status.is_error());
    assert_eq!(
        sent(timed_out),
        (
            "timeout",
            "bash was stopped after 50ms, the longest the call could take".to_owned()
        )
    );
    assert!(
        (50..10_000).contains(&timed_out.latency_ms),
        "{}",
        timed_out.latency_ms
    );
    assert_eq!(
        sent(&turns[1].tool_calls()[0]),
        ("ok", "on it goes\nexit code: 0".to_owned())
    );

    let exported = scratch.exported();
    let traced = Traced::of(&exported, outcome.run_id.as_str());
    let tools = traced.tools();
    assert_eq!(tools[0].attributes[key::ERROR_TYPE], json!("timeout"));
    assert_eq!(
        tools[0].attributes[key::LABLET_TOOL_STATUS],
        json!("timeout")
    );
    assert!(!tools[1].attributes.contains_key(key::ERROR_TYPE));
    assert_eq!(
        traced.wide().attributes[key::LABLET_TOOL_CALLS_ERRORS],
        json!(1)
    );
    assert_eq!(
        traced.chats().len(),
        3,
        "the model was asked again after the timeout"
    );
}

#[tokio::test]
async fn a_command_has_lablet_s_environment_less_the_key_and_no_result_shows_the_key() {
    let key = std::env::var(KEY_VARIABLE)
        .expect("cargo sets the variable for a test, so lablet's environment holds it");
    let scratch = Scratch::new("environment");
    scratch.write("secret.txt", SECRET);
    scratch.write("work/key.txt", &key);
    symlink(scratch.at("secret.txt"), scratch.at("work/link.txt")).unwrap();
    let script = calling(&[
        ("bash", json!({ "command": "env" })),
        ("read_file", json!({ "path": "link.txt" })),
        ("bash", json!({ "command": "echo leaving; exit 3" })),
        ("bash", json!({ "command": "cat key.txt" })),
    ]);
    let mut tree = scratch.tree(
        &script,
        json!({ "tools": { "builtin": scratch.builtin(&["bash", "read_file"]) } }),
    );
    tree["model"]["api_key_env"] = json!(KEY_VARIABLE);
    tree["tools"]["builtin"]["env"] = json!({ "LABLET_TEST_ADDED": "added by the config" });
    tree["telemetry"]["capture_content"] = json!(true);
    let mut lablet = lablet::build(crate::harness::read(&tree)).await.unwrap();

    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    let turns = finished.transcript.turns();

    let (status, environment) = sent(&turns[0].tool_calls()[0]);
    assert_eq!(status, "ok");
    assert!(environment.ends_with("exit code: 0"), "{environment}");
    assert!(!environment.contains(KEY_VARIABLE), "{environment}");
    assert!(!environment.contains(&key), "{environment}");
    let not_a_secret = std::env::var(NOT_A_SECRET).unwrap();
    assert!(
        environment.contains(&format!("{NOT_A_SECRET}={not_a_secret}\n")),
        "{environment}"
    );
    assert!(
        environment.contains("LABLET_TEST_ADDED=added by the config"),
        "{environment}"
    );
    assert!(environment.contains("PATH="), "{environment}");

    let (status, read) = sent(&turns[1].tool_calls()[0]);
    assert_eq!(status, "tool_error");
    assert!(read.contains("outside the run's root directory"), "{read}");
    assert!(!read.contains(SECRET), "{read}");

    let left = &turns[2].tool_calls()[0];
    assert!(!left.status.is_error());
    assert_eq!(sent(left), ("ok", "leaving\nexit code: 3".to_owned()));

    let printed = &turns[3].tool_calls()[0];
    assert_eq!(
        sent(printed),
        ("ok", "[secret withheld]\nexit code: 0".to_owned())
    );
    let telemetry = std::fs::read_to_string(scratch.telemetry()).unwrap();
    assert!(telemetry.contains("[secret withheld]"), "{telemetry}");
    assert!(!telemetry.contains(&key), "telemetry holds the key");
}

#[tokio::test]
async fn the_key_variable_that_tools_builtin_env_names_is_passed_on() {
    let scratch = Scratch::new("environment-passed-on");
    let script = calling(&[(
        "bash",
        json!({ "command": format!("echo \"${KEY_VARIABLE}\"") }),
    )]);
    let mut tree = scratch.tree(
        &script,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    tree["model"]["api_key_env"] = json!(KEY_VARIABLE);
    tree["tools"]["builtin"]["env"] = json!({ KEY_VARIABLE: "passed on by the config" });

    let finished = run_tree(&tree).await;

    let turns = finished.transcript.turns();
    assert_eq!(
        sent(&turns[0].tool_calls()[0]),
        ("ok", "passed on by the config\nexit code: 0".to_owned())
    );
}
