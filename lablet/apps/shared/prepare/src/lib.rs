//! What checking a config comes to, which both roots go on from: the
//! config with `${VAR}` substituted and its settings checked, the provider's
//! key variable, the system prompt and the fake provider's script read, and
//! lablet's secrets derived. It reads no telemetry setting of its own: in
//! between, each root settles what it reads of the telemetry, the command
//! line's root its SDK's settings and the library root only the capture
//! variable, and hands in what the rest of the check needs of it. It
//! constructs no adapter: each root builds its tools and its provider
//! through the wiring kernels, and connects what they return itself.

mod root;
mod settings;

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use lablet_config::{
    Config, ConfigError, Context, Env, Format, Model, Place, Provider, Refusal, ResolvedConfig,
    Substituted, Tools, TranscriptFormat,
};
use lablet_model::{RunOutcome, StopReason, ToolSpec};
use lablet_provider_fake::{Script, ScriptFormat, ScriptSource};
use lablet_run::FilterList;
use lablet_secrets::{self as secrets, Derived, Named};
use secrecy::SecretString;

pub use root::{OwnFile, held};
pub use settings::{Selected, Settings, System};

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
    /// What a config passed its check with: `resolved`, the specs of the
    /// tools the root built for it, and its `secrets`.
    #[must_use]
    pub fn new(resolved: ResolvedConfig, tools: Vec<ToolSpec>, secrets: Derived) -> Self {
        Self {
            resolved,
            tools,
            withheld: secrets.withheld,
            cut: secrets.cut,
        }
    }

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

/// A config checked up to its telemetry, which the root settles before
/// the check goes on: what [`prepare`] comes to.
pub struct Settled {
    /// The config with `${VAR}` substituted, which is what runs.
    real: Substituted,
    settings: Settings,
    provider: Ready,
    system: String,
}

/// What a root read of the telemetry's settings, which the rest of a check
/// goes on from.
#[derive(Debug, Default)]
pub struct Telemetry<'a> {
    /// The telemetry file the root writes, which the root of the built-in
    /// tools may not hold.
    pub file: Option<PathBuf>,
    /// What the environment says of content capture, when it says
    /// anything. The config's `telemetry.capture_content` wins over it.
    pub capture_content: Option<bool>,
    /// The client keys the root's network exporters read, each the
    /// variable that names its file and what the file holds, which are
    /// cut from what a tool returns.
    pub client_keys: &'a [(&'a str, &'a SecretString)],
}

/// What checking a config comes to, which a build goes on from once the
/// root has built its tools.
pub struct Prepared {
    /// The config with `${VAR}` substituted, which is what runs.
    real: Substituted,
    settings: Settings,
    provider: Ready,
    system: String,
    /// The telemetry file the root writes, when it writes one.
    telemetry_file: Option<PathBuf>,
    /// Whether content reaches telemetry.
    capture_content: bool,
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
/// what a variable holds. It stops before the telemetry, which the root
/// settles and hands to [`Settled::finish`], and the tools are left for the
/// root to build through `lablet-tools-wiring`, which refuses what only
/// building them finds.
///
/// # Errors
///
/// Returns the [`BuildError`] of the first thing the check refuses, in the
/// order a build meets them.
pub fn prepare(written: &Config, env: Env<'_>) -> Result<Settled, BuildError> {
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
    Ok(Settled {
        real,
        settings,
        provider,
        system,
    })
}

impl Settled {
    /// The config with `${VAR}` substituted, which is what runs, and what
    /// a root reads its telemetry's settings from.
    #[must_use]
    pub fn real(&self) -> &Substituted {
        &self.real
    }

    /// The check of `written`, this config as it's written, gone on with
    /// what the root read of the telemetry: content capture settled, and
    /// lablet's secrets derived, with each variable's value read from
    /// `env`.
    #[must_use]
    pub fn finish(self, written: &Config, env: Env<'_>, telemetry: Telemetry<'_>) -> Prepared {
        let Self {
            real,
            settings,
            provider,
            system,
        } = self;
        let capture_content = real
            .telemetry
            .capture_content
            .or(telemetry.capture_content)
            .unwrap_or(false);
        // The values are held only for an executor to cut, and the built-in
        // tools are the one executor a run can have.
        let secrets = secrets::derived(
            written,
            &real,
            env,
            telemetry.client_keys,
            settings.builtin.is_some(),
        );
        Prepared {
            real,
            settings,
            provider,
            system,
            telemetry_file: telemetry.file,
            capture_content,
            secrets,
        }
    }
}

impl Prepared {
    /// The config with `${VAR}` substituted, which is what runs.
    #[must_use]
    pub fn real(&self) -> &Substituted {
        &self.real
    }

    /// What the loop and its adapters are built from.
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The path of the telemetry file, when the run writes one, which the
    /// root of the built-in tools may not hold.
    #[must_use]
    pub fn telemetry_file(&self) -> Option<&Path> {
        self.telemetry_file.as_deref()
    }

    /// Whether content reaches telemetry.
    #[must_use]
    pub fn capture_content(&self) -> bool {
        self.capture_content
    }

    /// The variables no command inherits.
    #[must_use]
    pub fn withheld(&self) -> &BTreeSet<String> {
        &self.secrets.withheld
    }

    /// lablet's secrets, for a check that ends here.
    #[must_use]
    pub fn into_secrets(self) -> Derived {
        self.secrets
    }
}

/// What a checked config's `Lablet` is wired from, beside its telemetry and
/// its tools.
pub struct Wiring {
    /// The config with `${VAR}` substituted, which is what runs.
    pub real: Substituted,
    /// What the loop and its adapters are built from.
    pub settings: Settings,
    /// The fake provider's script, the one provider there's an adapter for.
    pub script: Script,
    /// The system prompt.
    pub system: String,
    /// Whether content reaches telemetry.
    pub capture_content: bool,
    /// lablet's secrets, whose values the loop cuts.
    pub secrets: Derived,
}

impl Prepared {
    /// What the rest is wired from.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::Unsupported`] for a provider there's no
    /// adapter for yet.
    pub fn wiring(self) -> Result<Wiring, BuildError> {
        let Self {
            real,
            settings,
            provider,
            system,
            telemetry_file: _,
            capture_content,
            secrets,
        } = self;
        let script = match provider {
            Ready::Fake(script) => script,
            Ready::Anthropic => return Err(Unsupported::Anthropic.into()),
            Ready::Openai => return Err(Unsupported::Openai.into()),
        };
        Ok(Wiring {
            real,
            settings,
            script,
            system,
            capture_content,
            secrets,
        })
    }
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

#[cfg(test)]
mod tests;
