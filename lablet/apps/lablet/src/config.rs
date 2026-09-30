//! The config: what a `Lablet` is built from, read from YAML or JSON.
//!
//! A key the config doesn't know is an error, and that's the whole policy:
//! nothing is ignored, so nothing a config states is without effect.
//!
//! A path is taken as the config writes it. One that isn't absolute starts
//! at the working directory, wherever the config's own file is, so a config
//! read from text and the same config read from a file name the same
//! files.

mod de;
mod key;
mod model;
mod prompt;
mod raw;
mod resolved;
mod run;
mod substitute;
mod telemetry;
mod tools;
mod written;

use std::fmt;
use std::path::{Path, PathBuf};

use lablet_model::ConfigDigest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) use key::KeyPath;
use key::Places;

pub use key::Place;
pub(crate) use model::Setting;
pub use model::{Api, CacheScope, Effort, Model, Pricing, Provider, Thinking};
pub use prompt::{Prompt, SkillsMode};
pub use raw::RawConfig;
pub use resolved::{Applied, ResolvedBuiltin, ResolvedConfig, ResolvedModel, ResolvedTools};
pub use run::{Completion, Context, Run, TranscriptFormat};
pub(crate) use substitute::Env;
pub use telemetry::{Otlp, OtlpProtocol, Telemetry, TelemetryFile};
pub use tools::{
    Builtin, BuiltinTool, McpLifetime, McpNames, McpResult, McpServer, OutputCut, Tools,
};

/// The format a config's text is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// YAML.
    Yaml,
    /// JSON.
    Json,
}

impl Format {
    /// The format a file of this name is in, which its extension says:
    /// `.yaml` or `.yml`, or `.json`. `None` for any other name.
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "yaml" | "yml" => Some(Self::Yaml),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// The format's name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Yaml => "YAML",
            Self::Json => "JSON",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a config was refused.
///
/// Each refusal of a setting names its key and where it was written, and
/// shows its value as the config writes it, before `${VAR}` substitution,
/// so nothing a variable holds reaches a message. `model.api_key_env`'s
/// value is shown only when it's written in capitals, digits and `_`, as a
/// variable's name is and few key formats are: anything else there may be
/// a key written where its variable's name belongs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The config's file couldn't be read.
    #[error("the config {path} couldn't be read: {reason}")]
    Unreadable {
        /// The file.
        path: String,
        /// What the operating system said.
        reason: String,
    },
    /// The name of the config's file doesn't say which format it's in.
    #[error("the config {path} is named as neither YAML (`.yaml`, `.yml`) nor JSON (`.json`)")]
    UnknownFormat {
        /// The file.
        path: String,
    },
    /// The text isn't in its format, writes a key twice, or isn't a mapping
    /// of sections.
    #[error("the config can't be read as {format}: {reason}")]
    Syntax {
        /// The format the text was read as.
        format: Format,
        /// The reader's own words, with the place.
        reason: String,
    },
    /// An override isn't one a config can take.
    #[error("{}the override is refused: {reason}", key_of(key.as_deref()))]
    Override {
        /// The key the override names, when it names one.
        key: Option<String>,
        /// What's wrong with it.
        reason: String,
    },
    /// The config states a key that no setting has.
    #[error("{key}{} is refused: no setting has the key; the keys beside it are {known}", at(*place))]
    UnknownKey {
        /// The key.
        key: String,
        /// Where it was written.
        place: Option<Place>,
        /// The keys the section has.
        known: String,
    },
    /// A setting holds a value of the wrong kind, one it doesn't take, or
    /// one that breaks a rule of the config, and the refusal says which
    /// values it takes.
    #[error("{key}{}: {} is refused: {reason}", at(*place), value.as_deref().unwrap_or("its value"))]
    Invalid {
        /// The setting.
        key: String,
        /// Where it was written; `None` for a setting the config doesn't
        /// state, whose default was refused beside what it does state.
        place: Option<Place>,
        /// The value it holds, as the config writes it; `None` for a value
        /// of `model.api_key_env` that isn't written as a variable's name.
        value: Option<String>,
        /// The rule the value breaks, or the values the setting takes.
        reason: String,
    },
    /// `model.api_key_env` holds what no variable is named. A key that was
    /// written where the variable's name belongs is a secret, so the
    /// refusal holds nothing of the value.
    #[error("model.api_key_env{} is refused: {reason}", at(*place))]
    KeyVariable {
        /// Where it was written.
        place: Option<Place>,
        /// The rule the value breaks.
        reason: &'static str,
    },
    /// A setting the config needs isn't stated.
    #[error("{key} isn't set: {reason}")]
    Missing {
        /// The setting.
        key: String,
        /// Why it's needed.
        reason: String,
    },
    /// The config states a setting that the provider it selects can't
    /// apply. A setting that was ignored would make two runs look different
    /// that weren't.
    #[error("{key}{}: {} is refused: {reached} can't apply it", at(*place), value.as_deref().unwrap_or("its value"))]
    NotApplied {
        /// The setting.
        key: &'static str,
        /// Where it was written.
        place: Option<Place>,
        /// The value the config states, as it writes it.
        value: Option<String>,
        /// The provider, with its API when it has more than one.
        reached: String,
    },
}

