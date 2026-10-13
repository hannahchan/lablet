//! What a checked config's telemetry is built from, read from the config's
//! `telemetry` section alone between the two halves of `lablet-prepare`'s
//! check, and its building, so that the kernel reads no SDK setting and the
//! command line's root alone builds an SDK. Nothing of the environment is
//! read here: a setting comes from a variable only where the file wrote a
//! `${VAR}`, and every value the crates would read from the environment for
//! themselves is stated on their builders.

use lablet_config::{Config, ConfigError, KeyPath, Place, Secret, shown, user_information};
use lablet_prepare::{BuildError, Prepared, Settled};
use lablet_run::telemetry::SCOPE;
use lablet_run::telemetry::generated::SCHEMA_URL;
use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::Resource;
use secrecy::{ExposeSecret as _, SecretString};

use crate::export::{self, FileTarget, OtelBuildError, OtlpSettings, Telemetry, Transport};
use crate::otel_env::Sdk;
use crate::otlp;

/// The version of lablet, the workspace's, which the instrumentation scope
/// names.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The compression variables the 0.33 exporter reads for itself, which
/// lablet can't state none for on its builders: the one read of the
/// environment a run makes beyond the config, said on `check`'s
/// `telemetry:` line when an endpoint is named.
pub const COMPRESSION_VARIABLES: [&str; 3] = [
    "OTEL_EXPORTER_OTLP_COMPRESSION",
    "OTEL_EXPORTER_OTLP_TRACES_COMPRESSION",
    "OTEL_EXPORTER_OTLP_LOGS_COMPRESSION",
];

/// What a checked config's telemetry is built from.
pub struct Exports {
    /// Where the file exporter writes, when the run has one.
    pub target: Option<FileTarget>,
    /// What the network exporter is built from, when the run has one.
    pub otlp: Option<OtlpSettings>,
    /// What every export describes itself with.
    pub resource: Resource,
}

/// The check gone on from `settled`, with the telemetry's settings read
/// from the config, once, here, so a check and a build agree on what they
/// come to: the check's end and what the telemetry is built from. What a
/// client key's file holds is handed on to be cut from what a tool returns
/// whenever the config states the key, endpoint or none.
///
/// # Errors
///
/// Returns the refusal of a client key's file that can't be read, whether
/// or not an endpoint is named, of a TLS file that can't be loaded, and of
/// what the network exporter would refuse but for what only making it
/// finds (see `export::validate`).
pub fn settle(settled: Settled) -> Result<(Prepared, Exports), BuildError> {
    let config = settled.config();
    let resource = export::resource(VERSION, config.telemetry.resource.pairs());
    let key = otlp::client_key(config)?;
    let otlp = network(config, key.as_ref())?;
    let target = file_target(config);
    let client_key = key.map(|key| Secret::new(key.expose_secret()));
    let prepared = settled.finish(lablet_prepare::Telemetry {
        file: match &target {
            Some(FileTarget::Path(path)) => Some(path.clone()),
            Some(FileTarget::Stderr) | None => None,
        },
        client_key: client_key.as_ref(),
    });
    drop(client_key);
    Ok((
        prepared,
        Exports {
            target,
            otlp,
            resource,
        },
    ))
}

impl Exports {
    /// The telemetry of every run, under lablet's instrumentation scope,
    /// with the specification's defaults stated on every builder: the
    /// sampler, parent-based always on, the span limits and the batch
    /// processors' settings, which the crates would otherwise read from the
    /// environment.
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
            resource,
        } = self;
        let scope = InstrumentationScope::builder(SCOPE)
            .with_version(VERSION)
            .with_schema_url(SCHEMA_URL)
            .build();
        let mut telemetry = Telemetry::builder(scope)
            .resource(resource)
            .sdk(Sdk::default());
        if let Some(target) = target {
            telemetry = telemetry.file(target);
        }
        if let Some(settings) = otlp {
            telemetry = telemetry.otlp(settings);
        }
        telemetry.build()
    }

    /// Where a run of `config` sends and writes its telemetry, as `check`'s
    /// `telemetry:` line says it: `file (line 17)`, `OTLP http/protobuf to
    /// http://localhost:4318 (line 16)`, and, with an endpoint named, that
    /// a compression variable `set` says is set is read by the exporter
    /// itself. `None` when nothing is named.
    #[must_use]
    pub fn report(&self, config: &Config, set: &dyn Fn(&str) -> bool) -> Vec<String> {
        let mut forms = Vec::new();
        if self.target.is_some() {
            forms.push(format!(
                "file{}",
                at(config.place_of("telemetry.file.path"))
            ));
        }
        if let Some(otlp) = &self.otlp {
            let transport = otlp
                .destinations()
                .next()
                .map_or(Transport::HttpProtobuf, |(_, destination)| {
                    destination.transport
                });
            let protocol = match transport {
                Transport::Grpc => "grpc",
                Transport::HttpProtobuf => "http/protobuf",
                Transport::HttpJson => "http/json",
            };
            let endpoint = config
                .telemetry
                .otlp
                .endpoint
                .as_deref()
                .map_or_else(String::new, |endpoint| {
                    format!(" to {}", endpoint_shown(endpoint))
                });
            forms.push(format!(
                "OTLP {protocol}{endpoint}{}",
                at(config.place_of("telemetry.otlp.endpoint"))
            ));
            for variable in COMPRESSION_VARIABLES {
                if set(variable) {
                    forms.push(format!(
                        "{variable} is set, and the exporter reads it itself"
                    ));
                }
            }
        }
        forms
    }
}

