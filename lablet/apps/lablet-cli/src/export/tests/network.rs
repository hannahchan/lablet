//! The network exporters against the in-process receiver, each built from
//! settings a config resolves to: what both destinations hold of one run,
//! what a closed port costs, what a collector that accepts and never
//! answers costs a run's end and leaves the file, each protocol and
//! endpoint, TLS to the collector with the trust roots and the client's
//! identity the config names, and the headers on the wire, which pins what
//! the exporter merges from the process environment and what lablet does
//! after it.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use lablet_test_support::Scratch;
use lablet_test_support::conformance::otlp::Exported;
use lablet_test_support::conformance::receiver::{self, Mode, Received, Receiver};

use super::harness::{
    CONTENT, CONTENT_PER_RUN, RUN, Records, SPANS_PER_RUN, Settings, WIDE, built, emit_run,
    otlp_of, providers, run, scope,
};
use crate::export::network::EXPORTER_AGENT;
use crate::export::network::validate;
use crate::export::{FileTarget, OtelBuildError, OtlpSettings, Signal, Telemetry, exporters};

/// The endpoint of `receiver` that `protocol` speaks to, as a config
/// states it.
fn endpoint_of(receiver: &Receiver, protocol: &str) -> String {
    match protocol {
        "grpc" => receiver.grpc_endpoint(),
        _ => receiver.http_endpoint(),
    }
}

/// The config that sends to `endpoint` over `protocol`, stating
/// `headers` when they're given.
fn to(protocol: &str, endpoint: &str, headers: Option<&str>) -> String {
    let headers = headers.map_or_else(String::new, |headers| format!(", headers: {headers}"));
    format!("telemetry: {{ otlp: {{ protocol: {protocol}, endpoint: '{endpoint}'{headers} }} }}")
}

/// The config that sends to `endpoint` over `protocol` with the `tls`
/// section `tls`.
fn to_with_tls(protocol: &str, endpoint: &str, tls: &str) -> String {
    format!(
        "telemetry: {{ otlp: {{ protocol: {protocol}, endpoint: '{endpoint}', tls: {{ {tls} }} }} }}"
    )
}

/// The settings that send to `endpoint` over `protocol`.
fn sending(protocol: &str, endpoint: &str) -> OtlpSettings {
    otlp_of(&to(protocol, endpoint, None))
}

/// The listener a request of `protocol` comes in on, without TLS.
fn listener_of(protocol: &str) -> receiver::Transport {
    match protocol {
        "grpc" => receiver::Transport::Grpc,
        _ => receiver::Transport::Http,
    }
}

/// The value of `name` among `request`'s headers, when it's there.
fn header<'a>(request: &'a Received, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(held, _)| held == name)
        .map(|(_, value)| value.as_str())
}

// O5 and O9: both destinations, content captured

async fn both_hold_the_same_run(protocol: &str, test: &str) {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new(test);
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.clone())),
        otlp: Some(sending(protocol, &endpoint_of(&receiver, protocol))),
        ..Settings::in_scratch(&scratch)
    });

    run(&telemetry, RUN, Records::Captured).await.unwrap();

    let file = Exported::read(&path).unwrap();
    let network = receiver.exported().unwrap();
    assert_eq!(file.spans.len(), SPANS_PER_RUN);
    assert_eq!(file.records_of(CONTENT).len(), CONTENT_PER_RUN);
    assert_eq!(file.records_of(WIDE).len(), 1);
    assert_eq!(
        file.ungrouped(),
        network.ungrouped(),
        "the same spans and records, whatever the batches"
    );
    let requests = receiver.requests();
    assert!(
        requests
            .iter()
            .all(|request| request.transport == listener_of(protocol)),
        "{requests:?}"
    );
    telemetry.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_file_and_the_grpc_receiver_hold_the_same_spans_and_records() {
    both_hold_the_same_run("grpc", "network-grpc").await;
}

#[tokio::test]
async fn the_file_and_the_http_protobuf_receiver_hold_the_same_spans_and_records() {
    both_hold_the_same_run("http/protobuf", "network-http").await;
}

#[tokio::test]
async fn the_file_and_the_http_json_receiver_hold_the_same_spans_and_records() {
    both_hold_the_same_run("http/json", "network-json").await;
}