/// Where a setting was written, as a message says it after the key.
fn at(place: Option<Place>) -> String {
    place.map(|place| format!(" ({place})")).unwrap_or_default()
}

fn key_of(key: Option<&str>) -> String {
    key.map(|key| format!("{key}: ")).unwrap_or_default()
}

/// The longest a message shows of a value, in characters.
const SHOWN_CHARS: usize = 120;

/// A value as a message shows it, which is as JSON writes it, cut to
/// [`SHOWN_CHARS`]. `model.api_key_env`'s is shown only when it's written as
/// a variable's name is.
pub(crate) fn shown(key: &KeyPath, value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;
    if *key == KeyPath::of("model.api_key_env")
        && !value.as_str().is_some_and(model::is_written_as_a_variable)
    {
        return None;
    }
    let written = value.to_string();
    Some(match written.char_indices().nth(SHOWN_CHARS) {
        Some((cut, _)) => format!("{}…", &written[..cut]),
        None => written,
    })
}

/// A setting refused where lablet checks a config, by its key alone: the
/// config the refusal is shown from says what it holds and where it was
/// written, so a value a variable gave is never shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    Invalid { key: KeyPath, reason: String },
    NotApplied { setting: Setting, reached: String },
    Missing { key: &'static str, reason: String },
    KeyVariable { reason: &'static str },
}

impl Refusal {
    pub(crate) fn invalid(key: &str, reason: impl Into<String>) -> Self {
        Self::Invalid {
            key: KeyPath::of(key),
            reason: reason.into(),
        }
    }
}

/// What a `Lablet` is built from.
///
/// The sections hold what the config states, as it's written, with
/// lablet's default wherever it states nothing and every provider shares
/// the default. A setting of the `model` section that's one provider's is
/// held as it was stated, and [`Config::resolved`] fills in its default.
/// `${VAR}` is held as it's written, and substituted when a `Lablet` is
/// built from the config or the config is checked.
///
/// Reading a config checks its shape. What a config may not state is
/// refused when it's checked, and when a `Lablet` is built from it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// How a run completes, what bounds it, and where its transcript goes.
    pub run: Run,
    /// The provider, the model, and the request settings.
    pub model: Model,
    /// The system prompt and the skills.
    pub prompt: Prompt,
    /// The tools a run offers, and what bounds a call.
    pub tools: Tools,
    /// What telemetry captures, and where it's exported.
    pub telemetry: Telemetry,
    /// The file the config was read from, which the root of the built-in
    /// tools may not hold.
    #[serde(skip)]
    source: Option<PathBuf>,
    /// Where each setting was written, for the messages that name it.
    #[serde(skip)]
    places: Places,
    /// The file a run's task prompt is read from, which the root of the
    /// built-in tools may not hold either.
    #[serde(skip)]
    prompt_file: Option<PathBuf>,
}

impl Config {
    /// The config `text` holds: [`RawConfig::from_str`] and
    /// [`RawConfig::config`], with no override between them.
    ///
    /// # Errors
    ///
    /// Returns what those two return.
    pub fn from_str(text: &str, format: Format) -> Result<Self, ConfigError> {
        RawConfig::from_str(text, format)?.config()
    }

