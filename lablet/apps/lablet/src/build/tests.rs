//! What lablet reads of where it runs, against an environment the test
//! states, since a test can't set a variable of its own process: the check
//! of the key's variable, the names of lablet's secrets, and `${VAR}`.

use super::*;

/// What stands for a key that was written where the variable's name
/// belongs. It reads as a name, so only the environment refuses it.
const PASTED: &str = "sk_live_0123456789abcdef";

/// A key as long as a key is, and so one that's cut.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

fn model(text: &str) -> Config {
    config(text)
}

/// Whether the key's variable is set, for the config as it runs and as
/// it's written, which here are one.
fn key_set(config: &Config, held: impl Fn(&str) -> Option<OsString>) -> Result<(), BuildError> {
    key_is_set(&config.model, config, &held)
}

/// An environment that holds `value` under `name` and nothing else.
fn holding(name: &'static str, value: &'static str) -> impl Fn(&str) -> Option<OsString> {
    move |asked| (asked == name).then(|| value.into())
}

fn nothing(_: &str) -> Option<OsString> {
    None
}

fn refusal(model: &Config, held: impl Fn(&str) -> Option<OsString>) -> String {
    let error = key_set(model, held).unwrap_err();
    assert!(matches!(error, BuildError::KeyVariable { .. }), "{error:?}");
    assert!(!format!("{error:?}").contains(PASTED), "{error:?}");
    error.to_string()
}

#[test]
fn a_variable_that_holds_a_key_is_taken() {
    let named = model(&format!("model: {{ api_key_env: {PASTED} }}"));

    assert_eq!(key_set(&named, holding(PASTED, "a key")), Ok(()));
    assert_eq!(
        key_set(&model(""), holding("ANTHROPIC_API_KEY", "a key")),
        Ok(())
    );
}

#[test]
fn a_variable_that_is_not_set_is_refused_and_a_name_not_written_in_capitals_is_not_shown() {
    for named in [PASTED, "work_key", "Work_Key", "ANTHROPIC_API_KEy"] {
        let config = model(&format!("model: {{ api_key_env: {named} }}"));

        let shown = refusal(&config, holding("ANTHROPIC_API_KEY", "a key"));

        assert_eq!(
            shown,
            "model.api_key_env (line 1) is refused: the variable it names isn't set, and the provider \
             `anthropic` needs a key",
            "{named}"
        );
        assert!(!shown.contains(named), "{named}: {shown}");
    }
}

#[test]
fn a_variable_that_is_empty_is_refused_and_a_name_not_written_in_capitals_is_not_shown() {
    let named = model(&format!("model: {{ api_key_env: {PASTED} }}"));

    assert_eq!(
        refusal(&named, holding(PASTED, "")),
        "model.api_key_env (line 1) is refused: the variable it names is empty, and the provider \
         `anthropic` needs a key"
    );
}

#[test]
fn a_variable_the_config_names_in_capitals_is_named_when_it_is_not_set_or_empty() {
    for named in ["ANTHROPIC_API_KEY", "WORK_KEY_2", "_K"] {
        let config = model(&format!("model: {{ api_key_env: {named} }}"));

        assert_eq!(
            refusal(&config, nothing),
            format!(
                "model.api_key_env (line 1) is refused: `{named}`, the variable it names, isn't set, \
                 and the provider `anthropic` needs a key"
            )
        );
    }
    let empty = model("model: { api_key_env: WORK_KEY }");
    assert_eq!(
        refusal(&empty, holding("WORK_KEY", "")),
        "model.api_key_env (line 1) is refused: `WORK_KEY`, the variable it names, is empty, and the \
         provider `anthropic` needs a key"
    );
}

#[test]
fn the_variable_that_is_read_when_the_config_names_none_is_shown() {
    assert_eq!(
        refusal(&model(""), nothing),
        "model.api_key_env is refused: `ANTHROPIC_API_KEY`, the variable that's read when \
         the config names none, isn't set, and the provider `anthropic` needs a key"
    );
    assert_eq!(
        refusal(&model(""), holding("ANTHROPIC_API_KEY", "")),
        "model.api_key_env is refused: `ANTHROPIC_API_KEY`, the variable that's read when \
         the config names none, is empty, and the provider `anthropic` needs a key"
    );
}

#[test]
fn a_provider_that_needs_no_key_has_no_variable_read() {
    let never = |_: &str| -> Option<OsString> { panic!("the environment was read") };

    for provider in ["openai", "fake"] {
        let named = model(&format!(
            "model: {{ provider: {provider}, api_key_env: {PASTED} }}"
        ));

        assert_eq!(key_set(&named, never), Ok(()), "{provider}");
    }
}

