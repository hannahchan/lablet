//! What the library root's check and build read of where they run,
//! against an environment the test states, since a test can't set a
//! variable of its own process: the names of lablet's secrets, `${VAR}`,
//! and which variables a library run reads at all.

use std::ffi::OsString;

use lablet_config::{ConfigError, Format, Place};
use lablet_prepare::ErrorClass;
use lablet_run::telemetry::generated::GenAiClientInferenceOperationDetails;
use lablet_run::telemetry::generated::key;

use super::*;

/// A key as long as a key is, and so one that's cut.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

/// An environment that holds `value` under `name` and nothing else.
fn holding(name: &'static str, value: &'static str) -> impl Fn(&str) -> Option<OsString> {
    move |asked| (asked == name).then(|| value.into())
}

fn nothing(_: &str) -> Option<OsString> {
    None
}

const ENDS: &str = "
- response:
    content:
      - text: Done.
    finish: end_turn
";

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

/// What a library run reads of the OpenTelemetry environment is the
/// capture variable and the variables whose values the secrets cut, the
/// OTLP header and endpoint variables: read for every config, since a
/// command inherits them, and never a variable only the SDK reads, nor a
/// context variable.
#[tokio::test]
async fn a_config_that_names_no_secret_reads_only_the_capture_header_and_endpoint_variables() {
    let read = std::sync::Mutex::new(Vec::new());
    let never = |name: &str| -> Option<OsString> {
        read.lock().unwrap().push(name.to_owned());
        None
    };
    let scratch = lablet_test_support::Scratch::new("check-no-secret");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}",
        script.display()
    );

    let checked = check_in(&config(&text), &never).await.unwrap();

    assert!(checked.withheld().is_empty());
    assert_eq!(checked.cut(), Vec::<String>::new());
    let mut read = read.into_inner().unwrap();
    read.sort();
    read.dedup();
    let mut expected: Vec<String> = lablet_otel_env::HEADER_VARIABLES
        .into_iter()
        .chain(lablet_otel_env::ENDPOINT_VARIABLES)
        .chain([lablet_otel_env::CAPTURE_CONTENT])
        .map(str::to_owned)
        .collect();
    expected.sort();
    assert_eq!(read, expected);
}

/// C19 at the library: a header variable the exporter reads is named among
/// what's cut and not among what's withheld, since every command inherits
/// it, and its value is nowhere.
#[tokio::test]
async fn the_check_names_an_otlp_header_variable_among_the_cut_and_withholds_it_from_no_command() {
    const TOKEN: &str = "otlp-0123456789abcdef";
    let scratch = lablet_test_support::Scratch::new("check-otlp-headers");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}",
        script.display()
    );
    let held = |name: &str| match name {
        "OTEL_EXPORTER_OTLP_TRACES_HEADERS" => {
            Some(format!("authorization=Bearer%20{TOKEN}").into())
        }
        _ => None,
    };

    let checked = check_in(&config(&text), &held).await.unwrap();

    assert!(checked.withheld().is_empty());
    assert_eq!(checked.cut(), ["OTEL_EXPORTER_OTLP_TRACES_HEADERS"]);
    assert!(!format!("{checked:?}").contains(TOKEN), "{checked:?}");
}

/// C19 at the library: an endpoint variable the exporter reads, holding
/// credentials, is named among what's cut and not among what's withheld,
/// and its password is nowhere.
#[tokio::test]
async fn the_check_names_an_otlp_endpoint_variable_with_credentials_among_the_cut_and_withholds_it_from_no_command()
 {
    const PASSWORD: &str = "otlp-pw-0123456789abcdef";
    let scratch = lablet_test_support::Scratch::new("check-otlp-endpoint");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}",
        script.display()
    );
    let held = |name: &str| match name {
        "OTEL_EXPORTER_OTLP_ENDPOINT" => {
            Some(format!("https://collector:{PASSWORD}@collector.internal:4317").into())
        }
        _ => None,
    };

    let checked = check_in(&config(&text), &held).await.unwrap();

    assert!(checked.withheld().is_empty());
    assert_eq!(checked.cut(), ["OTEL_EXPORTER_OTLP_ENDPOINT"]);
    assert!(!format!("{checked:?}").contains(PASSWORD), "{checked:?}");
}

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
telemetry: {{ capture_content: true }}
",
        script.display(),
    );
    let mut runs = Vec::new();

    for (model, language) in [("scripted-1", "Rust"), ("scripted-2", "Go")] {
        let held = |name: &str| match name {
            "MODEL" => Some(model.into()),
            "LANGUAGE" => Some(language.into()),
            _ => None,
        };
        let host = lablet_conformance::host::Host::new();
        let builder = Lablet::builder(config(&text), host.logger_provider())
            .with_tracer_provider(host.tracer_provider());
        let mut lablet = build_in(builder, &held).await.unwrap();
        let finished = lablet.run(crate::RunRequest::new("Fix it.").unwrap()).await;
        lablet.shutdown().await;
        runs.push((finished.summary.outcome.run_id, host.exported()));
    }

    // What each run was built with, as its own telemetry says: the digest
    // and the model on its root span, and the system prompt in the content
    // record of its first provider call.
    let written = config(&text).digest().to_string();
    let seen: Vec<(String, String, bool)> = runs
        .iter()
        .zip(["You fix Rust tests.", "You fix Go tests."])
        .map(|((run_id, exported), system)| {
            let of_run = |attributes: &lablet_conformance::otlp::Attributes| {
                attributes.get(key::GEN_AI_CONVERSATION_ID)
                    == Some(&serde_json::Value::String(run_id.to_string()))
            };
            let root = exported
                .spans_of(
                    lablet_run::telemetry::generated::LabletInvokeAgent::GEN_AI_OPERATION_NAME,
                )
                .into_iter()
                .find(|span| of_run(&span.attributes))
                .unwrap();
            let content = exported
                .records_of(GenAiClientInferenceOperationDetails::NAME)
                .into_iter()
                .find(|record| {
                    of_run(&record.attributes)
                        && record
                            .attributes
                            .contains_key(key::GEN_AI_SYSTEM_INSTRUCTIONS)
                })
                .unwrap();
            (
                root.attributes[key::LABLET_CONFIG_DIGEST]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                root.attributes[key::GEN_AI_REQUEST_MODEL]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                content.attributes[key::GEN_AI_SYSTEM_INSTRUCTIONS]
                    .as_str()
                    .unwrap()
                    .contains(system),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (written.clone(), "scripted-1".to_owned(), true),
            (written, "scripted-2".to_owned(), true),
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
        serde_saphyr::from_str(include_str!("../../../shared/config/src/tests/spec.yaml")).unwrap();
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
