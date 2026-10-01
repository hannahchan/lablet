//! The network exporter through the binary, with the `OTEL_*` environment
//! a run inherits given to the process, against the in-process receiver on
//! both transports: which endpoint turns the exporter on, whose headers go
//! with it, and what turns it off. A test can't set a variable of its own
//! process, so every case here is a process of its own.

use lablet_conformance::receiver::{self, Mode, Received, Receiver};
use serde_json::{Value, json};

use super::harness::{CONFIG, ENDS, Lab, PROMPT, Ran, ran};

/// The authorization the config states, which wins over the environment's.
const CONFIGS_AUTHORIZATION: &str = "Bearer config-0123456789abcdef";

/// `OTEL_EXPORTER_OTLP_HEADERS` as a framework sets it: a header of its
/// own, and an authorization of its own under the name the config states
/// too.
const ENVIRONMENTS_HEADERS: &str = "x-env=1,authorization=Bearer%20env-0123456789abcdef";

/// The two transports, and how each is named in the config and in the
/// environment.
#[derive(Debug, Clone, Copy)]
enum Over {
    Grpc,
    Http,
}

impl Over {
    const BOTH: [Self; 2] = [Self::Grpc, Self::Http];

    fn endpoint(self, receiver: &Receiver) -> String {
        match self {
            Self::Grpc => receiver.grpc_endpoint(),
            Self::Http => receiver.http_endpoint(),
        }
    }

    /// The value of `telemetry.otlp.protocol`.
    const fn protocol(self) -> &'static str {
        match self {
            Self::Grpc => "grpc",
            Self::Http => "http",
        }
    }

    const fn transport(self) -> receiver::Transport {
        match self {
            Self::Grpc => receiver::Transport::Grpc,
            Self::Http => receiver::Transport::Http,
        }
    }

    fn lab(self, test: &str) -> Lab {
        Lab::new(&format!("{test}-{}", self.protocol()))
    }
}

/// Runs the lab's config, with `more` stated over it and `env` given to
/// the process, to a completed run.
fn completed(lab: &Lab, more: Value, env: &[(&str, &str)]) -> Ran {
    lab.write_config(ENDS, more);
    let mut command = lab.lablet(&["run", "--config", CONFIG, "--prompt", PROMPT]);
    command.envs(env.iter().copied());
    let run = ran(command, "");
    assert_eq!(run.code, Some(0), "{run:?}");
    assert_eq!(run.outcome()["stop_reason"], "completed", "{run:?}");
    run
}

/// The requests the receiver kept, which all came in over `over` and
/// carried the run whole.
fn received(receiver: &Receiver, over: Over) -> Vec<Received> {
    let requests = receiver.requests();
    assert!(!requests.is_empty(), "{over:?}: nothing was received");
    for request in &requests {
        assert_eq!(request.transport, over.transport(), "{request:?}");
    }
    let exported = receiver.exported().unwrap();
    assert_eq!(exported.records_of("lablet.run").len(), 1, "{over:?}");
    assert_eq!(exported.spans_of("invoke_agent").len(), 1, "{over:?}");
    requests
}

/// Every value `request` carries under the header `name`.
fn header<'a>(request: &'a Received, name: &str) -> Vec<&'a str> {
    request
        .headers
        .iter()
        .filter(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
        .collect()
}

/// O18: with the config's endpoint, the environment's headers are on no
/// export, and the config's are, winning a name both state.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_configs_endpoint_the_environments_headers_are_on_no_export_and_the_configs_win() {
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-config-endpoint");

        completed(
            &lab,
            json!({ "telemetry": { "otlp": {
                "endpoint": over.endpoint(&receiver),
                "protocol": over.protocol(),
                "headers": { "authorization": CONFIGS_AUTHORIZATION, "x-config": "yes" },
            } } }),
            &[("OTEL_EXPORTER_OTLP_HEADERS", ENVIRONMENTS_HEADERS)],
        );

        for request in received(&receiver, over) {
            assert_eq!(
                header(&request, "authorization"),
                [CONFIGS_AUTHORIZATION],
                "{over:?}: {request:?}"
            );
            assert_eq!(header(&request, "x-config"), ["yes"], "{over:?}");
            assert_eq!(
                header(&request, "x-env"),
                Vec::<&str>::new(),
                "{over:?}: the environment's header reached the config's endpoint: {request:?}"
            );
        }
    }
}

/// O17: with the environment's endpoint, its headers are sent, and the
/// config's win a name both state.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_environments_endpoint_its_headers_are_sent_and_the_configs_win_a_shared_name() {
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-env-endpoint-headers");
        let endpoint = over.endpoint(&receiver);

        completed(
            &lab,
            json!({ "telemetry": { "otlp": {
                "protocol": over.protocol(),
                "headers": { "authorization": CONFIGS_AUTHORIZATION },
            } } }),
            &[
                ("OTEL_EXPORTER_OTLP_ENDPOINT", &endpoint),
                ("OTEL_EXPORTER_OTLP_HEADERS", ENVIRONMENTS_HEADERS),
            ],
        );

        for request in received(&receiver, over) {
            assert_eq!(header(&request, "x-env"), ["1"], "{over:?}: {request:?}");
            assert_eq!(
                header(&request, "authorization"),
                [CONFIGS_AUTHORIZATION],
                "{over:?}: {request:?}"
            );
        }
    }
}

/// O17: an endpoint from the environment alone turns the exporter on, over
/// gRPC unless `OTEL_EXPORTER_OTLP_PROTOCOL` says otherwise when the config
/// states no protocol, and a null path then writes no file.
#[tokio::test(flavor = "multi_thread")]
async fn an_endpoint_from_the_environment_turns_the_exporter_on_over_its_protocol_and_writes_no_file()
 {
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-env-endpoint-no-file");
        let endpoint = over.endpoint(&receiver);
        let mut env = vec![("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str())];
        if let Over::Http = over {
            env.push(("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf"));
        }

        completed(
            &lab,
            json!({ "telemetry": { "file": { "path": null } } }),
            &env,
        );

        received(&receiver, over);
        assert!(!lab.telemetry().exists(), "{over:?}");
        let files: Vec<String> = std::fs::read_dir(lab.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".otlp.jsonl"))
            .collect();
        assert_eq!(
            files,
            Vec::<String>::new(),
            "{over:?}: a run with the environment's endpoint and no path writes no file"
        );
    }
}

/// O19: `telemetry.otlp.enabled: false` and `OTEL_TRACES_EXPORTER=none`
/// each turn the exporter off whatever endpoint the environment names, and
/// the file is written.
#[tokio::test(flavor = "multi_thread")]
async fn enabled_false_and_otel_traces_exporter_none_each_turn_the_exporter_off_and_the_file_stays()
{
    let receiver = Receiver::start(Mode::Answers).await;
    let endpoint = receiver.grpc_endpoint();
    let cases = [
        (
            "enabled-false",
            json!({ "telemetry": { "otlp": { "enabled": false } } }),
            vec![("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str())],
        ),
        (
            "traces-exporter-none",
            json!({}),
            vec![
                ("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str()),
                ("OTEL_TRACES_EXPORTER", "none"),
            ],
        ),
    ];

    for (what, more, env) in cases {
        let lab = Lab::new(&format!("otlp-off-{what}"));

        completed(&lab, more, &env);

        assert!(
            receiver.requests().is_empty(),
            "{what}: {:?}",
            receiver.requests()
        );
        assert_eq!(
            lab.exported().records_of("lablet.run").len(),
            1,
            "{what}: the file is written"
        );
    }
}