    /// The config the file at `path` holds, in the format its name says:
    /// [`RawConfig::from_path`] and [`RawConfig::config`], with no override
    /// between them.
    ///
    /// # Errors
    ///
    /// Returns what those two return.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        RawConfig::from_path(path)?.config()
    }

    /// The file the config was read from; `None` for one read from text.
    #[must_use]
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// The same config, told that a run's task prompt is read from the file
    /// at `path`, as `lablet run --prompt-file` reads it. The root of the
    /// built-in tools may not hold it, as it may not hold the system
    /// prompt's file: a run could read what it's measured with, and write
    /// over the task of the next.
    #[must_use]
    pub fn with_prompt_file(self, path: impl Into<PathBuf>) -> Self {
        Self {
            prompt_file: Some(path.into()),
            ..self
        }
    }

    /// The file a run's task prompt is read from, when the config was told
    /// of one.
    #[must_use]
    pub fn prompt_file(&self) -> Option<&Path> {
        self.prompt_file.as_deref()
    }

    /// The config with every default filled in, and without the settings
    /// that nothing of it applies: a default the provider can't apply, how
    /// an output is cut where nothing is, the length of a preview under a
    /// cut that makes none, the MCP settings where there's no server, and
    /// the built-in tools' settings where none is enabled.
    ///
    /// It holds the config as it's written, so a `${VAR}` is in it as it's
    /// written and nothing a variable holds is.
    #[must_use]
    pub fn resolved(&self) -> ResolvedConfig {
        let Self {
            run,
            model,
            prompt,
            tools,
            telemetry,
            source: _,
            places: _,
            prompt_file: _,
        } = self;
        let applied = |setting| model.applies(setting);
        ResolvedConfig {
            run: run.clone(),
            model: ResolvedModel {
                provider: model.provider,
                api: model.openai_api().map_or(Applied::No, Applied::Yes),
                script: when(applied(Setting::Script), model.script.clone()),
                name: model.name.clone(),
                api_key_env: model.key_variable().map(str::to_owned),
                base_url: when(applied(Setting::BaseUrl), model.base_url.clone()),
                max_tokens: model.max_tokens,
                temperature: model.temperature,
                thinking: when(
                    applied(Setting::Thinking),
                    model.thinking.unwrap_or_default(),
                ),
                effort: when(applied(Setting::Effort), model.effort),
                seed: when(applied(Setting::Seed), model.seed),
                reasoning_replay: when(
                    applied(Setting::ReasoningReplay),
                    model.reasoning_replay.unwrap_or(false),
                ),
                cache: when(applied(Setting::Cache), model.cache.unwrap_or(true)),
                cache_scope: model.cache_scope,
                pricing: model.pricing,
            },
            prompt: prompt.clone(),
            tools: tools.into(),
            telemetry: telemetry.clone(),
        }
    }

    /// The digest of the resolved config, which groups the runs made from
    /// it: [`ResolvedConfig::digest`]. It's of the config as it's written,
    /// so a value a variable gives never enters it.
    #[must_use]
    pub fn digest(&self) -> ConfigDigest {
        self.resolved().digest()
    }

    /// `refusal` as this config shows it: with the value of the setting as
    /// it's written here, and where it was written.
    pub(crate) fn refused(&self, refusal: Refusal) -> ConfigError {
        // Nothing of a config fails to serialise: a path is written as
        // it's shown, and a number JSON can't hold is written as `null`.
        let written = serde_json::to_value(self).unwrap_or_default();
        let value = |key: &KeyPath| shown(key, key.find(&written));
        match refusal {
            Refusal::Invalid { key, reason } => ConfigError::Invalid {
                place: self.places.of(&key),
                value: value(&key),
                key: key.to_string(),
                reason,
            },
            Refusal::NotApplied { setting, reached } => {
                let key = KeyPath::of(setting.key());
                ConfigError::NotApplied {
                    key: setting.key(),
                    place: self.places.of(&key),
                    value: value(&key),
                    reached,
                }
            }
            Refusal::Missing { key, reason } => ConfigError::Missing {
                key: key.to_owned(),
                reason,
            },
            Refusal::KeyVariable { reason } => ConfigError::KeyVariable {
                place: self.places.of(&KeyPath::of("model.api_key_env")),
                reason,
            },
        }
    }

    /// Where the setting at `key` was written, when it was.
    pub(crate) fn place_of(&self, key: &str) -> Option<Place> {
        self.places.of(&KeyPath::of(key))
    }

    /// Where item `index` of the list at `key` was written, when it was.
    pub(crate) fn place_of_item(&self, key: &str, index: usize) -> Option<Place> {
        self.places.of(&KeyPath::of(key).index(index))
    }

    /// The setting at `key` as this config writes it, for a message that
    /// shows it in its own words: a text as it is, anything else as JSON
    /// writes it.
    pub(crate) fn written_text(&self, key: &str) -> Option<String> {
        let written = serde_json::to_value(self).ok()?;
        let key = KeyPath::of(key);
        match key.find(&written)? {
            serde_json::Value::String(text) => Some(text.clone()),
            other => shown(&key, Some(other)),
        }
    }
}

/// The JSON Schema of a config, as `lablet schema` prints it and
/// `lablet/schema.json` holds it: every key, the values each takes and its
/// default, from the types a config is read into, so the schema can't say
/// what the reader doesn't.
#[must_use]
pub fn schema() -> serde_json::Value {
    schemars::schema_for!(Config).to_value()
}

/// `value` as a setting that's applied, and nothing for one that isn't.
fn when<T>(applied: bool, value: T) -> Applied<T> {
    if applied {
        Applied::Yes(value)
    } else {
        Applied::No
    }
}

#[cfg(test)]
mod tests;