#[tokio::test]
async fn http_protobuf_is_sent_as_protobuf_and_http_json_as_json() {
    for (protocol, content_type) in [
        ("http/protobuf", "application/x-protobuf"),
        ("http/json", "application/json"),
    ] {
        let receiver = Receiver::start(Mode::Answers).await;
        let scratch = Scratch::new("network-content-type");
        let telemetry = built(Settings {
            target: None,
            otlp: Some(sending(protocol, &receiver.http_endpoint())),
            ..Settings::in_scratch(&scratch)
        });

        run(&telemetry, RUN, Records::None).await.unwrap();
        telemetry.shutdown().await.unwrap();

        let requests = receiver.requests();
        assert!(!requests.is_empty(), "{protocol}");
        for request in &requests {
            assert_eq!(
                header(request, "content-type"),
                Some(content_type),
                "{protocol}"
            );
        }
    }
}

// O3: what a port nothing listens on costs

async fn a_run_end_with_a_port_that_refuses_costs_seconds_and_changes_nothing(
    protocol: &str,
    test: &str,
) {
    let closed = Receiver::closed();
    let scratch = Scratch::new(test);
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.clone())),
        otlp: Some(sending(protocol, &format!("http://{closed}"))),
        ..Settings::in_scratch(&scratch)
    });

    let began = Instant::now();
    let flushed = run(&telemetry, RUN, Records::None).await;
    let shut = telemetry.shutdown().await;
    let cost = began.elapsed();

    let failures = flushed.unwrap_err();
    assert_eq!(
        providers(&failures),
        ["spans", "log records"],
        "the run's only log record is its wide event, which failed with the spans: {failures}"
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
async fn a_run_end_with_a_grpc_port_that_refuses_costs_seconds_and_changes_nothing() {
    a_run_end_with_a_port_that_refuses_costs_seconds_and_changes_nothing(
        "grpc",
        "network-closed-grpc",
    )
    .await;
}

#[tokio::test]
async fn a_run_end_with_an_http_port_that_refuses_costs_seconds_and_changes_nothing() {
    a_run_end_with_a_port_that_refuses_costs_seconds_and_changes_nothing(
        "http/protobuf",
        "network-closed-http",
    )
    .await;
}

// O16: a collector that accepts and never answers, with the file beside it

/// How long the SDK gives a processor's flush, and a processor's shutdown.
const SDK_BOUND: Duration = Duration::from_secs(5);

/// What a test allows beyond a bound, for the threads and the runtime.
const TOLERANCE: Duration = Duration::from_millis(1_500);

/// The run's end flushes the file's processors before the network's, and
/// the two providers side by side, so the file is whole while the flush
/// still waits on the network, and it waits for one bound of the SDK's,
/// once.
async fn a_receiver_that_never_answers_holds_the_run_end_under_one_flush_bound(
    protocol: &str,
    test: &str,
) {
    let receiver = Receiver::start(Mode::NeverAnswers).await;
    let scratch = Scratch::new(test);
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.clone())),
        otlp: Some(sending(protocol, &endpoint_of(&receiver, protocol))),
        ..Settings::in_scratch(&scratch)
    });
    let wide = emit_run(&telemetry, RUN, Records::Captured);

    let began = Instant::now();
    let flushing = tokio::spawn({
        let telemetry = telemetry.clone();
        async move { telemetry.flush(wide).await }
    });
    let whole = loop {
        let whole = Exported::read(&path).is_ok_and(|read| {
            read.spans.len() == SPANS_PER_RUN && read.records_of(WIDE).len() == 1
        });
        if whole || began.elapsed() > Duration::from_secs(1) {
            break whole;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(
        whole && !flushing.is_finished(),
        "the file wasn't whole within a second, while the flush waited on the network"
    );
    let flushed = flushing.await.unwrap();
    let waited = began.elapsed();

    let failures = flushed.unwrap_err();
    assert_eq!(providers(&failures), ["spans", "log records"], "{failures}");
    assert!(
        waited < SDK_BOUND + TOLERANCE,
        "the run's end waited {waited:?}, past one flush bound"
    );
    // A port that refused the connection, or a listener never bound, would
    // fail at once: the receiver kept each request before it hung, so what
    // it kept says the connection was accepted and the spans read.
    let requests = receiver.requests();
    assert!(
        requests
            .iter()
            .any(|request| request.transport == listener_of(protocol)
                && request.signal == receiver::Signal::Traces),
        "the receiver accepted no export of spans over {protocol}: {requests:?}"
    );
    let file = Exported::read(&path).unwrap();
    assert_eq!(file.spans.len(), SPANS_PER_RUN);
    assert_eq!(file.records_of(CONTENT).len(), CONTENT_PER_RUN);
    assert_eq!(
        file.records_of(WIDE).len(),
        1,
        "the file holds the wide event"
    );

    // The exports the flush gave up on still hold the processors' threads,
    // so the shutdown gives up on them by the SDK's bound.
    let began = Instant::now();
    let _ = telemetry.shutdown().await;
    let waited = began.elapsed();
    assert!(
        waited < SDK_BOUND + TOLERANCE,
        "the shutdown waited {waited:?}, past its bound"
    );
}

#[tokio::test]
async fn a_grpc_receiver_that_never_answers_holds_the_run_end_under_one_flush_bound_and_the_file_is_whole()
 {
    a_receiver_that_never_answers_holds_the_run_end_under_one_flush_bound(
        "grpc",
        "network-hang-grpc",
    )
    .await;
}

#[tokio::test]
async fn an_http_receiver_that_never_answers_holds_the_run_end_under_one_flush_bound_and_the_file_is_whole()
 {
    a_receiver_that_never_answers_holds_the_run_end_under_one_flush_bound(
        "http/protobuf",
        "network-hang-http",
    )
    .await;
}

// TLS, from the config

/// One run of spans and its wide event, exported to `settings` alone.
async fn exported_to(settings: OtlpSettings, test: &str) -> Result<(), String> {
    let scratch = Scratch::new(test);
    let telemetry = built(Settings {
        target: None,
        otlp: Some(settings),
        ..Settings::in_scratch(&scratch)
    });
    let flushed = run(&telemetry, RUN, Records::None).await;
    let _ = telemetry.shutdown().await;
    flushed.map_err(|error| error.to_string())
}

/// The receiver's CA, its client's certificate and key, each in a file of
/// `scratch`.
struct Material {
    ca: String,
    certificate: String,
    key: String,
}

impl Material {
    fn of(receiver: &Receiver, scratch: &Scratch) -> Self {
        let file = |name: &str, text: &str| scratch.write(name, text).display().to_string();
        Self {
            ca: file("ca.pem", receiver.ca_certificate()),
            certificate: file("client.pem", receiver.client_certificate()),
            key: file("client.key", receiver.client_key()),
        }
    }

    /// The `tls` section that trusts the receiver's CA.
    fn trusting(&self) -> String {
        format!("ca_file: '{}'", self.ca)
    }

    /// The `tls` section that trusts the receiver's CA and shows the
    /// client's certificate.
    fn identified(&self) -> String {
        format!(
            "ca_file: '{}', client_cert: '{}', client_key: '{}'",
            self.ca, self.certificate, self.key
        )
    }
}

/// The receiver's CA is in no platform's store, and `tls.ca_file` alone
/// names it, so a run that reaches the listeners that speak TLS trusted the
/// certificate the config names.
#[tokio::test]
async fn a_private_ca_the_config_names_is_the_one_trusted() {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new("network-private-ca");
    let material = Material::of(&receiver, &scratch);

    for (protocol, endpoint, listener) in [
        (
            "grpc",
            receiver.grpc_tls_endpoint(),
            receiver::Transport::GrpcTls,
        ),
        (
            "http/protobuf",
            receiver.https_endpoint(),
            receiver::Transport::Https,
        ),
    ] {
        let trusting = otlp_of(&to_with_tls(protocol, &endpoint, &material.trusting()));
        let platform = otlp_of(&to(protocol, &endpoint, None));

        let trusted = exported_to(trusting, "network-private-ca").await;
        let refused = exported_to(platform, "network-private-ca").await;

        trusted.unwrap();
        refused.unwrap_err();
        let requests = receiver.requests();
        assert!(
            requests.iter().any(|request| request.transport == listener),
            "{protocol}: {requests:?}"
        );
    }
}

/// A collector that asks for a client certificate takes the run only with
/// `tls.client_cert` and `tls.client_key` stated: TLS to the collector end
/// to end, on both transports.
#[tokio::test]
async fn a_receiver_that_asks_for_a_client_certificate_gets_the_one_the_config_names() {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new("network-client-tls");
    let material = Material::of(&receiver, &scratch);

    for (protocol, endpoint, listener) in [
        (
            "grpc",
            receiver.grpc_client_tls_endpoint(),
            receiver::Transport::GrpcClientTls,
        ),
        (
            "http/protobuf",
            receiver.https_client_tls_endpoint(),
            receiver::Transport::HttpsClientTls,
        ),
    ] {
        let before = receiver.requests().len();

        let anonymous = exported_to(
            otlp_of(&to_with_tls(protocol, &endpoint, &material.trusting())),
            "network-client-tls",
        )
        .await;
        let after_anonymous = receiver.requests().len();
        let shown = exported_to(
            otlp_of(&to_with_tls(protocol, &endpoint, &material.identified())),
            "network-client-tls",
        )
        .await;

        anonymous.unwrap_err();
        assert_eq!(
            after_anonymous, before,
            "{protocol}: taken without a certificate"
        );
        shown.unwrap();
        let requests = receiver.requests();
        assert!(
            requests[after_anonymous..]
                .iter()
                .all(|request| request.transport == listener),
            "{protocol}: {requests:?}"
        );
        assert!(requests.len() > after_anonymous, "{protocol}");
    }
}

/// A collector whose address drops the connection's first packet holds a
/// connect for as long as the platform retries it, a minute or more, and
/// nothing on this host can stand in for one, so the bound is read off the
/// endpoint the channel is made from: the specification's ten seconds, for
/// both signals.
#[test]
fn a_grpc_connection_is_bounded_by_the_specifications_timeout() {
    let settings = otlp_of(&to("grpc", "https://localhost:4317", None));

    let bounds: Vec<_> = settings
        .destinations()
        .map(|(signal, destination)| {
            let endpoint = crate::export::network::endpoint(signal, destination).unwrap();
            (signal, endpoint.get_connect_timeout())
        })
        .collect();

    assert_eq!(
        bounds,
        [
            (Signal::Traces, Some(Duration::from_secs(10))),
            (Signal::Logs, Some(Duration::from_secs(10))),
        ]
    );
}

/// tonic bounds a request by the timeout but connects a lazy channel
/// before the request, so a handshake with a peer that holds the socket
/// and never answers isn't bounded by it unless lablet states it there
/// too. The timeout is shortened here, since the config has no field for
/// it and ten seconds would outlast the bound the test holds.
#[tokio::test]
async fn a_grpc_tls_handshake_that_never_ends_fails_the_export_by_its_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let silent = listener.local_addr().unwrap();
    let holding = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new("network-stalled-handshake");
    let material = Material::of(&receiver, &scratch);
    let mut settings = otlp_of(&to_with_tls(
        "grpc",
        &format!("https://{silent}"),
        &material.trusting(),
    ));
    for destination in [&mut settings.traces, &mut settings.logs]
        .into_iter()
        .flatten()
    {
        destination.timeout = Duration::from_millis(500);
    }

    let began = Instant::now();
    let exported = exported_to(settings, "network-stalled-handshake").await;
    let waited = began.elapsed();
    holding.abort();

    exported.unwrap_err();
    assert!(
        waited < SDK_BOUND,
        "the export and the shutdown took {waited:?}, so the handshake waited past its timeout"
    );
}

