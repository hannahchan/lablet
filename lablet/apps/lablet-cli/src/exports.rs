//! What a checked config's telemetry is built from, read from the config
//! and the SDK's environment between the two halves of `lablet-prepare`'s
//! check, and its building, so that the kernel reads no SDK setting and the
//! command line's root alone builds an SDK. What the seam still reads of
//! the environment and sends, the OTLP header variables and the endpoint
//! variables' user information, is handed in to be cut, since every command
//! still inherits those variables.

use lablet_config::{Config, ConfigError, KeyPath, shown};
use lablet_otel_env::{ENDPOINT_VARIABLES, Env, HEADER_VARIABLES};
use lablet_prepare::{BuildError, Prepared, Settled};
use lablet_run::telemetry::SCOPE;
use lablet_run::telemetry::generated::SCHEMA_URL;
use lablet_secrets::{Handed, HandedKind};
use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::Resource;
use secrecy::SecretString;

use crate::export::{self, FileTarget, OtelBuildError, OtlpSettings, Signal, Telemetry};
use crate::input;
use crate::otel_env::{Exporter, OtelEnv, Sdk};
use crate::otlp;
use crate::propagation::{self, Inbound};

/// The version of lablet, the workspace's, which the instrumentation scope
/// names.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What a checked config's telemetry is built from.
pub struct Exports {
    /// Where the file exporter writes, when the run has one.
    pub target: Option<FileTarget>,
    /// What the network exporter is built from, when the run has one.
    pub otlp: Option<OtlpSettings>,
    /// Whether `OTEL_SDK_DISABLED` turns all telemetry off.
    pub disabled: bool,
    /// The sampler, the span limits and the batch processors' settings.
    pub sdk: Sdk,
    /// What every export describes itself with.
    pub resource: Resource,
}

/// The check gone on from `settled`, with the SDK's settings read from the
/// config and from `env`, once, here, so a check and a build warn of the
/// same values and agree on what they come to: the check's end, what the
/// telemetry is built from, and the context every run starts from. The
/// values the seam reads and sends, and what a client key's file holds,
/// are handed on to be cut from what a tool returns.
///
/// # Errors
///
/// Returns the refusal of what the network exporter would refuse, but for
/// what only making it finds (see [`export::validate`]).
pub fn settle(settled: Settled, env: Env<'_>) -> Result<(Prepared, Exports, Inbound), BuildError> {
    let OtelEnv {
        sdk,
        exporter,
        context,
    } = OtelEnv::read(env);
    let config = settled.config();
    let resource = export::resource(
        VERSION,
        config.telemetry.resource.clone().into_iter().collect(),
        &context,
    );
    let inbound = propagation::inbound(&context);
    let otlp = network(config, &exporter)?;
    let disabled = exporter.sdk_disabled;
    let target = file_target(config, disabled);
    let read: Vec<(&str, SecretString, HandedKind)> = HEADER_VARIABLES
        .iter()
        .map(|name| (*name, HandedKind::Headers))
        .chain(
            ENDPOINT_VARIABLES
                .iter()
                .map(|name| (*name, HandedKind::Url)),
        )
        .filter_map(|(name, kind)| {
            // The environment is lossily read as a command's output is, so
            // a value that isn't UTF-8 is cut from the output of the same
            // bytes.
            let value = env(name)?;
            Some((
                name,
                SecretString::from(value.to_string_lossy().into_owned()),
                kind,
            ))
        })
        .collect();
    let client_keys = otlp
        .as_ref()
        .map(OtlpSettings::client_keys)
        .unwrap_or_default();
    let handed: Vec<Handed<'_>> = read
        .iter()
        .map(|(name, value, kind)| Handed {
            name,
            value,
            kind: *kind,
        })
        .chain(client_keys.iter().map(|(name, contents)| Handed {
            name,
            value: contents,
            kind: HandedKind::Contents,
        }))
        .collect();
    let prepared = settled.finish(lablet_prepare::Telemetry {
        file: match &target {
            Some(FileTarget::Path(path)) => Some(path.clone()),
            Some(FileTarget::Stderr) | None => None,
        },
        handed: &handed,
    });
    drop(handed);
    drop(read);
    drop(client_keys);
    Ok((
        prepared,
        Exports {
            target,
            otlp,
            disabled,
            sdk,
            resource,
        },
        inbound,
    ))
}

impl Exports {
    /// The variable each signal's endpoint was read from, traces then logs,
    /// for a refusal of what only making the network exporter finds.
    #[must_use]
    pub fn endpoint_variables(&self) -> [Option<&'static str>; 2] {
        self.otlp
            .as_ref()
            .map_or([None; 2], OtlpSettings::endpoint_variables)
    }

    /// The telemetry of every run, under lablet's instrumentation scope.
    ///
    /// # Errors
    ///
    /// Returns what only making the network exporter finds: trust roots
    /// that can't be loaded, or other TLS to the collector that can't be
    /// set up, and an HTTP client that can't be made.
    pub fn telemetry(self) -> Result<Telemetry, OtelBuildError> {
        let Self {
            target,
            otlp,
            disabled,
            sdk,
            resource,
        } = self;
        let scope = InstrumentationScope::builder(SCOPE)
            .with_version(VERSION)
            .with_schema_url(SCHEMA_URL)
            .build();
        let mut telemetry = Telemetry::builder(scope).resource(resource).sdk(sdk);
        if disabled {
            telemetry = telemetry.disabled();
        }
        if let Some(target) = target {
            telemetry = telemetry.file(target);
        }
        if let Some(settings) = otlp {
            telemetry = telemetry.otlp(settings);
        }
        telemetry.build()
    }
}

