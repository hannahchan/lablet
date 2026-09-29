//! `build`: a config in, a `Lablet` out. The one place that knows every
//! adapter, and that selects among them.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lablet_provider_fake::{FakeProvider, Script, ScriptFormat, ScriptSource};
use lablet_run::{FilterList, RunObserver, RunService, ToolExecutor, ToolSet, ToolSetError};
use lablet_telemetry_otel::{FileTarget, OtelObserver};
use lablet_tools_builtin::{BuiltinTools, SettingsError};

use crate::clock::{NeverCancelled, TokioClock};
use crate::config::{Config, ConfigError, Context, Format, Provider, TranscriptFormat};
use crate::fanout::FanOut;
use crate::lablet::{Fixed, Lablet, Played};
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
}

impl Unsupported {
    /// The phase of the build plan that delivers it.
    const fn phase(self) -> &'static str {
        match self {
            Self::OtlpEndpoint => "6",
            Self::Anthropic => "7",
            Self::ContextMask => "7a",
            Self::McpServers => "8",
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

/// Why a `Lablet` couldn't be built. Each refusal of a setting names its
/// key and the value that was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// The config states something a config may not.
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
    /// A setting holds a value that can't be used where lablet runs: a
    /// file that can't be read, a script that's refused, a variable that
    /// isn't set.
    #[error("{key}: {value} is refused: {reason}")]
    Refused {
        /// The setting.
        key: &'static str,
        /// The value it holds.
        value: String,
        /// Why it can't be used.
        reason: String,
    },
    /// The root of the built-in tools holds a file of lablet's own, which
    /// the model could then read and write over.
    #[error("tools.builtin.root: {root} is refused: it holds {holds}, {path}")]
    RootHolds {
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
    #[error("tools.{list}: {name} is refused: no tool the lists apply to has the name")]
    UnknownTool {
        /// The list the name is in.
        list: FilterList,
        /// The name.
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

fn refused(key: &'static str, value: &dyn fmt::Display, reason: &dyn fmt::Display) -> BuildError {
    BuildError::Refused {
        key,
        value: value.to_string(),
        reason: reason.to_string(),
    }
}

/// A `Lablet` that runs as `config` says.
///
/// # Errors
///
/// Returns [`BuildError::Config`] for a config that states what a config
/// may not. The config is checked whole before any adapter is selected, so
/// the answer doesn't depend on which adapters exist.
///
/// Returns [`BuildError::Refused`] when `model.api_key_env` names a
/// variable that isn't set and the provider needs a key, when a file the
/// config names can't be read, when the script is refused, and when
/// `tools.builtin.root` isn't a directory.
///
/// Returns [`BuildError::Unsupported`] for a config that selects an
/// adapter this lablet doesn't have yet.
///
/// Returns [`BuildError::RootHolds`] when `tools.builtin.root` holds the
/// config, the system prompt's file, the transcript or the telemetry file.
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
    let settings = Settings::of(&config)?;
    key_is_set(&config)?;
    let script = match &settings.provider {
        Selected::Fake { script } => script,
        Selected::Anthropic => return Err(Unsupported::Anthropic.into()),
        Selected::Openai => return Err(Unsupported::Openai.into()),
    };
    supported(&config)?;

    let provider = Arc::new(FakeProvider::new(
        config.model.name.clone(),
        read_script(script)?,
    ));
    let system = match &settings.system {
        System::Text(text) => text.clone(),
        System::File(file) => std::fs::read_to_string(file)
            .map_err(|error| refused("prompt.system_file", &file.display(), &error))?,
    };

    let target = match &config.telemetry.file.path {
        None => FileTarget::EachRun {
            directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        },
        Some(path) if path.as_os_str() == "-" => FileTarget::Stderr,
        Some(path) => FileTarget::Path(path.clone()),
    };

    let executors = match settings.builtin {
        Some(builtin) => {
            outside_root(&builtin.root, &config, &target)?;
            let tools = BuiltinTools::new(builtin).map_err(|error| match error {
                SettingsError::Root { ref root, .. }
                | SettingsError::RootIsNoDirectory { ref root } => {
                    refused("tools.builtin.root", root, &error)
                }
                SettingsError::Variable { name, reason } => {
                    refused("tools.builtin.env", &name, &reason)
                }
            })?;
            vec![Arc::new(tools) as Arc<dyn ToolExecutor>]
        }
        None => Vec::new(),
    };
    let tools = ToolSet::build(executors, &settings.filter, settings.completion, None)
        .await
        .map(Arc::new)
        .map_err(|error| match error {
            ToolSetError::UnknownFilterName { name, list } => BuildError::UnknownTool {
                list,
                name: name.into(),
            },
            error @ (ToolSetError::DuplicateName { .. } | ToolSetError::Specs(_)) => {
                BuildError::Tools {
                    reason: error.to_string(),
                }
            }
        })?;

    let telemetry = OtelObserver::builder(crate::VERSION)
        .resource(config.telemetry.resource.clone().into_iter().collect())
        .file(target)
        .build();
    let mut told: Vec<Arc<dyn RunObserver>> = vec![Arc::new(telemetry.clone())];
    told.extend(observers);

    let service = RunService::new(
        Arc::clone(&provider) as _,
        Arc::clone(&tools),
        Arc::new(FanOut::new(told)),
        Arc::new(TokioClock),
        Arc::new(NeverCancelled),
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
        Fixed {
            system,
            config_digest: config.digest(),
            capture_content: config.telemetry.capture_content,
            transcript_path: config.run.transcript_path,
        },
    ))
}

/// Holds `model.api_key_env` to a variable that's set, for a provider that
/// needs a key. Only whether it's set is read, and nothing of what it
/// holds is kept.
fn key_is_set(config: &Config) -> Result<(), BuildError> {
    let (Provider::Anthropic, Some(variable)) =
        (config.model.provider, config.model.key_variable())
    else {
        return Ok(());
    };
    match std::env::var_os(variable) {
        Some(key) if !key.is_empty() => Ok(()),
        Some(_) => Err(refused(
            "model.api_key_env",
            &variable,
            &"the variable is empty, and the provider `anthropic` needs a key",
        )),
        None => Err(refused(
            "model.api_key_env",
            &variable,
            &"the variable isn't set, and the provider `anthropic` needs a key",
        )),
    }
}

/// Refuses what the config selects, beside its provider, that this lablet
/// has no adapter for.
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
    ];
    match selected.into_iter().find(|(selected, _)| *selected) {
        Some((_, unsupported)) => Err(unsupported),
        None => Ok(()),
    }
}