// The headers on the wire, from a child process whose environment sets some

/// Set in the environment of the child process the test below starts, to
/// the receiver's two endpoints, gRPC then HTTP, with a comma between.
const CHILD: &str = "LABLET_TEST_OTLP_HEADERS_CHILD";

/// What the child's environment sets `OTEL_EXPORTER_OTLP_HEADERS` to.
const GENERIC: &str = "x-generic=env%20value,authorization=Bearer%20generic";

/// What the child's environment sets `OTEL_EXPORTER_OTLP_LOGS_HEADERS` to.
/// `OTEL_EXPORTER_OTLP_TRACES_HEADERS` is set to nothing.
const LOGS: &str = "x-logs=l";

/// The child's side, which does nothing unless the test below started it:
/// it exports to the receiver the parent started, with no headers, with
/// the config's map and with the config's text, on each transport, under
/// the environment the parent gave it.
#[test]
fn a_child_process_exports_with_the_headers_its_config_states() {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test reads the variable its parent set, to know which side it's on"
    )]
    let Some(endpoints) = std::env::var_os(CHILD) else {
        return;
    };
    let endpoints = endpoints.to_str().unwrap().to_owned();
    let (grpc, http) = endpoints.split_once(',').unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for (protocol, endpoint) in [("grpc", grpc), ("http/protobuf", http)] {
            for headers in [
                None,
                Some("{ x-lablet: config }"),
                Some("'x-lablet=config,x-text=t%20t'"),
            ] {
                exported_to(otlp_of(&to(protocol, endpoint, headers)), "headers-child")
                    .await
                    .unwrap();
            }
        }
    });
}

