//! What `lablet check` asks of the library: the config checked whole,
//! resolved, and its tools listed, with no provider selected.

use lablet::{
    BuildError, Config, ConfigError, ErrorClass, Format, Otel, OwnFile, Place, RawConfig, Secret,
    Unsupported,
};
use serde_json::{Value, json};

use crate::harness::{
    ENDS, KEY, Lab, filled, quietly, read, read_filled, refusal, request, with_key,
};

/// A variable nothing sets.
const UNSET: &str = "LABLET_TEST_A_VARIABLE_NOTHING_SETS";

fn yaml(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

fn names(tools: &[lablet::ToolSpec]) -> Vec<&str> {
    tools.iter().map(|tool| tool.name.as_str()).collect()
}

/// The check of `config`, on the API's no-op `Otel` with the test's own
/// environment handed in.
async fn check(config: &Config) -> Result<lablet::Checked, BuildError> {
    quietly(config.clone()).check().await
}

/// The check stops before a provider is selected, so a config whose
/// provider has no adapter yet passes it.
#[tokio::test]
async fn a_config_whose_provider_has_no_adapter_yet_passes_the_check_and_fails_the_build() {
    for config in [
        with_key(yaml("prompt: { system: Hi. }")),
        yaml("model: { provider: openai, name: gpt-5 }\nprompt: { system: Hi. }"),
    ] {
        let checked = check(&config).await.unwrap();

        assert_eq!(checked.resolved(), &config.resolved());
        assert!(matches!(
            crate::harness::quiet(config).await,
            Err(BuildError::Unsupported {
                kind: Unsupported::Anthropic | Unsupported::Openai,
                ..
            })
        ));
    }
}

/// The display goes inside the thinking object a request sends, which
/// `provider_default` leaves out and `disabled` takes no display on, so
/// `check` and `build` alike refuse a display stated with either, by its
/// line and with its fix; and a budget under the API's floor the same way.
#[tokio::test]
async fn a_thinking_display_without_adaptive_thinking_or_a_budget_is_refused_by_check_and_build() {
    for thinking in ["provider_default", "disabled"] {
        let config = with_key(yaml(&format!(
            "model:\n  thinking: {thinking}\n  thinking_display: omitted\nprompt: {{ system: Hi. }}"
        )));

        let refused = refusal(config).await;

        assert_eq!(
            refused.to_string(),
            format!(
                "model.thinking_display (line 3): \"omitted\" is refused: a display goes with \
                 `adaptive` or a budget, and `model.thinking` is `{thinking}`; set \
                 `model.thinking` to `adaptive` or a budget"
            )
        );
    }
    let low = with_key(yaml(
        "model:\n  thinking: { budget: 512 }\n  thinking_display: summarized\nprompt: { system: Hi. }",
    ));
    assert_eq!(
        refusal(low).await.to_string(),
        "model.thinking (line 2): {\"budget\":512} is refused: a budget is at least 1,024 \
         tokens; raise it, or set `model.thinking` to `adaptive`"
    );

    let taken = with_key(yaml(
        "model:\n  thinking: adaptive\n  thinking_display: summarized\nprompt: { system: Hi. }",
    ));
    let checked = check(&taken).await.unwrap();
    assert_eq!(
        serde_json::to_value(checked.resolved()).unwrap()["model"]["thinking_display"],
        json!("summarized")
    );
}

/// C4: a key the provider needs that's left unset by its variable is
/// refused naming the variable, one that isn't written at all is refused
/// with its fix, and a provider that needs no key passes with the variable
/// unset.
#[tokio::test]
async fn the_check_refuses_a_key_the_provider_needs_that_is_unset_and_names_its_variable() {
    let anthropic = filled(
        &format!("model: {{ api_key: '${{{UNSET}}}' }}\nprompt: {{ system: Hi. }}"),
        Format::Yaml,
        &[],
    )
    .unwrap();

    let named = check(&anthropic).await.unwrap_err();
    assert_eq!(
        named.to_string(),
        format!(
            "model.api_key (line 1): \"${{{UNSET}}}\" is refused: `{UNSET}` isn't set, and the \
             provider `anthropic` needs a key; set it"
        )
    );
    assert_eq!(named.class(), ErrorClass::Config);

    let empty = filled(
        "model: { api_key: '${K}' }\nprompt: { system: Hi. }",
        Format::Yaml,
        &[("K", "")],
    )
    .unwrap();
    assert_eq!(
        check(&empty).await.unwrap_err().to_string(),
        "model.api_key (line 1): \"${K}\" is refused: `K` is empty, and the provider `anthropic` \
         needs a key; set it"
    );

    let unwritten = check(&yaml("prompt: { system: Hi. }")).await.unwrap_err();
    assert_eq!(
        unwritten.to_string(),
        "model.api_key isn't set: the provider `anthropic` needs a key; add api_key: \
         ${ANTHROPIC_API_KEY}"
    );

    let mut in_code = Config::default();
    in_code.prompt.system = Some("Hi.".to_owned());
    let refused = lablet::Lablet::builder(in_code, Otel::noop())
        .check()
        .await
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "model.api_key isn't set: the provider `anthropic` needs a key, and a library reads no \
         variable; set config.model.api_key = Some(Secret::new(key))"
    );

    let scratch = Lab::new("check-fake-key");
    let mut fake = scratch.tree(ENDS, json!({}));
    fake["model"]["api_key"] = json!(format!("${{{UNSET}}}"));
    let checked = check(&read_filled(&fake, &[])).await.unwrap();
    assert_eq!(checked.own().len(), 1, "{:?}", checked.own());
    assert_eq!(
        checked.own()[0].as_own(),
        format!("{UNSET} (line 1) (unset, not cut) -> model.api_key; no process inherits it")
    );
}

