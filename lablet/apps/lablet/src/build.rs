//! `build` and `check`: a config in, a `Lablet` or a checked config out.
//! The one place that knows every adapter, and that selects among them.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lablet_model::{RunOutcome, StopReason, ToolSpec};
use lablet_otlp::{FileTarget, OtelBuildError, OtlpSettings, Telemetry};
use lablet_provider_fake::{FakeProvider, Script, ScriptFormat, ScriptSource};
use lablet_run::{FilterList, RunService, ToolExecutor, ToolSet, ToolSetError};
use lablet_tools_builtin::{BuiltinTools, SettingsError};
use opentelemetry::InstrumentationScope;

use crate::cancel::RunCancellation;
use crate::clock::TokioClock;
use crate::config::{
    Config, ConfigError, Context, Env, Format, KeyPath, Model, Place, Provider, Refusal,
    ResolvedConfig, Substituted, Tools, TranscriptFormat, shown,
};
use crate::lablet::{Fixed, Lablet, Played, TranscriptPath};
use crate::otlp;
use crate::root::{self, OwnFile};
use crate::secrets::{self, Derived, Named};
use crate::settings::{Selected, Settings, System};

/// The name of a run's own telemetry file, with the run id where a run's
/// is.
const EACH_RUN_FILE: &str = "lablet-{run_id}.otlp.jsonl";

/// The name of lablet's instrumentation scope, which every span and record
/// of a run is emitted under, with lablet's version and the registry's
/// schema URL.
const SCOPE: &str = "lablet";

/// What a config may state that this lablet has no adapter for yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unsupported {
    /// `model.provider` is `anthropic`.
    Anthropic,
    /// `model.provider` is `openai`.
    Openai,
    /// `tools.mcp` lists a server.
    McpServers,
    /// `prompt.skills` lists a skill.
    Skills,
    /// `run.context` is `mask`.
    ContextMask,
    /// `run.transcript_format` is `atif`.
    Atif,
    /// `run.completion_schema` is set.
    CompletionSchema,
    /// `tools.max_description_chars` is anything but its default. Nothing
    /// that's built yet has a description to cut, so another value would
    /// change the config's digest and nothing else.
    MaxDescriptionChars,
}

impl Unsupported {
    /// The phase of the build plan that delivers it.
    const fn phase(self) -> &'static str {
        match self {
            Self::Anthropic => "7",
            Self::ContextMask => "7a",
            Self::McpServers | Self::MaxDescriptionChars => "8",
            Self::Openai => "9",
            Self::Skills | Self::Atif | Self::CompletionSchema => "10",
        }
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Anthropic => "`model.provider: anthropic`",
            Self::Openai => "`model.provider: openai`",
            Self::McpServers => "a server in `tools.mcp`",
            Self::Skills => "a skill in `prompt.skills`",
            Self::ContextMask => "`run.context: mask`",
            Self::Atif => "`run.transcript_format: atif`",
            Self::CompletionSchema => "`run.completion_schema`",
            Self::MaxDescriptionChars => {
                "a value of `tools.max_description_chars` other than its default"
            }
        })
    }
}

impl From<Unsupported> for BuildError {
    fn from(kind: Unsupported) -> Self {
        Self::Unsupported {
            kind,
            phase: kind.phase(),
        }
    }
}

/// Which of the three kinds of failure a message is of, which is what its
/// prefix says: a CLI prints `config:`, `mcp:` or `provider:` before it, so
/// a script can tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    /// The config states what a config may not, or what can't be used
    /// where lablet runs.
    Config,
    /// An MCP server couldn't be started.
    Mcp,
    /// The provider rejected a call: a run that has begun, and whose
    /// outcome says so.
    Provider,
}

impl ErrorClass {
    /// The class of the failure a run ended with: [`ErrorClass::Provider`]
    /// for a run the provider's error ended, whose outcome's error says
    /// what the provider said, and `None` for any other run, since the
    /// others end for the run's own reasons.
    #[must_use]
    pub fn of_run(outcome: &RunOutcome) -> Option<Self> {
        (outcome.stop_reason() == StopReason::ProviderError).then_some(Self::Provider)
    }

