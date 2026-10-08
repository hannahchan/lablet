//! What `build` refuses, and what it says of each refusal.

use std::os::unix::fs::symlink;

use lablet::{BuildError, Config, ConfigError, FilterList, Format, OwnFile, Place, Unsupported};
use serde_json::{Value, json};

use crate::harness::{ENDS, Lab, read, refusal, request};

/// A variable cargo sets for every test, as a key is set for lablet.
const KEY_VARIABLE: &str = "CARGO_MANIFEST_DIR";

/// A variable nothing sets.
const NO_VARIABLE: &str = "LABLET_TEST_A_VARIABLE_NOTHING_SETS";

/// Where every setting of a config the harness writes is: its JSON is on
/// one line.
const LINE: Option<Place> = Some(Place::Line(1));

fn refused(key: &str, value: &str, reason: &str) -> BuildError {
    BuildError::Config(ConfigError::Invalid {
        key: key.to_owned(),
        place: LINE,
        value: Some(value.to_owned()),
        reason: reason.to_owned(),
    })
}

/// The refusal of a setting that holds a file, by its key and the file as
/// the config writes it, with what the system said.
fn refused_file(error: BuildError, key: &str, file: &std::path::Path) -> String {
    match error {
        BuildError::Config(ConfigError::Invalid {
            key: refused,
            place,
            value,
            reason,
        }) => {
            assert_eq!(refused, key);
            assert_eq!(place, LINE);
            assert_eq!(value, Some(json!(file).to_string()));
            reason
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_name_in_a_list_that_no_tool_has_is_refused_with_the_list_it_is_in() {
    let scratch = Lab::new("filter-names");
    let with = |tools: Value| {
        let mut tree = scratch.tree(
            ENDS,
            json!({ "tools": { "builtin": scratch.builtin(&["bash", "read_file"]) } }),
        );
        for (key, value) in tools.as_object().unwrap() {
            tree["tools"][key] = value.clone();
        }
        read(&tree)
    };

    let denied = refusal(with(json!({ "deny": ["raed_file"] }))).await;
    assert_eq!(
        denied,
        BuildError::UnknownTool {
            list: FilterList::Deny,
            place: LINE,
            name: "raed_file".to_owned(),
        }
    );
    assert_eq!(
        denied.to_string(),
        "tools.deny (line 1): raed_file is refused: no tool the lists apply to has the name"
    );

    let allowed = refusal(with(json!({ "allow": ["bash", "raed_file"] }))).await;
    assert_eq!(
        allowed,
        BuildError::UnknownTool {
            list: FilterList::Allow,
            place: LINE,
            name: "raed_file".to_owned(),
        }
    );
    assert_eq!(
        allowed.to_string(),
        "tools.allow (line 1): raed_file is refused: no tool the lists apply to has the name"
    );

    assert_eq!(
        refusal(with(json!({ "allow": ["write_file"] }))).await,
        BuildError::UnknownTool {
            list: FilterList::Allow,
            place: LINE,
            name: "write_file".to_owned(),
        },
        "a built-in tool that isn't enabled is a tool the run doesn't have"
    );
    scratch
        .build(with(json!({ "allow": ["bash"], "deny": ["read_file"] })))
        .await
        .unwrap();
}

#[tokio::test]
async fn task_complete_is_in_neither_list_in_explicit_mode() {
    let scratch = Lab::new("filter-task-complete");
    let with = |completion: &str, tools: Value| {
        let mut tree = scratch.tree(
            ENDS,
            json!({
                "run": { "completion": completion },
                "tools": { "builtin": scratch.builtin(&["bash"]) },
            }),
        );
        for (key, value) in tools.as_object().unwrap() {
            tree["tools"][key] = value.clone();
        }
        read(&tree)
    };

    for (list, tools) in [
        (
            FilterList::Allow,
            json!({ "allow": ["bash", "task_complete"] }),
        ),
        (FilterList::Deny, json!({ "deny": ["task_complete"] })),
    ] {
        let error = refusal(with("explicit", tools.clone())).await;

        assert_eq!(
            error,
            BuildError::UnknownTool {
                list,
                place: LINE,
                name: "task_complete".to_owned(),
            }
        );
        assert!(
            error
                .to_string()
                .starts_with(&format!("tools.{list} (line 1): task_complete is refused")),
            "{error}"
        );
        assert_eq!(
            refusal(with("natural", tools)).await,
            BuildError::UnknownTool {
                list,
                place: LINE,
                name: "task_complete".to_owned(),
            },
            "in natural mode no tool has the name at all"
        );
    }
}

#[tokio::test]
async fn a_built_in_tool_without_a_root_is_refused_by_the_key_of_the_root() {
    let scratch = Lab::new("no-root");
    let config = scratch.config(
        ENDS,
        json!({ "tools": { "builtin": { "enabled": ["read_file", "bash"] } } }),
    );

    let error = refusal(config).await;

    assert_eq!(
        error,
        BuildError::Config(ConfigError::Missing {
            key: "tools.builtin.root".to_owned(),
            reason: "`tools.builtin.enabled` holds `bash`, and a built-in tool works under the \
                     root"
                .to_owned(),
        })
    );
    assert!(
        error
            .to_string()
            .starts_with("tools.builtin.root isn't set")
    );
}

#[tokio::test]
async fn a_root_that_is_no_directory_is_refused_by_its_key_and_its_value() {
    let scratch = Lab::new("bad-root");
    let file = scratch.write("a-file", "");
    let missing = scratch.at("no-such-directory");

    for root in [&file, &missing] {
        let config = scratch.config(
            ENDS,
            json!({ "tools": { "builtin": { "root": root, "enabled": ["bash"] } } }),
        );

        let reason = refused_file(refusal(config).await, "tools.builtin.root", root);
        assert!(!reason.is_empty());
    }
}

#[tokio::test]
async fn a_root_that_holds_a_file_of_lablets_own_is_refused_with_the_file_it_holds() {
    let scratch = Lab::new("root-holds");
    let root = scratch.root();
    let holds = |holds, path: &std::path::Path| BuildError::RootHolds {
        place: LINE,
        root: root.display().to_string(),
        holds,
        path: path.display().to_string(),
    };
    let tools = json!({ "tools": { "builtin": scratch.builtin(&["read_file"]) } });

    // The config's own file.
    let inside = scratch.at("work/lablet.json");
    std::fs::write(&inside, scratch.tree(ENDS, tools.clone()).to_string()).unwrap();
    let error = refusal(Config::from_path(&inside).unwrap()).await;
    assert_eq!(error, holds(OwnFile::Config, &inside));
    assert_eq!(
        error.to_string(),
        format!(
            "tools.builtin.root (line 1): {} is refused: it holds the config, {}",
            root.display(),
            inside.display()
        )
    );
    let beside = scratch.at("lablet.json");
    std::fs::write(&beside, scratch.tree(ENDS, tools.clone()).to_string()).unwrap();
    scratch
        .build(Config::from_path(&beside).unwrap())
        .await
        .unwrap();

    // The system prompt's file.
    let prompt = scratch.write("work/system.md", "You fix tests.");
    let mut tree = scratch.tree(ENDS, tools.clone());
    tree["prompt"] = json!({ "system_file": prompt });
    assert_eq!(
        refusal(read(&tree)).await,
        holds(OwnFile::SystemPrompt, &prompt)
    );

    // The transcript, which doesn't exist yet, and whose name isn't known
    // before the run.
    let transcript = scratch.at("work/runs/{run_id}.json");
    let mut tree = scratch.tree(ENDS, tools.clone());
    tree["run"] = json!({ "transcript_path": transcript });
    assert_eq!(
        refusal(read(&tree)).await,
        holds(OwnFile::Transcript, &transcript)
    );

    // A telemetry file the config names is no file of lablet's in library
    // mode, since the host's SDK writes what a run emits, so a root may
    // hold it.
    symlink(&root, scratch.at("linked")).unwrap();
    let telemetry = scratch.at("linked/telemetry.otlp.jsonl");
    let mut tree = scratch.tree(ENDS, tools);
    tree["telemetry"] = json!({ "file": { "path": telemetry } });
    scratch.build(read(&tree)).await.unwrap();
    assert!(!telemetry.exists());

    // A root that serves no tool holds nothing the model can reach.
    let mut tree = scratch.tree(ENDS, json!({ "tools": { "builtin": { "root": root } } }));
    tree["run"] = json!({ "transcript_path": transcript });
    scratch.build(read(&tree)).await.unwrap();
}

#[tokio::test]
async fn a_variable_no_command_can_start_with_is_refused_by_its_name() {
    let scratch = Lab::new("bad-variable");
    let mut builtin = scratch.builtin(&["bash"]);
    builtin["env"] = json!({ "A=B": "1" });

    assert_eq!(
        refusal(scratch.config(ENDS, json!({ "tools": { "builtin": builtin } }))).await,
        refused(
            "tools.builtin.env.A=B",
            "\"1\"",
            "its name holds `=` or a NUL"
        )
    );
}

/// What each later phase adds, as a config states it.
fn later(scratch: &Lab) -> Vec<(Value, Unsupported, &'static str, &'static str)> {
    let fake = |more: Value| scratch.tree(ENDS, more);
    let mut anthropic = fake(json!({}));
    anthropic["model"] = json!({ "api_key_env": KEY_VARIABLE });
    let mut openai = fake(json!({}));
    openai["model"] = json!({ "provider": "openai", "name": "gpt-5" });
    vec![
        (
            anthropic,
            Unsupported::Anthropic,
            "7",
            "`model.provider: anthropic`",
        ),
        (openai, Unsupported::Openai, "9", "`model.provider: openai`"),
        (
            fake(json!({ "tools": { "mcp": [
                { "name": "docs", "transport": "stdio", "command": "npx" },
            ] } })),
            Unsupported::McpServers,
            "8",
            "a server in `tools.mcp`",
        ),
        (
            fake(json!({ "prompt": { "skills": ["skills/review/SKILL.md"] } })),
            Unsupported::Skills,
            "10",
            "a skill in `prompt.skills`",
        ),
        (
            fake(json!({ "run": { "context": { "mask": {
                "trigger_tokens": 150_000,
                "keep_last": 5,
            } } } })),
            Unsupported::ContextMask,
            "7a",
            "`run.context: mask`",
        ),
        (
            fake(json!({ "run": { "transcript_format": "atif" } })),
            Unsupported::Atif,
            "10",
            "`run.transcript_format: atif`",
        ),
        (
            fake(json!({ "run": {
                "completion": "explicit",
                "completion_schema": { "type": "object", "required": ["fixed"] },
            } })),
            Unsupported::CompletionSchema,
            "10",
            "`run.completion_schema`",
        ),
        (
            fake(json!({ "tools": { "max_description_chars": 1_024 } })),
            Unsupported::MaxDescriptionChars,
            "8",
            "a value of `tools.max_description_chars` other than its default",
        ),
        (
            fake(json!({ "tools": { "max_description_chars": null } })),
            Unsupported::MaxDescriptionChars,
            "8",
            "a value of `tools.max_description_chars` other than its default",
        ),
    ]
}

#[tokio::test]
async fn what_a_later_phase_delivers_is_refused_with_the_phase_that_delivers_it() {
    let scratch = Lab::new("unsupported");

    for (tree, kind, phase, named) in later(&scratch) {
        let error = refusal(read(&tree)).await;

        assert_eq!(error, BuildError::Unsupported { kind, phase }, "{tree}");
        assert_eq!(
            error.to_string(),
            format!("{named} isn't supported yet: phase {phase} of the build plan delivers it")
        );
    }
}

#[tokio::test]
async fn a_default_that_a_later_phase_applies_builds_stated_or_not() {
    let scratch = Lab::new("default-stated");
    let stated = scratch.config(ENDS, json!({ "tools": { "max_description_chars": 2_048 } }));
    let plain = scratch.config(ENDS, json!({}));
    assert_eq!(stated.digest(), plain.digest());

    for config in [stated, plain] {
        let mut lablet = scratch.build(config).await.unwrap();

        let finished = lablet.run(request()).await;
        lablet.shutdown().await;
        assert_eq!(
            finished.summary.outcome.stop_reason(),
            lablet::StopReason::Completed
        );
    }
}

#[tokio::test]
async fn where_a_script_is_served_is_refused_since_nothing_serves_it() {
    let scratch = Lab::new("fake-base-url");
    let mut tree = scratch.tree(ENDS, json!({}));
    tree["model"]["base_url"] = json!("http://localhost:4000");

    let error = refusal(read(&tree)).await;

    assert_eq!(
        error,
        BuildError::Config(ConfigError::NotApplied {
            key: "model.base_url",
            place: LINE,
            value: Some("\"http://localhost:4000\"".to_owned()),
            reached: "the provider `fake`".to_owned(),
        })
    );
    assert_eq!(
        error.to_string(),
        "model.base_url (line 1): \"http://localhost:4000\" is refused: the provider `fake` \
         can't apply it"
    );
}

#[tokio::test]
async fn a_config_is_checked_whole_before_any_adapter_is_selected() {
    let scratch = Lab::new("checked-first");

    for (mut tree, kind, _, _) in later(&scratch) {
        tree["run"]["retry_jitter"] = json!(2);

        let error = refusal(read(&tree)).await;

        assert_eq!(
            error,
            refused(
                "run.retry_jitter",
                "2.0",
                "the jitter is a share of a wait, from 0 to 1"
            ),
            "{kind:?}"
        );
    }

    // A setting the provider can't apply is the config's fault, whether or
    // not the provider has an adapter yet.
    let config =
        Config::from_str("model: { seed: 7 }\nprompt: { system: Hi. }", Format::Yaml).unwrap();
    assert_eq!(
        refusal(config).await,
        BuildError::Config(ConfigError::NotApplied {
            key: "model.seed",
            place: LINE,
            value: Some("7".to_owned()),
            reached: "the provider `anthropic`".to_owned(),
        })
    );
}

#[tokio::test]
async fn a_provider_that_needs_a_key_needs_the_variable_that_holds_it_to_be_set() {
    let anthropic = |variable: &str| {
        Config::from_str(
            &format!("model: {{ api_key_env: {variable} }}\nprompt: {{ system: Hi. }}"),
            Format::Yaml,
        )
        .unwrap()
    };

    let error = refusal(anthropic(NO_VARIABLE)).await;
    assert_eq!(
        error.to_string(),
        format!(
            "model.api_key_env (line 1) is refused: `{NO_VARIABLE}`, the variable it names, \
             isn't set, and the provider `anthropic` needs a key"
        )
    );
    assert!(matches!(error, BuildError::KeyVariable { .. }), "{error:?}");
    let lower = NO_VARIABLE.to_lowercase();
    let error = refusal(anthropic(&lower)).await;
    assert_eq!(
        error.to_string(),
        "model.api_key_env (line 1) is refused: the variable it names isn't set, and the \
         provider `anthropic` needs a key"
    );
    assert!(!format!("{error:?}").contains(&lower), "{error:?}");
    assert_eq!(
        refusal(anthropic(KEY_VARIABLE)).await,
        BuildError::from(Unsupported::Anthropic),
        "the variable is checked before the adapter is selected"
    );
}

#[tokio::test]
async fn a_key_written_where_its_variable_is_named_is_refused_and_never_shown() {
    let pasted = "sk-ant-api03-0123456789abcdef";
    let scratch = Lab::new("pasted-key");
    let mut fake = scratch.tree(ENDS, json!({}));
    fake["model"]["api_key_env"] = json!(pasted);
    let anthropic = json!({ "model": { "api_key_env": pasted }, "prompt": { "system": "Hi." } });

    for tree in [fake, anthropic] {
        let error = refusal(read(&tree)).await;

        assert!(
            matches!(error, BuildError::Config(ConfigError::KeyVariable { .. })),
            "{error:?}"
        );
        assert!(
            error
                .to_string()
                .starts_with("model.api_key_env (line 1) is refused: "),
            "{error}"
        );
        for shown in [error.to_string(), format!("{error:?}")] {
            assert!(
                !shown.contains(pasted) && !shown.contains("sk-ant"),
                "{shown}"
            );
        }
    }
}

#[tokio::test]
async fn a_provider_that_needs_no_key_is_built_whatever_variable_the_config_names() {
    let scratch = Lab::new("no-key");
    let mut tree = scratch.tree(ENDS, json!({}));
    tree["model"]["api_key_env"] = json!(NO_VARIABLE);

    let mut lablet = scratch.build(read(&tree)).await.unwrap();

    let finished = lablet.run(request()).await;
    lablet.shutdown().await;
    assert_eq!(
        finished.summary.outcome.stop_reason(),
        lablet::StopReason::Completed
    );

    let mut openai = scratch.tree(ENDS, json!({}));
    openai["model"] = json!({ "provider": "openai", "api_key_env": NO_VARIABLE });
    assert_eq!(
        refusal(read(&openai)).await,
        BuildError::from(Unsupported::Openai)
    );
}

#[tokio::test]
async fn a_script_that_cannot_be_played_is_refused_by_its_key_and_its_path() {
    let scratch = Lab::new("bad-script");
    let with = |script: &std::path::Path| {
        let mut tree = scratch.tree(ENDS, json!({}));
        tree["model"]["script"] = json!(script);
        read(&tree)
    };
    let said =
        |error: BuildError, script: &std::path::Path| refused_file(error, "model.script", script);

    let missing = scratch.at("missing.yaml");
    assert!(!said(refusal(with(&missing)).await, &missing).is_empty());

    let named = scratch.write("script.txt", ENDS);
    assert_eq!(
        said(refusal(with(&named)).await, &named),
        "its name says neither YAML (`.yaml`, `.yml`) nor JSON (`.json`)"
    );

    let empty = scratch.write("empty.yaml", "[]");
    assert_eq!(
        said(refusal(with(&empty)).await, &empty),
        "it holds no entry, so it could answer no provider call"
    );

    let misspelt = scratch.write(
        "misspelt.json",
        r#"[{ "response": { "content": [], "finish": "end_turn", "usage": { "input_token": 12 } } }]"#,
    );
    let reason = said(refusal(with(&misspelt)).await, &misspelt);
    assert!(
        reason.starts_with("entry 1: ") && reason.contains("input_token"),
        "{reason}"
    );

    let json = scratch.write(
        "script.json",
        r#"[{ "response": { "content": [{ "text": "Done." }], "finish": "end_turn" } }]"#,
    );
    scratch.build(with(&json)).await.unwrap();
}

#[tokio::test]
async fn the_system_prompt_is_read_from_the_file_the_config_names() {
    let scratch = Lab::new("system-file");
    let file = scratch.write("system.md", "You fix tests, from a file.\n");
    let mut tree = scratch.tree(ENDS, json!({}));
    tree["prompt"] = json!({ "system_file": file });
    let mut lablet = scratch.build(read(&tree)).await.unwrap();

    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    assert_eq!(
        finished.transcript.system(),
        "You fix tests, from a file.\n"
    );

    let missing = scratch.at("missing.md");
    tree["prompt"] = json!({ "system_file": missing });
    let reason = refused_file(refusal(read(&tree)).await, "prompt.system_file", &missing);
    assert!(!reason.is_empty());
}
