//! What checking a config comes to, which both roots go on from: the
//! filled config with its settings checked, its credentials held to their
//! rules, the system prompt and the fake provider's script read, and
//! lablet's credentials derived. It reads no environment variable and no
//! telemetry setting of its own: the config was filled before it got here,
//! the environment a run's processes start from is handed in, and in
//! between the command line's root settles what it reads of the telemetry
//! and hands in what the rest of the check needs of it. It constructs no
//! adapter: each root builds its tools and its provider through the wiring
//! kernels, and connects what they return itself.

mod root;
mod settings;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use lablet_config::{
    Config, ConfigError, Context, Format, KeyPath, Place, Provider, Refusal, ResolvedConfig, Tools,
    TranscriptFormat,
};
use lablet_model::{RunOutcome, StopReason, ToolSpec};
use lablet_provider_fake::{Script, ScriptFormat, ScriptSource};
use lablet_run::FilterList;
use lablet_secrets::{self as secrets, Credential, Derived, Handed, Named, Variables};

pub use root::{OwnFile, held};
pub use settings::{BuiltinSettings, Selected, Settings, System};

/// The target of this crate's warnings, so a `RUST_LOG` directive written
/// against the composition's lines catches them too.
const TARGET: &str = "lablet::lablet";

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
/// shows its value as the config writes it, `${VAR}` and all: what a
/// variable holds is in no message, and a credential's value never is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// The config states something a config may not, or a value that can't
    /// be used where lablet runs: a credential written out, a file that
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

/// The environment every process a run starts begins from, as the root
/// handed it in: the command line's own, or what a host chose. `None` when
/// nothing was handed in, which `bash` refuses.
pub type Base = Option<Vec<(OsString, OsString)>>;

/// What the environment a command starts from came to, which the root
/// computed from the base once the tools were built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    /// The variables of the base a command inherits as set, sorted: the
    /// base less the drops and the context variables.
    pub inherited: Vec<String>,
    /// The variables dropped by name: those that filled an own or given
    /// field.
    pub by_name: BTreeSet<String>,
    /// The variables dropped by value: those holding an own or given value
    /// under another name.
    pub by_value: BTreeSet<String>,
    /// Each variable dropped by value, with the field whose value it holds.
    pub copies: Vec<(KeyPath, String)>,
    /// The context variables a command is given in place of the ones it
    /// would inherit: the propagator's fields, as variables.
    pub context: Vec<String>,
}

/// An entry of `tools.builtin.env`, as `check` prints it under `env:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passed {
    /// The variable's name.
    pub name: String,
    /// Where it was written.
    pub place: Option<Place>,
    /// The variable it's a copy of, when the entry is one `${VAR}` of
    /// another name.
    pub copied_from: Option<String>,
}

impl fmt::Display for Passed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.copied_from {
            Some(_) => write!(
                f,
                "{}{} reaches bash uncut; if it's a credential, move it under \
                 tools.builtin.credentials",
                self.name,
                at(self.place)
            ),
            None => write!(f, "{}{} -> bash", self.name, at(self.place)),
        }
    }
}

/// What a config passed its check with: what it resolves to, the tools a
/// run of it is offered, its credentials and who gets each, and what its
/// commands inherit.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    resolved: ResolvedConfig,
    tools: Vec<ToolSpec>,
    own: Vec<Credential>,
    given: Vec<Credential>,
    env: Vec<Passed>,
    environment: Option<Environment>,
    cut: Vec<Named>,
}

impl Checked {
    /// What `config` passed its check with: the specs of the tools the root
    /// built for it, its `secrets`, and the `environment` its commands
    /// start from, when it has `bash`.
    #[must_use]
    pub fn new(
        config: &Config,
        tools: Vec<ToolSpec>,
        secrets: Derived,
        environment: Option<Environment>,
    ) -> Self {
        let Derived {
            mut own,
            mut given,
            dropped: _,
            cut,
            values: _,
            dropping: _,
        } = secrets;
        if let Some(environment) = &environment {
            for credential in own.iter_mut().chain(&mut given) {
                credential.copies = environment
                    .copies
                    .iter()
                    .filter(|(field, _)| *field == credential.field)
                    .map(|(_, name)| name.clone())
                    .collect();
            }
        }
        let env = config
            .tools
            .builtin
            .env
            .keys()
            .map(|name| {
                let key = KeyPath::of("tools.builtin.env").key(name);
                let copied_from = config
                    .filled()
                    .at(&key)
                    .filter(|fill| fill.is_one_variable())
                    .and_then(|fill| fill.names().next())
                    .filter(|variable| *variable != name)
                    .map(str::to_owned);
                Passed {
                    name: name.clone(),
                    place: config.place_at(&key),
                    copied_from,
                }
            })
            .collect();
        Self {
            resolved: config.resolved(),
            tools,
            own,
            given,
            env,
            environment,
            cut,
        }
    }