/// T16: the check names the variables a run withholds and the names whose
/// values it cuts, with a note on a value that isn't, and never a value.
#[tokio::test]
async fn the_check_names_what_a_run_withholds_and_cuts_and_never_a_value() {
    let scratch = lablet_test_support::Scratch::new("check-names");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}', api_key_env: WORK_KEY }}\n\
         prompt: {{ system: Hi. }}\n\
         telemetry: {{ otlp: {{ headers: {{ X-Short: '${{SHORT}}' }} }} }}",
        script.display()
    );
    let held = |name: &str| match name {
        "WORK_KEY" => Some(KEY.into()),
        "SHORT" => Some("short".into()),
        _ => None,
    };

    let checked = check_in(&config(&text), &held).await.unwrap();

    assert_eq!(
        checked.withheld(),
        &["SHORT".to_owned(), "WORK_KEY".to_owned()].into()
    );
    assert_eq!(
        checked.cut(),
        ["SHORT (under 16 bytes, not cut)", "WORK_KEY"]
    );
    assert!(!format!("{checked:?}").contains(KEY), "{checked:?}");
}

#[tokio::test]
async fn a_config_that_names_no_secret_reads_nothing_of_the_environment() {
    let never = |_: &str| -> Option<OsString> { panic!("the environment was read") };
    let scratch = lablet_test_support::Scratch::new("check-no-secret");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}",
        script.display()
    );

    let checked = check_in(&config(&text), &never).await.unwrap();

    assert!(checked.withheld().is_empty());
    assert_eq!(checked.cut(), Vec::<String>::new());
}

/// Every event of a run's start that tells what the run was built with.
#[derive(Default)]
struct Started {
    seen: std::sync::Mutex<Vec<(String, String, Option<String>)>>,
}

#[async_trait::async_trait]
impl RunObserver for Started {
    async fn on(&self, event: lablet_run::RunEvent) {
        if let lablet_run::EventKind::RunStarted {
            context,
            model,
            system_prompt,
            ..
        } = event.kind
        {
            self.seen.lock().unwrap().push((
                context.config_digest.to_string(),
                model.name,
                system_prompt,
            ));
        }
    }
}

const ENDS: &str = "
- response:
    content:
      - text: Done.
    finish: end_turn
";

/// C5: the digest a run carries is of the config as it's written, so two
/// runs whose variables hold different values share it, while each runs
/// with what its variable held.
#[tokio::test]
async fn two_runs_whose_variables_differ_share_a_config_digest_and_run_with_their_values() {
    let scratch = lablet_test_support::Scratch::new("digest-before-substitution");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "
model: {{ provider: fake, script: '{}', name: '${{MODEL}}' }}
prompt: {{ system: 'You fix ${{LANGUAGE}} tests.' }}
telemetry: {{ capture_content: true, file: {{ path: '{}' }} }}
",
        script.display(),
        scratch.at("telemetry.otlp.jsonl").display()
    );
    let started = Arc::new(Started::default());

    for (model, language) in [("scripted-1", "Rust"), ("scripted-2", "Go")] {
        let held = |name: &str| match name {
            "MODEL" => Some(model.into()),
            "LANGUAGE" => Some(language.into()),
            _ => None,
        };
        let observers = vec![Arc::clone(&started) as Arc<dyn RunObserver>];
        let mut lablet = build_in(config(&text), observers, &held).await.unwrap();
        lablet.run(crate::RunRequest::new("Fix it.").unwrap()).await;
        lablet.shutdown().await;
    }

    let seen = started.seen.lock().unwrap().clone();
    let written = config(&text).digest().to_string();
    assert_eq!(
        seen,
        [
            (
                written.clone(),
                "scripted-1".to_owned(),
                Some("You fix Rust tests.".to_owned())
            ),
            (
                written,
                "scripted-2".to_owned(),
                Some("You fix Go tests.".to_owned())
            ),
        ]
    );
}

/// C16: a value a variable gave is refused as the config writes it, and
/// nothing of what the variable holds is in the message.
#[tokio::test]
async fn a_value_a_variable_gave_is_refused_as_it_is_written() {
    const HELD: &str = "internal-gateway-7f3a";
    let text = "model: { provider: openai, base_url: '${GATEWAY}' }\nprompt: { system: Hi. }";
    let held = holding("GATEWAY", HELD);

    let error = check_in(&config(text), &held).await.unwrap_err();

    assert_eq!(
        error,
        BuildError::Config(ConfigError::Invalid {
            key: "model.base_url".to_owned(),
            place: Some(Place::Line(1)),
            value: Some("\"${GATEWAY}\"".to_owned()),
            reason: "a URL begins `http://` or `https://` and names a host, as \
                     `http://localhost:11434/v1` does"
                .to_owned(),
        })
    );
    for shown in [error.to_string(), format!("{error:?}")] {
        assert!(!shown.contains(HELD), "{shown}");
    }
    assert_eq!(error.class(), ErrorClass::Config);
    assert_eq!(
        check_in(
            &config(text),
            &holding("GATEWAY", "http://localhost:4000/v1")
        )
        .await
        .map(drop),
        Ok(()),
        "a URL passes, and the provider that has no adapter yet isn't selected"
    );
}

