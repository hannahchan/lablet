//! The one place lablet reads the process's environment: the command line's
//! input module. It fills the file's `${VAR}`s, through a function the
//! config's fill is handed, reads the context variables the configured
//! propagators' fields name, as a process convention, takes the environment
//! every process a run starts begins from, once, as the base the
//! composition hands in, and says which names of the retired roster are
//! set, by name alone. Nothing else in lablet reads a variable: the library
//! takes a `Config` its host filled and the environment its host hands in,
//! and the root `clippy.toml` holds every other crate to that.

use std::ffi::OsString;

/// What the variable `name` holds in the process's environment, for the
/// config's fill and the context variables.
#[must_use]
pub fn variable(name: &str) -> Option<OsString> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the input module is where the command line reads its environment"
    )]
    std::env::var_os(name)
}

/// Whether the variable `name` is set in the process's environment, to
/// anything: what `check`'s `unread:` line asks of each name on the roster,
/// never reading the value.
#[must_use]
pub fn is_set(name: &str) -> bool {
    variable(name).is_some()
}

/// The process's whole environment, as the base every process a run
/// starts begins from.
#[must_use]
pub fn process_environment() -> Vec<(OsString, OsString)> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the input module is where the command line reads its environment"
    )]
    std::env::vars_os().collect()
}

/// A variable lablet read before the caller owned the config and reads no
/// more, with the setting a `${VAR}` maps it onto, when one does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retired {
    /// The variable.
    pub name: &'static str,
    /// The setting the variable's value belongs in, written as
    /// `${NAME}`, or `None` when no setting takes it yet.
    pub field: Option<&'static str>,
}

/// The roster `check`'s `unread:` line names the set ones of: every
/// `OTEL_*` variable the command line read before its settings came from
/// the config alone, less the context variables, which it still reads, and
/// the compression variables, which the exporter reads for itself, and the
/// provider's key variable. The metrics variables aren't on it, since
/// lablet never read them: it emits no metrics. It's retired at the
/// release.
pub const UNREAD: [Retired; 46] = [
    Retired {
        name: "ANTHROPIC_API_KEY",
        field: Some("model.api_key"),
    },
    Retired {
        name: "OTEL_SDK_DISABLED",
        field: None,
    },
    Retired {
        name: "OTEL_TRACES_EXPORTER",
        field: None,
    },
    Retired {
        name: "OTEL_LOGS_EXPORTER",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_PROTOCOL",
        field: Some("telemetry.otlp.protocol"),
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_ENDPOINT",
        field: Some("telemetry.otlp.endpoint"),
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_HEADERS",
        field: Some("telemetry.otlp.headers"),
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_TIMEOUT",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_LOGS_TIMEOUT",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_INSECURE",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_TRACES_INSECURE",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_LOGS_INSECURE",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_CERTIFICATE",
        field: Some("telemetry.otlp.tls.ca_file"),
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_LOGS_CERTIFICATE",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_CLIENT_KEY",
        field: Some("telemetry.otlp.tls.client_key"),
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_TRACES_CLIENT_KEY",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_LOGS_CLIENT_KEY",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE",
        field: Some("telemetry.otlp.tls.client_cert"),
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_TRACES_CLIENT_CERTIFICATE",
        field: None,
    },
    Retired {
        name: "OTEL_EXPORTER_OTLP_LOGS_CLIENT_CERTIFICATE",
        field: None,
    },
    Retired {
        name: "OTEL_TRACES_SAMPLER",
        field: None,
    },
    Retired {
        name: "OTEL_TRACES_SAMPLER_ARG",
        field: None,
    },
    Retired {
        name: "OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT",
        field: None,
    },
    Retired {
        name: "OTEL_SPAN_EVENT_COUNT_LIMIT",
        field: None,
    },
    Retired {
        name: "OTEL_SPAN_LINK_COUNT_LIMIT",
        field: None,
    },
    Retired {
        name: "OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT",
        field: None,
    },
    Retired {
        name: "OTEL_LINK_ATTRIBUTE_COUNT_LIMIT",
        field: None,
    },
    Retired {
        name: "OTEL_ATTRIBUTE_COUNT_LIMIT",
        field: None,
    },
    Retired {
        name: "OTEL_BSP_SCHEDULE_DELAY",
        field: None,
    },
    Retired {
        name: "OTEL_BSP_MAX_QUEUE_SIZE",
        field: None,
    },
    Retired {
        name: "OTEL_BSP_MAX_EXPORT_BATCH_SIZE",
        field: None,
    },
    Retired {
        name: "OTEL_BLRP_SCHEDULE_DELAY",
        field: None,
    },
    Retired {
        name: "OTEL_BLRP_MAX_QUEUE_SIZE",
        field: None,
    },
    Retired {
        name: "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE",
        field: None,
    },
    Retired {
        name: "OTEL_SERVICE_NAME",
        field: Some("telemetry.resource"),
    },
    Retired {
        name: "OTEL_RESOURCE_ATTRIBUTES",
        field: Some("telemetry.resource"),
    },
    Retired {
        name: "OTEL_PROPAGATORS",
        field: Some("telemetry.propagators"),
    },
    Retired {
        name: "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT",
        field: Some("telemetry.capture_content"),
    },
];
