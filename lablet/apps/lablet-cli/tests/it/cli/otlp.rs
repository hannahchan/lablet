//! The network exporter through the binary, with the `OTEL_*` environment
//! a run inherits given to the process, against the in-process receiver on
//! both transports: whose headers go with each signal, which endpoint and
//! protocol it's sent to, what turns a signal off, what's refused before a
//! run, and what `OTEL_SDK_DISABLED` and the GenAI capture variable do. A
//! test can't set a variable of its own process, so every case here is a
//! process of its own. Every config here states a receiver's endpoint, or
//! keeps the lab's `telemetry.otlp.enabled: false`.

use lablet_conformance::otlp::Exported;
use lablet_conformance::receiver::{self, Mode, Received, Receiver};
use serde_json::{Value, json};

use super::harness::{CONFIG, ENDS, Lab, PROMPT, Ran, ran};
use crate::key;

/// The authorization the config states.
const CONFIGS_AUTHORIZATION: &str = "Bearer config-0123456789abcdef";

/// `OTEL_EXPORTER_OTLP_HEADERS` as a framework sets it: a header of its
/// own, and an authorization of its own under the name the config states
/// too.
const ENVIRONMENTS_HEADERS: &str = "x-env=1,authorization=Bearer%20env-0123456789abcdef";

/// The two transports, and how each is named in the config.
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
            Self::Http => "http/protobuf",
        }
    }

    const fn transport(self) -> receiver::Transport {
        match self {
            Self::Grpc => receiver::Transport::Grpc,
            Self::Http => receiver::Transport::Http,
        }
    }

    fn lab(self, test: &str) -> Lab {
        Lab::new(&format!("{test}-{self:?}"))
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

/// The names of the files in `lab` that hold telemetry.
fn telemetry_files(lab: &Lab) -> Vec<String> {
    std::fs::read_dir(lab.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".otlp.jsonl"))
        .collect()
}

/// O18: with the config's headers stated, they're all that's sent, and no
/// header the environment names reaches an export, whichever variable
/// named it: an empty signal variable beside the generic one included.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_configs_headers_stated_only_they_arrive() {
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-config-headers");

        completed(
            &lab,
            json!({ "telemetry": { "otlp": {
                "enabled": true,
                "endpoint": over.endpoint(&receiver),
                "protocol": over.protocol(),
                "headers": { "authorization": CONFIGS_AUTHORIZATION, "x-config": "yes" },
            } } }),
            &[
                ("OTEL_EXPORTER_OTLP_HEADERS", ENVIRONMENTS_HEADERS),
                ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", ""),
                ("OTEL_EXPORTER_OTLP_LOGS_HEADERS", "x-logs=l"),
            ],
        );

        for request in received(&receiver, over) {
            assert_eq!(
                header(&request, "authorization"),
                [CONFIGS_AUTHORIZATION],
                "{over:?}: {request:?}"
            );
            assert_eq!(header(&request, "x-config"), ["yes"], "{over:?}");
            for name in ["x-env", "x-logs"] {
                assert_eq!(
                    header(&request, name),
                    Vec::<&str>::new(),
                    "{over:?}: the environment's header reached the export: {request:?}"
                );
            }
        }
    }
}

/// O17 and O18: with `headers: null`, the environment's headers arrive at
/// the config's endpoint, the signal's variable's when it holds one, else
/// the generic one's, an empty signal variable leaving the generic one.
#[tokio::test(flavor = "multi_thread")]
async fn with_headers_null_the_environments_arrive_at_the_configs_endpoint() {
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-environment-headers");

        completed(
            &lab,
            json!({ "telemetry": { "otlp": {
                "enabled": true,
                "endpoint": over.endpoint(&receiver),
                "protocol": over.protocol(),
            } } }),
            &[
                ("OTEL_EXPORTER_OTLP_HEADERS", ENVIRONMENTS_HEADERS),
                ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", ""),
                ("OTEL_EXPORTER_OTLP_LOGS_HEADERS", "x-logs=l"),
            ],
        );

        for request in received(&receiver, over) {
            match request.signal {
                receiver::Signal::Traces => {
                    assert_eq!(header(&request, "x-env"), ["1"], "{over:?}: {request:?}");
                    assert_eq!(
                        header(&request, "authorization"),
                        ["Bearer env-0123456789abcdef"],
                        "{over:?}"
                    );
                    assert_eq!(header(&request, "x-logs"), Vec::<&str>::new());
                }
                receiver::Signal::Logs => {
                    assert_eq!(header(&request, "x-logs"), ["l"], "{over:?}: {request:?}");
                    assert_eq!(header(&request, "x-env"), Vec::<&str>::new());
                    assert_eq!(header(&request, "authorization"), Vec::<&str>::new());
                }
            }
        }
    }
}

