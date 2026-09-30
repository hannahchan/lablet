//! What `lablet check` asks of the library: the config checked whole,
//! resolved, and its tools listed, with no provider selected.

use lablet::{
    BuildError, Config, ConfigError, ErrorClass, Format, OwnFile, Place, RawConfig, Unsupported,
};
use serde_json::{Value, json};

use crate::harness::{ENDS, Lab, read, request};

/// A variable cargo sets for every test, and so one that's set.
const SET: &str = "CARGO_MANIFEST_DIR";

/// A variable nothing sets.
const UNSET: &str = "LABLET_TEST_A_VARIABLE_NOTHING_SETS";

fn yaml(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

fn names(tools: &[lablet::ToolSpec]) -> Vec<&str> {
    tools.iter().map(|tool| tool.name.as_str()).collect()
}

/// The check stops before a provider is selected, so a config whose
/// provider has no adapter yet passes it.
#[tokio::test]
async fn a_config_whose_provider_has_no_adapter_yet_passes_the_check_and_fails_the_build() {
    for text in [
        format!("model: {{ api_key_env: {SET} }}\nprompt: {{ system: Hi. }}"),
        "model: { provider: openai, name: gpt-5 }\nprompt: { system: Hi. }".to_owned(),
    ] {
        let checked = lablet::check(&yaml(&text)).await.unwrap();

        assert_eq!(checked.resolved(), &yaml(&text).resolved());
        assert!(matches!(
            lablet::build(yaml(&text)).await,
            Err(BuildError::Unsupported {
                kind: Unsupported::Anthropic | Unsupported::Openai,
                ..
            })
        ));
    }
}

/// C4: an unset key variable written in capitals is named, one written
/// otherwise isn't, and a provider that needs no key passes.
#[tokio::test]
async fn the_check_refuses_an_unset_key_variable_and_names_it_only_when_it_is_written_as_one() {
    let anthropic = |variable: &str| {
        yaml(&format!(
            "model: {{ api_key_env: {variable} }}\nprompt: {{ system: Hi. }}"
        ))
    };

    let named = lablet::check(&anthropic(UNSET)).await.unwrap_err();
    assert_eq!(
        named.to_string(),
        format!(
            "model.api_key_env (line 1) is refused: `{UNSET}`, the variable it names, isn't \
             set, and the provider `anthropic` needs a key"
        )
    );
    assert_eq!(named.class(), ErrorClass::Config);

    let lower = UNSET.to_lowercase();
    let unnamed = lablet::check(&anthropic(&lower)).await.unwrap_err();
    assert!(
        matches!(unnamed, BuildError::KeyVariable { .. }),
        "{unnamed:?}"
    );
    for shown in [unnamed.to_string(), format!("{unnamed:?}")] {
        assert!(!shown.contains(&lower), "{shown}");
    }

    let scratch = Lab::new("check-fake-key");
    let mut fake = scratch.tree(ENDS, json!({}));
    fake["model"]["api_key_env"] = json!(UNSET);
    lablet::check(&read(&fake)).await.unwrap();
}

/// C16 through the environment itself: a variable's value that isn't a
/// URL is refused as the config writes it.
#[tokio::test]
async fn a_base_url_a_variable_gives_is_refused_without_the_variable_s_value() {
    let text =
        format!("model: {{ provider: openai, base_url: '${{{SET}}}' }}\nprompt: {{ system: Hi. }}");
    let held = std::env::var(SET).unwrap();

    let error = lablet::check(&yaml(&text)).await.unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "model.base_url (line 1): \"${{{SET}}}\" is refused: a URL begins `http://` or \
             `https://` and names a host, as `http://localhost:11434/v1` does"
        )
    );
    assert!(!format!("{error}{error:?}").contains(&held), "{error:?}");
}

/// C10: an unset variable is a config error, by its key.
#[tokio::test]
async fn a_variable_that_is_not_set_fails_the_check_by_the_key_it_is_in() {
    let scratch = Lab::new("check-unset");
    let mut tree = scratch.tree(ENDS, json!({}));
    tree["prompt"]["system"] = json!(format!("You fix ${{{UNSET}}} tests."));

    let error = lablet::check(&read(&tree)).await.unwrap_err();

    assert_eq!(
        error,
        BuildError::Config(ConfigError::Invalid {
            key: "prompt.system".to_owned(),
            place: Some(Place::Line(1)),
            value: Some(json!(format!("You fix ${{{UNSET}}} tests.")).to_string()),
            reason: format!("the variable `{UNSET}` isn't set"),
        })
    );
    assert_eq!(error.class().prefix(), "config:");
}

