//! The network exporter through the binary, with the `OTEL_*` environment
//! a run inherits given to the process, against the in-process receiver on
//! both transports: which endpoint turns the exporter on, whose headers go
//! with it, what turns it off, and what the resource takes from the
//! environment. A test can't set a variable of its own process, so every
//! case here is a process of its own.

use lablet_conformance::otlp::Exported;
use lablet_conformance::receiver::{self, Mode, Received, Receiver};
use lablet_telemetry_registry::attribute as key;
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

/// O21: `OTEL_RESOURCE_ATTRIBUTES` gives every export the attributes it
/// names beneath the config's `telemetry.resource`, which wins a key both
/// name, and `service.name` stays lablet's whatever it and
/// `OTEL_SERVICE_NAME` say, in the file and over the network alike.
#[tokio::test(flavor = "multi_thread")]
async fn the_environments_resource_attributes_go_beneath_the_configs_and_the_service_stays_lablet()
{
    let receiver = Receiver::start(Mode::Answers).await;
    let lab = Lab::new("otlp-resource-attributes");

    completed(
        &lab,
        json!({ "telemetry": {
            "otlp": { "endpoint": receiver.grpc_endpoint(), "protocol": "grpc" },
            "resource": { "team": "b" },
        } }),
        &[
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "deployment.environment=test,team=a,service.name=other",
            ),
            ("OTEL_SERVICE_NAME", "other"),
        ],
    );

    received(&receiver, Over::Grpc);
    let destinations: [(&str, Exported); 2] = [
        ("the file", lab.exported()),
        ("the receiver", receiver.exported().unwrap()),
    ];
    for (destination, exported) in destinations {
        assert_eq!(exported.records_of("lablet.run").len(), 1, "{destination}");
        let resources = exported
            .spans
            .iter()
            .map(|span| &span.resource)
            .chain(exported.records.iter().map(|record| &record.resource));
        for resource in resources {
            assert_eq!(
                resource.get("deployment.environment"),
                Some(&json!("test")),
                "{destination}: {resource:?}"
            );
            assert_eq!(
                resource.get("team"),
                Some(&json!("b")),
                "{destination}: the config's wins a key both name: {resource:?}"
            );
            assert_eq!(
                resource.get(key::SERVICE_NAME),
                Some(&json!("lablet")),
                "{destination}: {resource:?}"
            );
        }
    }
}

/// An endpoint the environment names that the exporter doesn't accept is
/// refused by `check` and by `run` alike, before any run, naming the
/// variable the exporter read, the signal's own before the generic one,
/// and never what it holds.
#[test]
fn an_endpoint_variable_the_exporter_refuses_is_refused_by_check_and_run_naming_the_variable() {
    const BAD: &str = "http://[not a host";
    let lab = Lab::new("otlp-env-bad-endpoint");
    lab.write_config(ENDS, json!({}));
    let check = ["check", "--config", CONFIG];
    let run = ["run", "--config", CONFIG, "--prompt", PROMPT];
    let generic = vec![("OTEL_EXPORTER_OTLP_ENDPOINT", BAD)];
    let own = vec![
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:1"),
        ("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", BAD),
    ];

    for (args, env, named) in [
        (&check[..], &generic, "OTEL_EXPORTER_OTLP_ENDPOINT"),
        (&run[..], &generic, "OTEL_EXPORTER_OTLP_ENDPOINT"),
        (&check[..], &own, "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"),
        (&run[..], &own, "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"),
    ] {
        let mut command = lab.lablet(args);
        command.envs(env.iter().copied());

        let ran = ran(command, "");

        assert_eq!(ran.code, Some(1), "{ran:?}");
        assert_eq!(ran.stdout, "", "{ran:?}");
        assert_eq!(
            ran.stderr,
            format!(
                "config: telemetry.otlp.endpoint: its value is refused: `{named}`, which is read \
                 since the config states no endpoint, holds what isn't a URL the exporter \
                 accepts\n"
            ),
            "{ran:?}"
        );
        assert!(!ran.stderr.contains("not a host"), "{ran:?}");
    }
    assert!(
        !lab.telemetry().exists(),
        "no run began, so nothing was exported"
    );
}
