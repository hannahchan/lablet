//! The network exporter through the binary, against the in-process receiver
//! on both transports, with the `OTEL_*` environment a run inherits given to
//! the process: what's sent where the config names an endpoint and nothing
//! otherwise, whose headers go, what's refused before a run, that the
//! environment's variables change nothing, and the pod block. A test can't
//! set a variable of its own process, so every case here is a process of
//! its own. Every config here states a receiver's endpoint or none.

use lablet_conformance::otlp::Exported;
use lablet_conformance::receiver::{self, Mode, Received, Receiver};
use serde_json::{Value, json};

use super::harness::{CONFIG, ENDS, Lab, PROMPT, Ran, ran};
use crate::key;

/// The authorization the config states, through `${LABLET_TEST_AUTH}`.
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
    completed_with(lab, more, env, &[])
}

/// [`completed`], with `args` after the prompt.
fn completed_with(lab: &Lab, more: Value, env: &[(&str, &str)], args: &[&str]) -> Ran {
    lab.write_config(ENDS, more);
    let mut command =
        lab.lablet(&[&["run", "--config", CONFIG, "--prompt", PROMPT][..], args].concat());
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
/// header the environment names reaches an export: the variables aren't
/// read, and what the exporter merges of them on its own comes off.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_configs_headers_stated_only_they_arrive() {
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-config-headers");

        completed(
            &lab,
            json!({ "telemetry": { "otlp": {
                "endpoint": over.endpoint(&receiver),
                "protocol": over.protocol(),
                "headers": {
                    "authorization": "${LABLET_TEST_AUTH}",
                    "x-config": "${LABLET_TEST_CONFIG}",
                },
            } } }),
            &[
                ("LABLET_TEST_AUTH", CONFIGS_AUTHORIZATION),
                ("LABLET_TEST_CONFIG", "yes"),
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

/// O17 and O18, inverted: with `headers: null`, nothing of the
/// environment's header variables arrives either, since none is read.
#[tokio::test(flavor = "multi_thread")]
async fn with_headers_null_nothing_of_the_environments_header_variables_arrives() {
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-environment-headers");

        completed(
            &lab,
            json!({ "telemetry": { "otlp": {
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
            for name in ["x-env", "x-logs", "authorization"] {
                assert_eq!(
                    header(&request, name),
                    Vec::<&str>::new(),
                    "{over:?}: the environment's header reached the export: {request:?}"
                );
            }
        }
    }
}

/// `headers` as OpenTelemetry's `k=v` text: each value decoded, so `curl -v`
/// shows no bare token, and a pair written with no `${VAR}` refused by
/// `check` and `run` alike without the value shown (R62).
#[tokio::test(flavor = "multi_thread")]
async fn headers_as_text_decode_each_value_and_a_literal_token_is_refused() {
    let token = "text-0123456789abcdef";
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-headers-text");

        completed(
            &lab,
            json!({ "telemetry": { "otlp": {
                "endpoint": over.endpoint(&receiver),
                "protocol": over.protocol(),
                "headers": "authorization=Bearer%20${LABLET_TEST_TOKEN},x-tenant=${LABLET_TEST_TENANT}",
            } } }),
            &[
                ("LABLET_TEST_TOKEN", token),
                ("LABLET_TEST_TENANT", "acme%20inc"),
            ],
        );

        for request in received(&receiver, over) {
            assert_eq!(
                header(&request, "authorization"),
                [format!("Bearer {token}")],
                "{over:?}: {request:?}"
            );
            assert_eq!(header(&request, "x-tenant"), ["acme inc"], "{over:?}");
        }
    }

    let lab = Lab::new("otlp-headers-literal");
    lab.write_config(
        ENDS,
        json!({ "telemetry": { "otlp": {
            "endpoint": "http://127.0.0.1:1",
            "headers": format!("authorization=Bearer%20{token}"),
        } } }),
    );
    for args in [
        &["check", "--config", CONFIG][..],
        &["run", "--config", CONFIG, "--prompt", PROMPT][..],
    ] {
        let said = refused_before_a_run(&lab, args, &[]);
        assert_eq!(
            said,
            "config: telemetry.otlp.headers (line 1) is refused: it holds no ${VAR}, and what it \
             holds isn't shown; write each value as a ${VAR}, as authorization=Bearer%20${TOKEN}, \
             and rotate it if this was a token\n",
            "{args:?}"
        );
        assert!(!said.contains(token), "{said}");
    }
}

/// O17, inverted: an endpoint and a protocol the environment names aren't
/// read, so with none stated nothing is sent, nothing is written for a null
/// path, and the run's end waits on no exporter.
#[tokio::test(flavor = "multi_thread")]
async fn an_endpoint_the_environment_names_is_not_read_so_nothing_is_sent_and_a_null_path_writes_no_file()
 {
    for over in Over::BOTH {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = over.lab("otlp-env-endpoint-no-file");
        let endpoint = over.endpoint(&receiver);
        let mut env = vec![("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str())];
        if let Over::Grpc = over {
            env.push(("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"));
        }

        let run = completed(
            &lab,
            json!({ "telemetry": { "file": { "path": null } } }),
            &env,
        );

        assert!(
            receiver.requests().is_empty(),
            "{over:?}: {:?}",
            receiver.requests()
        );
        assert_eq!(
            telemetry_files(&lab),
            Vec::<String>::new(),
            "{over:?}: a null path writes no file"
        );
        for said in ["ExportError", "wasn't exported whole"] {
            assert!(!run.stderr.contains(said), "{over:?}: {run:?}");
        }
    }
}

/// A null path writes no file, in the working directory or anywhere,
/// with an endpoint named as with none.
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

/// O19, as the caller owning the config rewords it: a stated endpoint sends
/// both signals whatever the selectors say, since they aren't read, and the
/// named file gets both too; `--set telemetry.otlp.endpoint=null` unstates
/// a stated endpoint, so nothing is sent.
#[tokio::test(flavor = "multi_thread")]
async fn a_stated_endpoint_sends_both_signals_whatever_the_selectors_say_and_set_null_unstates_it()
{
    let receiver = Receiver::start(Mode::Answers).await;
    let lab = Lab::new("otlp-selectors");
    let otlp = json!({ "endpoint": receiver.grpc_endpoint(), "protocol": "grpc" });
    let selectors = [
        ("OTEL_TRACES_EXPORTER", "none"),
        ("OTEL_LOGS_EXPORTER", "none"),
    ];

    completed(&lab, json!({ "telemetry": { "otlp": otlp } }), &selectors);

    let requests = receiver.requests();
    let of = |signal| requests.iter().any(|request| request.signal == signal);
    assert!(
        of(receiver::Signal::Traces) && of(receiver::Signal::Logs),
        "{requests:?}"
    );
    let exported = receiver.exported().unwrap();
    assert_eq!(exported.records_of("lablet.run").len(), 1);
    let file = lab.exported();
    assert_eq!(file.records_of("lablet.run").len(), 1);
    assert_eq!(file.spans_of("invoke_agent").len(), 1);

    let before = receiver.requests().len();
    let unstated = Lab::new("otlp-set-null");
    completed_with(
        &unstated,
        json!({ "telemetry": { "otlp": otlp } }),
        &[],
        &["--set", "telemetry.otlp.endpoint=null"],
    );
    assert_eq!(
        receiver.requests().len(),
        before,
        "{:?}",
        receiver.requests()
    );
    assert_eq!(unstated.exported().records_of("lablet.run").len(), 1);
}

/// A run with nothing named connects to nowhere and writes nothing: a
/// receiver whose endpoint the environment names sees no request, no file
/// appears, and the run's end waits on no exporter.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_with_nothing_named_sends_nothing_and_writes_nothing() {
    let receiver = Receiver::start(Mode::Answers).await;
    let lab = Lab::new("otlp-nothing-named");
    let endpoint = receiver.http_endpoint();
    lab.write_config(ENDS, json!({ "telemetry": { "file": { "path": null } } }));
    let mut command = lab.lablet(&["run", "--config", CONFIG, "--prompt", PROMPT]);
    command.env("OTEL_EXPORTER_OTLP_ENDPOINT", &endpoint);

    let run = ran(command, "");

    assert_eq!(run.code, Some(0), "{run:?}");
    assert!(receiver.requests().is_empty(), "{:?}", receiver.requests());
    assert_eq!(telemetry_files(&lab), Vec::<String>::new());
    assert_eq!(run.stderr_lines().len(), 1, "the summary alone: {run:?}");
}

/// O21, inverted: `OTEL_RESOURCE_ATTRIBUTES` and `OTEL_SERVICE_NAME` aren't
/// read, so the resource is the config's alone, as a map or as `k=v` text,
/// whose first `service.name` names the service, and `lablet` when none
/// does, in the file and over the network alike.
#[tokio::test(flavor = "multi_thread")]
async fn the_resource_variables_are_not_read_and_the_configs_resource_names_the_service_or_lablet_does()
 {
    let environment = [
        (
            "OTEL_RESOURCE_ATTRIBUTES",
            "deployment.environment=test,team=a,service.name=pairs",
        ),
        ("OTEL_SERVICE_NAME", "other"),
    ];
    for (form, resource, service) in [
        ("map", json!({ "team": "b" }), "lablet"),
        (
            "text",
            json!("service.name=first,team=b%20c,service.name=second,"),
            "first",
        ),
    ] {
        let receiver = Receiver::start(Mode::Answers).await;
        let lab = Lab::new(&format!("otlp-resource-{form}"));

        completed(
            &lab,
            json!({ "telemetry": {
                "otlp": { "endpoint": receiver.grpc_endpoint(), "protocol": "grpc" },
                "resource": resource,
            } }),
            &environment,
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
            for attributes in resources {
                assert_eq!(
                    attributes.get("deployment.environment"),
                    None,
                    "{form} {destination}: the variable was read: {attributes:?}"
                );
                let team = if form == "map" { "b" } else { "b c" };
                assert_eq!(
                    attributes.get("team"),
                    Some(&json!(team)),
                    "{form} {destination}: {attributes:?}"
                );
                assert_eq!(
                    attributes.get(key::SERVICE_NAME),
                    Some(&json!(service)),
                    "{form} {destination}: {attributes:?}"
                );
            }
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

/// C20: an endpoint a variable fills that the exporter doesn't accept is
/// refused by `check` and by `run` alike, before any run, naming the
/// variable and never what it holds (R10's family).
#[test]
fn a_filled_endpoint_that_does_not_parse_is_refused_naming_the_variable_and_not_its_value() {
    const BAD: &str = "http://[not a host";
    let lab = Lab::new("otlp-env-bad-endpoint");
    lab.write_config(
        ENDS,
        json!({ "telemetry": { "otlp": { "endpoint": "${LABLET_TEST_ENDPOINT:-}" } } }),
    );
    let check = ["check", "--config", CONFIG];
    let run = ["run", "--config", CONFIG, "--prompt", PROMPT];

    for args in [&check[..], &run[..]] {
        let said = refused_before_a_run(&lab, args, &[("LABLET_TEST_ENDPOINT", BAD)]);

        assert_eq!(
            said,
            "config: telemetry.otlp.endpoint (line 1): \"${LABLET_TEST_ENDPOINT:-}\" is refused: \
             `LABLET_TEST_ENDPOINT` holds what isn't a URL the exporter accepts\n"
        );
        assert!(!said.contains("not a host"), "{said}");
    }
}

/// O28: a `ca_file` that can't be read is refused by `check` and by `run`
/// alike, before any run, by its key and naming the variable that filled
/// the path, never what a file holds; a `client_key` without its
/// certificate is refused by its key.
#[test]
fn a_tls_file_that_cannot_be_read_is_refused_naming_a_filled_paths_variable_and_a_key_without_its_certificate_is_refused()
 {
    let lab = Lab::new("otlp-bad-certificate");
    let key = lab.write(
        "client.key",
        "-----BEGIN PRIVATE KEY-----\nnot-a-key-0123456789\n",
    );
    let key = key.display().to_string();
    let missing = lab.at("missing.pem").display().to_string();

    for args in [
        &["check", "--config", CONFIG][..],
        &["run", "--config", CONFIG, "--prompt", PROMPT][..],
    ] {
        lab.write_config(
            ENDS,
            json!({ "telemetry": { "otlp": {
                "endpoint": "https://collector.internal:4318",
                "tls": { "ca_file": "${LABLET_TEST_CA:-}" },
            } } }),
        );
        let unread = refused_before_a_run(&lab, args, &[("LABLET_TEST_CA", &missing)]);
        assert!(
            unread.starts_with(
                "config: telemetry.otlp.tls.ca_file (line 1): \"${LABLET_TEST_CA:-}\" is refused: \
                 `LABLET_TEST_CA` names a file that can't be read: "
            ),
            "{unread}"
        );
        assert!(!unread.contains("missing.pem"), "{unread}");

        lab.write_config(
            ENDS,
            json!({ "telemetry": { "otlp": {
                "endpoint": "https://collector.internal:4318",
                "tls": { "client_key": key },
            } } }),
        );
        let alone = refused_before_a_run(&lab, args, &[]);
        assert_eq!(
            alone,
            format!(
                "config: telemetry.otlp.tls.client_key (line 1): \"{key}\" is refused: a client \
                 certificate and its key land together, and `telemetry.otlp.tls.client_cert` \
                 isn't stated; state it beside\n"
            )
        );
        assert!(!alone.contains("not-a-key"), "{alone}");
    }
}

/// O28: a `ca_file`, and a `client_cert` and `client_key`, whose PEM is
/// whole and whose bodies are no DER are refused by `check` and by `run`
/// alike, over either protocol, by their keys and never what a file holds
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

    for protocol in ["grpc", "http/protobuf"] {
        for args in [
            &["check", "--config", CONFIG][..],
            &["run", "--config", CONFIG, "--prompt", PROMPT][..],
        ] {
            lab.write_config(
                ENDS,
                json!({ "telemetry": { "otlp": {
                    "endpoint": "https://collector.internal:4318",
                    "protocol": protocol,
                    "tls": { "ca_file": certificate },
                } } }),
            );
            let roots = refused_before_a_run(&lab, args, &[]);
            lab.write_config(
                ENDS,
                json!({ "telemetry": { "otlp": {
                    "endpoint": "https://collector.internal:4318",
                    "protocol": protocol,
                    "tls": { "client_cert": certificate, "client_key": key },
                } } }),
            );
            let identity = refused_before_a_run(&lab, args, &[]);

            assert_eq!(
                roots,
                format!(
                    "config: telemetry.otlp.tls.ca_file (line 1): \"{certificate}\" is refused: the \
                     file names a file that doesn't hold PEM certificates that can be loaded\n"
                ),
                "{protocol} {args:?}"
            );
            assert_eq!(
                identity,
                format!(
                    "config: telemetry.otlp.tls.client_cert (line 1): \"{certificate}\" is refused: \
                     the file and `telemetry.otlp.tls.client_key` name files that don't hold a PEM \
                     client certificate and its private key that can be loaded\n"
                ),
                "{protocol} {args:?}"
            );
            assert!(!identity.contains("AAAA"), "{identity}");
        }
    }
}

/// O29, inverted: `OTEL_SDK_DISABLED=true` and `OTEL_TRACES_SAMPLER` are
/// read by nothing, so a run under them writes its file whole, spans
/// included, sends to the endpoint it names, and with a path of `-` writes
/// its telemetry on standard error in place of the summary.
#[tokio::test(flavor = "multi_thread")]
async fn otel_sdk_disabled_and_the_sampler_variable_change_nothing_a_run_sends_or_writes() {
    let receiver = Receiver::start(Mode::Answers).await;
    let otlp = json!({ "endpoint": receiver.grpc_endpoint(), "protocol": "grpc" });
    let hostile = [
        ("OTEL_SDK_DISABLED", "true"),
        ("OTEL_TRACES_SAMPLER", "always_off"),
    ];

    let lab = Lab::new("otlp-sdk-disabled-file");
    completed(&lab, json!({ "telemetry": { "otlp": otlp } }), &hostile);
    let dash = Lab::new("otlp-sdk-disabled-dash");
    let to_stderr = completed(
        &dash,
        json!({ "telemetry": { "otlp": otlp, "file": { "path": "-" } } }),
        &hostile,
    );

    let requests = receiver.requests();
    assert!(!requests.is_empty());
    for request in &requests {
        assert_eq!(request.transport, receiver::Transport::Grpc, "{request:?}");
    }
    let network = receiver.exported().unwrap();
    assert_eq!(network.records_of("lablet.run").len(), 2, "both runs sent");
    assert_eq!(network.spans_of("invoke_agent").len(), 2, "both sampled");
    let file = lab.exported();
    assert_eq!(file.spans_of("invoke_agent").len(), 1, "sampled");
    assert_eq!(file.records_of("lablet.run").len(), 1);
    let lines = to_stderr.stderr_lines();
    assert!(
        lines.len() > 1,
        "the telemetry, not the summary: {to_stderr:?}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("completed: ")),
        "{to_stderr:?}"
    );
}

/// O30, as the caller owning the config rewords it: the GenAI capture
/// variable isn't read, so it turns nothing on; a stated
/// `telemetry.capture_content: true` alone captures; and a `${VAR}` there
/// is typed, `true` capturing, empty unstated, and `yes` refused without the
/// value shown (R13).
#[test]
fn the_genai_capture_variable_is_not_read_and_a_stated_capture_content_alone_captures() {
    const CONTENT: &str = "gen_ai.client.inference.operation.details";
    const CAPTURE: &str = "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT";

    let unstated = Lab::new("otlp-capture-unstated");
    completed(&unstated, json!({}), &[(CAPTURE, "true")]);
    let stated = Lab::new("otlp-capture-stated");
    completed(
        &stated,
        json!({ "telemetry": { "capture_content": true } }),
        &[],
    );
    let typed = Lab::new("otlp-capture-typed");
    completed(
        &typed,
        json!({ "telemetry": { "capture_content": format!("${{{CAPTURE}}}") } }),
        &[(CAPTURE, "true")],
    );
    let empty = Lab::new("otlp-capture-empty");
    completed(
        &empty,
        json!({ "telemetry": { "capture_content": format!("${{{CAPTURE}:-}}") } }),
        &[],
    );

    assert!(
        unstated.exported().records_of(CONTENT).is_empty(),
        "the variable was read"
    );
    assert!(!stated.exported().records_of(CONTENT).is_empty());
    assert!(!typed.exported().records_of(CONTENT).is_empty(), "typed");
    assert!(empty.exported().records_of(CONTENT).is_empty(), "unstated");

    let refused = Lab::new("otlp-capture-refused");
    refused.write_config(
        ENDS,
        json!({ "telemetry": { "capture_content": format!("${{{CAPTURE}}}") } }),
    );
    let said = refused_before_a_run(
        &refused,
        &["run", "--config", CONFIG, "--prompt", PROMPT],
        &[(CAPTURE, "yes")],
    );
    assert_eq!(
        said,
        format!(
            "config: telemetry.capture_content (line 1): \"${{{CAPTURE}}}\" is refused: `{CAPTURE}` \
             holds neither `true` nor `false`, and isn't shown; fix it\n"
        )
    );
    assert!(!said.contains("yes"), "{said}");
}

/// A filled number is typed where the setting is one: `max_turns: ${TURNS}`
/// with `8` caps the run, and with `eight` is refused by `check` and `run`
/// alike without the value shown (R13).
#[test]
fn a_filled_number_is_typed_and_one_that_does_not_parse_is_refused_without_its_value() {
    let lab = Lab::new("otlp-typed-number");
    lab.write_config(
        ENDS,
        json!({ "run": { "max_turns": "${LABLET_TEST_TURNS}" } }),
    );
    let mut command = lab.lablet(&["check", "--config", CONFIG, "--resolved"]);
    command.env("LABLET_TEST_TURNS", "8");
    let typed = ran(command, "");
    assert_eq!(typed.code, Some(0), "{typed:?}");
    assert!(
        typed
            .stdout
            .contains("  max_turns: ${LABLET_TEST_TURNS} # environment\n"),
        "{typed:?}"
    );

    for args in [
        &["check", "--config", CONFIG][..],
        &["run", "--config", CONFIG, "--prompt", PROMPT][..],
    ] {
        let said = refused_before_a_run(&lab, args, &[("LABLET_TEST_TURNS", "eight")]);
        assert_eq!(
            said,
            "config: run.max_turns (line 1): \"${LABLET_TEST_TURNS}\" is refused: \
             `LABLET_TEST_TURNS` holds no whole number, and isn't shown; fix it\n",
            "{args:?}"
        );
        assert!(!said.contains("eight"), "{said}");
    }
}

/// A client key the config states is named on `own:` and its contents cut
/// whenever it's stated, with no endpoint named and nothing sent: the file
/// is a secret wherever the run exports.
#[test]
fn a_client_key_stated_with_no_endpoint_is_named_on_own_and_nothing_is_sent() {
    let lab = Lab::new("otlp-client-key-alone");
    lab.write("script.yaml", ENDS);
    lab.write(
        "client.key",
        "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/nwoy/HU++CXqI9EdVhC\n-----END PRIVATE KEY-----\n",
    );
    lab.write(
        "lablet.yaml",
        "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntelemetry:\n  otlp:\n    \
         tls:\n      client_key: client.key\n",
    );

    let checked = lab.run(&["check", "--config", "lablet.yaml"]);

    assert_eq!(checked.code, Some(0), "{checked:?}");
    for line in [
        "own:       telemetry.otlp.tls.client_key (line 9) -> its file's contents; cut\n",
        "telemetry: none named, so a run sends and writes nothing\n",
    ] {
        assert!(checked.stderr.contains(line), "{line}: {checked:?}");
    }
}

/// The pod block of the design, for a cluster that injects the OpenTelemetry
/// variables: each setting a `${VAR:-}`, and a second token for a command
/// that exports.
const POD: &str = "
telemetry:
  propagators: ${OTEL_PROPAGATORS:-}
  otlp:
    endpoint: ${OTEL_EXPORTER_OTLP_ENDPOINT:-}
    protocol: ${OTEL_EXPORTER_OTLP_PROTOCOL:-}
    headers: ${OTEL_EXPORTER_OTLP_HEADERS:-}
    tls:
      ca_file: ${OTEL_EXPORTER_OTLP_CERTIFICATE:-}
      client_cert: ${OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE:-}
      client_key: ${OTEL_EXPORTER_OTLP_CLIENT_KEY:-}
  resource: service.name=${OTEL_SERVICE_NAME:-},${OTEL_RESOURCE_ATTRIBUTES:-}
  capture_content: ${OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT:-}
tools:
  builtin:
    credentials:
      OTEL_EXPORTER_OTLP_HEADERS: ${COMMAND_OTLP_HEADERS:-}
";

/// Writes the pod block into `lab`'s `lablet.yaml`, under a fake model that
/// plays [`ENDS`], with `read_file` under the lab's root.
fn write_pod(lab: &Lab) {
    lab.write("script.yaml", ENDS);
    lab.write(
        "lablet.yaml",
        &format!(
            "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\n{}\n    root: work\n    enabled: [read_file]\n",
            POD.trim()
        ),
    );
}

/// The pod block as a child process with the cluster's variables set:
/// `lablet schema` takes it, the run exports to the receiver with the
/// headers the variables name, and `check` passes with `ca_file`,
/// `client_cert` and `client_key` naming readable PEM files, which it loads.
/// On a laptop, with none of the variables set, the same file sends nothing,
/// `check` names the unset credential on `given:` and nothing on `unread:`.
#[tokio::test(flavor = "multi_thread")]
async fn the_pod_block_passes_schema_exports_with_the_headers_the_variables_name_and_sends_nothing_on_a_laptop()
 {
    let receiver = Receiver::start(Mode::Answers).await;
    let lab = Lab::new("otlp-pod");
    write_pod(&lab);
    let ca = lab.write("ca.pem", receiver.ca_certificate());
    let certificate = lab.write("client.pem", receiver.client_certificate());
    let key = lab.write("client.key", receiver.client_key());
    let token = "pod-0123456789abcdef";
    let headers = format!("authorization=Bearer%20{token},x-tenant=acme");
    let endpoint = receiver.http_endpoint();
    let (ca, certificate, key) = (
        ca.display().to_string(),
        certificate.display().to_string(),
        key.display().to_string(),
    );
    let injected: Vec<(&str, &str)> = vec![
        ("OTEL_PROPAGATORS", "tracecontext,baggage"),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", &endpoint),
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf"),
        ("OTEL_EXPORTER_OTLP_HEADERS", &headers),
        ("OTEL_EXPORTER_OTLP_CERTIFICATE", &ca),
        ("OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE", &certificate),
        ("OTEL_EXPORTER_OTLP_CLIENT_KEY", &key),
        ("OTEL_SERVICE_NAME", "pod-service"),
        (
            "OTEL_RESOURCE_ATTRIBUTES",
            "k8s.pod.name=lablet-1,service.name=not-this",
        ),
        ("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT", "true"),
    ];

    // The schema takes the block: `--resolved` reads it back.
    let mut schema = lab.lablet(&["check", "--config", "lablet.yaml", "--resolved"]);
    schema.envs(injected.iter().copied());
    let checked = ran(schema, "");
    assert_eq!(checked.code, Some(0), "{checked:?}");
    assert!(
        checked
            .stdout
            .contains("endpoint: ${OTEL_EXPORTER_OTLP_ENDPOINT:-} # environment\n"),
        "{checked:?}"
    );
    assert!(
        checked
            .stderr
            .contains("own:       OTEL_EXPORTER_OTLP_HEADERS (line 11) -> telemetry.otlp.headers; no process inherits it"),
        "{checked:?}"
    );
    assert!(
        checked.stderr.contains(
            "own:       telemetry.otlp.tls.client_key (line 15) -> its file's contents; cut"
        ),
        "the key file was loaded: {checked:?}"
    );
    assert!(
        checked.stderr.contains(&format!(
            "telemetry: OTLP http/protobuf to {endpoint} (line 9)"
        )),
        "{checked:?}"
    );
    assert!(checked.stderr.contains("unread:    none\n"), "{checked:?}");
    assert!(!checked.stderr.contains(token), "{checked:?}");

    // The run exports to the receiver with the headers the variables name,
    // the resource the variables name, and its content.
    let mut run = lab.lablet(&["run", "--config", "lablet.yaml", "--prompt", PROMPT]);
    run.envs(injected.iter().copied());
    let run = ran(run, "");
    assert_eq!(run.code, Some(0), "{run:?}");
    let requests = received(&receiver, Over::Http);
    for request in &requests {
        assert_eq!(
            header(request, "authorization"),
            [format!("Bearer {token}")]
        );
        assert_eq!(header(request, "x-tenant"), ["acme"]);
    }
    let exported = receiver.exported().unwrap();
    for resource in exported.spans.iter().map(|span| &span.resource) {
        assert_eq!(resource.get(key::SERVICE_NAME), Some(&json!("pod-service")));
        assert_eq!(resource.get("k8s.pod.name"), Some(&json!("lablet-1")));
    }
    assert!(
        !exported
            .records_of("gen_ai.client.inference.operation.details")
            .is_empty(),
        "content captured"
    );

    the_pod_file_on_a_laptop(&receiver);
}

/// The pod file with none of the variables set: nothing is sent to
/// `receiver`, no file appears, the unset credential is named on `given:`,
/// and nothing is unread.
fn the_pod_file_on_a_laptop(receiver: &Receiver) {
    let laptop = Lab::new("otlp-pod-laptop");
    write_pod(&laptop);
    let before = receiver.requests().len();
    let quiet = laptop.run(&["run", "--config", "lablet.yaml", "--prompt", PROMPT]);
    assert_eq!(quiet.code, Some(0), "{quiet:?}");
    assert_eq!(receiver.requests().len(), before);
    assert_eq!(telemetry_files(&laptop), Vec::<String>::new());
    let checked = laptop.run(&["check", "--config", "lablet.yaml"]);
    assert_eq!(checked.code, Some(0), "{checked:?}");
    for line in [
        "own:       OTEL_EXPORTER_OTLP_HEADERS (line 11) (unset, not cut) -> telemetry.otlp.headers; no process inherits it",
        "given:     none: COMMAND_OTLP_HEADERS (line 21) is unset",
        "unread:    none",
        "telemetry: none named, so a run sends and writes nothing",
    ] {
        assert!(checked.stderr.contains(line), "{line}: {checked:?}");
    }
}