/// O17: an endpoint and a protocol from the environment are where the run
/// goes when the config leaves both to it, HTTP/protobuf unless the
/// protocol variable says otherwise, and a null path writes no file.
#[tokio::test(flavor = "multi_thread")]
async fn an_endpoint_from_the_environment_is_sent_over_its_protocol_and_a_null_path_writes_no_file()
{
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-env-endpoint-no-file");
        let endpoint = over.endpoint(&receiver);
        let mut env = vec![("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str())];
        if let Over::Grpc = over {
            env.push(("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"));
        }

        completed(
            &lab,
            json!({ "telemetry": { "file": { "path": null }, "otlp": { "enabled": null } } }),
            &env,
        );

        received(&receiver, over);
        assert_eq!(
            telemetry_files(&lab),
            Vec::<String>::new(),
            "{over:?}: a null path writes no file"
        );
    }
}

/// A null path writes no file, in the working directory or anywhere,
/// with the network off as with it on.
#[test]
fn a_null_file_path_writes_no_file_in_the_working_directory() {
    let lab = Lab::new("otlp-null-path");

    completed(
        &lab,
        json!({ "telemetry": { "file": { "path": null } } }),
        &[],
    );

    assert_eq!(telemetry_files(&lab), Vec::<String>::new());
}

/// O19: `enabled: false` sends nothing; each selector `none` turns its
/// signal off and leaves the other; `enabled: true` sends both whatever
/// both selectors say; and the named file gets both in every case.
#[tokio::test(flavor = "multi_thread")]
async fn each_signal_takes_its_own_selector_enabled_wins_over_both_and_the_file_gets_both() {
    /// One case: what it's called, `telemetry.otlp.enabled`, the
    /// environment, and whether spans and log records are then sent.
    struct Case {
        what: &'static str,
        enabled: Option<bool>,
        env: &'static [(&'static str, &'static str)],
        sent: (bool, bool),
    }
    let cases = [
        Case {
            what: "enabled-false",
            enabled: Some(false),
            env: &[],
            sent: (false, false),
        },
        Case {
            what: "traces-none",
            enabled: None,
            env: &[("OTEL_TRACES_EXPORTER", "none")],
            sent: (false, true),
        },
        Case {
            what: "logs-none",
            enabled: None,
            env: &[("OTEL_LOGS_EXPORTER", "none")],
            sent: (true, false),
        },
        Case {
            what: "enabled-true",
            enabled: Some(true),
            env: &[
                ("OTEL_TRACES_EXPORTER", "none"),
                ("OTEL_LOGS_EXPORTER", "none"),
            ],
            sent: (true, true),
        },
    ];

    for Case {
        what,
        enabled,
        env,
        sent,
    } in cases
    {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = Lab::new(&format!("otlp-selectors-{what}"));
        let otlp = match enabled {
            Some(false) => json!({ "enabled": false }),
            _ => json!({
                "enabled": enabled,
                "endpoint": receiver.grpc_endpoint(),
                "protocol": "grpc",
            }),
        };

        completed(&lab, json!({ "telemetry": { "otlp": otlp } }), env);

        let requests = receiver.requests();
        let of = |signal| requests.iter().any(|request| request.signal == signal);
        assert_eq!(
            (of(receiver::Signal::Traces), of(receiver::Signal::Logs)),
            sent,
            "{what}: {requests:?}"
        );
        if sent.1 {
            let exported = receiver.exported().unwrap();
            assert_eq!(
                exported.records_of("lablet.run").len(),
                1,
                "{what}: the wide event goes with the logs"
            );
        }
        let file = lab.exported();
        assert_eq!(file.records_of("lablet.run").len(), 1, "{what}");
        assert_eq!(file.spans_of("invoke_agent").len(), 1, "{what}");
    }
}

/// O21: `OTEL_RESOURCE_ATTRIBUTES` gives every export the attributes it
/// names beneath the config's `telemetry.resource`, which wins a key both
/// name, and `OTEL_SERVICE_NAME` names the service over the `service.name`
/// it names, in the file and over the network alike.
#[tokio::test(flavor = "multi_thread")]
async fn the_resource_variables_go_beneath_the_configs_and_otel_service_name_names_the_service() {
    let receiver = Receiver::start(Mode::Answers).await;
    let lab = Lab::new("otlp-resource-attributes");

    completed(
        &lab,
        json!({ "telemetry": {
            "otlp": { "enabled": true, "endpoint": receiver.grpc_endpoint(), "protocol": "grpc" },
            "resource": { "team": "b" },
        } }),
        &[
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "deployment.environment=test,team=a,service.name=pairs",
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
                Some(&json!("other")),
                "{destination}: `OTEL_SERVICE_NAME` over the pairs' and lablet's: {resource:?}"
            );
        }
    }
}

