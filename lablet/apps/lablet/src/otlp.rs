//! Where a run exports over the network, settled from the config and the
//! `OTEL_*` environment lablet inherits: a field the config states wins,
//! and a field it leaves out is the environment's, which the exporter reads
//! itself. What's settled here is only what the exporter can't settle
//! alone: whether a run has a network exporter at all, the transport it
//! speaks, and whether the environment's headers go with it.

use crate::config::{Config, Env, OtlpProtocol, Refusal};
use crate::export::{OtlpSettings, Signal, Transport};

/// The endpoint variable every signal falls back on.
const ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// The endpoint variables, generic and per signal. Any of them turns the
/// exporter on; which one each signal is sent to, the exporter resolves.
pub(crate) const ENDPOINT_VARIABLES: [&str; 3] = [
    ENDPOINT,
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
];

/// The transport, which the exporter reads only where the code lets it
/// choose, and lablet never does: the per-signal protocol variables aren't
/// read, since one transport serves both signals.
const PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";

/// The switch the specification gives the traces exporter, which the Rust
/// SDK doesn't read. `none` turns lablet's one network exporter off; any
/// other value leaves it on, and `OTEL_LOGS_EXPORTER` isn't read.
const TRACES_EXPORTER: &str = "OTEL_TRACES_EXPORTER";

/// The variables the exporter reads headers from, generic and per signal,
/// whose values are secrets whenever they're set.
pub(crate) const HEADER_VARIABLES: [&str; 3] = [
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
];

/// What the network exporter is built from, for `real`, a config with
/// `${VAR}` substituted, in the environment `env`, or `None` when the run
/// has no network exporter: `telemetry.otlp.enabled` is `false`,
/// `OTEL_TRACES_EXPORTER` is `none`, or neither the config nor the
/// environment names an endpoint. The endpoint is set only when the config
/// states one, and an endpoint of nothing, written so or given by a
/// variable, states none, as a variable set to nothing is unset: so the
/// exporter resolves an unstated one from the environment, by signal, and
/// the environment's headers are kept off every export to the config's
/// endpoint.
///
/// A refusal names the key as `written`, the same config before
/// substitution, writes it: `telemetry.otlp.enabled: false` beside an
/// endpoint, which it would make a setting without effect, and a protocol
/// from the environment that lablet can't send. Nothing an endpoint
/// variable holds is in a message.
pub(crate) fn settings(
    written: &Config,
    real: &Config,
    env: Env<'_>,
) -> Result<Option<OtlpSettings>, Refusal> {
    let otlp = &real.telemetry.otlp;
    let endpoint = otlp
        .endpoint
        .clone()
        .filter(|endpoint| !endpoint.is_empty());
    if !otlp.enabled {
        if endpoint.is_some() {
            let endpoint = written
                .written_text("telemetry.otlp.endpoint")
                .unwrap_or_default();
            return Err(Refusal::invalid(
                "telemetry.otlp.enabled",
                format!(
                    "it turns the network exporter off, and `telemetry.otlp.endpoint: \
                     {endpoint}` turns it on; a config that wants it off states no endpoint"
                ),
            ));
        }
        return Ok(None);
    }
    if set(env, TRACES_EXPORTER).is_some_and(|value| value.trim().eq_ignore_ascii_case("none")) {
        return Ok(None);
    }
    if endpoint.is_none()
        && !ENDPOINT_VARIABLES
            .iter()
            .any(|variable| set(env, variable).is_some())
    {
        return Ok(None);
    }
    Ok(Some(OtlpSettings {
        transport: transport(otlp.protocol, env)?,
        strip_environment_headers: endpoint.is_some(),
        endpoint,
        headers: otlp.headers.clone().into_iter().collect(),
    }))
}

/// The variable the exporter read the endpoint of `signal` from, when the
/// config states none: the signal's own when it's set to something, else
/// the generic one. Only which is set is read, never what it holds.
pub(crate) fn endpoint_variable(signal: Signal, env: Env<'_>) -> &'static str {
    let own = signal.endpoint_variable();
    if set(env, own).is_some() {
        own
    } else {
        ENDPOINT
    }
}

/// The transport: the config's protocol when it states one, else the
/// environment's, else gRPC. A protocol lablet can't send is refused by
/// the key, naming the variable and what it holds, which is no secret.
fn transport(stated: Option<OtlpProtocol>, env: Env<'_>) -> Result<Transport, Refusal> {
    match stated {
        Some(OtlpProtocol::Grpc) => return Ok(Transport::Grpc),
        Some(OtlpProtocol::Http) => return Ok(Transport::HttpProtobuf),
        None => {}
    }
    let Some(value) = set(env, PROTOCOL).filter(|value| !value.trim().is_empty()) else {
        return Ok(Transport::Grpc);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "grpc" => Ok(Transport::Grpc),
        "http/protobuf" => Ok(Transport::HttpProtobuf),
        _ => Err(Refusal::invalid(
            "telemetry.otlp.protocol",
            format!(
                "`{PROTOCOL}` holds {value:?}, which lablet can't send; it sends `grpc` and \
                 `http/protobuf`"
            ),
        )),
    }
}

/// What `variable` holds when it's set to something, read as the exporter
/// reads it: a variable set to nothing is unset.
fn set(env: Env<'_>, variable: &str) -> Option<String> {
    env(variable)
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests;
