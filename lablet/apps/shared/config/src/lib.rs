//! The config: what a `Lablet` is built from, read from YAML or JSON.
//!
//! A key the config doesn't know is an error, and that's the whole policy:
//! nothing is ignored, so nothing a config states is without effect.
//!
//! A path is taken as the config writes it, and one that isn't absolute is
//! the caller's to place: [`Config::relative_to`] makes every relative path
//! the file wrote absolute against a directory, the file's own, and leaves
//! one an override gave where lablet runs, while the written form is kept
//! for every message, the resolved config and the digest. A config made in
//! code or from text has no directory, so a relative path in one is refused
//! when the config is checked unless `relative_to` was called.
//!
//! A `${VAR}` is filled before the config is typed, by
//! [`RawConfig::substituted`], from wherever the caller says a variable's
//! value comes: the config itself reads no environment. What the fill
//! changed is recorded on the config, so every message, the resolved config
//! and the digest show a filled setting as the file wrote it, through one
//! mask, [`shown`].

mod de;
mod key;
mod model;
mod prompt;
mod raw;
mod resolved;
mod run;
mod secret;
mod shown;
mod substitute;
mod telemetry;
mod tools;
mod url;
mod written;

use std::fmt;
use std::path::{Path, PathBuf};

use lablet_model::ConfigDigest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use key::{KeyPath, Place, Places};
pub use model::Setting;
pub use model::{Api, CacheScope, Effort, Model, Pricing, Provider, Thinking, ThinkingDisplay};
pub use prompt::{Prompt, SkillsMode};
pub use raw::RawConfig;
pub use resolved::{
    Applied, Commented, ResolvedBuiltin, ResolvedConfig, ResolvedModel, ResolvedTools,
};
pub use run::{Completion, Context, Run, TranscriptFormat};
pub use secret::Secret;
pub use shown::{Masked, WrittenBack, is_credential, shown};
pub use substitute::{Fill, Filled, Variable, came_out_unstated};
pub use telemetry::{
    Headers, Otlp, OtlpProtocol, Propagator, Propagators, Resource, Telemetry, TelemetryFile, Tls,
};
pub use tools::{
    Builtin, BuiltinTool, McpLifetime, McpNames, McpResult, McpServer, OutputCut, Tools,
};
pub use url::{
    Unreadable, WrittenInformation, holds_an_unread_at, host_and_port, outside_user_information,
    unreadable, user_information, without_user_information, writes_an_unread_at,
    written_information,
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
/// shows its value as the config writes it, `${VAR}` and all, so nothing a
/// variable holds reaches a message. A credential's value is never shown,
/// nor a value that may hold one: [`shown`] decides.
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
        /// that's never shown, a credential's or one that may hold one.
        value: Option<String>,
        /// The rule the value breaks, or the values the setting takes.
        reason: String,
    },
    /// A setting is refused and nothing of its value is shown: a credential
    /// written out, or a setting refused for what stands beside it.
    #[error("{key}{} is refused: {reason}", at(*place))]
    Refused {
        /// The setting.
        key: String,
        /// Where it was written.
        place: Option<Place>,
        /// The rule the value breaks, and the fix.
        reason: String,
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

/// A setting refused where lablet checks a config, by its key alone: the
/// config the refusal is shown from says what it holds and where it was
/// written, so a value a variable gave is never shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The value at `key` is refused, for `reason`.
    Invalid {
        /// The key of the setting.
        key: KeyPath,
        /// Why it's refused.
        reason: String,
    },
    /// The provider or API the config reaches doesn't apply `setting`.
    NotApplied {
        /// The setting.
        setting: Setting,
        /// What the config reaches, as a message names it.
        reached: String,
    },
    /// `key` holds nothing where it must hold something.
    Missing {
        /// The key of the setting.
        key: &'static str,
        /// Why it must.
        reason: String,
    },
}

impl Refusal {
    /// The value at `key` refused, for `reason`.
    #[must_use]
    pub fn invalid(key: &str, reason: impl Into<String>) -> Self {
        Self::Invalid {
            key: KeyPath::of(key),
            reason: reason.into(),
        }
    }
}