/// The script the file at `path` holds, in the format its name says.
fn read_script(path: &Path) -> Result<Script, BuildError> {
    let refuse = |reason: &dyn fmt::Display| refused("model.script", &path.display(), reason);
    let format = match Format::of(path) {
        Some(Format::Yaml) => ScriptFormat::Yaml,
        Some(Format::Json) => ScriptFormat::Json,
        None => {
            return Err(refuse(
                &"its name says neither YAML (`.yaml`, `.yml`) nor JSON (`.json`)",
            ));
        }
    };
    let text = std::fs::read_to_string(path).map_err(|error| refuse(&error))?;
    Script::read(ScriptSource {
        name: &path.display().to_string(),
        text: &text,
        format,
    })
    .map_err(|error| refuse(&error.fault))
}

/// Refuses a root that holds a file of lablet's own.
fn outside_root(root: &Path, config: &Config, telemetry: &FileTarget) -> Result<(), BuildError> {
    let shown = root.display().to_string();
    let resolved = std::fs::canonicalize(root)
        .map_err(|error| refused("tools.builtin.root", &shown, &error))?;
    let telemetry = match telemetry {
        FileTarget::EachRun { directory } => Some(directory.join(EACH_RUN_FILE)),
        FileTarget::Path(path) => Some(path.clone()),
        FileTarget::Stderr => None,
    };
    let files = [
        (OwnFile::Config, config.source()),
        (OwnFile::SystemPrompt, config.prompt.system_file.as_deref()),
        (OwnFile::Transcript, config.run.transcript_path.as_deref()),
        (OwnFile::Telemetry, telemetry.as_deref()),
    ];
    let files = files
        .into_iter()
        .filter_map(|(file, path)| Some((file, path?)));
    match root::held(&resolved, files) {
        Some((holds, path)) => Err(BuildError::RootHolds {
            root: shown,
            holds,
            path: path.display().to_string(),
        }),
        None => Ok(()),
    }
}