/// The headers on the wire are the config's alone, in either form: the
/// environment's header variables, which the exporter merges on its own,
/// reach no request, whether the config states headers or none.
#[tokio::test(flavor = "multi_thread")]
async fn the_headers_on_the_wire_are_the_configs_alone_whatever_the_environment_sets() {
    let receiver = Receiver::start(Mode::Answers).await;
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "export::tests::network::a_child_process_exports_with_the_headers_its_config_states",
            "--test-threads=1",
        ])
        .env(
            CHILD,
            format!("{},{}", receiver.grpc_endpoint(), receiver.http_endpoint()),
        )
        .env("OTEL_EXPORTER_OTLP_HEADERS", GENERIC)
        .env("OTEL_EXPORTER_OTLP_TRACES_HEADERS", "")
        .env("OTEL_EXPORTER_OTLP_LOGS_HEADERS", LOGS)
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
        let of: Vec<&Received> = requests
            .iter()
            .filter(|request| request.transport == transport)
            .collect();
        let (none, configs): (Vec<&Received>, Vec<&Received>) = of
            .into_iter()
            .partition(|request| header(request, "x-lablet").is_none());
        assert_eq!(none.len(), 2, "{transport:?}: {requests:?}");
        assert_eq!(configs.len(), 4, "{transport:?}: {requests:?}");
        for request in none.iter().chain(&configs) {
            for name in ["x-generic", "authorization", "x-logs"] {
                assert_eq!(
                    header(request, name),
                    None,
                    "{transport:?}: an environment header reached a request: {request:?}"
                );
            }
        }
        for request in configs {
            assert_eq!(header(request, "x-lablet"), Some("config"), "{transport:?}");
        }
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.transport == transport
                    && header(request, "x-text") == Some("t t"))
                .count(),
            2,
            "the text form's second header, decoded, on both signals: {requests:?}"
        );
    }
    assert_eq!(requests.len(), 2 * 6);
}