/// C10: a variable a config names that isn't set is a config error.
#[tokio::test]
async fn a_variable_that_is_not_set_is_refused_by_the_key_it_is_in() {
    let error = check_in(
        &config("model: { provider: openai, name: '${MODEL}' }\nprompt: { system: Hi. }"),
        &nothing,
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "model.name (line 1): \"${MODEL}\" is refused: the variable `MODEL` isn't set"
    );
}

/// C13: a setting the provider can't apply is refused by the check, and
/// the config without it passes, with the resolved config leaving out what
/// the provider can't apply.
#[tokio::test]
async fn the_check_refuses_a_setting_the_provider_cannot_apply_and_resolves_the_rest() {
    let key = holding("ANTHROPIC_API_KEY", "a key");
    for (text, key_name, reached) in [
        (
            "model: { seed: 7 }\nprompt: { system: Hi. }",
            "model.seed",
            "the provider `anthropic`",
        ),
        (
            "model: { provider: openai, base_url: 'http://localhost:11434/v1', thinking: adaptive }\n\
             prompt: { system: Hi. }",
            "model.thinking",
            "the provider `openai` over its API `chat_completions`",
        ),
    ] {
        let error = check_in(&config(text), &key).await.unwrap_err();
        assert!(
            matches!(
                &error,
                BuildError::Config(ConfigError::NotApplied { key, reached: said, .. })
                    if *key == key_name && said == reached
            ),
            "{error:?}"
        );
        assert!(error.to_string().contains(key_name), "{error}");
        assert!(error.to_string().contains(reached), "{error}");
    }

    for (text, left_out) in [
        ("prompt: { system: Hi. }", "seed"),
        (
            "model: { provider: openai, base_url: 'http://localhost:11434/v1' }\n\
             prompt: { system: Hi. }",
            "thinking",
        ),
    ] {
        let checked = check_in(&config(text), &key).await.unwrap();
        let resolved = serde_json::to_value(checked.resolved()).unwrap();

        assert!(resolved["model"].get(left_out).is_none(), "{resolved}");
        assert_eq!(checked.resolved(), &config(text).resolved());
    }
}

/// C17: a config that states only what it must is checked into the
/// resolved config of the spec's own block, which states every default.
#[tokio::test]
async fn a_config_that_states_only_what_it_must_resolves_to_every_default_of_the_spec() {
    let mut spec: serde_json::Value =
        serde_saphyr::from_str(include_str!("../config/tests/spec.yaml")).unwrap();
    // The spec's block lists two servers as examples, which no default has.
    spec["tools"]["mcp"] = serde_json::json!([]);
    let spec = Config::from_str(&spec.to_string(), Format::Json).unwrap();

    let checked = check_in(
        &config("prompt: { system: You are ... }"),
        &holding("ANTHROPIC_API_KEY", "a key"),
    )
    .await
    .unwrap();

    assert_eq!(checked.resolved(), &spec.resolved());
    assert_eq!(checked.tools(), []);
}

/// C15: whether the telemetry is on standard error is read from the path
/// as a run writes to it, once `${VAR}` is substituted, and only a path
/// that's `-` whole is standard error.
#[test]
fn the_telemetry_is_on_standard_error_when_its_path_is_a_dash_once_variables_are_substituted() {
    let on_stderr = |path: &str, env: Env<'_>| {
        let config = config(&format!("telemetry: {{ file: {{ path: {path} }} }}"));
        telemetry_on_stderr_in(&config, env)
    };

    assert!(on_stderr(r#""-""#, &nothing));
    assert!(on_stderr("'${T}'", &holding("T", "-")));

    assert!(!on_stderr("'${T}'", &holding("T", "telemetry.jsonl")));
    assert!(!on_stderr("'${T}'", &nothing), "a build refuses it");
    assert!(!on_stderr(r#""-/""#, &nothing), "it names a file");
    assert!(!on_stderr("'${T}/'", &holding("T", "-")), "it names a file");
    assert!(!on_stderr(r#""-.jsonl""#, &nothing));
    assert!(!on_stderr("null", &nothing));
}
