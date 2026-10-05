//! The network exporter against the in-process receiver: what both
//! destinations hold of one run, what a closed port costs, what a collector
//! that accepts and never answers costs the flush and leaves the file, the
//! headers on the wire, which pins what the exporter does with the environment's and
//! what lablet does after it, and TLS to the collector with the trust roots
//! the environment names.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant, UNIX_EPOCH};

use lablet_conformance::otlp::Exported;
use lablet_conformance::receiver::{self, Mode, Received, Receiver};
use lablet_test_support::Scratch;
use opentelemetry::InstrumentationScope;
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry_otlp::{
    SpanExporter, WithExportConfig as _, WithHttpConfig as _, WithTonicConfig as _,
};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanExporter as _, SpanLinks};
use tonic::metadata::{MetadataMap, MetadataValue};

use super::harness::{
    CONTENT, CONTENT_PER_RUN, RUN, Records, SPANS_PER_RUN, Settings, VERSION, WIDE, built, queues,
    run, scope,
};
use crate::export::{
    FileTarget, OtelBuildError, OtlpSettings, Signal, Telemetry, Transport, validate,
};

/// The endpoint of `receiver` that `transport` speaks to.
fn endpoint_of(receiver: &Receiver, transport: Transport) -> String {
    match transport {
        Transport::Grpc => receiver.grpc_endpoint(),
        Transport::HttpProtobuf => receiver.http_endpoint(),
    }
}

fn settings(transport: Transport, endpoint: String, headers: &[(&str, &str)]) -> OtlpSettings {
    OtlpSettings {
        transport,
        endpoint: Some(endpoint),
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        strip_environment_headers: true,
    }
}

// O5 and O9: both destinations, content captured

async fn both_hold_the_same_run(transport: Transport, test: &str) {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new(test);
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.clone())),
        otlp: Some(settings(transport, endpoint_of(&receiver, transport), &[])),
        ..Settings::in_scratch(&scratch)
    });

    run(&telemetry, RUN, Records::Captured).await.unwrap();

    let file = Exported::read(&path).unwrap();
    let network = receiver.exported().unwrap();
    assert_eq!(file.spans.len(), SPANS_PER_RUN);
    assert_eq!(file.records_of(CONTENT).len(), CONTENT_PER_RUN);
    assert_eq!(file.records_of(WIDE).len(), 1);
    assert_eq!(
        file.records_of(CONTENT).len(),
        network.records_of(CONTENT).len(),
        "the content records reached both"
    );
    assert_eq!(
        file.ungrouped(),
        network.ungrouped(),
        "the same spans and records, whatever the batches"
    );
    telemetry.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_file_and_the_grpc_receiver_hold_the_same_spans_and_records() {
    both_hold_the_same_run(Transport::Grpc, "network-grpc").await;
}

#[tokio::test]
async fn the_file_and_the_http_receiver_hold_the_same_spans_and_records() {
    both_hold_the_same_run(Transport::HttpProtobuf, "network-http").await;
}

// O3: what a port nothing listens on costs

async fn a_closed_port_costs_under_five_seconds(transport: Transport, test: &str) {
    let closed = Receiver::closed();
    let scratch = Scratch::new(test);
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.clone())),
        otlp: Some(settings(transport, format!("http://{closed}"), &[])),
        ..Settings::in_scratch(&scratch)
    });

    let began = Instant::now();
    let flushed = run(&telemetry, RUN, Records::None).await;
    let shut = telemetry.shutdown().await;
    let cost = began.elapsed();

    let failures = flushed.unwrap_err();
    assert_eq!(
        queues(&failures),
        ["otlp spans", "the otlp wide event"],
        "the run has no log records, so that queue had nothing to fail: {failures}"
    );
    assert!(
        !failures.to_string().contains(&closed.to_string()),
        "{failures}"
    );
    assert_eq!(
        shut,
        Ok(()),
        "the flush reported every failure, and left nothing for the shutdown to"
    );
    assert!(
        cost < Duration::from_secs(5),
        "the flush and the shutdown cost {cost:?} together"
    );
    let file = Exported::read(&path).unwrap();
    assert_eq!(file.records_of(WIDE).len(), 1, "the file is whole");
    assert_eq!(file.spans.len(), SPANS_PER_RUN);
}