/// The exporter's own headers are its own on the wire whatever the
/// environment's header variables set them to: the environment's value
/// replaced the exporter's, so on HTTP the body's type is the protocol's
/// and its encoding the one the body is compressed in, uncompressed or
/// under the compression variable the exporter reads itself, and the user
/// agent is the pinned exporter's on both transports, whether the
/// environment's is another or one that begins as the exporter's does. A
/// header of another name in the generic variable, with neither signal's
/// variable set, so the crate merges it into both signals, reaches no
/// request either.
#[tokio::test(flavor = "multi_thread")]
async fn the_exporters_own_headers_keep_its_values_whatever_the_environment_sets_them_to() {
    for (compression, named, encoding) in [
        (None, "gzip", None),
        (Some("gzip"), "identity", Some("gzip")),
    ] {
        for agent in ["hostile-agent", "OTel-OTLP-Exporter-Rust/anything"] {
            let receiver = Receiver::start(Mode::Answers).await;
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "export::tests::network::a_child_process_exports_with_the_headers_its_config_states",
                    "--test-threads=1",
                ])
                .env(
                    CHILD,
                    format!("{},{}", receiver.grpc_endpoint(), receiver.http_endpoint()),
                )
                .env(
                    "OTEL_EXPORTER_OTLP_HEADERS",
                    format!(
                        "content-type=text/plain,user-agent={agent},content-encoding={named},\
                         x-generic=g"
                    ),
                )
                .env_remove("OTEL_EXPORTER_OTLP_TRACES_HEADERS")
                .env_remove("OTEL_EXPORTER_OTLP_LOGS_HEADERS");
            if let Some(compression) = compression {
                command.env("OTEL_EXPORTER_OTLP_COMPRESSION", compression);
            }
            let child = command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .unwrap();
            assert!(
                child.status.success(),
                "the child failed, so a collector refused what it was sent:\n{}\n{}",
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            );

            let requests = receiver.requests();
            assert_eq!(requests.len(), 2 * 6, "{requests:?}");
            for transport in [receiver::Transport::Grpc, receiver::Transport::Http] {
                assert!(
                    requests
                        .iter()
                        .any(|request| request.transport == transport),
                    "{transport:?}: {requests:?}"
                );
            }
            for request in &requests {
                assert!(
                    request
                        .headers
                        .iter()
                        .all(|(_, value)| !value.contains(agent) && value != "text/plain"),
                    "{request:?}"
                );
                assert_eq!(header(request, "x-generic"), None, "{request:?}");
                let sent = header(request, "user-agent").unwrap_or_default();
                if request.transport == receiver::Transport::Http {
                    assert_eq!(sent, EXPORTER_AGENT, "{request:?}");
                    assert_eq!(
                        header(request, "content-type"),
                        Some("application/x-protobuf"),
                        "{request:?}"
                    );
                    assert_eq!(header(request, "content-encoding"), encoding, "{request:?}");
                } else {
                    assert!(sent.starts_with(EXPORTER_AGENT), "{request:?}");
                }
            }
        }
    }
}