/// Runs `args` in `lab` with `env`, to a refusal of the config before any
/// run, and returns the one line it said.
fn refused_before_a_run(lab: &Lab, args: &[&str], env: &[(&str, &str)]) -> String {
    let mut command = lab.lablet(args);
    command.envs(env.iter().copied());

    let ran = ran(command, "");

    assert_eq!(ran.code, Some(1), "{ran:?}");
    assert_eq!(ran.stdout, "", "{ran:?}");
    assert!(!lab.telemetry().exists(), "a run began: {ran:?}");
    ran.stderr
}

/// C20: an endpoint the environment names that the exporter doesn't accept
/// is refused by `check` and by `run` alike, before any run, naming the
/// variable read, the signal's own before the generic one, and never what
/// it holds.
#[test]
fn an_endpoint_variable_that_does_not_parse_is_refused() {
    const BAD: &str = "http://[not a host";
    let lab = Lab::new("otlp-env-bad-endpoint");
    lab.write_config(
        ENDS,
        json!({ "telemetry": { "otlp": { "enabled": null } } }),
    );
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
        let said = refused_before_a_run(&lab, args, env);

        assert_eq!(
            said,
            format!(
                "config: telemetry.otlp.endpoint: its value is refused: `{named}`, which is read \
                 since the config states no endpoint, holds what isn't a URL the exporter \
                 accepts\n"
            ),
        );
        assert!(!said.contains("not a host"), "{said}");
    }
}

/// O28: a certificate file that can't be read, and a client key named
/// without its certificate, are refused by `check` and by `run` alike,
/// before any run, naming the variable and never what a file holds.
#[test]
fn a_certificate_file_that_cannot_be_read_is_refused_by_check_and_run_naming_the_variable() {
    let lab = Lab::new("otlp-bad-certificate");
    lab.write_config(
        ENDS,
        json!({ "telemetry": { "otlp": {
            "enabled": true,
            "endpoint": "https://collector.internal:4318",
        } } }),
    );
    let key = lab.write(
        "client.key",
        "-----BEGIN PRIVATE KEY-----\nnot-a-key-0123456789\n",
    );
    let missing = lab.at("missing.pem");
    let (key, missing) = (key.display().to_string(), missing.display().to_string());
    let prefix = "config: telemetry.otlp.endpoint (line 1): its value is refused: TLS to the \
                  collector for traces can't be set up: ";

    for args in [
        &["check", "--config", CONFIG][..],
        &["run", "--config", CONFIG, "--prompt", PROMPT][..],
    ] {
        let unread =
            refused_before_a_run(&lab, args, &[("OTEL_EXPORTER_OTLP_CERTIFICATE", &missing)]);
        let alone = refused_before_a_run(&lab, args, &[("OTEL_EXPORTER_OTLP_CLIENT_KEY", &key)]);

        assert!(
            unread.starts_with(&format!(
                "{prefix}`OTEL_EXPORTER_OTLP_CERTIFICATE` names a file that can't be read: "
            )),
            "{unread}"
        );
        assert!(!unread.contains("missing.pem"), "{unread}");
        assert_eq!(
            alone,
            format!(
                "{prefix}`OTEL_EXPORTER_OTLP_CLIENT_KEY` names a client key, and no client \
                 certificate is named beside it\n"
            )
        );
        assert!(!alone.contains("not-a-key"), "{alone}");
    }
}

