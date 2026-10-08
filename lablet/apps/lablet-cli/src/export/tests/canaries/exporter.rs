//! The canaries of the gaps lablet fills in the OTLP exporter's reading
//! of its environment.
//!
//! Each canary is a pair: a parent test that starts this test binary again
//! with the environment the gap is about, and the child's side, which does
//! nothing unless [`CHILD`] names it.

use std::process::Stdio;
use std::time::UNIX_EPOCH;

use lablet_conformance::receiver::{self, Mode, Received, Receiver};
use opentelemetry::InstrumentationScope;
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState, Tracer as _,
};
use opentelemetry_otlp::{SpanExporter, WithExportConfig as _};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanExporter as _, SpanLinks};

use crate::export::Telemetry;
use crate::export::testing::memory::Memory;
use crate::export::testing::{RUN, Records, emit_run, otlp_of, scope};

/// Set in the environment of a child process to the canary it runs, and
/// what that canary needs, after a `:`.
const CHILD: &str = "LABLET_TEST_EXPORTER_CANARY";

/// What the child the parent started was given, when it's the child of
/// `canary`.
fn given(canary: &str) -> Option<String> {
    let given = std::env::var(CHILD).ok()?;
    let (named, rest) = given.split_once(':')?;
    (named == canary).then(|| rest.to_owned())
}

/// Runs the child test `test` of this file with `canary` and `rest` in
/// [`CHILD`] and the variables `set`, and holds it to passing as the one
/// test run, since a name that matches none runs nothing and passes.
async fn child(test: &str, canary: &str, rest: &str, set: &[(&str, &str)]) {
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("export::tests::canaries::exporter::{test}"),
            "--test-threads=1",
        ])
        .env(CHILD, format!("{canary}:{rest}"))
        .envs(set.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("test result: ok. 1 passed;"),
        "the child ran other than one test:\n{stdout}"
    );
}

/// One span, exported by the crate's exporter alone, with nothing of
/// lablet's.
fn one_span() -> SpanData {
    SpanData {
        span_context: SpanContext::new(
            TraceId::from_bytes([1; 16]),
            SpanId::from_bytes([2; 8]),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id: SpanId::INVALID,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: "canary".into(),
        start_time: UNIX_EPOCH,
        end_time: UNIX_EPOCH,
        attributes: Vec::new(),
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope::builder("canary").build(),
    }
}

/// The crate's own exporter of spans to `endpoint`, over gRPC or HTTP, as
/// the crate's builder makes it from what it reads of the process
/// environment.
fn plain_exporter(grpc: bool, endpoint: &str) -> SpanExporter {
    let built = if grpc {
        SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()
    } else {
        SpanExporter::builder()
            .with_http()
            .with_endpoint(format!("{endpoint}/v1/traces"))
            .build()
    };
    built.unwrap()
}

/// What exporting [`one_span`] through `exporter` comes to, on a thread
/// with no reactor, as the SDK's batch threads are.
async fn exported_plainly(exporter: SpanExporter) -> Result<(), String> {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(exporter.export(vec![one_span()])))
        .await
        .unwrap()
        .map_err(|error| error.to_string())
}

/// The value of `name` among `request`'s headers, when it's there.
fn header<'a>(request: &'a Received, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(held, _)| held == name)
        .map(|(_, value)| value.as_str())
}

// OTEL_SDK_DISABLED

#[tokio::test]
async fn child_builds_a_provider_as_lablet_builds_it() {
    if given("sdk-disabled").is_none() {
        return;
    }
    let memory = Memory::default();
    let telemetry = Telemetry::builder(scope())
        .exporting_to(memory.spans(), memory.records())
        .build()
        .unwrap();

    let tracer = telemetry.tracer();
    tracer.in_span("canary", |_| {});
    telemetry.flush_providers().await.unwrap();

    assert_eq!(
        memory.exported_spans().len(),
        1,
        "the SDK reads `OTEL_SDK_DISABLED`: delete lablet's own reading of it if the SDK's \
         means what the specification's no-op SDK does, or say in decisions.md why it stays"
    );
}

/// With `OTEL_SDK_DISABLED=true`, a processor on a provider built as lablet
/// builds it still sees a span: the SDK doesn't read the variable, so
/// lablet does.
#[tokio::test(flavor = "multi_thread")]
async fn canary_the_sdk_does_not_read_otel_sdk_disabled() {
    child(
        "child_builds_a_provider_as_lablet_builds_it",
        "sdk-disabled",
        "",
        &[("OTEL_SDK_DISABLED", "true")],
    )
    .await;
}

// OTEL_TRACES_EXPORTER

#[tokio::test]
async fn child_exports_a_run_through_lablets_builder_chain() {
    let Some(endpoints) = given("traces-exporter") else {
        return;
    };
    let (grpc, http) = endpoints.split_once(',').unwrap();
    for (protocol, endpoint) in [("grpc", grpc), ("http/protobuf", http)] {
        let settings = otlp_of(
            &format!("telemetry: {{ otlp: {{ protocol: {protocol}, endpoint: '{endpoint}' }} }}"),
            &[],
        );
        let telemetry = Telemetry::builder(scope()).otlp(settings).build().unwrap();
        let wide = emit_run(&telemetry, RUN, Records::None);
        telemetry.flush(wide).await.unwrap();
        telemetry.shutdown().await.unwrap();
    }
}