/// Where a setting was written, as a line says it after the name.
fn at(place: Option<Place>) -> String {
    place.map(|place| format!(" ({place})")).unwrap_or_default()
}

/// The endpoint as a line may show it: its scheme, host, port and path,
/// which a parsed URL's are, less its user information and anything from a
/// `?` or a `#` on, which may hold a credential.
fn endpoint_shown(endpoint: &str) -> String {
    let without = match user_information(endpoint) {
        Some(information) => endpoint.replacen(&format!("{information}@"), "", 1),
        None => endpoint.to_owned(),
    };
    let end = without.find(['?', '#']).unwrap_or(without.len());
    without[..end].to_owned()
}

/// Whether a `Lablet` built from `config` writes its telemetry to standard
/// error, which a `telemetry.file.path` of `-` does. It's the answer a
/// build comes to, for a caller that has to know before the build: one that
/// shares standard error with the telemetry says nothing of its own there,
/// and a log that reports on the build is installed before it.
#[must_use]
pub fn telemetry_on_stderr(config: &Config) -> bool {
    file_target(config) == Some(FileTarget::Stderr)
}

/// Where the telemetry file of `config` goes, and `None` when it writes no
/// file. Only a path that's `-` whole is standard error, so `-/` names a
/// file.
fn file_target(config: &Config) -> Option<FileTarget> {
    match &config.telemetry.file.path {
        None => None,
        Some(path) if path.as_os_str() == "-" => Some(FileTarget::Stderr),
        Some(path) => Some(FileTarget::Path(path.clone())),
    }
}

/// The refusal of what the network exporter couldn't be built from, shown
/// from `config` as the file wrote it: a header by its key alone, since
/// every header value is a secret, and the endpoint as the config writes
/// it, or no value when it holds an `@`. User information is a secret, and
/// where it ends can't be told when it holds an unencoded `/`, `?` or `#`.
/// A refusal of an endpoint a variable filled names the variable and shows
/// nothing of what it holds.
#[must_use]
pub fn otlp_refused(config: &Config, error: OtelBuildError) -> BuildError {
    const KEY: &str = "telemetry.otlp.endpoint";
    let key = KeyPath::of(KEY);
    let variable = config
        .filled()
        .at(&key)
        .and_then(|fill| fill.names().next());
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
        OtelBuildError::Endpoint { .. } => match variable {
            Some(variable) => format!("`{variable}` holds what isn't a URL the exporter accepts"),
            None => "it isn't a URL the exporter accepts".to_owned(),
        },
        OtelBuildError::Exporter { reason, .. } => {
            format!("the exporter couldn't be made: {reason}")
        }
        OtelBuildError::Tls { reason, .. } => match variable {
            Some(variable) => format!(
                "`{variable}` names the collector, and TLS to it couldn't be set up: {reason}"
            ),
            None => format!("TLS to the collector couldn't be set up: {reason}"),
        },
        OtelBuildError::HttpClient { reason, .. } => {
            format!("the HTTP client couldn't be made: {reason}")
        }
    };
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
fn network(
    config: &Config,
    key: Option<&SecretString>,
) -> Result<Option<OtlpSettings>, BuildError> {
    let otlp = otlp::settings(config, key)?;
    if let Some(settings) = &otlp {
        export::validate(settings).map_err(|error| otlp_refused(config, error))?;
    }
    Ok(otlp)
}

#[cfg(test)]
mod tests;