/// C3: an override is in the resolved config the check prints.
#[tokio::test]
async fn an_override_is_in_the_resolved_config_the_check_gives() {
    let scratch = Lab::new("check-override");
    let path = scratch.write("lablet.json", &scratch.tree(ENDS, json!({})).to_string());
    let mut raw = RawConfig::from_path(&path).unwrap();

    raw.set("run.max_turns=5").unwrap();
    let checked = lablet::check(&raw.config().unwrap()).await.unwrap();

    let resolved = serde_json::to_value(checked.resolved()).unwrap();
    assert_eq!(resolved["run"]["max_turns"], json!(5));
}

/// The tools the check lists are those a built `Lablet` offers, after the
/// lists.
#[tokio::test]
async fn the_check_and_a_built_lablet_list_the_tools_a_run_is_offered() {
    let scratch = Lab::new("check-tools");
    let config = scratch.config(
        ENDS,
        json!({ "tools": {
            "builtin": scratch.builtin(&["bash", "read_file", "write_file"]),
            "deny": ["write_file"],
        } }),
    );

    let checked = lablet::check(&config).await.unwrap();
    let lablet = lablet::build(config).await.unwrap();

    assert_eq!(names(checked.tools()), ["bash", "read_file"]);
    assert_eq!(lablet.tools(), checked.tools());
    lablet.shutdown().await;
}

/// The file a task prompt is read from is lablet's own, as the system
/// prompt's is, so a root that holds it is refused by the check and the
/// build alike.
#[tokio::test]
async fn a_root_that_holds_the_task_prompt_s_file_is_refused() {
    let scratch = Lab::new("check-prompt-file");
    let config = || {
        scratch.config(
            ENDS,
            json!({ "tools": { "builtin": scratch.builtin(&["read_file"]) } }),
        )
    };
    let inside = scratch.write("work/task.md", "Fix the failing test.");
    let expected = BuildError::RootHolds {
        place: Some(Place::Line(1)),
        root: scratch.root().display().to_string(),
        holds: OwnFile::TaskPrompt,
        path: inside.display().to_string(),
    };

    let told = config().with_prompt_file(&inside);
    assert_eq!(told.prompt_file(), Some(inside.as_path()));
    assert_eq!(lablet::check(&told).await.unwrap_err(), expected);
    assert_eq!(lablet::build(told).await.unwrap_err(), expected);
    assert!(
        expected.to_string().ends_with(&format!(
            "it holds the task prompt's file, {}",
            inside.display()
        )),
        "{expected}"
    );

    let beside = scratch.write("task.md", "Fix the failing test.");
    let mut lablet = lablet::build(config().with_prompt_file(&beside))
        .await
        .unwrap();
    lablet.run(request()).await;
    lablet.shutdown().await;
    assert_eq!(config().prompt_file(), None);
}

#[test]
fn each_class_of_failure_has_the_prefix_a_script_matches() {
    assert_eq!(
        [ErrorClass::Config, ErrorClass::Mcp, ErrorClass::Provider].map(ErrorClass::prefix),
        ["config:", "mcp:", "provider:"]
    );
}

/// Every refusal the build makes today is the config's: a rule it breaks,
/// or what lablet found where it runs.
#[tokio::test]
async fn every_refusal_of_a_config_is_of_the_config_class() {
    let scratch = Lab::new("check-classes");
    let tree = |more: Value| scratch.tree(ENDS, more);
    let mut refused = Vec::new();
    for more in [
        json!({ "run": { "retry_jitter": 2 } }),
        json!({ "tools": { "mcp": [{ "name": "docs", "transport": "stdio", "command": "npx" }] } }),
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]), "deny": ["grep"] } }),
        json!({ "tools": { "builtin": { "root": scratch.at(""), "enabled": ["bash"] } } }),
    ] {
        refused.push(lablet::check(&read(&tree(more))).await.unwrap_err());
    }
    let mut anthropic = tree(json!({}));
    anthropic["model"] = json!({ "api_key_env": UNSET });
    refused.push(lablet::check(&read(&anthropic)).await.unwrap_err());

    for error in &refused {
        assert_eq!(error.class(), ErrorClass::Config, "{error:?}");
    }
    assert!(matches!(refused[0], BuildError::Config(_)));
    assert!(matches!(refused[1], BuildError::Unsupported { .. }));
    assert!(matches!(refused[2], BuildError::UnknownTool { .. }));
    assert!(matches!(refused[3], BuildError::RootHolds { .. }));
    assert!(matches!(refused[4], BuildError::KeyVariable { .. }));
    assert_eq!(
        BuildError::Tools {
            reason: "two tools share a name".to_owned()
        }
        .class(),
        ErrorClass::Config
    );
}
