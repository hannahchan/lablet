//! The exporter's variables, read over environments the tests state.

use super::*;
use crate::otel_env::tests::warned;

fn read(held: &[(&str, &str)]) -> (Exporter, Vec<String>) {
    warned(held, Exporter::read)
}

fn pairs(headers: Option<&Vec<(String, Hidden)>>) -> Vec<(&str, &str)> {
    headers
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.as_str(), value.0.as_str()))
        .collect()
}

#[test]
fn an_environment_that_sets_nothing_gives_the_specifications_defaults() {
    let (exporter, warnings) = read(&[]);

    assert!(warnings.is_empty());
    assert_eq!(exporter, Exporter::default());
    assert!(!exporter.sdk_disabled);
    assert_eq!(exporter.capture_content, None);
    for inherited in [&exporter.traces, &exporter.logs] {
        assert_eq!(
            *inherited,
            Inherited {
                selected: None,
                protocol: None,
                endpoint: None,
                headers: None,
                header_names: Vec::new(),
                timeout: Duration::from_secs(10),
                gzip: false,
                insecure: false,
                certificate: None,
                client_key: None,
                client_certificate: None,
            }
        );
    }
}

#[test]
fn each_signal_takes_its_own_exporter_selector() {
    let (traces_off, _) = read(&[("OTEL_TRACES_EXPORTER", "none")]);
    let (logs_off, _) = read(&[("OTEL_LOGS_EXPORTER", "NONE")]);

    assert_eq!(
        (traces_off.traces.selected, traces_off.logs.selected),
        (Some(false), None)
    );
    assert_eq!(
        (logs_off.traces.selected, logs_off.logs.selected),
        (None, Some(false))
    );
}

#[test]
fn none_beside_otlp_is_on_and_a_value_lablet_cannot_serve_is_ignored_with_a_warning() {
    let (exporter, warnings) = read(&[
        ("OTEL_TRACES_EXPORTER", "none, OTLP"),
        ("OTEL_LOGS_EXPORTER", "zipkin,otlp,otlp"),
    ]);

    assert_eq!(exporter.traces.selected, Some(true));
    assert_eq!(exporter.logs.selected, Some(true));
    assert_eq!(
        warnings,
        [
            "`OTEL_TRACES_EXPORTER` holds `none`, which is beside another value, so it's ignored",
            "`OTEL_LOGS_EXPORTER` holds `zipkin`, which isn't one lablet serves, so it's ignored",
        ]
    );
}

#[test]
fn a_selector_list_naming_nothing_lablet_serves_is_read_as_unset() {
    for value in ["console", "otlp/stdout, logging", "prometheus"] {
        let (exporter, warnings) = read(&[("OTEL_TRACES_EXPORTER", value)]);

        assert_eq!(exporter.traces.selected, None, "{value}");
        assert!(!warnings.is_empty(), "{value}");
    }
}