/// What a `Lablet` is built from.
///
/// The sections hold what the config states, with lablet's default wherever
/// it states nothing and every provider shares the default. A setting of
/// the `model` section that's one provider's is held as it was stated, and
/// [`Config::resolved`] fills in its default. Read through a [`RawConfig`],
/// every `${VAR}` was filled before the config was typed, by
/// [`RawConfig::substituted`], and the
/// config keeps the record of what the fill changed, so its `Debug` form,
/// its messages and its resolved form show each filled setting as the file
/// wrote it. Its serialisation shows the typed values, a secret as
/// `[secret]`; it's what the mask reads.
///
/// Reading a config checks its shape. What a config may not state is
/// refused when it's checked, and when a `Lablet` is built from it.
///
/// Its own `Deserialize` is the in-code path, as a struct literal is: it
/// fills nothing and keeps no record, so a `${VAR}` in what it reads is
/// text, and a credential in it counts as set in code. A text with
/// `${VAR}`s to fill is read through [`RawConfig`].
#[derive(Clone, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
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
    /// What the fill changed, each key with the text as written, and
    /// whether it ran; and each path [`Config::relative_to`] made absolute,
    /// as the file wrote it.
    #[serde(skip)]
    filled: Filled,
    /// Whether [`Config::relative_to`] placed the relative paths, so one
    /// left relative is an override's, which starts where lablet runs.
    #[serde(skip)]
    based: bool,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Config")
            .field(&shown::debug_tree(self, &self.filled))
            .finish()
    }
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

    /// What the fill changed: each key a `${VAR}` or a `$${` was written
    /// at, with the text as written and the variables it named, and each
    /// path [`Config::relative_to`] placed, with no variable.
    #[must_use]
    pub fn filled(&self) -> &Filled {
        &self.filled
    }

    /// Where each setting was written, for a message or a listing that
    /// names a setting's line or its override.
    #[must_use]
    pub fn places(&self) -> &Places {
        &self.places
    }

    /// Whether [`Config::relative_to`] was called, so every relative path
    /// left in the config is an override's and starts where lablet runs.
    #[must_use]
    pub fn based(&self) -> bool {
        self.based
    }

    /// The config with every relative path the file wrote placed under
    /// `base`, the file's directory, so a path in the file starts there
    /// wherever lablet runs. The paths are absolute when `base` is: a
    /// relative `base`, as `Path::new("lablet.yaml").parent()` gives, leaves
    /// them relative to where the process runs, since lablet reads no
    /// working directory of its own. A path an override gave is left as it
    /// is, since it starts where lablet runs, and so is `-`, which is
    /// standard error and no path. Each path placed is recorded as the
    /// file wrote it, so every message, the resolved config and the digest
    /// show the written form, and one file has one digest on every machine.
    #[must_use]
    pub fn relative_to(mut self, base: impl AsRef<Path>) -> Self {
        let base = base.as_ref();
        let mut placed: Vec<(KeyPath, String, String)> = Vec::new();
        let mut place = |key: KeyPath, path: &mut PathBuf, written_at: &Places| {
            let is_stderr = key == KeyPath::of("telemetry.file.path") && path.as_os_str() == "-";
            if path.is_absolute() || is_stderr || written_at.of(&key) == Some(Place::Override) {
                return;
            }
            let joined = base.join(&*path);
            placed.push((
                key,
                path.display().to_string(),
                joined.display().to_string(),
            ));
            *path = joined;
        };
        let written_at = &self.places;
        let mut paths: Vec<(KeyPath, &mut PathBuf)> = Vec::new();
        paths.extend(
            self.model
                .script
                .iter_mut()
                .map(|path| (KeyPath::of("model.script"), path)),
        );
        paths.extend(
            self.prompt
                .system_file
                .iter_mut()
                .map(|path| (KeyPath::of("prompt.system_file"), path)),
        );
        paths.extend(
            self.prompt
                .skills
                .iter_mut()
                .enumerate()
                .map(|(index, path)| (KeyPath::of("prompt.skills").index(index), path)),
        );
        paths.extend(
            self.run
                .transcript_path
                .iter_mut()
                .map(|path| (KeyPath::of("run.transcript_path"), path)),
        );
        paths.extend(
            self.tools
                .builtin
                .root
                .iter_mut()
                .map(|path| (KeyPath::of("tools.builtin.root"), path)),
        );
        paths.extend(
            self.telemetry
                .file
                .path
                .iter_mut()
                .map(|path| (KeyPath::of("telemetry.file.path"), path)),
        );
        let tls = &mut self.telemetry.otlp.tls;
        paths.extend(
            [
                ("ca_file", &mut tls.ca_file),
                ("client_cert", &mut tls.client_cert),
                ("client_key", &mut tls.client_key),
            ]
            .into_iter()
            .filter_map(|(name, path)| {
                path.as_mut()
                    .map(|path| (KeyPath::of("telemetry.otlp.tls").key(name), path))
            }),
        );
        for (key, path) in paths {
            place(key, path, written_at);
        }
        self.filled = self.filled.rebased(placed);
        self.based = true;
        self
    }

    /// Whether the config was made in code rather than read from text, so
    /// a credential in it was set as a `Secret` and no setting has a place.
    #[must_use]
    pub fn made_in_code(&self) -> bool {
        !self.places.are_known()
    }

    /// The config with every default filled in, and without the settings
    /// that nothing of it applies: a default the provider can't apply, how
    /// an output is cut where nothing is, the length of a preview under a
    /// cut that makes none, the MCP settings where there's no server, and
    /// the built-in tools' settings where none is enabled.
    ///
    /// It carries the record of what the fill changed, so written out it
    /// shows each filled setting as the file wrote it.
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
            filled,
            based: _,
        } = self;
        let applied = |setting| model.applies(setting);
        ResolvedConfig {
            run: run.clone(),
            model: ResolvedModel {
                provider: model.provider,
                api: model.openai_api().map_or(Applied::No, Applied::Yes),
                script: when(applied(Setting::Script), model.script.clone()),
                name: model.name.clone(),
                api_key: model.api_key.clone(),
                base_url: when(applied(Setting::BaseUrl), model.base_url.clone()),
                max_tokens: model.max_tokens,
                temperature: model.temperature,
                thinking: when(
                    applied(Setting::Thinking),
                    model.thinking.unwrap_or_default(),
                ),
                thinking_display: when(applied(Setting::ThinkingDisplay), model.thinking_display),
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
            filled: filled.clone(),
        }
    }

    /// The digest of the resolved config, which groups the runs made from
    /// it: [`ResolvedConfig::digest`].
    #[must_use]
    pub fn digest(&self) -> ConfigDigest {
        self.resolved().digest()
    }

    /// `refusal` as this config shows it: with the value of the setting as
    /// the file wrote it, where [`shown`] shows one, and where it was
    /// written.
    #[must_use]
    pub fn refused(&self, refusal: Refusal) -> ConfigError {
        // Nothing of a config fails to serialise: a path is written as
        // it's shown, and a number JSON can't hold is written as `null`.
        let written = serde_json::to_value(self).unwrap_or_default();
        let value = |key: &KeyPath| shown(key, key.find(&written), &self.filled);
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
        }
    }

    /// Where the setting at `key` was written, when it was.
    #[must_use]
    pub fn place_of(&self, key: &str) -> Option<Place> {
        self.places.of(&KeyPath::of(key))
    }

    /// Where item `index` of the list at `key` was written, when it was.
    #[must_use]
    pub fn place_of_item(&self, key: &str, index: usize) -> Option<Place> {
        self.places.of(&KeyPath::of(key).index(index))
    }

    /// Where the setting at `key` was written, when it was.
    #[must_use]
    pub fn place_at(&self, key: &KeyPath) -> Option<Place> {
        self.places.of(key)
    }

    /// The setting at `key` as the file wrote it, for a message that shows
    /// it in its own words: a text as it is, anything else as JSON writes
    /// it, and nothing for what [`shown`] never shows.
    #[must_use]
    pub fn written_text(&self, key: &str) -> Option<String> {
        self.written_text_at(&KeyPath::of(key))
    }

    /// [`Config::written_text`] for a setting named by a [`KeyPath`], as an
    /// item of a list is.
    #[must_use]
    pub fn written_text_at(&self, key: &KeyPath) -> Option<String> {
        let written = serde_json::to_value(self).ok()?;
        match shown::shown_whole(key, key.find(&written)?, &self.filled)? {
            serde_json::Value::String(text) => Some(text),
            other => shown(key, Some(&other), &Filled::default()),
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