    /// The config with every default filled in, which is what `lablet
    /// check --resolved` prints: each filled setting as written, and a
    /// secret set in code as `[secret]`.
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

    /// Lablet's own credentials, each with its source, the field it fills
    /// and who gets it, which no process inherits.
    #[must_use]
    pub fn own(&self) -> &[Credential] {
        &self.own
    }

    /// The credentials given to `bash`, each under its entry's name.
    #[must_use]
    pub fn given(&self) -> &[Credential] {
        &self.given
    }

    /// The entries of `tools.builtin.env`, each passed to `bash` as set.
    #[must_use]
    pub fn env(&self) -> &[Passed] {
        &self.env
    }

    /// What `bash` inherits of the environment handed in, with what was
    /// dropped on the way; `None` when the config enables no `bash`.
    #[must_use]
    pub fn environment(&self) -> Option<&Environment> {
        self.environment.as_ref()
    }

    /// The names of the variables `bash` inherits as set, sorted.
    #[must_use]
    pub fn inherited(&self) -> &[String] {
        self.environment
            .as_ref()
            .map_or(&[], |environment| environment.inherited.as_slice())
    }

    /// The digest of the resolved config, which groups the runs made from
    /// it.
    #[must_use]
    pub fn digest(&self) -> lablet_model::ConfigDigest {
        self.resolved.digest()
    }

    /// The names whose values the command line still cuts without dropping
    /// them, the OTLP variables its seam sends, each followed by a note when
    /// its value isn't cut: not set, empty, or under the floor. Never a
    /// value.
    #[must_use]
    pub fn cut(&self) -> Vec<String> {
        self.cut.iter().map(ToString::to_string).collect()
    }
}

/// A config checked up to its telemetry, which the root settles before
/// the check goes on: what [`prepare`] comes to.
pub struct Settled {
    config: Config,
    settings: Settings,
    provider: Ready,
    system: String,
    base: Base,
}

/// What a root read of the telemetry's settings, which the rest of a check
/// goes on from.
#[derive(Debug, Default)]
pub struct Telemetry<'a> {
    /// The telemetry file the root writes, which the root of the built-in
    /// tools may not hold.
    pub file: Option<PathBuf>,
    /// The values the root still reads from its environment and sends, to
    /// be cut from what a tool returns without dropping their variables.
    pub handed: &'a [Handed<'a>],
}

/// What checking a config comes to, which a build goes on from once the
/// root has built its tools.
pub struct Prepared {
    config: Config,
    settings: Settings,
    provider: Ready,
    system: String,
    /// The telemetry file the root writes, when it writes one.
    telemetry_file: Option<PathBuf>,
    /// Whether content reaches telemetry.
    capture_content: bool,
    /// Lablet's credentials: what's dropped, what's given, what's cut, and
    /// the values when the run has an executor to hand them to.
    secrets: Derived,
    base: Base,
}

/// The provider a checked config selects, with what was read for it, so a
/// build reads no file its check didn't.
enum Ready {
    Anthropic,
    Openai,
    /// The fake provider, and the script it plays.
    Fake(Script),
}

/// Checks `config` whole, as it was filled, with `base` the environment
/// every process a run starts begins from. It stops before the telemetry,
/// which the root settles and hands to [`Settled::finish`], and the tools
/// are left for the root to build through `lablet-tools-wiring`, which
/// refuses what only building them finds.
///
/// # Errors
///
/// Returns the [`BuildError`] of the first thing the check refuses, in the
/// order a build meets them: a setting a rule refuses; a credential written
/// out, or with a default that isn't empty; a key the provider needs that
/// isn't set; a `tools.builtin.env` entry that names a credential's
/// variable, or one `bash` inherits already; a `credentials` entry `env`
/// sets too; `bash` with no environment handed in; what this lablet has no
/// adapter for; and a file the config names that can't be read.
pub fn prepare(config: &Config, base: Base) -> Result<Settled, BuildError> {
    let refused = |refusal| BuildError::Config(config.refused(refusal));
    let settings = Settings::of(config).map_err(refused)?;
    let variables = secrets::variables(config);
    credentials_are_variables(config)?;
    key_is_set(config)?;
    env_is_no_credential(config, &variables, base.as_deref())?;
    bash_has_a_base(config, &settings, base.is_some())?;
    warn_of_an_own_credential_given(config, &variables);
    supported(config)?;

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
            let name = config
                .written_text("model.script")
                .unwrap_or_else(|| script.display().to_string());
            Ready::Fake(read_script(script, &name).map_err(refused)?)
        }
    };
    Ok(Settled {
        config: config.clone(),
        settings,
        provider,
        system,
        base,
    })
}

