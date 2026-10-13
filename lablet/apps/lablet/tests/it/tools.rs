//! The built-in tools, as a run built from a config has them.

use std::os::unix::fs::symlink;

use lablet::{Credential, FinishedRun, StopReason};
use lablet_model::{ToolCallOutcome, ToolResultContent};
use serde_json::{Value, json};

use crate::harness::{KEY, Lab, Traced, request};
use crate::key;

const SECRET: &str = "what the model is not to read";

/// A variable cargo sets for every test, as a key is set for lablet. Its
/// value is a long path, as long as a key.
const KEY_VARIABLE: &str = "CARGO_MANIFEST_DIR";

/// A variable cargo sets for every test, which holds no secret of lablet's.
const NOT_A_SECRET: &str = "CARGO_PKG_NAME";

/// A variable cargo sets for every test, whose value is under the floor a
/// secret is cut from.
const SHORT_VARIABLE: &str = "CARGO_PKG_VERSION_MAJOR";

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

/// One run of `config`, on the no-op `Otel`.
async fn run_config(config: lablet::Config) -> FinishedRun {
    let mut lablet = crate::harness::quiet(config).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;
    finished
}

/// One run of `script`, with `tools` stated, and what it exported.
async fn run(scratch: &Lab, script: &str, tools: Value) -> FinishedRun {
    let config = scratch.config(script, json!({ "tools": tools }));
    let mut lablet = scratch.build(config).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;
    finished
}

#[tokio::test]
async fn an_allow_list_offers_the_tools_it_names_and_no_other() {
    let scratch = Lab::new("allow");
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
    let mut lablet = scratch.build(config).await.unwrap();

    assert_eq!(
        lablet
            .tools()
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["bash"]
    );
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

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
    let scratch = Lab::new("deny");
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
    let scratch = Lab::new("explicit");
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
    let mut lablet = scratch.build(config).await.unwrap();

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
    let scratch = Lab::new("outside");
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
    assert!(!scratch.exported_text().contains(SECRET));
}

#[tokio::test]
async fn a_command_past_the_timeout_is_an_error_result_of_kind_timeout_and_the_run_goes_on() {
    let scratch = Lab::new("timeout");
    scratch.write("work/notes.md", "on it goes");
    let script = calling(&[
        ("bash", json!({ "command": "sleep 60" })),
        ("read_file", json!({ "path": "notes.md" })),
    ]);
    let mut tools = scratch.builtin_tools(&["bash", "read_file"]);
    tools["builtin"]["timeout"] = json!("1s");

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
            "bash was stopped after 1s, the longest the call could take".to_owned()
        )
    );
    assert!(
        (1_000..10_000).contains(&timed_out.latency_ms),
        "{}",
        timed_out.latency_ms
    );
    // The call after the timeout starts no process, so no real-time limit
    // bounds work that has to finish.
    assert_eq!(
        sent(&turns[1].tool_calls()[0]),
        ("ok", "on it goes".to_owned())
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

/// T16: what a run drops and cuts, and what `check` says of it: the key's
/// variable fills `model.api_key`, so no command inherits it, nor any
/// variable holding its value, and no result shows the key.
#[tokio::test]
async fn a_command_has_the_environment_less_the_key_and_no_result_shows_the_key() {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test reads the variables cargo set, to know what a command inherits"
    )]
    let (key, not_a_secret, named_in_the_environment): (String, String, Vec<String>) = (
        std::env::var(KEY_VARIABLE).expect("cargo sets the variable for a test"),
        std::env::var(NOT_A_SECRET).unwrap(),
        std::env::vars()
            .filter(|(_, value)| value.contains(&std::env::var(KEY_VARIABLE).unwrap()))
            .map(|(name, _)| name)
            .collect(),
    );
    let scratch = Lab::new("environment");
    scratch.write("secret.txt", SECRET);
    scratch.write("work/key.txt", &key);
    scratch.write("work/header.txt", &format!("Bearer {key}"));
    symlink(scratch.at("secret.txt"), scratch.at("work/link.txt")).unwrap();
    let script = calling(&[
        ("bash", json!({ "command": "env" })),
        ("read_file", json!({ "path": "link.txt" })),
        ("bash", json!({ "command": "echo leaving; exit 3" })),
        ("bash", json!({ "command": "cat key.txt" })),
        ("bash", json!({ "command": "cat header.txt" })),
    ]);
    let mut tree = scratch.tree(
        &script,
        json!({ "tools": { "builtin": scratch.builtin(&["bash", "read_file"]) } }),
    );
    tree["model"]["api_key"] = json!(format!("${{{KEY_VARIABLE}}}"));
    tree["tools"]["builtin"]["env"] = json!({ "LABLET_TEST_ADDED": "added by the config" });
    tree["telemetry"]["capture_content"] = json!(true);
    tree["telemetry"]["otlp"]["headers"] = json!({
        "Authorization": format!("Bearer ${{{KEY_VARIABLE}}}"),
        "X-Short": format!("${{{SHORT_VARIABLE}}}"),
    });
    let config = crate::harness::read_filled(&tree, &[(KEY_VARIABLE, &key), (SHORT_VARIABLE, "7")]);

    let checked = scratch.builder(config.clone()).check().await.unwrap();
    t16_check_names_the_key(&checked, &key, &named_in_the_environment);

    let mut lablet = scratch.build(config).await.unwrap();
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
    let header = &turns[4].tool_calls()[0];
    assert_eq!(
        sent(header),
        ("ok", "[secret withheld]\nexit code: 0".to_owned()),
        "the header's value is cut whole"
    );
    let telemetry = scratch.exported_text();
    assert!(telemetry.contains("[secret withheld]"), "{telemetry}");
    assert!(!telemetry.contains(&key), "telemetry holds the key");
}

