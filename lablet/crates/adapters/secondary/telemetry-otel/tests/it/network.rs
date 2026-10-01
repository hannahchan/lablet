//! The network exporter against the in-process receiver: what both
//! destinations hold of one run, a run whose content outgrows a batch, what
//! a closed port costs, and the headers on the wire, which pins what the
//! exporter does with the environment's and what lablet does after it.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant, UNIX_EPOCH};

use lablet_conformance::observer::assert_hold_the_same_run;
use lablet_conformance::otlp::Exported;
use lablet_conformance::receiver::{self, Mode, Received, Receiver};
use lablet_telemetry_otel::{ATTRIBUTE_MAX_BYTES, FileTarget, OtlpSettings, Transport};
use lablet_telemetry_registry::attribute as key;
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

use crate::content::writing;
use crate::harness::{FAILS_CALLS_ENDS, Harness, RUN, Settings};

/// The event name of a content record.
const CONTENT: &str = "gen_ai.client.inference.operation.details";

/// A response that ends the run, and nothing before it.
const ENDS: &str = "
- response:
    content:
      - text: Nothing to fix.
    usage: { input_tokens: 100, output_tokens: 10 }
    finish: end_turn
";

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

async fn both_hold_the_content_of_one_run(transport: Transport, test: &str) {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new(test);
    let path = scratch.at("runs.otlp.jsonl");
    let mut harness = Harness::playing(
        FAILS_CALLS_ENDS,
        Settings {
            target: Some(FileTarget::Path(path.clone())),
            otlp: Some(settings(transport, endpoint_of(&receiver, transport), &[])),
            capture_content: true,
            ..Settings::in_scratch(&scratch)
        },
    )
    .await;

    harness.run(RUN).await;
    harness.observer.flush().await.unwrap();

    let file = Exported::read(&path).unwrap();
    let network = receiver.exported().unwrap();
    assert!(!file.records_of(CONTENT).is_empty());
    assert_eq!(
        file.records_of(CONTENT).len(),
        network.records_of(CONTENT).len(),
        "the content records reached both"
    );
    assert_hold_the_same_run(&file, &network, RUN);
    harness.observer.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_file_and_the_grpc_receiver_hold_the_same_content_records() {
    both_hold_the_content_of_one_run(Transport::Grpc, "network-content-grpc").await;
}

#[tokio::test(start_paused = true)]
async fn the_file_and_the_http_receiver_hold_the_same_content_records() {
    both_hold_the_content_of_one_run(Transport::HttpProtobuf, "network-content-http").await;
}

// O13: O10 against the receiver

#[tokio::test(start_paused = true)]
async fn the_receiver_holds_the_wide_event_of_a_run_whose_content_outgrew_a_batch() {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new("network-o13");
    let turns = 300;
    let mut harness = Harness::playing(
        &writing(turns, ATTRIBUTE_MAX_BYTES + 4_096),
        Settings {
            target: None,
            otlp: Some(settings(Transport::Grpc, receiver.grpc_endpoint(), &[])),
            capture_content: true,
            ..Settings::in_scratch(&scratch)
        },
    )
    .await;

    harness.run(RUN).await;
    let flushed = harness.observer.flush().await;

    let exported = receiver.exported().unwrap();
    let wide = exported.records_of("lablet.run");
    assert_eq!(
        wide.len(),
        1,
        "the receiver holds the wide event: {flushed:?}"
    );
    let content = exported.records_of(CONTENT);
    let dropped = wide[0].attributes[key::LABLET_TELEMETRY_DROPPED_RECORDS]
        .as_u64()
        .unwrap();
    assert_eq!(
        content.len() as u64 + dropped,
        1 + (turns as u64 + 1) + turns as u64,
        "every content record either arrived or is counted dropped: {flushed:?}"
    );
    assert_eq!(
        exported.spans.len() as u64 + dropped
            - (1 + (turns as u64 + 1) + turns as u64 - content.len() as u64),
        1 + (turns as u64 + 1) + turns as u64,
        "and so is every span"
    );
    let longest = content
        .iter()
        .filter_map(|record| record.attributes.get(key::GEN_AI_TOOL_CALL_RESULT))
        .map(|value| value.as_str().unwrap().len())
        .max();
    assert!(
        longest.is_none_or(|longest| longest <= ATTRIBUTE_MAX_BYTES),
        "a value that arrived is cut at {ATTRIBUTE_MAX_BYTES}: {longest:?}"
    );
    assert!(
        exported.spans.iter().all(|span| span.line < wide[0].line),
        "the wide event arrived after every span"
    );
    let _ = harness.observer.shutdown().await;
}

// O3: what a port nothing listens on costs

async fn a_closed_port_costs_under_five_seconds(transport: Transport, test: &str) {
    let closed = Receiver::closed();
    let scratch = Scratch::new(test);
    let path = scratch.at("runs.otlp.jsonl");
    let mut harness = Harness::playing(
        ENDS,
        Settings {
            target: Some(FileTarget::Path(path.clone())),
            otlp: Some(settings(transport, format!("http://{closed}"), &[])),
            ..Settings::in_scratch(&scratch)
        },
    )
    .await;

    let finished = harness.run(RUN).await;
    let began = Instant::now();
    let flushed = harness.observer.flush().await;
    let shut = harness.observer.shutdown().await;
    let cost = began.elapsed();

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        lablet_model::StopReason::Completed
    );
    let failures = flushed.unwrap_err();
    let queues: Vec<_> = failures
        .failures()
        .iter()
        .map(|failure| failure.split(": ").next().unwrap())
        .collect();
    assert_eq!(
        queues,
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
    assert_eq!(file.records_of("lablet.run").len(), 1, "the file is whole");
}

#[tokio::test]
async fn a_grpc_port_nothing_listens_on_costs_the_flush_and_the_shutdown_under_five_seconds() {
    a_closed_port_costs_under_five_seconds(Transport::Grpc, "network-closed-grpc").await;
}

#[tokio::test]
async fn an_http_port_nothing_listens_on_costs_the_flush_and_the_shutdown_under_five_seconds() {
    a_closed_port_costs_under_five_seconds(Transport::HttpProtobuf, "network-closed-http").await;
}

// The headers on the wire, from a child process whose environment sets some

/// Set in the environment of the child process the test below starts, to
/// the receiver's two endpoints, gRPC then HTTP, with a comma between.
const CHILD: &str = "LABLET_TEST_OTLP_HEADERS_CHILD";

/// What the child's environment sets `OTEL_EXPORTER_OTLP_HEADERS` to: one
/// header the code also sets, and one the config also sets.
const ENVIRONMENT: &str = "x-plain=env%20value,x-lablet=env";

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

/// Exports one span through the exporter alone, with `x-plain: code` set
/// as the exporter's own API sets a header, on a thread with no reactor,
/// as the SDK's batch threads are.
async fn exported_plainly(transport: Transport, endpoint: String) {
    let exporter = match transport {
        Transport::Grpc => {
            let mut metadata = MetadataMap::new();
            metadata.insert("x-plain", MetadataValue::from_static("code"));
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
            .with_headers(HashMap::from([("x-plain".to_owned(), "code".to_owned())]))
            .build()
            .unwrap(),
    };
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(exporter.export(vec![one_span()])))
        .await
        .unwrap()
        .unwrap();
}

/// Runs one run through lablet's observer, with the config stating
/// `x-lablet: config`, and the environment's headers stripped or not.
async fn exported_by_lablet(transport: Transport, endpoint: String, strip: bool, test: &str) {
    let scratch = Scratch::new(test);
    let mut harness = Harness::playing(
        ENDS,
        Settings {
            target: None,
            otlp: Some(OtlpSettings {
                strip_environment_headers: strip,
                ..settings(transport, endpoint, &[("x-lablet", "config")])
            }),
            ..Settings::in_scratch(&scratch)
        },
    )
    .await;
    harness.run(RUN).await;
    harness.observer.flush().await.unwrap();
    harness.observer.shutdown().await.unwrap();
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
/// exporter alone, by lablet keeping the environment's headers, or by
/// lablet stripping them, told apart by what `x-plain` and `x-lablet` hold.
fn sent<'a>(
    requests: &'a [Received],
    transport: receiver::Transport,
    how: &str,
) -> Vec<&'a Received> {
    requests
        .iter()
        .filter(|request| request.transport == transport)
        .filter(|request| {
            let (plain, lablet) = (header(request, "x-plain"), header(request, "x-lablet"));
            match how {
                "plainly" => lablet == Some("env"),
                "keeping" => lablet == Some("config") && plain.is_some(),
                "stripping" => lablet == Some("config") && plain.is_none(),
                _ => unreachable!(),
            }
        })
        .collect()
}