#[tokio::test]
async fn a_grpc_port_nothing_listens_on_costs_the_flush_and_the_shutdown_under_five_seconds() {
    a_closed_port_costs_under_five_seconds(Transport::Grpc, "network-closed-grpc").await;
}

#[tokio::test]
async fn an_http_port_nothing_listens_on_costs_the_flush_and_the_shutdown_under_five_seconds() {
    a_closed_port_costs_under_five_seconds(Transport::HttpProtobuf, "network-closed-http").await;
}

// O16: a collector that accepts and never answers, with the file beside it

/// How long the flush below is given, so that a collector that hangs costs
/// it that and not the SDK's seconds.
const FLUSH_BOUND: Duration = Duration::from_millis(500);

/// How long the shutdown below is given. The flush that gave up left its
/// thread waiting on the exporter, which the shutdown waits for, so it
/// gives up too.
const SHUTDOWN_BOUND: Duration = Duration::from_millis(700);

async fn a_receiver_that_never_answers_leaves_the_file_whole(transport: Transport, test: &str) {
    let receiver = Receiver::start(Mode::NeverAnswers).await;
    let scratch = Scratch::new(test);
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.clone())),
        otlp: Some(settings(transport, endpoint_of(&receiver, transport), &[])),
        flush_timeout: Some(FLUSH_BOUND),
        shutdown_timeout: Some(SHUTDOWN_BOUND),
        ..Settings::in_scratch(&scratch)
    });

    let began = Instant::now();
    let flushed = run(&telemetry, RUN, Records::Captured).await;
    let waited = began.elapsed();

    assert_eq!(
        flushed.unwrap_err().to_string(),
        "telemetry wasn't exported whole: otlp: the flush didn't end within 500ms",
        "the network destination alone is named, by its bound, and the file isn't"
    );
    assert!(waited >= FLUSH_BOUND, "the flush gave up after {waited:?}");
    // A flush that waited on the exporter, in place of the bound it was
    // given, costs at least the SDK's five seconds, which the tolerance
    // rules out.
    assert!(
        waited < FLUSH_BOUND + Duration::from_secs(1),
        "the flush waited {waited:?}, past the bound it was given"
    );
    // A port that refused the connection, or a listener never bound, would
    // hold the flush for its bound just the same: the receiver kept each
    // request before it hung, so what it kept says the connection was
    // accepted and the spans read.
    let over = match transport {
        Transport::Grpc => receiver::Transport::Grpc,
        Transport::HttpProtobuf => receiver::Transport::Http,
    };
    let requests = receiver.requests();
    assert!(
        requests
            .iter()
            .any(|request| request.transport == over && request.signal == receiver::Signal::Traces),
        "the receiver accepted no export of spans over {over:?}: {requests:?}"
    );
    let file = Exported::read(&path).unwrap();
    assert_eq!(file.spans.len(), SPANS_PER_RUN);
    assert_eq!(file.records_of(CONTENT).len(), CONTENT_PER_RUN);
    assert_eq!(file.records_of(WIDE).len(), 1);
    let last = file.records.last().unwrap();
    assert_eq!(
        last.event_name, WIDE,
        "the wide event is the file's last line"
    );
    assert_eq!(last.line, file.lines);
    assert!(
        receiver.exported().unwrap().records_of(WIDE).is_empty(),
        "the receiver that never answered holds no wide event"
    );

    // The shutdown waits for the thread the flush left, which the receiver
    // still holds, so it gives up by its own bound and nothing else.
    let began = Instant::now();
    let shut = telemetry.shutdown().await;
    let waited = began.elapsed();
    assert_eq!(
        shut.unwrap_err().failures(),
        ["the shutdown didn't end within 700ms"]
    );
    assert!(
        waited < SHUTDOWN_BOUND + Duration::from_secs(1),
        "the shutdown waited {waited:?}, past the bound it was given"
    );
}

