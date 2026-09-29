//! The config: what a `Lablet` is built from, read from YAML or JSON.
//!
//! A key the config doesn't know is an error, and that's the whole policy:
//! nothing is ignored, so nothing a config states is without effect.
//!
//! A path is taken as the config writes it. One that isn't absolute starts
//! at the working directory, wherever the config's own file is, so a config
//! read from text and the same config read from a file name the same
//! files.

mod model;
mod prompt;
mod resolved;
mod run;
mod telemetry;
mod tools;
mod written;

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub(crate) use model::Setting;
pub use model::{Api, CacheScope, Effort, Model, Pricing, Provider, Thinking};
pub use prompt::{Prompt, SkillsMode};
pub use resolved::{Applied, ResolvedConfig, ResolvedModel};
pub use run::{Completion, Context, Run, TranscriptFormat};
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

/// Why a config was refused. Each refusal of a setting names its key and
/// the value that was refused.
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
    /// The text isn't a config: it isn't in the format, it holds a key the
    /// config doesn't know, or a key holds a value of the wrong kind.
    #[error("the config can't be read as {format}: {reason}")]
    Syntax {
        /// The format the text was read as.
        format: Format,
        /// The parser's own words, with the key and the place.
        reason: String,
    },
    /// A setting holds a value that breaks a rule of the config.
    #[error("{key}: {value} is refused: {reason}")]
    Invalid {
        /// The setting.
        key: String,
        /// The value it holds, as a config writes it.
        value: String,
        /// The rule the value breaks.
        reason: String,
    },
    /// A setting the config needs isn't stated.
    #[error("{key} isn't set: {reason}")]
    Missing {
        /// The setting.
        key: &'static str,
        /// Why it's needed.
        reason: String,
    },
    /// The config states a setting that the provider it selects can't
    /// apply. A setting that was ignored would make two runs look different
    /// that weren't.
    #[error("{key}: {value} is refused: {reached} can't apply it")]
    NotApplied {
        /// The setting.
        key: &'static str,
        /// The value the config states.
        value: String,
        /// The provider, with its API when it has more than one.
        reached: String,
    },
}

/// What a `Lablet` is built from.
///
/// The sections hold what the config states, with lablet's default wherever
/// it states nothing and every provider shares the default. A setting of
/// the `model` section that's one provider's is held as it was stated, and
/// [`Config::resolved`] fills in its default.
///
/// Reading a config checks its shape. What a config may not state is
/// refused when a `Lablet` is built from it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
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
}

impl Config {
    /// The config `text` holds.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Syntax`] when the text isn't in `format`,
    /// holds a key the config doesn't know, or holds a value of the wrong
    /// kind for its key.
    pub fn from_str(text: &str, format: Format) -> Result<Self, ConfigError> {
        match format {
            // Older YAML reads `yes` and `no` as booleans, and a system
            // prompt that says `yes` would be refused for holding one.
            Format::Yaml => serde_saphyr::from_str_with_options(
                text,
                serde_saphyr::options! {
                    strict_booleans: true,
                    with_snippet: false,
                },
            )
            .map_err(|error| error.to_string()),
            Format::Json => serde_json::from_str(text).map_err(|error| error.to_string()),
        }
        .map_err(|reason| ConfigError::Syntax { format, reason })
    }

    /// The config the file at `path` holds, in the format its name says.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::UnknownFormat`] when the file's name ends in
    /// none of `.yaml`, `.yml` and `.json`, [`ConfigError::Unreadable`]
    /// when the file can't be read, and what [`Config::from_str`] returns
    /// for what it holds.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let shown = || path.display().to_string();
        let format =
            Format::of(path).ok_or_else(|| ConfigError::UnknownFormat { path: shown() })?;
        let text = std::fs::read_to_string(path).map_err(|error| ConfigError::Unreadable {
            path: shown(),
            reason: error.to_string(),
        })?;
        let config = Self::from_str(&text, format)?;
        Ok(Self {
            source: Some(path.to_owned()),
            ..config
        })
    }

    /// The file the config was read from; `None` for one read from text.
    #[must_use]
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// The config with every default filled in, and without the defaults
    /// the provider can't apply.
    #[must_use]
    pub fn resolved(&self) -> ResolvedConfig {
        let Self {
            run,
            model,
            prompt,
            tools,
            telemetry,
            source: _,
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
                base_url: model.base_url.clone(),
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
            tools: tools.clone(),
            telemetry: telemetry.clone(),
        }
    }

    /// The digest of the resolved config, which groups the runs made from
    /// it: [`ResolvedConfig::digest`].
    #[must_use]
    pub fn digest(&self) -> String {
        self.resolved().digest()
    }
}

/// `value` as the setting of a provider that applies it, and nothing for
/// one that can't.
fn when<T>(applied: bool, value: T) -> Applied<T> {
    if applied {
        Applied::Yes(value)
    } else {
        Applied::No
    }
}

#[cfg(test)]
mod tests;
