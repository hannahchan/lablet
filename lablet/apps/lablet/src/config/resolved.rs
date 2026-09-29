//! The resolved config: what a config comes to once every default is filled
//! in, and the digest that groups the runs made from it.

use std::fmt::Write as _;
use std::path::PathBuf;

use serde::{Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::model::{Api, CacheScope, Effort, Pricing, Provider, Thinking};
use super::written::path;
use super::{Prompt, Run, Telemetry, Tools};

/// A config with every default filled in.
///
/// It holds what the config states and what lablet assumes where the config
/// states nothing, so two configs that come to the same thing resolve to the
/// same value. It isn't a config that was checked: `build` refuses what a
/// config may not state.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedConfig {
    /// The `run` section.
    pub run: Run,
    /// The `model` section.
    pub model: ResolvedModel,
    /// The `prompt` section.
    pub prompt: Prompt,
    /// The `tools` section.
    pub tools: Tools,
    /// The `telemetry` section.
    pub telemetry: Telemetry,
}

/// A setting that only some providers apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Applied<T> {
    /// The provider can't apply the setting, so the resolved config leaves
    /// it out.
    No,
    /// The provider applies the setting, with this value.
    Yes(T),
}

impl<T> Applied<T> {
    /// Whether the provider can't apply the setting.
    #[must_use]
    pub const fn is_no(&self) -> bool {
        matches!(self, Self::No)
    }

    /// The setting's value, when the provider applies it.
    #[must_use]
    pub fn value(self) -> Option<T> {
        match self {
            Self::No => None,
            Self::Yes(value) => Some(value),
        }
    }
}

impl<T: Serialize> Serialize for Applied<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::No => serializer.serialize_none(),
            Self::Yes(value) => value.serialize(serializer),
        }
    }
}

/// The `model` section with every default filled in.
///
/// A setting the provider can't apply is left out, whether the config
/// stated it or not, so a default that's one provider's is in no other
/// provider's resolved config or digest.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedModel {
    /// The provider the model is reached through.
    pub provider: Provider,
    /// The API an `openai` model is reached through.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub api: Applied<Api>,
    /// The file of scripted responses a `fake` model plays.
    #[serde(skip_serializing_if = "Applied::is_no", serialize_with = "script")]
    pub script: Applied<Option<PathBuf>>,
    /// The provider's name for the model.
    pub name: String,
    /// The environment variable that holds the API key.
    pub api_key_env: Option<String>,
    /// Where the API is served.
    pub base_url: Option<String>,
    /// The cap on output tokens for each call.
    pub max_tokens: u32,
    /// The sampling temperature.
    pub temperature: Option<f64>,
    /// How the model is asked to reason.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub thinking: Applied<Thinking>,
    /// The reasoning effort.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub effort: Applied<Option<Effort>>,
    /// The sampling seed.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub seed: Applied<Option<i64>>,
    /// Whether reasoning is sent back.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub reasoning_replay: Applied<bool>,
    /// Whether requests carry cache breakpoints.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub cache: Applied<bool>,
    /// Which runs share what the provider caches.
    pub cache_scope: CacheScope,
    /// What the model's tokens cost.
    pub pricing: Option<Pricing>,
}

fn script<S: Serializer>(
    script: &Applied<Option<PathBuf>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match script {
        Applied::No => serializer.serialize_none(),
        Applied::Yes(script) => path::optional(script, serializer),
    }
}

impl ResolvedConfig {
    /// SHA-256 of the resolved config as canonical JSON, in lower-case hex.
    ///
    /// It covers the settings that say what a run does and leaves out the
    /// ones that say where its output goes: `run.transcript_path`,
    /// `run.transcript_format` and the `telemetry` section. So two runs
    /// that differ only in where they write share a digest.
    ///
    /// Canonical JSON is compact, with the keys of every object in order.
    #[must_use]
    pub fn digest(&self) -> String {
        // Nothing of a resolved config fails to serialise: every key is
        // text, a path is written as it's shown, and a number JSON can't
        // hold is written as `null`.
        let mut tree = serde_json::to_value(self).unwrap_or_default();
        if let Value::Object(sections) = &mut tree {
            sections.remove("telemetry");
            if let Some(Value::Object(run)) = sections.get_mut("run") {
                run.remove("transcript_path");
                run.remove("transcript_format");
            }
        }
        let mut canonical = String::new();
        write_canonical(&tree, &mut canonical);
        Sha256::digest(&canonical)
            .iter()
            .fold(String::new(), |mut hex, byte| {
                // Writing to a `String` can't fail, so there's no error to
                // report.
                let _ = write!(hex, "{byte:02x}");
                hex
            })
    }
}

/// Writes `value` as compact JSON with the keys of every object in order,
/// so one value is always written as the same bytes, however the map that
/// holds its keys keeps them.
fn write_canonical(value: &Value, to: &mut String) {
    match value {
        Value::Object(entries) => {
            let mut entries: Vec<_> = entries.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            to.push('{');
            for (place, (key, value)) in entries.into_iter().enumerate() {
                if place > 0 {
                    to.push(',');
                }
                to.push_str(&Value::String(key.clone()).to_string());
                to.push(':');
                write_canonical(value, to);
            }
            to.push('}');
        }
        Value::Array(items) => {
            to.push('[');
            for (place, item) in items.iter().enumerate() {
                if place > 0 {
                    to.push(',');
                }
                write_canonical(item, to);
            }
            to.push(']');
        }
        scalar => to.push_str(&scalar.to_string()),
    }
}

#[cfg(test)]
mod tests;