#[tokio::test]
async fn a_grpc_receiver_that_never_answers_holds_the_flush_for_its_bound_and_the_file_is_whole() {
    a_receiver_that_never_answers_leaves_the_file_whole(Transport::Grpc, "network-hang-grpc").await;
}

#[tokio::test]
async fn an_http_receiver_that_never_answers_holds_the_flush_for_its_bound_and_the_file_is_whole() {
    a_receiver_that_never_answers_leaves_the_file_whole(
        Transport::HttpProtobuf,
        "network-hang-http",
    )
    .await;
}

// The headers on the wire, from a child process whose environment sets some

/// Set in the environment of the child process the test below starts, to
/// the receiver's two endpoints, gRPC then HTTP, with a comma between.
const CHILD: &str = "LABLET_TEST_OTLP_HEADERS_CHILD";

/// What the child's environment sets `OTEL_EXPORTER_OTLP_HEADERS` to: one
/// header the code also sets, and one the config also sets.
const ENVIRONMENT: &str = "x-plain=env%20value,x-lablet=env";

/// What the child's environment sets `OTEL_EXPORTER_OTLP_TRACES_HEADERS`
/// to: a header of the traces signal's own, which the code also sets, so
/// the wire says which variable the exporter read for each signal.
const TRACES_ENVIRONMENT: &str = "x-traces=t";

/// One span, exported by the exporter alone, with nothing of lablet's.
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
        name: "plain".into(),
        start_time: UNIX_EPOCH,
        end_time: UNIX_EPOCH,
        attributes: Vec::new(),
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope::builder("plain").build(),
    }
}

/// Exports one span through the exporter alone, with `x-plain: code` and
/// `x-traces: code` set as the exporter's own API sets a header, on a
/// thread with no reactor, as the SDK's batch threads are.
async fn exported_plainly(transport: Transport, endpoint: String) {
    let exporter = match transport {
        Transport::Grpc => {
            let mut metadata = MetadataMap::new();
            metadata.insert("x-plain", MetadataValue::from_static("code"));
            metadata.insert("x-traces", MetadataValue::from_static("code"));
            SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .with_metadata(metadata)
                .build()
                .unwrap()
        }
        Transport::HttpProtobuf => SpanExporter::builder()
            .with_http()
            .with_endpoint(format!("{endpoint}/v1/traces"))
            .with_headers(HashMap::from([
                ("x-plain".to_owned(), "code".to_owned()),
                ("x-traces".to_owned(), "code".to_owned()),
            ]))
            .build()
            .unwrap(),
    };
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(exporter.export(vec![one_span()])))
        .await
        .unwrap()
        .unwrap();
}

/// Runs one run of spans alone through lablet's telemetry, with the config
/// stating `x-lablet: config`, and the environment's headers stripped or
/// not, so each export is the spans and the wide event.
async fn exported_by_lablet(transport: Transport, endpoint: String, strip: bool, test: &str) {
    let scratch = Scratch::new(test);
    let telemetry = built(Settings {
        target: None,
        otlp: Some(OtlpSettings {
            strip_environment_headers: strip,
            ..settings(transport, endpoint, &[("x-lablet", "config")])
        }),
        ..Settings::in_scratch(&scratch)
    });
    run(&telemetry, RUN, Records::None).await.unwrap();
    telemetry.shutdown().await.unwrap();
}