// TLS to the collector, from a child process whose environment names the
// roots it trusts

/// Set in the environment of the child process the first test below
/// starts, to the endpoints it exports to over TLS, with a comma between.
const TLS_CHILD: &str = "LABLET_TEST_OTLP_TLS_CHILD";

/// Set in the environment of the child process the second test below
/// starts, to the cases it makes the exporters of, with a comma between:
/// each `refused` or `made`, which is what becomes of it, and an endpoint.
const NO_ROOTS_CHILD: &str = "LABLET_TEST_OTLP_NO_ROOTS_CHILD";

/// Runs the child test `name` with `variable` set to `value`, and with
/// `SSL_CERT_FILE` naming `roots` in place of the platform's, and holds it
/// to passing as the one test run, since a name that matches none runs
/// nothing and passes.
async fn child(name: &str, variable: &str, value: &str, roots: &Path) {
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--test-threads=1"])
        .env(variable, value)
        .env("SSL_CERT_FILE", roots)
        .env_remove("SSL_CERT_DIR")
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
    #[expect(
        clippy::disallowed_methods,
        reason = "a test reads the variable its parent set, to know which side it's on"
    )]
    let Some(endpoints) = std::env::var_os(TLS_CHILD) else {
        return;
    };
    let endpoints = endpoints.to_str().unwrap().to_owned();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for endpoint in endpoints.split(',') {
            exported_to(sending("grpc", endpoint), "tls-child")
                .await
                .unwrap();
        }
    });
}

/// With no `tls.ca_file`, the gRPC exporter speaks TLS to an `https`
/// endpoint, and to one without a scheme, which it gives `https://`, and
/// trusts the roots the platform has or, in their place, the ones
/// `SSL_CERT_FILE` names, a process convention the crates read: here the
/// receiver's CA alone.
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
/// that test started it: with no roots to trust, validating each endpoint
/// takes it, since validating loads none, while making its exporters, which
/// the check does as the build does, makes those of one that's spoken to
/// without TLS and refuses the rest for TLS, without the endpoint in the
/// message; and the command line's check of a config at each endpoint
/// refuses what the build refuses, under the endpoint as the file wrote it.
#[test]
fn a_child_process_with_no_roots_to_trust_makes_no_exporter_that_speaks_tls() {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test reads the variable its parent set, to know which side it's on"
    )]
    let Some(cases) = std::env::var_os(NO_ROOTS_CHILD) else {
        return;
    };
    let cases = cases.to_str().unwrap().to_owned();
    let scratch = Scratch::new("network-no-roots-child");
    let script = scratch.write(
        "script.yaml",
        "- response:\n    content:\n      - text: Done.\n    finish: end_turn\n",
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for case in cases.split(',') {
            let (becomes, endpoint) = case.split_once(' ').unwrap();
            let settings = otlp_of(&to("grpc", endpoint, None));
            assert_eq!(validate(&settings), Ok(()), "{case}");
            let config = lablet_config::Config::from_str(
                &format!(
                    "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}\n{}",
                    script.display(),
                    to("grpc", endpoint, None)
                ),
                lablet_config::Format::Yaml,
            )
            .unwrap();

            let made = exporters(&settings);
            let built = Telemetry::builder(scope()).otlp(settings).build();
            let checked = crate::compose::check(&config, Vec::new()).await.map(drop);

            if becomes == "made" {
                made.unwrap();
                built.unwrap().shutdown().await.unwrap();
                checked.unwrap();
                continue;
            }
            let tls = OtelBuildError::Tls {
                signal: Signal::Traces,
                reason: "transport error: no native certs found".to_owned(),
            };
            assert_eq!(made.err().unwrap(), tls, "{case}");
            assert_eq!(built.err().unwrap(), tls, "{case}");
            assert!(!tls.to_string().contains("127.0.0.1"), "{tls}");
            assert_eq!(
                checked.unwrap_err().to_string(),
                format!(
                    "telemetry.otlp.endpoint (line 3): \"{endpoint}\" is refused: TLS to the \
                     collector couldn't be set up: transport error: no native certs found"
                ),
                "{case}"
            );
        }
    });
}