/// T16's check: the key's variable fills `model.api_key` and a header, so
/// `check` names it under `own:`, drops it and every copy of its value, and
/// shows the key nowhere.
fn t16_check_names_the_key(checked: &lablet::Checked, key: &str, in_the_environment: &[String]) {
    let own: Vec<String> = checked.own().iter().map(Credential::as_own).collect();
    assert_eq!(own.len(), 3, "{own:?}");
    assert!(
        own[0].starts_with(&format!("{KEY_VARIABLE} (line 1) -> model.api_key"))
            && own[0].ends_with("; no process inherits it"),
        "{own:?}"
    );
    assert!(
        own[1].starts_with(&format!(
            "{KEY_VARIABLE} (line 1) -> telemetry.otlp.headers.Authorization"
        )) && own[1].ends_with("; no process inherits it"),
        "{own:?}"
    );
    assert_eq!(
        own[2],
        format!(
            "{SHORT_VARIABLE} (line 1) (under 16 bytes, not cut) -> telemetry.otlp.headers.X-Short; \
             no process inherits it"
        )
    );
    let copies = &checked.own()[0].copies;
    for copy in copies {
        assert!(
            in_the_environment.contains(copy) && *copy != KEY_VARIABLE,
            "{copy} holds no copy of the key"
        );
    }
    let inherited = checked.inherited();
    assert!(
        !inherited.contains(&KEY_VARIABLE.to_owned()),
        "{inherited:?}"
    );
    assert!(
        inherited.contains(&NOT_A_SECRET.to_owned()),
        "{inherited:?}"
    );
    for copy in copies {
        assert!(!inherited.contains(copy), "{copy} is inherited");
    }
    assert!(!format!("{checked:?}").contains(key), "{checked:?}");
}