/// The child's side, which does nothing unless the test below started it:
/// it exports to the receiver the parent started, three ways on each
/// transport, with the environment the parent gave it.
#[test]
fn a_child_process_exports_with_the_headers_its_environment_and_its_config_state() {
    let Some(endpoints) = std::env::var_os(CHILD) else {
        return;
    };
    let endpoints = endpoints.to_str().unwrap().to_owned();
    let (grpc, http) = endpoints.split_once(',').unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for (transport, endpoint) in [(Transport::Grpc, grpc), (Transport::HttpProtobuf, http)] {
            exported_plainly(transport, endpoint.to_owned()).await;
            exported_by_lablet(transport, endpoint.to_owned(), false, "headers-child-kept").await;
            exported_by_lablet(
                transport,
                endpoint.to_owned(),
                true,
                "headers-child-stripped",
            )
            .await;
        }
    });
}

/// The value of `name` among `request`'s headers, when it's there.
fn header<'a>(request: &'a Received, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(held, _)| held == name)
        .map(|(_, value)| value.as_str())
}

/// The requests that came in by `transport` and were sent `how`: by the
/// exporter alone, which sets `x-plain: code` and nothing else does; by
/// lablet keeping the environment's headers, which carry the signal's
/// `x-traces` or the generic `x-plain` beside the config's `x-lablet`; or
/// by lablet stripping them, which leaves the config's alone.
fn sent<'a>(
    requests: &'a [Received],
    transport: receiver::Transport,
    how: &str,
) -> Vec<&'a Received> {
    requests
        .iter()
        .filter(|request| request.transport == transport)
        .filter(|request| {
            let (plain, traces, lablet) = (
                header(request, "x-plain"),
                header(request, "x-traces"),
                header(request, "x-lablet"),
            );
            match how {
                "plainly" => plain == Some("code"),
                "keeping" => lablet == Some("config") && (plain.is_some() || traces.is_some()),
                "stripping" => lablet == Some("config") && plain.is_none() && traces.is_none(),
                _ => unreachable!(),
            }
        })
        .collect()
}

/// How many of `requests` carried `signal`.
fn of_signal(requests: &[&Received], signal: receiver::Signal) -> usize {
    requests
        .iter()
        .filter(|request| request.signal == signal)
        .count()
}