/// Trust roots that can't be loaded fail the check as they fail the build,
/// as TLS rather than as the endpoint, since the check makes the exporters
/// too, and making one loads the roots and opens no connection. An
/// endpoint without a scheme is `https://`, so it's refused too, and one
/// written `http://` is spoken to without TLS, so no roots are loaded for
/// it and both take it.
#[tokio::test(flavor = "multi_thread")]
async fn a_grpc_endpoint_over_tls_with_no_roots_to_trust_is_refused_for_tls_by_check_and_build_alike()
 {
    let scratch = Scratch::new("network-no-roots");
    let roots = scratch.at("roots.pem");
    std::fs::write(&roots, "").unwrap();
    let closed = Receiver::closed();

    child(
        "export::tests::network::a_child_process_with_no_roots_to_trust_makes_no_exporter_that_speaks_tls",
        NO_ROOTS_CHILD,
        &format!("refused https://{closed},refused {closed},made http://{closed}"),
        &roots,
    )
    .await;
}

/// Set in the environment of the child process the test below starts, to
/// the endpoint it exports to over TLS and the file of a CA that didn't
/// sign that endpoint's certificate, with a comma between.
const REPLACED_CHILD: &str = "LABLET_TEST_OTLP_REPLACED_ROOTS_CHILD";

/// The child's side of the test below, which does nothing unless that test
/// started it: the export, with `tls.ca_file` naming the other CA, must
/// fail.
#[test]
fn a_child_process_exports_with_an_unrelated_ca_named() {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test reads the variable its parent set, to know which side it's on"
    )]
    let Some(given) = std::env::var_os(REPLACED_CHILD) else {
        return;
    };
    let given = given.to_str().unwrap().to_owned();
    let (endpoint, other_ca) = given.split_once(',').unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let settings = otlp_of(&to_with_tls(
            "grpc",
            endpoint,
            &format!("ca_file: '{other_ca}'"),
        ));
        let exported = exported_to(settings, "replaced-roots-child").await;
        assert!(
            exported.is_err(),
            "the roots `SSL_CERT_FILE` names were trusted beside the config's CA"
        );
    });
}

/// A certificate `tls.ca_file` names replaces the roots rather than joining
/// them: the gRPC exporter's platform roots, which `SSL_CERT_FILE` stands in
/// for here, hold the receiver's CA, and the config names another, so an
/// exporter that trusted both would reach the receiver. The HTTP exporter
/// can't be held to it this way on every platform: its client verifies
/// with the platform's verifier, which reads no `SSL_CERT_FILE` on macOS.
#[tokio::test(flavor = "multi_thread")]
async fn the_ca_file_replaces_the_roots_the_platform_has() {
    let receiver = Receiver::start(Mode::Answers).await;
    let other = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new("network-replaced-roots");
    let roots = scratch.at("roots.pem");
    std::fs::write(&roots, receiver.ca_certificate()).unwrap();
    let other_ca = scratch.at("other.pem");
    std::fs::write(&other_ca, other.ca_certificate()).unwrap();

    child(
        "export::tests::network::a_child_process_exports_with_an_unrelated_ca_named",
        REPLACED_CHILD,
        &format!("{},{}", receiver.grpc_tls_endpoint(), other_ca.display()),
        &roots,
    )
    .await;

    assert!(receiver.requests().is_empty(), "{:?}", receiver.requests());
}