/// With `OTEL_TRACES_EXPORTER=none`, lablet's builder chain still exports
/// spans: the crates don't read the variable, so lablet does.
#[tokio::test(flavor = "multi_thread")]
async fn canary_the_builders_do_not_read_otel_traces_exporter() {
    let receiver = Receiver::start(Mode::Answers).await;

    child(
        "child_exports_a_run_through_lablets_builder_chain",
        "traces-exporter",
        &format!("{},{}", receiver.grpc_endpoint(), receiver.http_endpoint()),
        &[("OTEL_TRACES_EXPORTER", "none")],
    )
    .await;

    let requests = receiver.requests();
    for transport in [receiver::Transport::Grpc, receiver::Transport::Http] {
        assert!(
            requests.iter().any(|request| request.transport == transport
                && request.signal == receiver::Signal::Traces),
            "{transport:?}: the crates read `OTEL_TRACES_EXPORTER`: delete lablet's own reading \
             of it if theirs means what the specification's does, or say in decisions.md why it \
             stays: {requests:?}"
        );
    }
}

// The empty per-signal headers variable

#[tokio::test]
async fn child_exports_a_span_through_the_crates_exporters() {
    let Some(endpoints) = given("headers") else {
        return;
    };
    let (grpc, http) = endpoints.split_once(',').unwrap();
    exported_plainly(plain_exporter(true, grpc)).await.unwrap();
    exported_plainly(plain_exporter(false, http)).await.unwrap();
}

/// With the per-signal headers variable empty and the generic one set, the
/// crate sends no header: it reads an empty variable as set, where the
/// specification reads it as unset. Lablet takes off every name either
/// variable sets and sets the resolved set itself.
#[tokio::test(flavor = "multi_thread")]
async fn canary_an_empty_signal_header_variable_hides_the_generic_one_in_the_crate() {
    let receiver = Receiver::start(Mode::Answers).await;

    child(
        "child_exports_a_span_through_the_crates_exporters",
        "headers",
        &format!("{},{}", receiver.grpc_endpoint(), receiver.http_endpoint()),
        &[
            ("OTEL_EXPORTER_OTLP_HEADERS", "x-generic=g"),
            ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", ""),
        ],
    )
    .await;

    let requests = receiver.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    for request in &requests {
        assert_eq!(
            header(request, "x-generic"),
            None,
            "the crate reads an empty variable as unset: lablet's taking off of every name \
             either variable sets still holds, but say in decisions.md that the gap is closed: \
             {request:?}"
        );
    }
}

// The empty per-signal insecure variable

#[tokio::test]
async fn child_exports_a_span_to_an_endpoint_without_a_scheme() {
    let Some(given) = given("insecure") else {
        return;
    };
    let (expected, endpoint) = given.split_once(',').unwrap();

    let exported = exported_plainly(plain_exporter(true, endpoint)).await;

    match expected {
        "https" => assert!(
            exported.is_err(),
            "the crate spoke plain gRPC to an endpoint without a scheme, so it reads an empty \
             insecure variable as unset: lablet states every endpoint with its scheme, but say \
             in decisions.md that the gap is closed"
        ),
        _ => exported.unwrap(),
    }
}

/// With the per-signal insecure variable empty and the generic one `true`,
/// the crate's own scheme for an endpoint without one is `https`: it reads
/// an empty variable as set to what isn't `true`. Lablet gives every
/// endpoint its scheme, so the crate's rule never applies. The child's
/// export to the receiver's plain listener fails over TLS, and with the
/// per-signal variable unset it succeeds, which says the listener takes
/// plain gRPC.
#[tokio::test(flavor = "multi_thread")]
async fn canary_an_empty_signal_insecure_variable_hides_the_generic_one_in_the_crate() {
    let receiver = Receiver::start(Mode::Answers).await;
    let schemeless = receiver
        .grpc_endpoint()
        .trim_start_matches("http://")
        .to_owned();

    child(
        "child_exports_a_span_to_an_endpoint_without_a_scheme",
        "insecure",
        &format!("https,{schemeless}"),
        &[
            ("OTEL_EXPORTER_OTLP_INSECURE", "true"),
            ("OTEL_EXPORTER_OTLP_TRACES_INSECURE", ""),
        ],
    )
    .await;
    assert!(receiver.requests().is_empty());
    child(
        "child_exports_a_span_to_an_endpoint_without_a_scheme",
        "insecure",
        &format!("http,{schemeless}"),
        &[("OTEL_EXPORTER_OTLP_INSECURE", "true")],
    )
    .await;
    assert_eq!(receiver.requests().len(), 1);
}

// The certificate variables

#[tokio::test]
async fn child_builds_the_crates_exporters_for_endpoints_over_tls() {
    if given("certificates").is_none() {
        return;
    }
    plain_exporter(true, "https://collector.internal:4317");
    plain_exporter(false, "https://collector.internal:4318");
}

/// With the certificate and client identity variables naming files that
/// don't exist, the crate's own build succeeds: it doesn't read them.
/// Lablet reads them, and gives the exporters TLS of its own.
#[tokio::test(flavor = "multi_thread")]
async fn canary_the_crate_does_not_read_the_certificate_variables() {
    let missing = "/nonexistent/lablet-canary.pem";

    child(
        "child_builds_the_crates_exporters_for_endpoints_over_tls",
        "certificates",
        "",
        &[
            ("OTEL_EXPORTER_OTLP_CERTIFICATE", missing),
            ("OTEL_EXPORTER_OTLP_CLIENT_KEY", missing),
            ("OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE", missing),
            ("OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE", missing),
        ],
    )
    .await;
}
