//! The `model` section: the provider, the model, and the request settings.
//!
//! Some settings are one provider's, and the config holds each of those as
//! it was stated, so that a setting left out is told from one spelled out:
//! the first takes the provider's default where the provider has one, and
//! the second is refused where the provider can't apply it.

use std::fmt;
use std::num::NonZeroU32;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The `model` section.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Model {
    /// The provider the model is reached through.
    pub provider: Provider,
    /// `openai` only: the API; `None` is the Responses API without a
    /// `base_url` and chat completions with one.
    pub api: Option<Api>,
    /// `fake` only: the file of scripted responses.
    pub script: Option<PathBuf>,
    /// The provider's name for the model.
    pub name: String,
    /// The environment variable that holds the API key, and never the key.
    /// `None` is `ANTHROPIC_API_KEY` for `anthropic` and no variable for
    /// the others.
    pub api_key_env: Option<String>,
    /// Where the API is served, for a gateway or a local server.
    pub base_url: Option<String>,
    /// The cap on output tokens for each call.
    pub max_tokens: u32,
    /// The sampling temperature, sent only when set.
    pub temperature: Option<f64>,
    /// `anthropic` only: how the model is asked to reason; `None` is the
    /// provider's default.
    pub thinking: Option<Thinking>,
    /// `anthropic`, and `openai` with the Responses API: the reasoning
    /// effort.
    pub effort: Option<Effort>,
    /// `openai` only: the sampling seed.
    pub seed: Option<i64>,
    /// Chat completions only: whether reasoning is sent back; `None` is
    /// `false`.
    pub reasoning_replay: Option<bool>,
    /// `anthropic` only: whether requests carry cache breakpoints; `None`
    /// is `true`.
    pub cache: Option<bool>,
    /// Which runs share what the provider caches.
    pub cache_scope: CacheScope,
    /// What the model's tokens cost, in US dollars per million.
    pub pricing: Option<Pricing>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            provider: Provider::Anthropic,
            api: None,
            script: None,
            name: "claude-sonnet-5".to_owned(),
            api_key_env: None,
            base_url: None,
            max_tokens: 32_000,
            temperature: None,
            thinking: None,
            effort: None,
            seed: None,
            reasoning_replay: None,
            cache: None,
            cache_scope: CacheScope::Shared,
            pricing: None,
        }
    }
}

/// A setting of the `model` section that not every provider applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Setting {
    Api,
    Script,
    Thinking,
    Effort,
    Seed,
    ReasoningReplay,
    Cache,
}

impl Setting {
    pub(crate) const ALL: [Self; 7] = [
        Self::Api,
        Self::Script,
        Self::Thinking,
        Self::Effort,
        Self::Seed,
        Self::ReasoningReplay,
        Self::Cache,
    ];

    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Api => "model.api",
            Self::Script => "model.script",
            Self::Thinking => "model.thinking",
            Self::Effort => "model.effort",
            Self::Seed => "model.seed",
            Self::ReasoningReplay => "model.reasoning_replay",
            Self::Cache => "model.cache",
        }
    }
}

impl Model {
    /// The API an `openai` model is reached through: the one the config
    /// states, or the Responses API without a `base_url` and chat
    /// completions with one. `None` for the other providers, which have one
    /// API each.
    #[must_use]
    pub fn openai_api(&self) -> Option<Api> {
        let chosen = self.api.unwrap_or(if self.base_url.is_some() {
            Api::ChatCompletions
        } else {
            Api::Responses
        });
        (self.provider == Provider::Openai).then_some(chosen)
    }

    /// The variable that holds the API key: the one the config names, or
    /// `ANTHROPIC_API_KEY` for `anthropic`.
    #[must_use]
    pub fn key_variable(&self) -> Option<&str> {
        match (&self.api_key_env, self.provider) {
            (Some(named), _) => Some(named),
            (None, Provider::Anthropic) => Some("ANTHROPIC_API_KEY"),
            (None, Provider::Openai | Provider::Fake) => None,
        }
    }