/// The exporter merges the environment's headers over the ones its own API
/// was given, name by name, and percent-decodes them; lablet sets the
/// config's after that merge, so the config's win; and with the config's
/// endpoint the environment's are sent to it at all.
#[tokio::test(flavor = "multi_thread")]
async fn the_environments_headers_win_in_the_exporter_and_the_configs_win_in_lablet() {
    let receiver = Receiver::start(Mode::Answers).await;
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "network::a_child_process_exports_with_the_headers_its_environment_and_its_config_state",
            "--test-threads=1",
        ])
        .env(CHILD, format!("{},{}", receiver.grpc_endpoint(), receiver.http_endpoint()))
        .env("OTEL_EXPORTER_OTLP_HEADERS", ENVIRONMENT)
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
        assert_eq!(
            header(plainly[0], "x-plain"),
            Some("env value"),
            "{transport:?}: the environment's value, decoded, in place of the code's"
        );

        let keeping = sent(&requests, transport, "keeping");
        assert_eq!(
            keeping.len(),
            2,
            "{transport:?}: the spans and the wide event, each with the config's header; the run \
             has no other records: {requests:?}"
        );
        for request in keeping {
            assert_eq!(
                header(request, "x-plain"),
                Some("env value"),
                "{transport:?}"
            );
        }

        let stripping = sent(&requests, transport, "stripping");
        assert_eq!(stripping.len(), 2, "{transport:?}: {requests:?}");
        for request in &stripping {
            assert!(
                request.headers.iter().all(|(name, _)| name != "x-plain"),
                "{transport:?}: the environment's header was sent to the config's endpoint: \
                 {request:?}"
            );
        }
    }
    assert_eq!(requests.len(), 2 * (1 + 2 + 2));
}
