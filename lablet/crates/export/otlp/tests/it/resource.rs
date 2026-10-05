//! The resource every export carries, from a child process whose
//! environment states attributes of its own, since a test can't set a
//! variable of its own process: the environment's are defaults beneath
//! the composer's, and lablet's own keys are lablet's.

use std::collections::BTreeSet;
use std::process::{Command, Stdio};

use lablet_conformance::otlp::Exported;
use lablet_otlp::FileTarget;
use lablet_test_support::Scratch;
use serde_json::json;

use crate::harness::{CONTENT, RUN, Records, Settings, VERSION, WIDE, built, run};

/// Set in the environment of the child process the test below starts, to
/// the file the child exports its run to.
const CHILD: &str = "LABLET_TEST_RESOURCE_CHILD";

/// What the child's environment sets `OTEL_RESOURCE_ATTRIBUTES` to: a key
/// nothing else states, a key the composer states too, and a key of
/// lablet's own.
const ENVIRONMENT: &str = "deployment.environment=test,team=a,service.name=other";

/// The child's side, which does nothing unless the test below started it:
/// one run, its content captured, exported to the file the parent named,
/// with the composer stating `team: b`.
#[tokio::test]
async fn a_child_process_exports_a_run_with_the_resource_its_environment_and_its_composer_state() {
    let Some(path) = std::env::var_os(CHILD) else {
        return;
    };
    let scratch = Scratch::new("resource-child");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.into())),
        resource: vec![("team".to_owned(), "b".to_owned())],
        ..Settings::in_scratch(&scratch)
    });

    run(&telemetry, RUN, Records::Captured).await.unwrap();
    telemetry.shutdown().await.unwrap();
}

#[test]
fn the_environments_resource_attributes_are_defaults_beneath_the_composers_and_lablets_own() {
    let scratch = Scratch::new("resource-environment");
    let path = scratch.at("runs.otlp.jsonl");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "resource::a_child_process_exports_a_run_with_the_resource_its_environment_and_its_composer_state",
            "--test-threads=1",
        ])
        .env(CHILD, &path)
        .env("OTEL_RESOURCE_ATTRIBUTES", ENVIRONMENT)
        .env("OTEL_SERVICE_NAME", "other")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "the child failed:\n{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );

    let exported = Exported::read(&path).unwrap();
    assert_eq!(exported.spans_of("invoke_agent").len(), 1, "the run whole");
    assert_eq!(exported.records_of(WIDE).len(), 1, "the run whole");
    assert!(
        exported.records_of(CONTENT).len() > 1,
        "content records beside the wide event"
    );
    let resources = exported
        .spans
        .iter()
        .map(|span| &span.resource)
        .chain(exported.records.iter().map(|record| &record.resource));
    for resource in resources {
        assert_eq!(
            resource.get("deployment.environment"),
            Some(&json!("test")),
            "a key only the environment states is the environment's: {resource:?}"
        );
        assert_eq!(
            resource.get("team"),
            Some(&json!("b")),
            "a key the composer states too is the composer's: {resource:?}"
        );
        assert_eq!(
            resource.get("service.name"),
            Some(&json!("lablet")),
            "lablet's own key is lablet's, whatever `OTEL_RESOURCE_ATTRIBUTES` and \
             `OTEL_SERVICE_NAME` say: {resource:?}"
        );
        assert_eq!(resource.get("service.version"), Some(&json!(VERSION)));
        let keys: BTreeSet<&str> = resource.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            BTreeSet::from([
                "deployment.environment",
                "service.name",
                "service.version",
                "team",
                "telemetry.sdk.language",
                "telemetry.sdk.name",
                "telemetry.sdk.version",
            ]),
            "nothing else"
        );
    }
}