/// O28: a certificate, and a client certificate and key, whose PEM is whole
/// and whose bodies are no DER are refused by `check` and by `run` alike,
/// over either protocol, naming the variables and never what a file holds
/// or what the TLS library said of it. The client's library only splits
/// PEM until a client is made, and the gRPC channel skips a root it can't
/// parse, so neither would refuse them before a run.
#[test]
fn tls_material_whose_pem_does_not_load_is_refused_by_check_and_run_on_both_protocols() {
    let lab = Lab::new("otlp-tls-not-der");
    let certificate = lab.write(
        "bad.pem",
        "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
    );
    let key = lab.write(
        "bad.key",
        "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
    );
    let (certificate, key) = (certificate.display().to_string(), key.display().to_string());
    let prefix = "config: telemetry.otlp.endpoint (line 1): its value is refused: TLS to the \
                  collector for traces can't be set up: ";

    for protocol in ["grpc", "http/protobuf"] {
        lab.write_config(
            ENDS,
            json!({ "telemetry": { "otlp": {
                "enabled": true,
                "endpoint": "https://collector.internal:4318",
                "protocol": protocol,
            } } }),
        );
        for args in [
            &["check", "--config", CONFIG][..],
            &["run", "--config", CONFIG, "--prompt", PROMPT][..],
        ] {
            let roots = refused_before_a_run(
                &lab,
                args,
                &[("OTEL_EXPORTER_OTLP_CERTIFICATE", &certificate)],
            );
            let identity = refused_before_a_run(
                &lab,
                args,
                &[
                    ("OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE", &certificate),
                    ("OTEL_EXPORTER_OTLP_CLIENT_KEY", &key),
                ],
            );

            assert_eq!(
                roots,
                format!(
                    "{prefix}`OTEL_EXPORTER_OTLP_CERTIFICATE` names a file that doesn't hold PEM \
                     certificates that can be loaded\n"
                ),
                "{protocol} {args:?}"
            );
            assert_eq!(
                identity,
                format!(
                    "{prefix}`OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE` and \
                     `OTEL_EXPORTER_OTLP_CLIENT_KEY` name files that don't hold a PEM client \
                     certificate and its private key that can be loaded\n"
                ),
                "{protocol} {args:?}"
            );
        }
    }
}

/// O29: `OTEL_SDK_DISABLED=true` with a file and an endpoint writes no file
/// and sends nothing, and the run is what it would be; with a path of `-`
/// standard error holds the summary line, since no telemetry is there.
#[tokio::test(flavor = "multi_thread")]
async fn otel_sdk_disabled_writes_no_file_and_sends_nothing_and_the_summary_prints_with_a_path_of_dash()
 {
    let receiver = Receiver::start(Mode::Answers).await;
    let otlp = json!({ "enabled": true, "endpoint": receiver.grpc_endpoint(), "protocol": "grpc" });
    let disabled = [("OTEL_SDK_DISABLED", "true")];

    let lab = Lab::new("otlp-sdk-disabled-file");
    let to_file = completed(&lab, json!({ "telemetry": { "otlp": otlp } }), &disabled);
    let dash = Lab::new("otlp-sdk-disabled-dash");
    let to_stderr = completed(
        &dash,
        json!({ "telemetry": { "otlp": otlp, "file": { "path": "-" } } }),
        &disabled,
    );

    assert!(receiver.requests().is_empty(), "{:?}", receiver.requests());
    assert_eq!(telemetry_files(&lab), Vec::<String>::new(), "{to_file:?}");
    let lines = to_stderr.stderr_lines();
    assert_eq!(lines.len(), 1, "{to_stderr:?}");
    assert!(lines[0].starts_with("completed: 1 turn, "), "{to_stderr:?}");
}

/// O30: the GenAI capture variable turns content capture on when the
/// config states nothing, and a stated `false` wins over it.
#[test]
fn the_genai_capture_variable_turns_capture_on_and_a_stated_false_wins() {
    const CONTENT: &str = "gen_ai.client.inference.operation.details";
    let capture = [("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT", "true")];

    let unstated = Lab::new("otlp-capture-unstated");
    completed(&unstated, json!({}), &capture);
    let stated = Lab::new("otlp-capture-stated");
    completed(
        &stated,
        json!({ "telemetry": { "capture_content": false } }),
        &capture,
    );

    assert!(!unstated.exported().records_of(CONTENT).is_empty());
    assert!(stated.exported().records_of(CONTENT).is_empty());
}
