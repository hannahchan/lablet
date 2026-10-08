//! The network exporter through the command line's composition, its
//! `build` and `run`: where the telemetry goes when an endpoint is stated,
//! and what a refused endpoint or header is shown as.
//! Each config states `telemetry.otlp.enabled: true` over the lab's
//! `false`, beside the endpoint it sends to.

use lablet::{BuildError, ConfigError, Place, StopReason};
use lablet_conformance::receiver::{self, Mode, Receiver};
use serde_json::json;

use crate::harness::{ENDS, Lab, refusal, request};

/// The name of a per-run file in the working directory, which a test
/// checks no run writes.
fn each_run_file(run_id: &str) -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap()
        .join(format!("lablet-{run_id}.otlp.jsonl"))
}

#[tokio::test]
async fn a_run_with_an_endpoint_and_no_file_path_goes_to_the_collector_and_to_no_file() {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Lab::new("otlp-no-file");
    let config = scratch.config(
        ENDS,
        json!({ "telemetry": {
            "file": { "path": null },
            "otlp": { "enabled": true, "endpoint": receiver.grpc_endpoint(), "protocol": "grpc" },
        } }),
    );
    assert!(!lablet_cli::compose::telemetry_on_stderr(&config));

    let mut lablet = lablet_cli::compose::build(config).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
    let run_id = finished.summary.outcome.run_id.to_string();
    assert!(
        !each_run_file(&run_id).exists(),
        "a run with an endpoint and no path writes no file"
    );
    assert!(!scratch.telemetry().exists());
    let exported = receiver.exported().unwrap();
    assert_eq!(exported.records_of("lablet.run").len(), 1);
    assert_eq!(exported.spans_of("invoke_agent").len(), 1);
    assert_eq!(
        exported.spans[0].resource["team"], "evals",
        "the composer's resource travels over the network too"
    );
    assert!(
        receiver
            .requests()
            .iter()
            .all(|request| request.transport == receiver::Transport::Grpc)
    );
}

#[tokio::test]
async fn a_run_with_an_endpoint_and_a_file_path_goes_to_both_over_http_when_the_config_says_so() {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Lab::new("otlp-both-http");
    let config = scratch.config(
        ENDS,
        json!({ "telemetry": { "otlp": {
            "enabled": true,
            "endpoint": receiver.http_endpoint(),
            "protocol": "http/protobuf",
            "headers": { "x-token": "a made-up token value" },
        } } }),
    );

    let mut lablet = lablet_cli::compose::build(config).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
    let file = scratch.exported();
    let network = receiver.exported().unwrap();
    assert_eq!(file.records_of("lablet.run").len(), 1);
    assert_eq!(network.records_of("lablet.run").len(), 1);
    assert_eq!(file.ungrouped(), network.ungrouped());
    let requests = receiver.requests();
    assert!(!requests.is_empty());
    for request in &requests {
        assert_eq!(request.transport, receiver::Transport::Http);
        assert!(
            request
                .headers
                .contains(&("x-token".to_owned(), "a made-up token value".to_owned())),
            "{request:?}"
        );
    }
}

/// C20: a stated endpoint the exporter doesn't accept is refused by `check`
/// and `build` alike, by its key and as the config writes it.
#[tokio::test]
async fn an_endpoint_the_exporter_refuses_is_refused_by_its_key_as_the_config_writes_it() {
    let scratch = Lab::new("otlp-bad-endpoint");
    let config = scratch.config(
        ENDS,
        json!({ "telemetry": { "otlp": { "enabled": true, "endpoint": "http://[not a host" } } }),
    );

    let refused = refusal(config).await;

    assert_eq!(
        refused,
        BuildError::Config(ConfigError::Invalid {
            key: "telemetry.otlp.endpoint".to_owned(),
            place: Some(Place::Line(1)),
            value: Some("\"http://[not a host\"".to_owned()),
            reason: "it isn't a URL the exporter accepts".to_owned(),
        })
    );
}

/// C20: the user information of a refused endpoint is a secret, and where
/// it ends can't be told once it may hold an unencoded `/`, so the refusal
/// of an endpoint that holds an `@` shows no value: with a scheme, without
/// one, which the gRPC exporter gives one, and with a `/` in the password,
/// which a URL reads as ending the authority.
#[tokio::test]
async fn a_refused_endpoint_that_holds_an_at_is_shown_with_no_value() {
    const PASSWORD: &str = "hunter2-0123456789abcdef";
    let scratch = Lab::new("otlp-bad-endpoint-user-information");
    for endpoint in [
        format!("http://user:{PASSWORD}@[not a host"),
        format!("user:{PASSWORD}@[not a host"),
        format!("https://user:ab/{PASSWORD}@[not a host"),
    ] {
        let config = scratch.config(
            ENDS,
            json!({ "telemetry": { "otlp": { "enabled": true, "endpoint": endpoint } } }),
        );

        let refused = refusal(config).await;

        assert_eq!(
            refused.to_string(),
            "telemetry.otlp.endpoint (line 1): its value is refused: it isn't a URL the exporter \
             accepts",
            "{endpoint}"
        );
        assert!(
            !format!("{refused:?}").contains(PASSWORD),
            "{endpoint}: {refused:?}"
        );
    }
}

#[tokio::test]
async fn a_header_that_is_not_one_is_refused_by_its_key_and_its_value_is_shown_nowhere() {
    let scratch = Lab::new("otlp-bad-header");
    let config = scratch.config(
        ENDS,
        json!({ "telemetry": { "otlp": {
            "enabled": true,
            "endpoint": "http://127.0.0.1:1",
            "headers": { "not a header name": "a made-up secret value" },
        } } }),
    );

    let refused = refusal(config).await;

    assert_eq!(
        refused,
        BuildError::Config(ConfigError::Invalid {
            key: "telemetry.otlp.headers.not a header name".to_owned(),
            place: Some(Place::Line(1)),
            value: None,
            reason: "its name isn't one a header may have".to_owned(),
        })
    );
    assert!(!refused.to_string().contains("made-up"), "{refused}");
}