/// The exporter reads the signal's header variable in place of the generic
/// one, merges what it read over the headers its own API was given, name by
/// name, and percent-decodes them; lablet sets the config's after that
/// merge, so the config's win; and with the config's endpoint, what the
/// environment set for each signal is taken off that signal's exports, so
/// a header the generic variable names reaches no traces request even by
/// mistake.
#[tokio::test(flavor = "multi_thread")]
async fn the_environments_headers_win_in_the_exporter_and_the_configs_win_in_lablet() {
    let receiver = Receiver::start(Mode::Answers).await;
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "export::tests::network::a_child_process_exports_with_the_headers_its_environment_and_its_config_state",
            "--test-threads=1",
        ])
        .env(CHILD, format!("{},{}", receiver.grpc_endpoint(), receiver.http_endpoint()))
        .env("OTEL_EXPORTER_OTLP_HEADERS", ENVIRONMENT)
        .env("OTEL_EXPORTER_OTLP_TRACES_HEADERS", TRACES_ENVIRONMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .unwrap();
    assert!(
        child.status.success(),
        "the child failed:\n{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );

    let requests = receiver.requests();
    for transport in [receiver::Transport::Grpc, receiver::Transport::Http] {
        let plainly = sent(&requests, transport, "plainly");
        assert_eq!(plainly.len(), 1, "{transport:?}: {requests:?}");
        assert_eq!(plainly[0].signal, receiver::Signal::Traces);
        assert_eq!(
            header(plainly[0], "x-traces"),
            Some("t"),
            "{transport:?}: the signal's variable, in place of the code's value"
        );
        assert_eq!(
            header(plainly[0], "x-plain"),
            Some("code"),
            "{transport:?}: the generic variable is left unread once the signal's is set, so the \
             code's value stands"
        );
        assert_eq!(header(plainly[0], "x-lablet"), None, "{transport:?}");

        let keeping = sent(&requests, transport, "keeping");
        assert_eq!(
            keeping.len(),
            2,
            "{transport:?}: the spans and the wide event, each with the config's header; the run \
             has no other records: {requests:?}"
        );
        assert_eq!(of_signal(&keeping, receiver::Signal::Traces), 1);
        for request in keeping {
            assert_eq!(
                header(request, "x-lablet"),
                Some("config"),
                "{transport:?}: the config's wins the name both state"
            );
            match request.signal {
                receiver::Signal::Traces => {
                    assert_eq!(header(request, "x-traces"), Some("t"), "{transport:?}");
                    assert_eq!(
                        header(request, "x-plain"),
                        None,
                        "{transport:?}: the generic variable's header reaches no traces request"
                    );
                }
                receiver::Signal::Logs => {
                    assert_eq!(
                        header(request, "x-plain"),
                        Some("env value"),
                        "{transport:?}: the environment's value, decoded"
                    );
                    assert_eq!(header(request, "x-traces"), None, "{transport:?}");
                }
            }
        }

        let stripping = sent(&requests, transport, "stripping");
        assert_eq!(stripping.len(), 2, "{transport:?}: {requests:?}");
        assert_eq!(of_signal(&stripping, receiver::Signal::Traces), 1);
        for request in &stripping {
            assert!(
                request
                    .headers
                    .iter()
                    .all(|(name, _)| name != "x-plain" && name != "x-traces"),
                "{transport:?}: an environment header was sent to the config's endpoint: \
                 {request:?}"
            );
            assert_eq!(header(request, "x-lablet"), Some("config"), "{transport:?}");
        }
    }
    assert_eq!(requests.len(), 2 * (1 + 2 + 2));
}

// TLS to the collector, from a child process whose environment names the
// roots it trusts

/// Set in the environment of the child process the first test below
/// starts, to the endpoints it exports to over TLS, with a comma between.
const TLS_CHILD: &str = "LABLET_TEST_OTLP_TLS_CHILD";

/// Set in the environment of the child process the second test below
/// starts, to the endpoints whose exporters it makes, with a comma between,
/// each after `refused:` or `made:`, which is what becomes of it.
const NO_ROOTS_CHILD: &str = "LABLET_TEST_OTLP_NO_ROOTS_CHILD";

/// Runs the child test `name` with `variable` set to `endpoints`, with
/// `SSL_CERT_FILE` naming `roots` in place of the platform's, and with
/// `more` set, and holds it to passing as the one test run, since a name
/// that matches none runs nothing and passes.
async fn child(name: &str, variable: &str, endpoints: &str, roots: &Path, more: &[(&str, &str)]) {
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--test-threads=1"])
        .env(variable, endpoints)
        .env("SSL_CERT_FILE", roots)
        .env_remove("SSL_CERT_DIR")
        .envs(more.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert!(
        child.status.success(),
        "the child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(
        stdout.contains("test result: ok. 1 passed;"),
        "the child ran other than one test:\n{stdout}"
    );
}

/// The child's side of the first test below, which does nothing unless that
/// test started it: one run exported over gRPC to each endpoint, each of
/// which must take the whole run.
#[test]
fn a_child_process_exports_a_run_to_each_endpoint_over_tls() {
    let Some(endpoints) = std::env::var_os(TLS_CHILD) else {
        return;
    };
    let endpoints = endpoints.to_str().unwrap().to_owned();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for endpoint in endpoints.split(',') {
            exported_by_lablet(Transport::Grpc, endpoint.to_owned(), true, "tls-child").await;
        }
    });
}

