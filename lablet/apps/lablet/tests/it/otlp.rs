//! The network exporter through `build` and `run`, and through the binary:
//! where the telemetry goes when an endpoint is stated, what a port nothing
//! listens on costs, and what a refused endpoint or header is shown as.

use std::time::{Duration, Instant};

use lablet::{BuildError, ConfigError, Place, StopReason};
use lablet_conformance::receiver::{self, Mode, Receiver};
use serde_json::json;

use crate::harness::{ENDS, Lab, refusal, request};

/// The name of a run's own telemetry file, in the working directory, which
/// a run with an endpoint and no path must not write.
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
            "otlp": { "endpoint": receiver.grpc_endpoint() },
        } }),
    );
    assert!(!lablet::telemetry_on_stderr(&config));

    let mut lablet = lablet::build(config).await.unwrap();
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
            "endpoint": receiver.http_endpoint(),
            "protocol": "http",
            "headers": { "x-token": "a made-up token value" },
        } } }),
    );

    let mut lablet = lablet::build(config).await.unwrap();
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

/// O3: a closed port changes neither the outcome nor the exit code, the
/// failure is on the diagnostic log, and the process is gone within five
/// seconds of the run's end.
#[test]
fn a_port_nothing_listens_on_changes_nothing_and_the_process_exits_within_five_seconds() {
    let closed = Receiver::closed();
    let lab = Lab::new("otlp-closed-port");
    lab.write_config(
        ENDS,
        json!({ "telemetry": { "otlp": { "endpoint": format!("http://{closed}") } } }),
    );
    let plain = Lab::new("otlp-closed-port-plain");
    plain.write_config(ENDS, json!({}));

    let began = Instant::now();
    let ran = lab.run_config(&[]);
    let cost = began.elapsed();
    let expected = plain.run_config(&[]);

    assert_eq!(ran.code, Some(0), "{ran:?}");
    assert_eq!(expected.code, Some(0), "{expected:?}");
    let (outcome, plain_outcome) = (ran.outcome(), expected.outcome());
    assert_eq!(outcome["stop_reason"], "completed");
    assert_eq!(outcome["stop_reason"], plain_outcome["stop_reason"]);
    assert_eq!(outcome["usage"], plain_outcome["usage"]);
    assert!(
        ran.stderr
            .contains("an export of spans failed, and the spans are lost"),
        "{}",
        ran.stderr
    );
    assert!(
        !ran.stderr.contains(&closed.to_string()),
        "the endpoint is in no diagnostic line: {}",
        ran.stderr
    );
    assert!(cost < Duration::from_secs(5), "the process took {cost:?}");
    assert_eq!(
        lab.exported().records_of("lablet.run").len(),
        1,
        "the file is whole whatever the port did"
    );
}

/// C20: a stated endpoint the exporter doesn't accept is refused by `check`
/// and `build` alike, by its key and as the config writes it.
#[tokio::test]
async fn an_endpoint_the_exporter_refuses_is_refused_by_its_key_as_the_config_writes_it() {
    let scratch = Lab::new("otlp-bad-endpoint");
    let config = scratch.config(
        ENDS,
        json!({ "telemetry": { "otlp": { "endpoint": "http://[not a host" } } }),
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
            json!({ "telemetry": { "otlp": { "endpoint": endpoint } } }),
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
