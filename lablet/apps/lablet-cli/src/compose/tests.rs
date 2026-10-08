//! What the command line's check and build read of where they run,
//! against an environment the test states, since a test can't set a
//! variable of its own process: every `OTEL_*` variable and the context
//! variables, and the SDK's settings reaching the providers a build makes.

use std::ffi::OsString;

use lablet_config::Format;
use lablet_run_request::RunRequest;

use super::*;

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

/// An environment that holds `value` under `name` and nothing else.
fn holding(name: &'static str, value: &'static str) -> impl Fn(&str) -> Option<OsString> {
    move |asked| (asked == name).then(|| value.into())
}

const ENDS: &str = "
- response:
    content:
      - text: Done.
    finish: end_turn
";

/// The `OTEL_*` variables and the context variables are the one
/// exception: read for every config, since the command line inherits them
/// and they configure its telemetry whatever the config names.
#[tokio::test]
async fn a_config_that_names_no_secret_reads_only_the_opentelemetry_variables() {
    let never = |name: &str| -> Option<OsString> {
        assert!(
            name.starts_with("OTEL_") || lablet_env_carrier::CONTEXT_VARIABLES.contains(&name),
            "the environment was read: {name}"
        );
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
}

/// The SDK's settings a build reads from its environment reach the
/// providers it makes: with the sampler `always_off`, a run exports no
/// span, and its wide event still.
#[tokio::test]
async fn the_sampler_the_environment_names_reaches_the_providers_a_build_makes() {
    let scratch = lablet_test_support::Scratch::new("build-sampler");
    let script = scratch.write("script.yaml", ENDS);
    let path = scratch.at("telemetry.otlp.jsonl");
    let text = format!(
        "
model: {{ provider: fake, script: '{}', name: scripted-1 }}
prompt: {{ system: 'You fix tests.' }}
telemetry: {{ file: {{ path: '{}' }}, otlp: {{ enabled: false }} }}
",
        script.display(),
        path.display()
    );
    let held = holding("OTEL_TRACES_SAMPLER", "always_off");

    let mut lablet = build_in(config(&text), &held).await.unwrap();
    lablet.run(RunRequest::new("Fix it.").unwrap()).await;
    lablet.shutdown().await;

    let exported = lablet_conformance::otlp::Exported::read(&path).unwrap();
    assert!(exported.spans.is_empty(), "{:?}", exported.spans);
    assert_eq!(
        exported
            .records_of(lablet_run::telemetry::generated::LabletRun::NAME)
            .len(),
        1
    );
}
