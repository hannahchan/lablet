//! A script: what the provider answers to each attempt, read from the text
//! a person wrote.

use std::fmt;
use std::time::Duration;

use lablet_documents::{ContentBlock, Usage};
use lablet_model::{self as model, FinishReason, ProviderErrorKind, ProviderResponse};
use lablet_run::ProviderError;
use serde::Deserialize;
use serde::de::{Deserializer, Error, Visitor};
use serde_json::Value;

use crate::tree::Tree;

/// The format a script's text is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptFormat {
    /// YAML.
    Yaml,
    /// JSON.
    Json,
}

impl ScriptFormat {
    /// The format's name, as a refusal spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Yaml => "YAML",
            Self::Json => "JSON",
        }
    }

    /// The tree `text` holds, or the parser's own words for why it holds
    /// none.
    fn parse(self, text: &str) -> Result<Value, String> {
        let tree = match self {
            // Older YAML reads `yes` and `no` as booleans, and a text block
            // that says `yes` would be refused for holding one. The
            // refusal comes without the parser's drawing of the place,
            // because a caller puts it inside a line of its own.
            Self::Yaml => serde_saphyr::from_str_with_options::<Tree>(
                text,
                serde_saphyr::options! {
                    strict_booleans: true,
                    with_snippet: false,
                },
            )
            .map_err(|error| error.to_string()),
            Self::Json => serde_json::from_str::<Tree>(text).map_err(|error| error.to_string()),
        };
        tree.map(|Tree(value)| value)
    }
}

impl fmt::Display for ScriptFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What a script is read from. The fields are named because a name and a
/// text are both strings, and two strings passed in a row swap silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptSource<'a> {
    /// What a refusal and a failed call name the script by: the path the
    /// config gave, for a script read from a file.
    pub name: &'a str,
    /// The script itself.
    pub text: &'a str,
    /// The format `text` is in.
    pub format: ScriptFormat,
}

/// Why a text was refused as a script.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("script {script:?}: {fault}")]
pub struct ScriptError {
    /// The name the script was read under.
    pub script: String,
    /// What's wrong with it, and where.
    pub fault: ScriptFault,
}

/// What's wrong with a script, and where in it. Entries and blocks are
/// counted from 1.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptFault {
    /// The text isn't in the format it was read as, or a mapping of it
    /// holds a key twice.
    #[error("it can't be read as {format}: {reason}")]
    Syntax {
        /// The format the text was read as.
        format: ScriptFormat,
        /// The parser's own words, with the line and the column.
        reason: String,
    },
    /// The text holds something other than a list.
    #[error("it holds {found} where a list of entries belongs")]
    NotAList {
        /// What it holds instead.
        found: &'static str,
    },
    /// The list has no entries, so the script could answer no call at all.
    #[error("it holds no entry, so it could answer no provider call")]
    Empty,
    /// An entry can't be read, or is one the domain refuses.
    #[error("entry {entry}: {reason}")]
    Entry {
        /// Which entry of the script.
        entry: usize,
        /// Why it's refused.
        reason: String,
    },
    /// A block of a response can't be read, or is one the domain refuses.
    #[error("entry {entry}, block {block}: {reason}")]
    Block {
        /// Which entry of the script.
        entry: usize,
        /// Which block of the entry's content.
        block: usize,
        /// Why it's refused.
        reason: String,
    },
}

/// What a scripted provider plays: one entry for each attempt of a provider
/// call, in order.
///
/// A script is never empty, and every entry of it is a response or a
/// failure the domain accepts, because [`Script::read`] is the only way to
/// one and refuses the rest. So nothing is left to refuse while a run
/// plays it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    name: String,
    entries: Vec<Entry>,
}

/// What the provider answers to one attempt, and how long it takes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) latency: Duration,
    pub(crate) answer: Result<ProviderResponse, ProviderError>,
}

impl Script {
    /// The script `source.text` holds.
    ///
    /// # Errors
    ///
    /// Returns a [`ScriptError`] that names the script and says what's
    /// wrong with it: the text isn't in the format, it isn't a list of
    /// entries, or the list is empty; an entry or a block holds a key the
    /// script format doesn't have, lacks one it needs, or holds a value of
    /// the wrong kind; or the domain refuses what was read, as it does a
    /// tool name no provider accepts and a tool call id that two blocks of
    /// one response share.
    pub fn read(source: ScriptSource<'_>) -> Result<Self, ScriptError> {
        let ScriptSource { name, text, format } = source;
        let refused = |fault| ScriptError {
            script: name.to_owned(),
            fault,
        };
        let tree = format
            .parse(text)
            .map_err(|reason| refused(ScriptFault::Syntax { format, reason }))?;
        let entries = match tree {
            Value::Array(entries) => Ok(entries),
            Value::Null => Err("nothing"),
            Value::Bool(_) => Err("a boolean"),
            Value::Number(_) => Err("a number"),
            Value::String(_) => Err("a string"),
            Value::Object(_) => Err("a mapping"),
        }
        .map_err(|found| refused(ScriptFault::NotAList { found }))?;
        if entries.is_empty() {
            return Err(refused(ScriptFault::Empty));
        }
        let entries = entries
            .into_iter()
            .zip(1..)
            .map(|(entry, number)| Entry::read(name, number, entry))
            .collect::<Result<_, _>>()
            .map_err(refused)?;
        Ok(Self {
            name: name.to_owned(),
            entries,
        })
    }