/// Whether a `Lablet` built from `config`, which was filled, writes its
/// telemetry to standard error, which a `telemetry.file.path` of `-` does
/// unless `OTEL_SDK_DISABLED` turns all telemetry off. It's the answer a
/// build comes to, for a caller that has to know before the build: one that
/// shares standard error with the telemetry says nothing of its own there,
/// and a log that reports on the build is installed before it.
#[must_use]
pub fn telemetry_on_stderr(config: &Config) -> bool {
    telemetry_on_stderr_in(config, &input::variable)
}

/// [`telemetry_on_stderr`], where `env` is what the seam reads of the
/// environment.
pub(crate) fn telemetry_on_stderr_in(config: &Config, env: Env<'_>) -> bool {
    // Whether the network exporter is on changes nothing about `-`, so the
    // answer is read from the path and `OTEL_SDK_DISABLED` alone, with no
    // other variable read and warned about before the log is installed.
    let disabled = Exporter::sdk_disabled_in(env);
    file_target(config, disabled) == Some(FileTarget::Stderr)
}

/// Where the telemetry file of `config` goes, and `None` when it writes no
/// file: a null path, or `OTEL_SDK_DISABLED` turning all telemetry off,
/// which `disabled` says. Only a path that's `-` whole is standard error,
/// so `-/` names a file.
fn file_target(config: &Config, disabled: bool) -> Option<FileTarget> {
    match &config.telemetry.file.path {
        _ if disabled => None,
        None => None,
        Some(path) if path.as_os_str() == "-" => Some(FileTarget::Stderr),
        Some(path) => Some(FileTarget::Path(path.clone())),
    }
}

/// The refusal of what the network exporter couldn't be built from, shown
/// from `config` as the file wrote it: a header by its key alone, since
/// every header value is a secret, and the endpoint as the config writes
/// it, or no value when it states none or one that holds an `@`. User
/// information is a secret, and where it ends can't be told when it holds
/// an unencoded `/`, `?` or `#`. A refusal of an endpoint a variable named,
/// or of TLS to it, names the variable, which `variables` gives for each
/// signal, and shows nothing of what it holds.
#[must_use]
pub fn otlp_refused(
    config: &Config,
    variables: [Option<&'static str>; 2],
    error: OtelBuildError,
) -> BuildError {
    const KEY: &str = "telemetry.otlp.endpoint";
    let variable = |signal| match signal {
        Signal::Traces => variables[0],
        Signal::Logs => variables[1],
    };
    let from_environment = |variable: &str, what: &str| {
        BuildError::Config(ConfigError::Invalid {
            key: KEY.to_owned(),
            place: config.place_of(KEY),
            value: None,
            reason: format!(
                "`{variable}`, which is read since the config states no endpoint, {what}"
            ),
        })
    };
    let reason = match error {
        OtelBuildError::Header { name, reason } => {
            let key = KeyPath::of("telemetry.otlp.headers").key(&name);
            return BuildError::Config(ConfigError::Invalid {
                place: config.place_at(&key),
                value: None,
                key: key.to_string(),
                reason: reason.to_owned(),
            });
        }
        OtelBuildError::Endpoint { signal } => match variable(signal) {
            Some(variable) => {
                return from_environment(variable, "holds what isn't a URL the exporter accepts");
            }
            None => "it isn't a URL the exporter accepts".to_owned(),
        },
        OtelBuildError::Exporter { reason, .. } => {
            format!("the exporter couldn't be made: {reason}")
        }
        OtelBuildError::Tls { signal, reason } => match variable(signal) {
            Some(variable) => {
                return from_environment(
                    variable,
                    &format!("names the collector, and TLS to it couldn't be set up: {reason}"),
                );
            }
            None => format!("TLS to the collector couldn't be set up: {reason}"),
        },
        OtelBuildError::HttpClient { reason, .. } => {
            format!("the HTTP client couldn't be made: {reason}")
        }
    };
    let key = KeyPath::of(KEY);
    let value = config
        .written_text(KEY)
        .filter(|_| config.telemetry.otlp.endpoint.is_some())
        .map(serde_json::Value::String);
    BuildError::Config(ConfigError::Invalid {
        place: config.place_of(KEY),
        value: shown(&key, value.as_ref(), config.filled()),
        key: KEY.to_owned(),
        reason,
    })
}

/// What the network exporters of `config` are built from, with each refusal
/// shown as the file wrote it. What an exporter would refuse is refused
/// here, so a check refuses what a build refuses but for what only making
/// the exporter finds, trust roots that can't be loaded or other TLS that
/// can't be set up, and the HTTP client (see `validate`); a check installs
/// no exporter.
fn network(config: &Config, exporter: &Exporter) -> Result<Option<OtlpSettings>, BuildError> {
    let otlp = otlp::settings(config, exporter)?;
    if let Some(settings) = &otlp {
        export::validate(settings)
            .map_err(|error| otlp_refused(config, settings.endpoint_variables(), error))?;
    }
    Ok(otlp)
}

#[cfg(test)]
mod tests;