    /// The prefix of a message of this class.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Config => "config:",
            Self::Mcp => "mcp:",
            Self::Provider => "provider:",
        }
    }
}

/// Why a `Lablet` couldn't be built, or a config didn't pass its check.
///
/// Each refusal of a setting names its key and where it was written, and
/// shows its value as the config writes it, before `${VAR}` substitution:
/// what a variable holds is in no message. `model.api_key_env`'s value is
/// shown only when it's written in capitals, digits and `_`, as
/// environment variables are and few key formats are.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// The config states something a config may not, or a value that can't
    /// be used where lablet runs: a variable that isn't set, a file that
    /// can't be read, a script that's refused, a root that's no directory.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The config selects something this lablet has no adapter for yet.
    #[error("{kind} isn't supported yet: phase {phase} of the build plan delivers it")]
    Unsupported {
        /// What the config selects.
        kind: Unsupported,
        /// The phase of the build plan that delivers it.
        phase: &'static str,
    },
    /// The provider needs a key, and the variable that `model.api_key_env`
    /// names holds none where lablet runs. What the config holds there may
    /// be a key that was written in the name's place, so the refusal names
    /// the variable only when it's written in capitals, digits and `_`.
    #[error("model.api_key_env{} is refused: {reason}", at(*place))]
    KeyVariable {
        /// Where `model.api_key_env` was written, when it was.
        place: Option<Place>,
        /// What's wrong with the variable.
        reason: String,
    },
    /// The root of the built-in tools holds a file of lablet's own, which
    /// the model could then read and write over. Both are shown as the
    /// config writes them.
    #[error("tools.builtin.root{}: {root} is refused: it holds {holds}, {path}", at(*place))]
    RootHolds {
        /// Where the root was written.
        place: Option<Place>,
        /// The root.
        root: String,
        /// Which of lablet's files it holds.
        holds: OwnFile,
        /// The file.
        path: String,
    },
    /// `tools.allow` or `tools.deny` names a tool the run doesn't have. In
    /// explicit mode `task_complete` is such a name: the lists don't apply
    /// to it.
    #[error("tools.{list}{}: {name} is refused: no tool the lists apply to has the name", at(*place))]
    UnknownTool {
        /// The list the name is in.
        list: FilterList,
        /// Where the name was written.
        place: Option<Place>,
        /// The name, as the config writes it.
        name: String,
    },
    /// The tools the run's executors serve couldn't be settled into one
    /// set.
    #[error("tools: {reason}")]
    Tools {
        /// What stood in the way.
        reason: String,
    },
}

impl BuildError {
    /// The class of the failure. Every refusal of a config is
    /// [`ErrorClass::Config`], whether a rule refused it or what lablet
    /// found where it runs.
    #[must_use]
    pub const fn class(&self) -> ErrorClass {
        match self {
            Self::Config(_)
            | Self::Unsupported { .. }
            | Self::KeyVariable { .. }
            | Self::RootHolds { .. }
            | Self::UnknownTool { .. }
            | Self::Tools { .. } => ErrorClass::Config,
        }
    }
}

/// Where a setting was written, as a message says it after the key.
fn at(place: Option<Place>) -> String {
    place.map(|place| format!(" ({place})")).unwrap_or_default()
}

/// What a config passed its check with: what it resolves to, the tools a
/// run of it is offered, and the names of its secrets.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    resolved: ResolvedConfig,
    tools: Vec<ToolSpec>,
    withheld: BTreeSet<String>,
    cut: Vec<Named>,
}

impl Checked {
    /// The config with every default filled in, as it's written, which is
    /// what `lablet check --resolved` prints.
    #[must_use]
    pub fn resolved(&self) -> &ResolvedConfig {
        &self.resolved
    }