impl Settled {
    /// The filled config, which is what runs, and what a root reads its
    /// telemetry's settings from.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The check gone on with what the root read of the telemetry: lablet's
    /// credentials derived, with the values the root hands in cut beside
    /// them.
    #[must_use]
    pub fn finish(self, telemetry: Telemetry<'_>) -> Prepared {
        let Self {
            config,
            settings,
            provider,
            system,
            base,
        } = self;
        let capture_content = config.telemetry.capture_content.unwrap_or(false);
        // The values are held only for an executor to cut, and the built-in
        // tools are the one executor a run can have.
        let secrets = secrets::derived(&config, telemetry.handed, settings.builtin.is_some());
        Prepared {
            config,
            settings,
            provider,
            system,
            telemetry_file: telemetry.file,
            capture_content,
            secrets,
            base,
        }
    }
}

impl Prepared {
    /// The filled config, which is what runs.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
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

    /// Lablet's credentials, derived.
    #[must_use]
    pub fn secrets(&self) -> &Derived {
        &self.secrets
    }

    /// The environment every process a run starts begins from, when one was
    /// handed in.
    #[must_use]
    pub fn base(&self) -> Option<&[(OsString, OsString)]> {
        self.base.as_deref()
    }

    /// Lablet's credentials, for a check that ends here.
    #[must_use]
    pub fn into_secrets(self) -> Derived {
        self.secrets
    }
}

/// What a checked config's `Lablet` is wired from, beside its telemetry and
/// its tools.
pub struct Wiring {
    /// The filled config, which is what runs.
    pub config: Config,
    /// What the loop and its adapters are built from.
    pub settings: Settings,
    /// The fake provider's script, the one provider there's an adapter for.
    pub script: Script,
    /// The system prompt.
    pub system: String,
    /// Whether content reaches telemetry.
    pub capture_content: bool,
    /// Lablet's credentials, whose values the loop cuts.
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
            config,
            settings,
            provider,
            system,
            telemetry_file: _,
            capture_content,
            secrets,
            base: _,
        } = self;
        let script = match provider {
            Ready::Fake(script) => script,
            Ready::Anthropic => return Err(Unsupported::Anthropic.into()),
            Ready::Openai => return Err(Unsupported::Openai.into()),
        };
        Ok(Wiring {
            config,
            settings,
            script,
            system,
            capture_content,
            secrets,
        })
    }
}

/// The refusal of the setting at `key`, with its value as the config writes
/// it where `shown` shows one.
fn refusal(config: &Config, key: &KeyPath, shown: Option<String>, reason: String) -> BuildError {
    BuildError::Config(ConfigError::Invalid {
        key: key.to_string(),
        place: config.place_at(key),
        value: shown,
        reason,
    })
}

/// The refusal of the setting at `key` that shows nothing of its value.
fn refusal_unshown(config: &Config, key: &KeyPath, reason: String) -> BuildError {
    BuildError::Config(ConfigError::Refused {
        key: key.to_string(),
        place: config.place_at(key),
        reason,
    })
}

