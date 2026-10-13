//! The resource every export carries is the config's, and only that: the
//! SDK's own reading of the process environment adds nothing to it, which a
//! child process whose environment says otherwise holds, since a test can't
//! set a variable of its own process.

use std::collections::BTreeSet;
use std::process::{Command, Stdio};

use lablet_conformance::otlp::Exported;
use lablet_test_support::Scratch;
use serde_json::json;

use super::harness::{CONTENT, RUN, Records, VERSION, WIDE, run, scope};
use crate::export::{FileTarget, Telemetry, resource};

/// Set in the environment of the child process the test below starts, to
/// the file the child exports its run to.
const CHILD: &str = "LABLET_TEST_RESOURCE_CHILD";

/// The child's side, which does nothing unless the test below started it:
/// one run, its content captured, exported to the file the parent named,
/// with the config's resource, `team: b` and a service name of its own.
#[tokio::test]
async fn a_child_process_exports_a_run_with_the_resource_of_its_config() {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test reads the variable its parent set, to know which side it's on"
    )]
    let Some(path) = std::env::var_os(CHILD) else {
        return;
    };
    let attributes = vec![
        ("team".to_owned(), "b".to_owned()),
        ("service.name".to_owned(), "configured-service".to_owned()),
    ];
    let telemetry = Telemetry::builder(scope())
        .resource(resource(VERSION, &attributes))
        .file(FileTarget::Path(path.into()))
        .build()
        .unwrap();

    run(&telemetry, RUN, Records::Captured).await.unwrap();
    telemetry.shutdown().await.unwrap();
}

#[test]
fn the_process_environments_resource_variables_add_nothing_to_the_resource() {
    let scratch = Scratch::new("resource-environment");
    let path = scratch.at("runs.otlp.jsonl");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "export::tests::resource::a_child_process_exports_a_run_with_the_resource_of_its_config",
            "--test-threads=1",
        ])
        .env(CHILD, &path)
        .env(
            "OTEL_RESOURCE_ATTRIBUTES",
            "leaked=yes,service.name=process-pairs,team=process",
        )
        .env("OTEL_SERVICE_NAME", "process-service")
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
            resource.get("service.name"),
            Some(&json!("configured-service")),
            "{resource:?}"
        );
        assert_eq!(resource.get("team"), Some(&json!("b")), "{resource:?}");
        assert_eq!(resource.get("service.version"), Some(&json!(VERSION)));
        let keys: BTreeSet<&str> = resource.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            BTreeSet::from([
                "service.name",
                "service.version",
                "team",
                "telemetry.sdk.language",
                "telemetry.sdk.name",
                "telemetry.sdk.version",
            ]),
            "nothing of the process's own"
        );
    }
}