/// R17, R18 and R37: a credential written out is refused without being
/// shown, whatever provider the config selects, and a bare variable's name
/// is refused with the `${VAR}` to write.
#[tokio::test]
async fn a_credential_written_out_is_refused_and_never_shown() {
    let pasted = "sk-ant-api03-0123456789abcdef";
    let scratch = Lab::new("check-literal");
    for (text, key, fix) in [
        (
            format!("model: {{ api_key: '{pasted}' }}\nprompt: {{ system: Hi. }}"),
            "model.api_key",
            "write api_key: ${ANTHROPIC_API_KEY}, and rotate the key if this was one",
        ),
        (
            format!(
                "model: {{ provider: fake, script: run.yaml, api_key: '{pasted}' }}\nprompt: {{ \
                 system: Hi. }}"
            ),
            "model.api_key",
            "write api_key: ${ANTHROPIC_API_KEY}, and rotate the key if this was one",
        ),
        (
            format!(
                "model: {{ provider: fake, script: run.yaml }}\nprompt: {{ system: Hi. }}\ntools: \
                 {{ builtin: {{ root: {}, enabled: [bash], credentials: {{ GITHUB_TOKEN: \
                 '{pasted}' }} }} }}",
                scratch.root().display()
            ),
            "tools.builtin.credentials.GITHUB_TOKEN",
            "write GITHUB_TOKEN: ${GITHUB_TOKEN}, and rotate it if this was the token",
        ),
    ] {
        let config = filled(&text, Format::Yaml, &[]).unwrap();

        let error = check(&config).await.unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "{key} (line {}) is refused: it holds no ${{VAR}}, and what it holds isn't \
                 shown; {fix}",
                if key == "model.api_key" { 1 } else { 3 }
            ),
            "{text}"
        );
        for shown in [
            error.to_string(),
            format!("{error:?}"),
            format!("{config:?}"),
        ] {
            assert!(
                !shown.contains(pasted) && !shown.contains("sk-ant"),
                "{text}: {shown}"
            );
        }
        assert!(matches!(
            error,
            BuildError::Config(ConfigError::Refused { .. })
        ));
    }

    let named = filled(
        "model: { api_key: ANTHROPIC_API_KEY }\nprompt: { system: Hi. }",
        Format::Yaml,
        &[],
    )
    .unwrap();
    assert_eq!(
        check(&named).await.unwrap_err().to_string(),
        "model.api_key (line 1): \"ANTHROPIC_API_KEY\" is refused: it's a variable's name, not a \
         ${VAR}; write api_key: ${ANTHROPIC_API_KEY}"
    );

    // R19: a credential's default can only be empty.
    let defaulted = filled(
        "model: { api_key: '${K:-sk-default-0123456789}' }\nprompt: { system: Hi. }",
        Format::Yaml,
        &[],
    )
    .unwrap();
    let error = check(&defaulted).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "model.api_key (line 1) is refused: a credential's default can only be empty, and what \
         follows :- isn't shown; write ${K:-}"
    );
    assert!(!format!("{error:?}").contains("sk-default"));

    // A key set in code after the file was read is no literal.
    let mut set_after = yaml("prompt: { system: Hi. }");
    set_after.model.api_key = Some(Secret::new(pasted));
    let checked = check(&set_after).await.unwrap();
    assert_eq!(
        checked.own()[0].as_own(),
        "model.api_key (set in code); no process inherits it"
    );
}