/// Whether `text` is written as a variable's name is: capitals, digits and
/// `_`, with at least one `_`, as `ANTHROPIC_API_KEY` is. The one shape of
/// a credential written out that a message may show, since it's a name and
/// never a key: an access key id or a hex token is capitals and digits too,
/// and the `_` tells them apart.
fn is_written_as_a_variable(text: &str) -> bool {
    text.contains('_')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Holds each credential the config's text writes to a `${VAR}`:
/// `model.api_key` and each entry of `tools.builtin.credentials` written in
/// the text is a variable, with a default that's empty if it has one. A
/// credential the text didn't write was set in code, as a `Secret`, and
/// passes.
fn credentials_are_variables(config: &Config) -> Result<(), BuildError> {
    let key = KeyPath::of("model.api_key");
    if let Some(secret) = &config.model.api_key {
        credential_is_a_variable(config, &key, secret.expose_secret(), "api_key", "the key")?;
    } else if config.filled().at(&key).is_some() {
        // The key's variable is unset, and what the text wrote beside its
        // `${VAR}` may still be a key.
        credential_is_a_variable(config, &key, "", "api_key", "the key")?;
    }
    let credentials = KeyPath::of("tools.builtin.credentials");
    for (name, secret) in &config.tools.builtin.credentials {
        credential_is_a_variable(
            config,
            &credentials.key(name),
            secret.expose_secret(),
            name,
            "it",
        )?;
    }
    Ok(())
}

/// Holds the credential at `key`, which `entry` names in a fix, to one
/// `${VAR}` and nothing else, with no default but an empty one, where the
/// text wrote it; what's written beside a `${VAR}` may be the value, so
/// it's refused unshown as a literal is.
fn credential_is_a_variable(
    config: &Config,
    key: &KeyPath,
    value: &str,
    entry: &str,
    rotated: &str,
) -> Result<(), BuildError> {
    if config.place_at(key).is_none() {
        return Ok(());
    }
    let rotate = if rotated == "it" {
        "rotate it if this was the token".to_owned()
    } else {
        format!("rotate {rotated} if this was one")
    };
    match config.filled().at(key) {
        Some(fill) if !fill.variables.is_empty() => match fill.default() {
            Some(default) if !default.is_empty() => Err(refusal_unshown(
                config,
                key,
                format!(
                    "a credential's default can only be empty, and what follows :- isn't shown; \
                     write ${{{}:-}}",
                    fill.variables[0].name
                ),
            )),
            _ if !fill.is_one_variable() => Err(refusal_unshown(
                config,
                key,
                format!(
                    "it isn't one ${{VAR}} and nothing else, and what it holds isn't shown; write \
                     {entry}: ${{{}}}, and {rotate}",
                    fill.variables[0].name
                ),
            )),
            _ => Ok(()),
        },
        _ if is_written_as_a_variable(value) => Err(refusal(
            config,
            key,
            Some(format!("{value:?}")),
            format!("it's a variable's name, not a ${{VAR}}; write {entry}: ${{{value}}}"),
        )),
        _ => {
            let written = match config.place_at(key) {
                Some(Place::Override) => format!("--set '{key}=${{MY_KEY}}'"),
                _ => format!("{entry}: ${{{}}}", suggested_variable(entry)),
            };
            Err(refusal_unshown(
                config,
                key,
                format!(
                    "it holds no ${{VAR}}, and what it holds isn't shown; write {written}, and {rotate}"
                ),
            ))
        }
    }
}

/// The variable a fix names for `entry`: the entry's own name, or the key's
/// conventional one.
fn suggested_variable(entry: &str) -> String {
    if entry == "api_key" {
        "ANTHROPIC_API_KEY".to_owned()
    } else {
        entry.to_owned()
    }
}

/// Holds `model.api_key` to a key that's set, for a provider that needs
/// one. Nothing of what it holds is kept, and the refusal names the
/// variable as the config writes it.
fn key_is_set(config: &Config) -> Result<(), BuildError> {
    if config.model.provider != Provider::Anthropic {
        return Ok(());
    }
    let key = KeyPath::of("model.api_key");
    let fill = config.filled().at(&key);
    let needs = "the provider `anthropic` needs a key";
    match &config.model.api_key {
        Some(secret) if !secret.expose_secret().trim().is_empty() => Ok(()),
        Some(_) => match fill.and_then(|fill| fill.names().next()) {
            Some(variable) => Err(refusal(
                config,
                &key,
                config
                    .written_text("model.api_key")
                    .map(|text| format!("{text:?}")),
                format!("`{variable}` is empty, and {needs}; set it"),
            )),
            None => Err(refusal_unshown(
                config,
                &key,
                format!("it's empty, and {needs}"),
            )),
        },
        None => match fill.and_then(|fill| fill.names().next()) {
            Some(variable) => Err(refusal(
                config,
                &key,
                config
                    .written_text("model.api_key")
                    .map(|text| format!("{text:?}")),
                format!("`{variable}` isn't set, and {needs}; set it"),
            )),
            None if config.made_in_code() => Err(BuildError::Config(ConfigError::Missing {
                key: key.to_string(),
                reason: format!(
                    "{needs}, and a library reads no variable; set config.model.api_key = \
                     Some(Secret::new(key))"
                ),
            })),
            None => Err(BuildError::Config(ConfigError::Missing {
                key: key.to_string(),
                reason: format!("{needs}; add api_key: ${{ANTHROPIC_API_KEY}}"),
            })),
        },
    }
}

/// Holds `tools.builtin.env` apart from the credentials: an entry filled
/// from an own or given variable is refused, since the environment gives
/// nothing knowingly, and so is one that names its own variable, which
/// `bash` inherits already. A name both `env` and `credentials` set is
/// refused too.
fn env_is_no_credential(
    config: &Config,
    variables: &Variables,
    base: Option<&[(OsString, OsString)]>,
) -> Result<(), BuildError> {
    let env = KeyPath::of("tools.builtin.env");
    let credentials = KeyPath::of("tools.builtin.credentials");
    for name in config.tools.builtin.credentials.keys() {
        if config.tools.builtin.env.contains_key(name) {
            return Err(refusal_unshown(
                config,
                &credentials.key(name),
                format!(
                    "`tools.builtin.env` sets {name} too{}; keep one",
                    at(config.place_at(&env.key(name)))
                ),
            ));
        }
    }
    for name in config.tools.builtin.env.keys() {
        let key = env.key(name);
        let Some(fill) = config.filled().at(&key) else {
            continue;
        };
        let shown = || Some(format!("{:?}", fill.written));
        for variable in fill.names() {
            if let Some(field) = variables.own_field(variable) {
                let what = if *field == KeyPath::of("model.api_key") {
                    "the key"
                } else {
                    "the value"
                };
                return Err(refusal(
                    config,
                    &key,
                    shown(),
                    format!(
                        "`{variable}` fills `{field}`, and no process inherits an own credential; \
                         to give bash {what} knowingly, list {name} under `tools.builtin.credentials`"
                    ),
                ));
            }
            if let Some(entry) = variables.given_entry(variable) {
                return Err(refusal(
                    config,
                    &key,
                    shown(),
                    format!(
                        "`{variable}` is given under `credentials`{}; list {name} there instead, \
                         as {name}: ${{{variable}}}",
                        at(config.place_at(entry))
                    ),
                ));
            }
            let inherited =
                base.is_some_and(|base| base.iter().any(|(set, _)| set.to_str() == Some(variable)));
            if variable == name && fill.is_one_variable() && inherited {
                return Err(refusal(
                    config,
                    &key,
                    shown(),
                    format!(
                        "bash inherits `{name}` as set already, uncut; remove the line, or move it \
                         under `tools.builtin.credentials` to have it cut"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Holds `bash` to an environment handed in, since a command starts from
/// nothing else.
fn bash_has_a_base(config: &Config, settings: &Settings, handed: bool) -> Result<(), BuildError> {
    let runs_bash = settings
        .builtin
        .as_ref()
        .is_some_and(|builtin| builtin.enabled.contains(&lablet_tools_builtin::Tool::Bash));
    if !runs_bash || handed {
        return Ok(());
    }
    // `bash` sorts first among the tools, so it's the list's first item as
    // the config comes to it.
    let key = KeyPath::of("tools.builtin.enabled").index(0);
    Err(refusal(
        config,
        &key,
        Some("\"bash\"".to_owned()),
        "no process environment was handed in; hand in what commands need, PATH among them, \
         as .with_process_environment([(\"PATH\", \"/usr/bin:/bin\")])"
            .to_owned(),
    ))
}

/// Warns of an own credential listed under `tools.builtin.credentials` too:
/// allowed, so a task that needs the key gets it when the config says so,
/// and said, since `bash` then has it.
fn warn_of_an_own_credential_given(config: &Config, variables: &Variables) {
    for (variable, entry) in &variables.given {
        let Some(field) = variables.own_field(variable) else {
            continue;
        };
        let what = if *field == KeyPath::of("model.api_key") {
            "lablet's own key".to_owned()
        } else {
            format!("lablet's own `{field}`")
        };
        tracing::warn!(
            target: TARGET,
            "{entry}{}: it's {what}, given to bash; a task that needs the key gets it only when \
             the config says so",
            at(config.place_at(entry))
        );
    }
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