#[test]
fn each_signal_takes_its_own_protocol_variable_before_the_generic_one() {
    let (exporter, warnings) = read(&[
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
        ("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "HTTP/JSON"),
    ]);

    assert!(warnings.is_empty());
    assert_eq!(exporter.traces.protocol, Some(Transport::Grpc));
    assert_eq!(exporter.logs.protocol, Some(Transport::HttpJson));
}

#[test]
fn an_unknown_protocol_is_ignored_with_a_warning_and_the_next_source_decides() {
    let (exporter, warnings) = read(&[
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf"),
        ("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "http/thrift"),
        ("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "http"),
    ]);

    assert_eq!(exporter.traces.protocol, Some(Transport::HttpProtobuf));
    assert_eq!(exporter.logs.protocol, Some(Transport::HttpProtobuf));
    assert_eq!(
        warnings,
        [
            "`OTEL_EXPORTER_OTLP_TRACES_PROTOCOL` holds `http/thrift`, which isn't one lablet \
             serves, so it's ignored",
            "`OTEL_EXPORTER_OTLP_LOGS_PROTOCOL` holds `http`, which isn't one lablet serves, so \
             it's ignored",
        ]
    );
}

#[test]
fn a_signals_endpoint_is_taken_as_written_and_the_generic_one_marked_for_the_path() {
    let (exporter, _) = read(&[
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://generic:4318"),
        (
            "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
            "http://logs:4318/custom",
        ),
    ]);

    let traces = exporter.traces.endpoint.unwrap();
    let logs = exporter.logs.endpoint.unwrap();
    assert_eq!(
        (traces.variable, traces.generic, traces.text.0.as_str()),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", true, "http://generic:4318")
    );
    assert_eq!(
        (logs.variable, logs.generic, logs.text.0.as_str()),
        (
            "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
            false,
            "http://logs:4318/custom"
        )
    );
}

#[test]
fn an_empty_signal_header_variable_leaves_the_generic_one() {
    let (exporter, warnings) = read(&[
        (
            "OTEL_EXPORTER_OTLP_HEADERS",
            "authorization=Bearer%20generic",
        ),
        ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", ""),
        ("OTEL_EXPORTER_OTLP_LOGS_HEADERS", "x-logs=own"),
    ]);

    assert!(warnings.is_empty());
    assert_eq!(
        pairs(exporter.traces.headers.as_ref()),
        [("authorization", "Bearer generic")],
        "the crate would read the empty variable and send no header"
    );
    assert_eq!(pairs(exporter.logs.headers.as_ref()), [("x-logs", "own")]);
}

#[test]
fn the_names_taken_off_are_every_name_either_header_variable_sets() {
    let (exporter, _) = read(&[
        (
            "OTEL_EXPORTER_OTLP_HEADERS",
            "Authorization=generic,x-both=g",
        ),
        ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", ""),
        ("OTEL_EXPORTER_OTLP_LOGS_HEADERS", "x-logs=own,x-both=l"),
    ]);

    assert_eq!(exporter.traces.header_names, ["authorization", "x-both"]);
    assert_eq!(
        exporter.logs.header_names,
        ["x-logs", "x-both", "authorization"]
    );
}

#[test]
fn a_header_pair_that_is_not_one_is_dropped_with_a_warning_that_holds_no_value() {
    let (exporter, warnings) = read(&[(
        "OTEL_EXPORTER_OTLP_HEADERS",
        "Bearer hunter2hunter2,not a name=hunter2hunter2,x-token=hunter2%0Ahunter2,x-ok=fine",
    )]);

    assert_eq!(pairs(exporter.traces.headers.as_ref()), [("x-ok", "fine")]);
    assert_eq!(exporter.traces.header_names, ["x-ok"]);
    assert_eq!(
        warnings,
        [
            "`OTEL_EXPORTER_OTLP_HEADERS` holds a value that has a pair that isn't `name=value` \
             with both, which is dropped",
            "`OTEL_EXPORTER_OTLP_HEADERS` holds a value that has a header whose name isn't one a \
             header may have, which is dropped",
            "`OTEL_EXPORTER_OTLP_HEADERS` holds a value that has a value of the header `x-token` \
             that isn't one a header may have, which is dropped",
        ]
    );
}

#[test]
fn a_header_variable_that_leaves_no_header_is_unset_and_the_generic_one_decides() {
    let (exporter, _) = read(&[
        ("OTEL_EXPORTER_OTLP_HEADERS", "x-generic=g"),
        ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", "nothing usable"),
    ]);

    assert_eq!(
        pairs(exporter.traces.headers.as_ref()),
        [("x-generic", "g")]
    );
}

#[test]
fn an_empty_signal_insecure_variable_leaves_the_generic_one() {
    let (exporter, _) = read(&[
        ("OTEL_EXPORTER_OTLP_INSECURE", "true"),
        ("OTEL_EXPORTER_OTLP_TRACES_INSECURE", ""),
        ("OTEL_EXPORTER_OTLP_LOGS_INSECURE", "false"),
    ]);

    assert!(exporter.traces.insecure);
    assert!(!exporter.logs.insecure);
}

#[test]
fn a_signals_insecure_variable_that_is_not_a_boolean_reads_as_false_with_a_warning() {
    let (exporter, warnings) = read(&[
        ("OTEL_EXPORTER_OTLP_INSECURE", "TRUE"),
        ("OTEL_EXPORTER_OTLP_TRACES_INSECURE", "yes"),
    ]);

    assert!(!exporter.traces.insecure);
    assert!(exporter.logs.insecure);
    assert_eq!(
        warnings,
        [
            "`OTEL_EXPORTER_OTLP_TRACES_INSECURE` holds `yes`, which isn't `true` or `false`, so \
             it's read as `false`"
        ]
    );
}

#[test]
fn a_timeout_that_is_not_a_number_is_ignored_with_a_warning() {
    let (exporter, warnings) = read(&[
        ("OTEL_EXPORTER_OTLP_TIMEOUT", "2500"),
        ("OTEL_EXPORTER_OTLP_TRACES_TIMEOUT", "5s"),
        ("OTEL_EXPORTER_OTLP_LOGS_TIMEOUT", "750"),
    ]);

    assert_eq!(exporter.traces.timeout, Duration::from_millis(2_500));
    assert_eq!(exporter.logs.timeout, Duration::from_millis(750));
    assert_eq!(
        warnings,
        [
            "`OTEL_EXPORTER_OTLP_TRACES_TIMEOUT` holds `5s`, which isn't a whole number of \
             milliseconds, so it's ignored"
        ]
    );
}

#[test]
fn gzip_is_read_for_each_signal_and_another_compression_is_ignored_with_a_warning() {
    let (exporter, warnings) = read(&[
        ("OTEL_EXPORTER_OTLP_COMPRESSION", "GZIP"),
        ("OTEL_EXPORTER_OTLP_TRACES_COMPRESSION", "zstd"),
        ("OTEL_EXPORTER_OTLP_LOGS_COMPRESSION", "none"),
    ]);

    assert!(
        exporter.traces.gzip,
        "zstd is ignored, and the generic gzip decides"
    );
    assert!(!exporter.logs.gzip);
    assert_eq!(
        warnings,
        [
            "`OTEL_EXPORTER_OTLP_TRACES_COMPRESSION` holds `zstd`, which isn't one lablet \
             serves, so it's ignored"
        ]
    );
}

/// The exporter reads a compression untrimmed, and lablet, which can't
/// state none on its builder, reads it the same way, so a padded `none`
/// is ignored by both and the generic gzip decides for both.
#[test]
fn a_compression_with_spaces_is_ignored_as_the_exporter_ignores_it() {
    let (exporter, warnings) = read(&[
        ("OTEL_EXPORTER_OTLP_COMPRESSION", "gzip"),
        ("OTEL_EXPORTER_OTLP_TRACES_COMPRESSION", " none"),
        ("OTEL_EXPORTER_OTLP_LOGS_COMPRESSION", "gzip "),
    ]);

    assert!(exporter.traces.gzip);
    assert!(exporter.logs.gzip);
    assert_eq!(
        warnings,
        [
            "`OTEL_EXPORTER_OTLP_TRACES_COMPRESSION` holds ` none`, which isn't one lablet \
             serves, so it's ignored",
            "`OTEL_EXPORTER_OTLP_LOGS_COMPRESSION` holds `gzip `, which isn't one lablet \
             serves, so it's ignored",
        ]
    );
}

#[test]
fn each_tls_file_is_the_signals_own_else_the_generic_one() {
    let (exporter, _) = read(&[
        ("OTEL_EXPORTER_OTLP_CERTIFICATE", "/ca.pem"),
        ("OTEL_EXPORTER_OTLP_LOGS_CERTIFICATE", "/logs-ca.pem"),
        ("OTEL_EXPORTER_OTLP_CLIENT_KEY", "/client.key"),
        (
            "OTEL_EXPORTER_OTLP_TRACES_CLIENT_CERTIFICATE",
            "/traces.pem",
        ),
    ]);

    let shown = |file: &Option<Variable<PathBuf>>| {
        file.as_ref()
            .map(|file| (file.name, file.value.display().to_string()))
    };
    assert_eq!(
        shown(&exporter.traces.certificate),
        Some(("OTEL_EXPORTER_OTLP_CERTIFICATE", "/ca.pem".to_owned()))
    );
    assert_eq!(
        shown(&exporter.logs.certificate),
        Some((
            "OTEL_EXPORTER_OTLP_LOGS_CERTIFICATE",
            "/logs-ca.pem".to_owned()
        ))
    );
    assert_eq!(
        shown(&exporter.logs.client_key),
        Some(("OTEL_EXPORTER_OTLP_CLIENT_KEY", "/client.key".to_owned()))
    );
    assert_eq!(
        shown(&exporter.traces.client_certificate),
        Some((
            "OTEL_EXPORTER_OTLP_TRACES_CLIENT_CERTIFICATE",
            "/traces.pem".to_owned()
        ))
    );
    assert_eq!(shown(&exporter.logs.client_certificate), None);
}

#[test]
fn otel_sdk_disabled_is_a_specification_boolean() {
    for (value, disabled) in [
        ("true", true),
        ("TRUE", true),
        ("false", false),
        ("", false),
    ] {
        let (exporter, warnings) = read(&[("OTEL_SDK_DISABLED", value)]);

        assert_eq!(exporter.sdk_disabled, disabled, "{value}");
        assert!(warnings.is_empty(), "{value}");
    }
    let (exporter, warnings) = read(&[("OTEL_SDK_DISABLED", "1")]);
    assert!(!exporter.sdk_disabled);
    assert_eq!(warnings.len(), 1);
    assert!(Exporter::sdk_disabled_in(&|name: &str| {
        (name == "OTEL_SDK_DISABLED").then(|| "True".into())
    }));
}

#[test]
fn the_genai_capture_variable_is_parsed_as_a_specification_boolean() {
    let capture = |value: &str| {
        warned(
            &[("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT", value)],
            Exporter::read,
        )
    };

    let (on, quiet) = capture("TRUE");
    assert_eq!((on.capture_content, quiet.len()), (Some(true), 0));
    let (off, quiet) = capture("false");
    assert_eq!((off.capture_content, quiet.len()), (Some(false), 0));
    let (empty, quiet) = capture("");
    assert_eq!((empty.capture_content, quiet.len()), (None, 0));
    let (one, warnings) = capture("1");
    assert_eq!(one.capture_content, None, "so content stays off");
    assert_eq!(
        warnings,
        [
            "`OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` holds `1`, which isn't `true` \
             or `false`, so it's read as `false`"
        ]
    );
}
