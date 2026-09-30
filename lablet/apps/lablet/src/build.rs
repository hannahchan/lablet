//! `build` and `check`: a config in, a `Lablet` or a checked config out.
//! The one place that knows every adapter, and that selects among them.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lablet_model::{RunOutcome, Secrets, StopReason, ToolSpec};
use lablet_provider_fake::{FakeProvider, Script, ScriptFormat, ScriptSource};
use lablet_run::{FilterList, RunObserver, RunService, ToolExecutor, ToolSet, ToolSetError};
use lablet_telemetry_otel::{FileTarget, OtelObserver};
use lablet_tools_builtin::{BuiltinTools, SettingsError, Withheld};

use crate::cancel::RunCancellation;
use crate::clock::TokioClock;
use crate::config::{
    Config, ConfigError, Context, Env, Format, KeyPath, Model, Place, Provider, Refusal,
    ResolvedConfig, Tools, TranscriptFormat,
};
use crate::fanout::FanOut;
use crate::lablet::{Fixed, Lablet, Played, TranscriptPath};
use crate::root::{self, OwnFile};
use crate::settings::{Selected, Settings, System};

/// The name of a run's own telemetry file, with the run id where a run's
/// is.
const EACH_RUN_FILE: &str = "lablet-{run_id}.otlp.jsonl";

/// What a config may state that this lablet has no adapter for yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unsupported {
    /// `model.provider` is `anthropic`.
    Anthropic,
    /// `model.provider` is `openai`.
    Openai,
    /// `tools.mcp` lists a server.
    McpServers,
    /// `telemetry.otlp.endpoint` is set.
    OtlpEndpoint,
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
            Self::OtlpEndpoint => "6",
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
            Self::OtlpEndpoint => "`telemetry.otlp.endpoint`",
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

/// What a config passed its check with: what it resolves to, and the tools
/// a run of it is offered.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    resolved: ResolvedConfig,
    tools: Vec<ToolSpec>,
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
    config
        .substituted(env)
        .is_ok_and(|real| file_target(&real) == FileTarget::Stderr)
}

/// Where the telemetry of `real`, a config with `${VAR}` substituted, goes.
/// Only a path that's `-` whole is standard error, so `-/` names a file.
fn file_target(real: &Config) -> FileTarget {
    match &real.telemetry.file.path {
        None => FileTarget::EachRun {
            directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        },
        Some(path) if path.as_os_str() == "-" => FileTarget::Stderr,
        Some(path) => FileTarget::Path(path.clone()),
    }
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
/// Returns what [`build`] returns, but for
/// [`BuildError::Unsupported`] of the provider.
pub async fn check(config: &Config) -> Result<Checked, BuildError> {
    check_in(config, &environment).await
}

/// [`check`], where `env` is lablet's environment.
pub(crate) async fn check_in(config: &Config, env: Env<'_>) -> Result<Checked, BuildError> {
    let checked = prepare(config, env).await?;
    Ok(Checked {
        resolved: config.resolved(),
        tools: checked.tools.specs().to_vec(),
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
    build_observed(config, Vec::new()).await
}

/// A `Lablet` that runs as `config` says and tells `observers` every event
/// of its runs, after it has told its own telemetry.
///
/// # Errors
///
/// Returns what [`build`] returns.
pub async fn build_observed(
    config: Config,
    observers: Vec<Arc<dyn RunObserver>>,
) -> Result<Lablet, BuildError> {
    build_in(config, observers, &environment).await
}

/// What checking a config comes to, which a build goes on from.
struct Prepared {
    /// The config with `${VAR}` substituted, which is what runs.
    real: Config,
    settings: Settings,
    provider: Ready,
    system: String,
    target: FileTarget,
    tools: Arc<ToolSet>,
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

    let target = file_target(&real);

    let executors = match settings.builtin.clone() {
        Some(builtin) => {
            outside_root(&builtin.root, &real, written, &target)?;
            let builtin = lablet_tools_builtin::Settings {
                withheld: withheld(&real, env),
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
        tools,
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

async fn build_in(
    config: Config,
    observers: Vec<Arc<dyn RunObserver>>,
    env: Env<'_>,
) -> Result<Lablet, BuildError> {
    let Prepared {
        real,
        settings,
        provider,
        system,
        target,
        tools,
    } = prepare(&config, env).await?;
    let script = match provider {
        Ready::Fake(script) => script,
        Ready::Anthropic => return Err(Unsupported::Anthropic.into()),
        Ready::Openai => return Err(Unsupported::Openai.into()),
    };
    let provider = Arc::new(FakeProvider::new(real.model.name.clone(), script));

    let telemetry = OtelObserver::builder(crate::VERSION)
        .resource(real.telemetry.resource.clone().into_iter().collect())
        .file(target)
        .build();
    let mut told: Vec<Arc<dyn RunObserver>> = vec![Arc::new(telemetry.clone())];
    told.extend(observers);

    let cancellation = Arc::new(RunCancellation::default());
    let service = RunService::new(
        Arc::clone(&provider) as _,
        Arc::clone(&tools),
        Arc::new(FanOut::new(told)),
        Arc::new(TokioClock),
        Arc::clone(&cancellation) as _,
        settings.stop,
        settings.retry,
        settings.request,
        settings.pricing,
        settings.calls,
    );
    Ok(Lablet::new(
        service,
        Played::Script(provider),
        tools,
        telemetry,
        cancellation,
        Fixed {
            system,
            config_digest: config.digest(),
            capture_content: real.telemetry.capture_content,
            transcript: real
                .run
                .transcript_path
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

/// lablet's own secrets: the variables it reads them from, which no
/// command inherits, and what they hold, which no tool result shows. Every
/// variable lablet reads a secret from is named here, so withholding another
/// is one more entry in the list.
///
/// It's read only when a built-in tool is enabled, since that executor is
/// the one place that holds a value beside lablet's environment. `env` is
/// lablet's environment.
fn withheld(config: &Config, env: Env<'_>) -> Withheld {
    let variables: BTreeSet<String> = [config.model.key_variable()]
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .collect();
    let values = variables
        .iter()
        .filter_map(|variable| Some(env(variable)?.to_string_lossy().into_owned()));
    Withheld {
        values: Secrets::new(values),
        variables,
    }
}

/// Refuses what the config selects, beside its provider, that this lablet
/// has no adapter for, and a setting that only such an adapter applies.
fn supported(config: &Config) -> Result<(), Unsupported> {
    let Config {
        run,
        prompt,
        tools,
        telemetry,
        ..
    } = config;
    let selected = [
        (!tools.mcp.is_empty(), Unsupported::McpServers),
        (telemetry.otlp.endpoint.is_some(), Unsupported::OtlpEndpoint),
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
    telemetry: &FileTarget,
) -> Result<(), BuildError> {
    let resolved = std::fs::canonicalize(root).map_err(|error| {
        BuildError::Config(
            written.refused(Refusal::invalid("tools.builtin.root", error.to_string())),
        )
    })?;
    let telemetry = match telemetry {
        FileTarget::EachRun { directory } => Some(directory.join(EACH_RUN_FILE)),
        FileTarget::Path(path) => Some(path.clone()),
        FileTarget::Stderr => None,
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