/// C16 through a filled config: a variable's value that isn't a URL is
/// refused as the config writes it, and the value is nowhere.
#[tokio::test]
async fn a_base_url_a_variable_gives_is_refused_without_the_variable_s_value() {
    const HELD: &str = "internal-gateway-7f3a";
    let config = filled(
        "model: { provider: openai, base_url: '${GATEWAY}' }\nprompt: { system: Hi. }",
        Format::Yaml,
        &[("GATEWAY", HELD)],
    )
    .unwrap();

    let error = check(&config).await.unwrap_err();

    assert_eq!(
        error.to_string(),
        "model.base_url (line 1): \"${GATEWAY}\" is refused: a URL begins `http://` or \
         `https://` and names a host, as `http://localhost:11434/v1` does"
    );
    assert!(!format!("{error}{error:?}").contains(HELD), "{error:?}");
}

/// C10: an unset variable is a config error, by its key, when the host
/// fills the file; a prompt is never filled, so one there passes.
#[tokio::test]
async fn a_variable_that_is_not_set_fails_the_fill_by_the_key_it_is_in_and_a_prompt_is_text() {
    let scratch = Lab::new("check-unset");
    let mut tree = scratch.tree(ENDS, json!({}));
    tree["model"]["name"] = json!(format!("scripted-${{{UNSET}}}"));

    let error = filled(&tree.to_string(), Format::Json, &[]).unwrap_err();

    assert_eq!(
        error,
        ConfigError::Invalid {
            key: "model.name".to_owned(),
            place: Some(Place::Line(1)),
            value: Some(json!(format!("scripted-${{{UNSET}}}")).to_string()),
            reason: format!(
                "`{UNSET}` isn't set; set it, or give a default, as ${{{UNSET}:-default}}"
            ),
        }
    );

    let mut prompt = scratch.tree(ENDS, json!({}));
    prompt["prompt"]["system"] = json!(format!("You fix ${{{UNSET}}} tests."));
    let config = read_filled(&prompt, &[]);
    assert_eq!(
        config.prompt.system.as_deref(),
        Some(format!("You fix ${{{UNSET}}} tests.").as_str())
    );
    check(&config).await.unwrap();
}

/// R49: a `${VAR}` left in a config that no fill filled is refused when the
/// config is read, so no `Config` holds one.
#[test]
fn a_variable_left_unfilled_in_a_config_is_refused_when_the_config_is_read() {
    let error = Config::from_str(
        "model: { provider: fake, script: run.yaml, name: '${MODEL}' }",
        Format::Yaml,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "model.name (line 1): \"${MODEL}\" is refused: it was never filled; call \
         .substituted(lookup) on the RawConfig before .config()"
    );
    let filled = RawConfig::from_str(
        "model: { provider: fake, script: run.yaml, name: '${MODEL}' }",
        Format::Yaml,
    )
    .unwrap()
    .substituted(|name: &str| (name == "MODEL").then(|| "scripted-1".into()))
    .unwrap()
    .config()
    .unwrap();
    assert_eq!(filled.model.name, "scripted-1");
}

/// C3: an override is in the resolved config the check prints.
#[tokio::test]
async fn an_override_is_in_the_resolved_config_the_check_gives() {
    let scratch = Lab::new("check-override");
    let path = scratch.write("lablet.json", &scratch.tree(ENDS, json!({})).to_string());
    let mut raw = RawConfig::from_path(&path).unwrap();

    raw.set("run.max_turns=5").unwrap();
    let checked = check(&raw.config().unwrap()).await.unwrap();

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

    let checked = check(&config).await.unwrap();
    let lablet = crate::harness::quiet(config).await.unwrap();

    assert_eq!(names(checked.tools()), ["bash", "read_file"]);
    assert_eq!(lablet.tools(), checked.tools());
    lablet.shutdown().await;
}

/// R50: `bash` needs the environment its commands start from, and nothing
/// else does.
#[tokio::test]
async fn bash_with_no_process_environment_handed_in_is_refused() {
    let scratch = Lab::new("check-no-environment");
    let config = scratch.config(
        ENDS,
        json!({ "tools": { "builtin": scratch.builtin(&["read_file", "bash"]) } }),
    );

    let builder = lablet::Lablet::builder(config.clone(), Otel::noop());
    let refused = builder.check().await.unwrap_err();

    assert_eq!(
        refused.to_string(),
        "tools.builtin.enabled[0] (line 1): \"bash\" is refused: no process environment was \
         handed in; hand in what commands need, PATH among them, as \
         .with_process_environment([(\"PATH\", \"/usr/bin:/bin\")])"
    );
    assert_eq!(
        builder.build().await.map(drop).unwrap_err().to_string(),
        refused.to_string()
    );
    let files_only = scratch.config(
        ENDS,
        json!({ "tools": { "builtin": scratch.builtin(&["read_file"]) } }),
    );
    lablet::Lablet::builder(files_only, Otel::noop())
        .check()
        .await
        .unwrap();
}