/// The gRPC exporter speaks TLS to an `https` endpoint, and to one without a
/// scheme, which it gives `https://`, and trusts the roots the platform has
/// or, in their place, the ones `SSL_CERT_FILE` names: here the receiver's
/// CA alone.
#[tokio::test(flavor = "multi_thread")]
async fn the_grpc_exporter_trusts_the_roots_its_environment_names_with_a_scheme_or_without() {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new("network-tls");
    let roots = scratch.at("roots.pem");
    std::fs::write(&roots, receiver.ca_certificate()).unwrap();
    let https = receiver.grpc_tls_endpoint();
    let schemeless = https.trim_start_matches("https://");

    child(
        "export::tests::network::a_child_process_exports_a_run_to_each_endpoint_over_tls",
        TLS_CHILD,
        &format!("{https},{schemeless}"),
        &roots,
        &[],
    )
    .await;

    let requests = receiver.requests();
    assert_eq!(
        requests.len(),
        2 * 2,
        "the spans and the wide event of each run: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .all(|request| request.transport == receiver::Transport::GrpcTls),
        "{requests:?}"
    );
    let exported = receiver.exported().unwrap();
    assert_eq!(exported.spans.len(), 2 * SPANS_PER_RUN);
    assert_eq!(exported.records_of(WIDE).len(), 2);
}

/// The child's side of the second test below, which does nothing unless
/// that test started it: with no roots to trust, the check of each endpoint
/// takes it, since it loads none, and the build makes the exporters of one
/// that's spoken to without TLS and refuses the rest for TLS, without the
/// endpoint in the message.
#[test]
fn a_child_process_with_no_roots_to_trust_makes_no_exporter_that_speaks_tls() {
    let Some(endpoints) = std::env::var_os(NO_ROOTS_CHILD) else {
        return;
    };
    let endpoints = endpoints.to_str().unwrap().to_owned();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for entry in endpoints.split(',') {
            let (becomes, endpoint) = entry.split_once(':').unwrap();
            let settings = settings(Transport::Grpc, endpoint.to_owned(), &[]);
            assert_eq!(validate(&settings), Ok(()), "{endpoint}");

            let built = Telemetry::builder(VERSION, scope()).otlp(settings).build();

            if becomes == "made" {
                built.unwrap().shutdown().await.unwrap();
                continue;
            }
            let refused = built.err().unwrap();
            assert_eq!(
                refused,
                OtelBuildError::Tls {
                    signal: Signal::Traces,
                    reason: "transport error: no native certs found".to_owned(),
                },
                "{endpoint}"
            );
            assert!(!refused.to_string().contains("127.0.0.1"), "{refused}");
        }
    });
}

/// Trust roots that can't be loaded fail the build, as TLS rather than as
/// the endpoint, and a check, which makes no exporter, takes the endpoint.
/// An endpoint without a scheme is spoken to without TLS when the
/// environment says it's insecure, as the exporter reads it: the signal's
/// own variable before the generic one, `true` in any case. So no roots are
/// loaded for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_grpc_endpoint_over_tls_with_no_roots_to_trust_is_refused_for_tls_by_the_build_alone() {
    const NAME: &str = "export::tests::network::a_child_process_with_no_roots_to_trust_makes_no_exporter_that_speaks_tls";
    const INSECURE: &str = "OTEL_EXPORTER_OTLP_INSECURE";
    let scratch = Scratch::new("network-no-roots");
    let roots = scratch.at("roots.pem");
    std::fs::write(&roots, "").unwrap();
    let closed = Receiver::closed();

    let cases: [(&[(&str, &str)], String); 4] = [
        (&[], format!("refused:https://{closed},refused:{closed}")),
        (
            &[(INSECURE, "true")],
            format!("refused:https://{closed},made:{closed}"),
        ),
        (&[(INSECURE, "TRUE")], format!("made:{closed}")),
        (
            &[
                (INSECURE, "true"),
                ("OTEL_EXPORTER_OTLP_TRACES_INSECURE", "false"),
            ],
            format!("refused:{closed}"),
        ),
    ];
    for (more, endpoints) in cases {
        child(NAME, NO_ROOTS_CHILD, &endpoints, &roots, more).await;
    }
}