    /// The name the script was read under.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How many entries the script holds, which is how many attempts it
    /// answers. At least one.
    #[must_use]
    pub const fn entries(&self) -> usize {
        self.entries.len()
    }

    /// The entry that answers the attempt after `played` others, or `None`
    /// when the script has run out.
    pub(crate) fn entry(&self, played: usize) -> Option<&Entry> {
        self.entries.get(played)
    }
}

/// An entry as a script writes one: `{"response": {...}}` or
/// `{"error": {...}}`.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Written {
    Response(WrittenResponse),
    Error(WrittenError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WrittenResponse {
    /// Each block is read on its own, so a refusal says which.
    content: Vec<Value>,
    #[serde(default)]
    usage: Usage,
    finish: String,
    response_id: Option<String>,
    response_model: Option<String>,
    latency: Option<Wait>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WrittenError {
    kind: Kind,
    message: Option<String>,
    usage: Option<Usage>,
    retry_after: Option<Wait>,
    latency: Option<Wait>,
}

/// A kind of provider failure, spelled as the failed call's `error.type`
/// spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Retryable,
    ContextExhausted,
    Auth,
    Fatal,
    Malformed,
}

impl From<Kind> for ProviderErrorKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Retryable => Self::Retryable,
            Kind::ContextExhausted => Self::ContextExhausted,
            Kind::Auth => Self::Auth,
            Kind::Fatal => Self::Fatal,
            Kind::Malformed => Self::Malformed,
        }
    }
}

/// A duration as a script writes one: text in the syntax the config's
/// durations have, as `250ms` or `2s`.
struct Wait(Duration);

impl Wait {
    /// The wait a script wrote, and none at all for one it left out.
    fn or_none(wait: Option<Self>) -> Duration {
        wait.map_or(Duration::ZERO, |Self(wait)| wait)
    }
}

impl<'de> Deserialize<'de> for Wait {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(AsText)
    }
}

struct AsText;

impl Visitor<'_> for AsText {
    type Value = Wait;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a duration as text, such as `250ms` or `2s`")
    }

    fn visit_str<E: Error>(self, text: &str) -> Result<Wait, E> {
        humantime::parse_duration(text).map(Wait).map_err(|error| {
            E::custom(format!(
                "{text:?} isn't a duration such as `250ms` or `2s`: {error}"
            ))
        })
    }
}

impl Entry {
    /// Entry `number` of the script `script`, read from `written` into the
    /// domain's own values, through the constructors that check them.
    fn read(script: &str, number: usize, written: Value) -> Result<Self, ScriptFault> {
        let refused = |reason| ScriptFault::Entry {
            entry: number,
            reason,
        };
        let written =
            serde_json::from_value(written).map_err(|error| refused(error.to_string()))?;
        match written {
            Written::Response(WrittenResponse {
                content,
                usage,
                finish,
                response_id,
                response_model,
                latency,
            }) => {
                let content = content
                    .into_iter()
                    .zip(1..)
                    .map(|(written, block)| {
                        serde_json::from_value::<ContentBlock>(written)
                            .map(model::ContentBlock::from)
                            .map_err(|error| ScriptFault::Block {
                                entry: number,
                                block,
                                reason: error.to_string(),
                            })
                    })
                    .collect::<Result<_, _>>()?;
                let response = ProviderResponse::new(
                    content,
                    usage.into(),
                    FinishReason::from(finish),
                    response_id,
                    response_model,
                )
                .map_err(|error| refused(error.to_string()))?;
                Ok(Self {
                    latency: Wait::or_none(latency),
                    answer: Ok(response),
                })
            }
            Written::Error(WrittenError {
                kind,
                message,
                usage,
                retry_after,
                latency,
            }) => {
                let kind = ProviderErrorKind::from(kind);
                let message = message.unwrap_or_else(|| {
                    format!("entry {number} of script {script:?} injects a failure of kind {kind}")
                });
                let mut failure = ProviderError::new(kind, message);
                if let Some(usage) = usage {
                    failure = failure.with_usage(usage.into());
                }
                if let Some(Wait(wait)) = retry_after {
                    failure = failure.with_retry_after(wait);
                }
                Ok(Self {
                    latency: Wait::or_none(latency),
                    answer: Err(failure),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests;