/// An `env` entry whose filled value holds a credential's value is refused
/// by the check and the build alike, by the field whose value it holds, and
/// the value is nowhere in the error.
#[tokio::test]
async fn an_env_entry_whose_value_holds_a_credential_s_value_is_refused_by_the_check() {
    let scratch = Lab::new("check-env-value");
    let other = "LABLET_TEST_OTHER_HOLDS_THE_KEY";
    let mut tree = scratch.tree(
        ENDS,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    tree["model"]["api_key"] = json!("${LABLET_TEST_KEY}");
    tree["tools"]["builtin"]["env"] = json!({ "COPY": format!("${{{other}}}") });
    let config = read_filled(&tree, &[("LABLET_TEST_KEY", KEY), (other, KEY)]);

    let error = refusal(config).await;

    assert_eq!(
        error.to_string(),
        format!(
            "tools.builtin.env.COPY (line 1): \"${{{other}}}\" is refused: its value holds the \
             one `model.api_key` has, and no process inherits an own credential; to give bash the \
             key knowingly, list COPY under `tools.builtin.credentials`"
        )
    );
    assert!(
        matches!(error, BuildError::Config(ConfigError::Invalid { .. })),
        "{error:?}"
    );
    for shown in [error.to_string(), format!("{error:?}")] {
        assert!(!shown.contains(KEY), "{shown}");
    }
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
    assert_eq!(check(&told).await.unwrap_err(), expected);
    assert_eq!(crate::harness::quiet(told).await.unwrap_err(), expected);
    assert!(
        expected.to_string().ends_with(&format!(
            "it holds the task prompt's file, {}",
            inside.display()
        )),
        "{expected}"
    );

    let beside = scratch.write("task.md", "Fix the failing test.");
    let mut lablet = crate::harness::quiet(config().with_prompt_file(&beside))
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
        json!({
            "run": { "transcript_path": scratch.at("transcript.json") },
            "tools": { "builtin": { "root": scratch.at(""), "enabled": ["bash"] } },
        }),
    ] {
        refused.push(check(&read(&tree(more))).await.unwrap_err());
    }
    let mut anthropic = tree(json!({}));
    anthropic["model"] = json!({ "api_key": format!("${{{UNSET}}}") });
    refused.push(check(&read_filled(&anthropic, &[])).await.unwrap_err());

    for error in &refused {
        assert_eq!(error.class(), ErrorClass::Config, "{error:?}");
    }
    assert!(matches!(refused[0], BuildError::Config(_)));
    assert!(matches!(refused[1], BuildError::Unsupported { .. }));
    assert!(matches!(refused[2], BuildError::UnknownTool { .. }));
    assert!(matches!(refused[3], BuildError::RootHolds { .. }));
    assert!(matches!(
        refused[4],
        BuildError::Config(ConfigError::Invalid { .. })
    ));
    assert_eq!(
        BuildError::Tools {
            reason: "two tools share a name".to_owned()
        }
        .class(),
        ErrorClass::Config
    );
}

/// R51: a relative path in a config made from text is refused by its key,
/// since the config has no directory, and accepted once `relative_to` has
/// placed it at one.
#[tokio::test]
async fn a_relative_path_is_refused_until_relative_to_places_it() {
    let scratch = Lab::new("check-relative-path");
    scratch.write("prompts/sys.md", "You fix tests.");
    let text = format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system_file: prompts/sys.md }}",
        scratch.write("run.yaml", ENDS).display()
    );
    let config = yaml(&text);

    let refused = check(&config).await.unwrap_err();
    assert_eq!(
        refused.to_string(),
        "prompt.system_file (line 2): \"prompts/sys.md\" is refused: a config made in code or from \
         text has no directory; make the path absolute, or call .relative_to(dir)"
    );
    assert_eq!(
        crate::harness::quiet(config.clone())
            .await
            .unwrap_err()
            .to_string(),
        refused.to_string()
    );

    let placed = config.relative_to(scratch.path());
    let checked = check(&placed).await.unwrap();
    assert_eq!(
        serde_json::to_value(checked.resolved()).unwrap()["prompt"]["system_file"],
        json!("prompts/sys.md"),
        "the resolved form keeps the path as written"
    );
    let mut lablet = crate::harness::quiet(placed).await.unwrap();
    lablet.run(request()).await;
    lablet.shutdown().await;
}
