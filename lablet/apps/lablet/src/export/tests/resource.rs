//! The resource every export carries is the one the environment lablet is
//! given says, and only that: the SDK's own reading of the process
//! environment adds nothing to it, which a child process whose environment
//! says otherwise holds, since a test can't set a variable of its own
//! process.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::process::{Command, Stdio};

use lablet_conformance::otlp::Exported;
use lablet_test_support::Scratch;
use serde_json::json;

use super::harness::{CONTENT, RUN, Records, VERSION, WIDE, run, scope};
use crate::export::{FileTarget, Telemetry, resource};
use crate::otel_env::OtelEnv;

/// Set in the environment of the child process the test below starts, to
/// the file the child exports its run to.
const CHILD: &str = "LABLET_TEST_RESOURCE_CHILD";

/// What the environment the child's lablet is given holds: a service name,
/// and pairs whose values are escaped, one of them a key the composer
/// states too.
const GIVEN: [(&str, &str); 2] = [
    ("OTEL_SERVICE_NAME", "given-service"),
    (
        "OTEL_RESOURCE_ATTRIBUTES",
        "deployment.environment=given%2Cdecoded,team=a",
    ),
];

/// The child's side, which does nothing unless the test below started it:
/// one run, its content captured, exported to the file the parent named,
/// with the resource of [`GIVEN`] and the composer stating `team: b`.
#[tokio::test]
async fn a_child_process_exports_a_run_with_the_resource_of_the_environment_it_is_given() {
    let Some(path) = std::env::var_os(CHILD) else {
        return;
    };
    let env = |name: &str| {
        GIVEN
            .iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| OsString::from(value))
    };
    let context = OtelEnv::read(&env).context;
    let telemetry = Telemetry::builder(scope())
        .resource(resource(
            VERSION,
            vec![("team".to_owned(), "b".to_owned())],
            &context,
        ))
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
            "export::tests::resource::a_child_process_exports_a_run_with_the_resource_of_the_environment_it_is_given",
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
            Some(&json!("given-service")),
            "{resource:?}"
        );
        assert_eq!(
            resource.get("deployment.environment"),
            Some(&json!("given,decoded")),
            "{resource:?}"
        );
        assert_eq!(resource.get("team"), Some(&json!("b")), "{resource:?}");
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
            "nothing of the process's own"
        );
    }
}