/// A variable the config sets under the key's variable's name is passed on
/// as set: what's given knowingly isn't dropped, and the key's value never
/// reaches the command.
#[tokio::test]
async fn a_variable_the_config_sets_under_the_key_s_name_is_passed_on() {
    let scratch = Lab::new("environment-passed-on");
    let script = calling(&[(
        "bash",
        json!({ "command": format!("echo \"${KEY_VARIABLE}\"") }),
    )]);
    let mut tree = scratch.tree(
        &script,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    tree["model"]["api_key"] = json!(format!("${{{KEY_VARIABLE}}}"));
    tree["tools"]["builtin"]["env"] = json!({ KEY_VARIABLE: "passed on by the config" });
    let config = crate::harness::read_filled(&tree, &[(KEY_VARIABLE, KEY)]);

    let finished = run_config(config).await;

    let turns = finished.transcript.turns();
    assert_eq!(
        sent(&turns[0].tool_calls()[0]),
        ("ok", "passed on by the config\nexit code: 0".to_owned())
    );
}

/// J1 in the library: a handed-in variable holding the key's value is
/// dropped from a command's environment under any name, as its `Bearer%20`
/// form is, and a value under the cut's floor isn't.
#[tokio::test]
async fn a_handed_in_variable_holding_the_key_s_value_is_dropped_and_a_short_one_is_not() {
    let scratch = Lab::new("dropped-by-value");
    // A key holding `+`, `/` and `=`, which encoding changes, so the encoded
    // copy is caught only through the decoder.
    let key = "sk+ant/0123456789abcdef0123456789==";
    let short = "shortvalue12";
    let script = calling(&[(
        "bash",
        json!({ "command": "echo \"${MY_COPY-dropped}|${AUTH-dropped}|${ENCODED-dropped}|${SHORT_COPY-dropped}|${PLAIN-dropped}\"" }),
    )]);
    let mut tree = scratch.tree(
        &script,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    tree["model"]["api_key"] = json!("${LABLET_TEST_KEY}");
    let config = crate::harness::read_filled(&tree, &[("LABLET_TEST_KEY", key)]);
    let base = [
        ("PATH", "/usr/bin:/bin"),
        ("MY_COPY", key),
        ("AUTH", &format!("Bearer {key}")),
        (
            "ENCODED",
            "authorization=Bearer%20sk%2Bant%2F0123456789abcdef0123456789%3D%3D",
        ),
        ("SHORT_COPY", short),
        ("PLAIN", "plain-0123456789abcdef"),
    ];
    let builder = lablet::Lablet::builder(config, scratch.otel()).with_process_environment(base);

    let checked = builder.check().await.unwrap();
    let mut lablet = builder.build().await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    assert_eq!(
        sent(&finished.transcript.turns()[0].tool_calls()[0]),
        (
            "ok",
            format!("dropped|dropped|dropped|{short}|plain-0123456789abcdef\nexit code: 0")
        )
    );
    assert_eq!(
        checked.own()[0].as_own(),
        "LABLET_TEST_KEY (line 1) -> model.api_key, or AUTH, which holds it, or ENCODED, which \
         holds it, or MY_COPY, which holds it; no process inherits it"
    );
    assert_eq!(checked.inherited(), ["PATH", "PLAIN", "SHORT_COPY"]);
    let environment = checked.environment().unwrap();
    assert_eq!(
        environment.by_value,
        ["AUTH", "ENCODED", "MY_COPY"].map(str::to_owned).into()
    );
    assert!(
        environment.by_name.is_empty(),
        "the base holds no LABLET_TEST_KEY"
    );
}

/// A `credentials` entry reaches `bash` under its name, after the variable
/// that held it was dropped, and its value is cut from the result; an
/// `env` entry that's one `${VAR}` reaches `bash` as the text it was
/// filled with.
#[tokio::test]
async fn a_credential_reaches_bash_under_its_name_and_is_cut_and_an_env_entry_is_text() {
    let scratch = Lab::new("credential-given");
    let token = "ghp_0123456789abcdef0123456789";
    let script = calling(&[(
        "bash",
        json!({ "command": "echo \"${GITHUB_TOKEN-dropped}|$TOK|$PORT|${#PORT}\"" }),
    )]);
    let mut tree = scratch.tree(
        &script,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    tree["tools"]["builtin"]["credentials"] = json!({ "TOK": "${GITHUB_TOKEN}" });
    tree["tools"]["builtin"]["env"] = json!({ "PORT": "${PORT}" });
    let config = crate::harness::read_filled(&tree, &[("GITHUB_TOKEN", token), ("PORT", "8080")]);
    let base = [("PATH", "/usr/bin:/bin"), ("GITHUB_TOKEN", token)];
    let builder = lablet::Lablet::builder(config, scratch.otel()).with_process_environment(base);

    let checked = builder.check().await.unwrap();
    let mut lablet = builder.build().await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    assert_eq!(
        sent(&finished.transcript.turns()[0].tool_calls()[0]),
        (
            "ok",
            "dropped|[secret withheld]|8080|4\nexit code: 0".to_owned()
        )
    );
    assert_eq!(
        checked.given()[0].as_given(),
        "GITHUB_TOKEN (line 1) -> bash as TOK"
    );
    assert!(checked.own().is_empty());
    assert_eq!(checked.env()[0].to_string(), "PORT (line 1) -> bash");
    assert_eq!(checked.inherited(), ["PATH"]);
    assert_eq!(
        checked.environment().unwrap().by_name,
        ["GITHUB_TOKEN".to_owned()].into()
    );
}

/// The check reports what `bash` inherits through the host's own
/// propagator, as a run gives it: a field it names is a context variable a
/// command gets in place of an inherited one, and isn't inherited as set.
#[tokio::test]
async fn the_check_reports_the_context_variables_of_the_host_s_propagator() {
    let scratch = Lab::new("check-propagator");
    let script = calling(&[("bash", json!({ "command": "true" }))]);
    let tree = scratch.tree(
        &script,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    let config = crate::harness::read_filled(&tree, &[]);
    let otel = lablet::Otel::new(
        opentelemetry::trace::noop::NoopTracerProvider::new(),
        opentelemetry::logs::NoopLoggerProvider::new(),
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
    let base = [
        ("PATH", "/usr/bin:/bin"),
        (
            "TRACEPARENT",
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
        ),
        ("TRACESTATE", "k=v"),
    ];

    let checked = lablet::Lablet::builder(config, otel)
        .with_process_environment(base)
        .check()
        .await
        .unwrap();

    let environment = checked.environment().unwrap();
    assert_eq!(environment.context, ["TRACEPARENT", "TRACESTATE"]);
    assert_eq!(checked.inherited(), ["PATH"]);
}