    /// Whether the provider can apply `setting`.
    pub(crate) fn applies(&self, setting: Setting) -> bool {
        let api = self.openai_api();
        match setting {
            Setting::Api | Setting::Seed => self.provider == Provider::Openai,
            Setting::Script => self.provider == Provider::Fake,
            Setting::Thinking | Setting::Cache => self.provider == Provider::Anthropic,
            Setting::Effort => self.provider == Provider::Anthropic || api == Some(Api::Responses),
            Setting::ReasoningReplay => api == Some(Api::ChatCompletions),
        }
    }

    /// What the config states for `setting`, as a config writes it; `None`
    /// when it states nothing.
    pub(crate) fn stated(&self, setting: Setting) -> Option<String> {
        match setting {
            Setting::Api => self.api.map(|api| api.to_string()),
            Setting::Script => self
                .script
                .as_ref()
                .map(|script| script.display().to_string()),
            Setting::Thinking => self.thinking.map(|thinking| thinking.to_string()),
            Setting::Effort => self.effort.map(|effort| effort.to_string()),
            Setting::Seed => self.seed.map(|seed| seed.to_string()),
            Setting::ReasoningReplay => self.reasoning_replay.map(|replay| replay.to_string()),
            Setting::Cache => self.cache.map(|cache| cache.to_string()),
        }
    }
}

/// A provider of models.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// Anthropic's Messages API.
    #[default]
    Anthropic,
    /// OpenAI's API, and any server that speaks chat completions.
    Openai,
    /// A script, played in place of a model.
    Fake,
}

impl Provider {
    /// How a config spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Fake => "fake",
        }
    }
}

/// One of the two APIs an `openai` model is reached through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Api {
    /// The Responses API.
    Responses,
    /// Chat completions.
    ChatCompletions,
}

impl Api {
    /// How a config spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
        }
    }
}

/// How the model is asked to reason before it answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Thinking {
    /// Nothing is sent, so the provider's default applies.
    #[default]
    ProviderDefault,
    /// The model decides how much to reason.
    Adaptive,
    /// Reasoning is refused.
    Disabled,
    /// Reasoning is asked for with this many tokens to spend on it, which
    /// is fewer than `max_tokens`.
    Budget(NonZeroU32),
}

impl fmt::Display for Thinking {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProviderDefault => f.write_str("provider_default"),
            Self::Adaptive => f.write_str("adaptive"),
            Self::Disabled => f.write_str("disabled"),
            Self::Budget(tokens) => write!(f, "{{ budget: {tokens} }}"),
        }
    }
}

impl From<Thinking> for lablet_model::Thinking {
    fn from(thinking: Thinking) -> Self {
        match thinking {
            Thinking::ProviderDefault => Self::ProviderDefault,
            Thinking::Adaptive => Self::Adaptive,
            Thinking::Disabled => Self::Disabled,
            Thinking::Budget(tokens) => Self::Budget(tokens),
        }
    }
}

/// A reasoning effort level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    /// The least effort.
    Low,
    /// Moderate effort.
    Medium,
    /// High effort.
    High,
    /// Above high.
    #[serde(rename = "xhigh")]
    XHigh,
    /// The most effort the provider offers.
    Max,
}

impl Effort {
    /// How a config spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

impl From<Effort> for lablet_model::Effort {
    fn from(effort: Effort) -> Self {
        match effort {
            Effort::Low => Self::Low,
            Effort::Medium => Self::Medium,
            Effort::High => Self::High,
            Effort::XHigh => Self::XHigh,
            Effort::Max => Self::Max,
        }
    }
}

/// Prints each as a config spells it.
macro_rules! display_as_str {
    ($($name:ty),+) => {$(
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    )+};
}

display_as_str!(Provider, Api, Effort);

/// Which runs share what a provider caches of a run's requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheScope {
    /// Every run that sends the same prefix shares what's cached of it.
    #[default]
    Shared,
    /// Each request carries the run id as its cache key.
    Run,
}

impl From<CacheScope> for lablet_model::CacheScope {
    fn from(scope: CacheScope) -> Self {
        match scope {
            CacheScope::Shared => Self::Shared,
            CacheScope::Run => Self::Run,
        }
    }
}

/// What a model's tokens cost, in US dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pricing {
    /// Input tokens that touched no cache.
    pub input: f64,
    /// Generated tokens, reasoning included.
    pub output: f64,
    /// Input tokens served from the prompt cache.
    pub cache_read: f64,
    /// Input tokens written to the prompt cache.
    pub cache_write: f64,
}