    /// The tools a run of the config is offered, in the order they're
    /// offered, after `tools.allow` and `tools.deny`.
    #[must_use]
    pub fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }

    /// The variables no command of a run inherits: the ones lablet reads
    /// its secrets from.
    #[must_use]
    pub fn withheld(&self) -> &BTreeSet<String> {
        &self.withheld
    }

    /// The names whose values no tool result shows, a variable's name or
    /// the key of a value written in the config, in order, each followed by
    /// a note when its value isn't cut: not set, empty, or under the floor.
    /// Never a value.
    #[must_use]
    pub fn cut(&self) -> Vec<String> {
        self.cut.iter().map(ToString::to_string).collect()
    }
}

/// What lablet reads of where it runs: its own environment.
fn environment(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// Whether a `Lablet` built from `config` writes its telemetry to standard
/// error, which a `telemetry.file.path` of `-` does once `${VAR}` is
/// substituted. It's the answer [`build`] comes to, for a caller that has
/// to know before the build: one that shares standard error with the
/// telemetry says nothing of its own there, and a log that reports on the
/// build is installed before it.
///
/// A config whose variables can't all be substituted gives `false`:
/// [`build`] refuses it before any telemetry is written.
#[must_use]
pub fn telemetry_on_stderr(config: &Config) -> bool {
    telemetry_on_stderr_in(config, &environment)
}

/// [`telemetry_on_stderr`], where `env` is lablet's environment.
pub(crate) fn telemetry_on_stderr_in(config: &Config, env: Env<'_>) -> bool {
    // Whether the network exporter is on changes nothing about `-`, so the
    // answer is read from the path alone.
    config
        .substituted(env)
        .is_ok_and(|real| file_target(&real, false) == Some(FileTarget::Stderr))
}

/// Where the telemetry file of `real`, a config with `${VAR}` substituted,
/// goes, and `None` when it writes no file: a null path with the network
/// exporter on, `network_on`, by the config's endpoint or the environment's.
/// Only a path that's `-` whole is standard error, so `-/` names a file.
fn file_target(real: &Config, network_on: bool) -> Option<FileTarget> {
    match &real.telemetry.file.path {
        None if network_on => None,
        None => Some(FileTarget::EachRun {
            directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }),
        Some(path) if path.as_os_str() == "-" => Some(FileTarget::Stderr),
        Some(path) => Some(FileTarget::Path(path.clone())),
    }
}

/// The refusal of what the network exporter couldn't be built from, shown
/// from `written`: a header by its key alone, since every header value is a
/// secret, and the endpoint as the config writes it, or no value when that
/// holds an `@`. User information is a secret, and where it ends can't be
/// told when it holds an unencoded `/`, `?` or `#`. When the config states
/// no endpoint, which `endpoint_stated` says, the exporter read one from
/// `env`, and a refusal of that endpoint, or of TLS to it, names the
/// variable it read and shows nothing of what it holds.
fn otlp_refused(
    written: &Config,
    env: Env<'_>,
    endpoint_stated: bool,
    error: OtelBuildError,
) -> BuildError {
    const KEY: &str = "telemetry.otlp.endpoint";
    let from_environment = |signal, what: &str| {
        let variable = otlp::endpoint_variable(signal, env);
        BuildError::Config(ConfigError::Invalid {
            key: KEY.to_owned(),
            place: written.place_of(KEY),
            value: None,
            reason: format!(
                "`{variable}`, which is read since the config states no endpoint, {what}"
            ),
        })
    };
    let reason = match error {
        OtelBuildError::Header { name, reason } => {
            let key = KeyPath::of("telemetry.otlp.headers").key(&name).to_string();
            return BuildError::Config(ConfigError::Invalid {
                place: written.place_of(&key),
                value: None,
                key,
                reason: reason.to_owned(),
            });
        }
        OtelBuildError::Endpoint { signal } if !endpoint_stated => {
            return from_environment(signal, "holds what isn't a URL the exporter accepts");
        }
        OtelBuildError::Endpoint { .. } => "it isn't a URL the exporter accepts".to_owned(),
        OtelBuildError::Exporter { reason, .. } => {
            format!("the exporter couldn't be made: {reason}")
        }
        OtelBuildError::Tls { signal, reason } if !endpoint_stated => {
            return from_environment(
                signal,
                &format!("names the collector, and TLS to it couldn't be set up: {reason}"),
            );
        }
        OtelBuildError::Tls { reason, .. } => {
            format!("TLS to the collector couldn't be set up: {reason}")
        }
        OtelBuildError::HttpClient { reason } => {
            format!("the HTTP client couldn't be made: {reason}")
        }
    };
    let key = KeyPath::of(KEY);
    let value = written
        .written_text(KEY)
        .filter(|endpoint| !endpoint.contains('@'))
        .map(serde_json::Value::String);
    BuildError::Config(ConfigError::Invalid {
        place: written.place_of(KEY),
        value: shown(&key, value.as_ref()),
        key: KEY.to_owned(),
        reason,
    })
}

/// Checks `config` whole, as [`build`] does, and stops before the provider
/// is selected, so the answer is the same for every provider: one this
/// lablet has no adapter for yet passes, and no call reaches a provider.
///
/// It substitutes `${VAR}`, checks every setting, that the provider's key
/// variable is set when the provider needs a key, that each file the config
/// names can be read, the fake provider's script among them, and the root
/// holds none of lablet's own, and builds the tool set, so the tools a run
/// is offered are listed.
///
/// # Errors
///
/// Returns what [`build`] returns, but for [`BuildError::Unsupported`] of
/// the provider, and for what only making the network exporter finds, which
/// a check passes: trust roots that can't be loaded, or other TLS to the
/// collector that can't be set up, and an HTTP client that can't be made.
pub async fn check(config: &Config) -> Result<Checked, BuildError> {
    check_in(config, &environment).await
}

/// [`check`], where `env` is lablet's environment.
pub(crate) async fn check_in(config: &Config, env: Env<'_>) -> Result<Checked, BuildError> {
    let checked = prepare(config, env).await?;
    Ok(Checked {
        resolved: config.resolved(),
        tools: checked.tools.specs().to_vec(),
        withheld: checked.secrets.withheld,
        cut: checked.secrets.cut,
    })
}

/// A `Lablet` that runs as `config` says.
///
/// # Errors
///
/// Returns [`BuildError::Config`] for a config that states what a config
/// may not: a setting a rule refuses, a `${VAR}` whose variable isn't set,
/// or a file the config names that can't be read, a script that's refused,
/// or a root that isn't a directory. The config is checked whole before
/// the provider is selected, so the answer doesn't depend on which
/// adapters exist.
///
/// Returns [`BuildError::KeyVariable`] when the provider needs a key and
/// the variable `model.api_key_env` names isn't set, or holds nothing.
///
/// Returns [`BuildError::Unsupported`] for a config that selects an
/// adapter this lablet doesn't have yet, or that states a setting only a
/// later one applies.
///
/// Returns [`BuildError::RootHolds`] when `tools.builtin.root` holds the
/// config, the system prompt's file, the task prompt's file, the
/// transcript or the telemetry file.
///
/// Returns [`BuildError::UnknownTool`] when `tools.allow` or `tools.deny`
/// names a tool the run doesn't have.
pub async fn build(config: Config) -> Result<Lablet, BuildError> {
    build_in(config, &environment).await
}

/// What checking a config comes to, which a build goes on from.
struct Prepared {
    /// The config with `${VAR}` substituted, which is what runs.
    real: Substituted,
    settings: Settings,
    provider: Ready,
    system: String,
    target: Option<FileTarget>,
    /// What the network exporter is built from, when the run has one.
    otlp: Option<OtlpSettings>,
    tools: Arc<ToolSet>,
    /// lablet's secrets: what's withheld, what's cut, and the values when
    /// the run has an executor to hand them to.
    secrets: Derived,
}

/// The provider a checked config selects, with what was read for it, so a
/// build reads no file its check didn't.
enum Ready {
    Anthropic,
    Openai,
    /// The fake provider, and the script it plays.
    Fake(Script),
}

/// Checks `written` whole, on the config it comes to once `${VAR}` is
/// substituted, and shows each refusal from `written`, so no message holds
/// what a variable holds.
async fn prepare(written: &Config, env: Env<'_>) -> Result<Prepared, BuildError> {
    let refused = |refusal| BuildError::Config(written.refused(refusal));
    let real = written.substituted(env).map_err(refused)?;
    let settings = Settings::of(&real).map_err(refused)?;
    key_is_set(&real.model, written, env)?;
    supported(&real)?;

    let system = match &settings.system {
        System::Text(text) => text.clone(),
        System::File(file) => std::fs::read_to_string(file)
            .map_err(|error| refused(Refusal::invalid("prompt.system_file", error.to_string())))?,
    };
    let provider = match &settings.provider {
        Selected::Anthropic => Ready::Anthropic,
        Selected::Openai => Ready::Openai,
        Selected::Fake { script } => {
            // The script is read where it is and named as the config writes
            // it, since the provider puts its name in the errors a run ends
            // with.
            let name = written
                .written_text("model.script")
                .unwrap_or_else(|| script.display().to_string());
            Ready::Fake(read_script(script, &name).map_err(refused)?)
        }
    };

    let otlp = otlp::settings(written, &real, env).map_err(refused)?;
    if let Some(settings) = &otlp {
        // What the exporter would refuse is refused here, so a check refuses
        // what a build refuses but for what only making the exporter finds,
        // trust roots that can't be loaded or other TLS that can't be set
        // up, and the HTTP client (see `validate`); a check installs no
        // exporter.
        lablet_otlp::validate(settings)
            .map_err(|error| otlp_refused(written, env, settings.endpoint.is_some(), error))?;
    }
    let target = file_target(&real, otlp.is_some());

    // The values are held only for an executor to cut, and the built-in
    // tools are the one executor a run can have.
    let secrets = secrets::derived(written, &real, env, settings.builtin.is_some());
    let executors = match settings.builtin.clone() {
        Some(builtin) => {
            outside_root(&builtin.root, &real, written, target.as_ref())?;
            let builtin = lablet_tools_builtin::Settings {
                withheld: secrets.withheld.clone(),
                ..builtin
            };
            let tools = BuiltinTools::new(builtin).map_err(|error| match error {
                SettingsError::Root { reason, .. } => {
                    refused(Refusal::invalid("tools.builtin.root", reason))
                }
                SettingsError::RootIsNoDirectory { .. } => refused(Refusal::invalid(
                    "tools.builtin.root",
                    "it isn't a directory",
                )),
                SettingsError::Variable { name, reason } => refused(Refusal::Invalid {
                    key: KeyPath::of("tools.builtin.env").key(&name),
                    reason: reason.to_owned(),
                }),
            })?;
            vec![Arc::new(tools) as Arc<dyn ToolExecutor>]
        }
        None => Vec::new(),
    };
    let tools = ToolSet::build(executors, &settings.filter, settings.completion, None)
        .await
        .map(Arc::new)
        .map_err(|error| match error {
            ToolSetError::UnknownFilterName { name, list } => {
                unknown_tool(&real, written, list, name.as_str())
            }
            error @ (ToolSetError::DuplicateName { .. } | ToolSetError::Specs(_)) => {
                BuildError::Tools {
                    reason: error.to_string(),
                }
            }
        })?;
    Ok(Prepared {
        real,
        settings,
        provider,
        system,
        target,
        otlp,
        tools,
        secrets,
    })
}

/// The refusal of a name in `tools.allow` or `tools.deny` that no tool
/// has, which shows the name as `written` writes it.
fn unknown_tool(real: &Config, written: &Config, list: FilterList, name: &str) -> BuildError {
    let (names, key, written_names) = match list {
        FilterList::Allow => (
            real.tools.allow.as_deref().unwrap_or_default(),
            "tools.allow",
            written.tools.allow.as_deref().unwrap_or_default(),
        ),
        FilterList::Deny => (
            real.tools.deny.as_slice(),
            "tools.deny",
            written.tools.deny.as_slice(),
        ),
    };
    let index = names.iter().position(|named| named == name);
    BuildError::UnknownTool {
        list,
        place: index.and_then(|index| written.place_of_item(key, index)),
        name: index
            .and_then(|index| written_names.get(index))
            .map_or_else(|| name.to_owned(), Clone::clone),
    }
}

async fn build_in(config: Config, env: Env<'_>) -> Result<Lablet, BuildError> {
    let Prepared {
        real,
        settings,
        provider,
        system,
        target,
        otlp,
        tools,
        secrets,
    } = prepare(&config, env).await?;
    let script = match provider {
        Ready::Fake(script) => script,
        Ready::Anthropic => return Err(Unsupported::Anthropic.into()),
        Ready::Openai => return Err(Unsupported::Openai.into()),
    };
    let provider = Arc::new(FakeProvider::new(real.model.name.clone(), script));

    let scope = InstrumentationScope::builder(SCOPE)
        .with_version(crate::VERSION)
        .with_schema_url(crate::telemetry::generated::SCHEMA_URL)
        .build();
    let mut telemetry = Telemetry::builder(crate::VERSION, scope)
        .resource(real.telemetry.resource.clone().into_iter().collect());
    if let Some(target) = target {
        telemetry = telemetry.file(target);
    }
    let endpoint_stated = otlp
        .as_ref()
        .is_some_and(|settings| settings.endpoint.is_some());
    if let Some(settings) = otlp {
        telemetry = telemetry.otlp(settings);
    }
    let telemetry = telemetry
        .build()
        .map_err(|error| otlp_refused(&config, env, endpoint_stated, error))?;

    let cancellation = Arc::new(RunCancellation::default());
    let service = RunService::new(
        Arc::clone(&provider) as _,
        Arc::clone(&tools),
        telemetry.tracer(),
        telemetry.logger(),
        Arc::new(TokioClock),
        Arc::clone(&cancellation) as _,
        settings.stop,
        settings.retry,
        settings.request,
        settings.pricing,
        settings.calls,
        Arc::new(secrets.values),
    );
    Ok(Lablet::new(
        service,
        Played::Script(provider),
        tools,
        telemetry.tracer(),
        telemetry,
        cancellation,
        Fixed {
            system,
            config_digest: config.digest(),
            capture_content: real.telemetry.capture_content,
            transcript: real
                .run
                .transcript_path
                .clone()
                .zip(config.run.transcript_path.clone())
                .map(|(real, written)| TranscriptPath { real, written }),
        },
    ))
}

/// Holds `model.api_key_env` to a variable that's set, for a provider that
/// needs a key. Only whether it's set is read, and nothing of what it
/// holds is kept. `model` is the model as it runs, and the refusal names
/// the variable as `written` writes it.
fn key_is_set(model: &Model, written: &Config, env: Env<'_>) -> Result<(), BuildError> {
    let (Provider::Anthropic, Some(variable)) = (model.provider, model.key_variable()) else {
        return Ok(());
    };
    let fault = match env(variable) {
        Some(key) if !key.is_empty() => return Ok(()),
        Some(_) => "is empty",
        None => "isn't set",
    };
    let variable = match (
        &written.model.api_key_env,
        written.model.shown_key_variable(),
    ) {
        (None, _) => format!("`{variable}`, the variable that's read when the config names none,"),
        (Some(_), Some(shown)) => format!("`{shown}`, the variable it names,"),
        (Some(_), None) => "the variable it names".to_owned(),
    };
    Err(BuildError::KeyVariable {
        place: written.place_of("model.api_key_env"),
        reason: format!("{variable} {fault}, and the provider `anthropic` needs a key"),
    })
}

/// Refuses what the config selects, beside its provider, that this lablet
/// has no adapter for, and a setting that only such an adapter applies.
fn supported(config: &Config) -> Result<(), Unsupported> {
    let Config {
        run, prompt, tools, ..
    } = config;
    let selected = [
        (!tools.mcp.is_empty(), Unsupported::McpServers),
        (!prompt.skills.is_empty(), Unsupported::Skills),
        (
            matches!(run.context, Context::Mask { .. }),
            Unsupported::ContextMask,
        ),
        (
            run.transcript_format == TranscriptFormat::Atif,
            Unsupported::Atif,
        ),
        (
            run.completion_schema.is_some(),
            Unsupported::CompletionSchema,
        ),
        (
            tools.max_description_chars != Tools::default().max_description_chars,
            Unsupported::MaxDescriptionChars,
        ),
    ];
    match selected.into_iter().find(|(selected, _)| *selected) {
        Some((_, unsupported)) => Err(unsupported),
        None => Ok(()),
    }
}

/// The script the file at `path` holds, in the format its name says,
/// named `name`.
fn read_script(path: &Path, name: &str) -> Result<Script, Refusal> {
    let refuse = |reason: String| Refusal::invalid("model.script", reason);
    let format = match Format::of(path) {
        Some(Format::Yaml) => ScriptFormat::Yaml,
        Some(Format::Json) => ScriptFormat::Json,
        None => {
            return Err(refuse(
                "its name says neither YAML (`.yaml`, `.yml`) nor JSON (`.json`)".to_owned(),
            ));
        }
    };
    let text = std::fs::read_to_string(path).map_err(|error| refuse(error.to_string()))?;
    Script::read(ScriptSource {
        name,
        text: &text,
        format,
    })
    .map_err(|error| refuse(error.fault.to_string()))
}

/// Refuses a root that holds a file of lablet's own. The files are those
/// of `real`, which is what runs, and the refusal shows each as `written`
/// writes it.
fn outside_root(
    root: &Path,
    real: &Config,
    written: &Config,
    telemetry: Option<&FileTarget>,
) -> Result<(), BuildError> {
    let resolved = std::fs::canonicalize(root).map_err(|error| {
        BuildError::Config(
            written.refused(Refusal::invalid("tools.builtin.root", error.to_string())),
        )
    })?;
    let telemetry = match telemetry {
        Some(FileTarget::EachRun { directory }) => Some(directory.join(EACH_RUN_FILE)),
        Some(FileTarget::Path(path)) => Some(path.clone()),
        Some(FileTarget::Stderr) | None => None,
    };
    let files = [
        (OwnFile::Config, real.source()),
        (OwnFile::SystemPrompt, real.prompt.system_file.as_deref()),
        (OwnFile::TaskPrompt, real.prompt_file()),
        (OwnFile::Transcript, real.run.transcript_path.as_deref()),
        (OwnFile::Telemetry, telemetry.as_deref()),
    ];
    let files = files
        .into_iter()
        .filter_map(|(file, path)| Some((file, path?)));
    let Some((holds, path)) = root::held(&resolved, files) else {
        return Ok(());
    };
    let shown = |key: &str, real: &Path| {
        written
            .written_text(key)
            .unwrap_or_else(|| real.display().to_string())
    };
    let path = match holds {
        OwnFile::SystemPrompt => shown("prompt.system_file", path),
        OwnFile::Transcript => shown("run.transcript_path", path),
        OwnFile::Telemetry if written.telemetry.file.path.is_some() => {
            shown("telemetry.file.path", path)
        }
        OwnFile::Config | OwnFile::TaskPrompt | OwnFile::Telemetry => path.display().to_string(),
    };
    Err(BuildError::RootHolds {
        place: written.place_of("tools.builtin.root"),
        root: shown("tools.builtin.root", root),
        holds,
        path,
    })
}

#[cfg(test)]
mod tests;
