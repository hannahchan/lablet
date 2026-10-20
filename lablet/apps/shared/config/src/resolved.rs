//! The resolved config: what a config comes to once every default is filled
//! in, and the digest that groups the runs made from it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use lablet_model::ConfigDigest;
use serde::{Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::key::KeyPath;
use super::model::{Api, CacheScope, Effort, Pricing, Provider, Thinking, ThinkingDisplay};
use super::secret::Secret;
use super::shown::{Masked, WrittenBack, debug_tree};
use super::substitute::Filled;
use super::tools::{Builtin, BuiltinTool, McpLifetime, McpResult, McpServer, OutputCut, Tools};
use super::written::{duration, path};
use super::{Prompt, Run, Telemetry, when};
use crate::url::without_user_information;

/// A config with every default filled in.
///
/// It holds what the config states and what lablet assumes where the config
/// states nothing, so two configs that come to the same thing resolve to the
/// same value. It isn't a config that was checked: `build` refuses what a
/// config may not state.
///
/// It's written out through the mask: a setting the fill changed is written
/// as the file wrote it, `${VAR}` and all, and a secret as `[secret]`, so
/// `check --resolved` and a `Debug` form hold nothing a variable gave.
#[derive(Clone, PartialEq)]
pub struct ResolvedConfig {
    /// The `run` section.
    pub run: Run,
    /// The `model` section.
    pub model: ResolvedModel,
    /// The `prompt` section.
    pub prompt: Prompt,
    /// The `tools` section.
    pub tools: ResolvedTools,
    /// The `telemetry` section.
    pub telemetry: Telemetry,
    /// What the fill changed, which the mask writes back.
    pub(crate) filled: Filled,
}

/// The sections as they serialise, in their order.
#[derive(Serialize)]
pub(crate) struct Sections<'a> {
    run: &'a Run,
    model: &'a ResolvedModel,
    prompt: &'a Prompt,
    tools: &'a ResolvedTools,
    telemetry: &'a Telemetry,
}

impl ResolvedConfig {
    /// What the fill changed, each key with the text as written, which the
    /// resolved form writes back.
    #[must_use]
    pub fn filled(&self) -> &Filled {
        &self.filled
    }

    pub(crate) fn sections(&self) -> Sections<'_> {
        Sections {
            run: &self.run,
            model: &self.model,
            prompt: &self.prompt,
            tools: &self.tools,
            telemetry: &self.telemetry,
        }
    }
}

impl Serialize for ResolvedConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Masked::new(&self.sections(), &self.filled, WrittenBack::Every).serialize(serializer)
    }
}

impl ResolvedConfig {
    /// The resolved config as it serialises, with a YAML comment after each
    /// setting `comments` gives one for: what `check --resolved` prints,
    /// where the comment says the line the setting was written on, that an
    /// override set it, or that the environment filled it.
    #[must_use]
    pub fn commented<'a>(
        &'a self,
        comments: &'a dyn Fn(&KeyPath) -> Option<String>,
    ) -> Commented<'a> {
        Commented {
            resolved: self,
            comments,
        }
    }
}

/// A resolved config with a comment after each setting its source names.
/// The comments go through `serde_saphyr`'s own wrapper, so they're YAML
/// comments to it and a pair of comment and value to any other format: it
/// serialises for YAML alone.
pub struct Commented<'a> {
    resolved: &'a ResolvedConfig,
    comments: &'a dyn Fn(&KeyPath) -> Option<String>,
}

impl Serialize for Commented<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let sections = self.resolved.sections();
        Masked::commented(
            &sections,
            &self.resolved.filled,
            WrittenBack::Every,
            self.comments,
        )
        .serialize(serializer)
    }
}

impl fmt::Debug for ResolvedConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ResolvedConfig")
            .field(&debug_tree(&self.sections(), &self.filled))
            .finish()
    }
}

/// A setting that only some configs apply: one that's one provider's, or
/// one that goes with another setting's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Applied<T> {
    /// Nothing applies the setting, so the resolved config leaves it out.
    No,
    /// The setting is applied, with this value.
    Yes(T),
}

impl<T> Applied<T> {
    /// Whether nothing applies the setting.
    #[must_use]
    pub const fn is_no(&self) -> bool {
        matches!(self, Self::No)
    }

    /// The setting's value, when it's applied.
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
    #[serde(
        skip_serializing_if = "Applied::is_no",
        serialize_with = "applied_path"
    )]
    pub script: Applied<Option<PathBuf>>,
    /// The provider's name for the model.
    pub name: String,
    /// The API key, written as `[secret]`, or as the file wrote it.
    pub api_key: Option<Secret>,
    /// Where the API is served.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub base_url: Applied<Option<String>>,
    /// The cap on output tokens for each call.
    pub max_tokens: u32,
    /// The sampling temperature.
    pub temperature: Option<f64>,
    /// How the model is asked to reason.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub thinking: Applied<Thinking>,
    /// How the model's reasoning is shown.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub thinking_display: Applied<Option<ThinkingDisplay>>,
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

fn applied_path<S: Serializer>(
    path: &Applied<Option<PathBuf>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match path {
        Applied::No => serializer.serialize_none(),
        Applied::Yes(path) => path::optional(path, serializer),
    }
}

fn applied_duration<S: Serializer>(
    duration: &Applied<Duration>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match duration {
        Applied::No => serializer.serialize_none(),
        Applied::Yes(duration) => duration::serialize(duration, serializer),
    }
}

/// The `tools` section with every default filled in.
///
/// What two configs state differently to the same effect is held one way,
/// so they share a digest: a run asks of `allow` and `deny` only whether
/// they hold a name, which makes each a set, and a setting that nothing
/// applies is left out. How an output is cut says nothing where nothing
/// is cut, the length of a preview nothing under a cut that makes none,
/// and the MCP settings nothing where there's no server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedTools {
    /// The built-in tools.
    pub builtin: ResolvedBuiltin,
    /// The MCP servers.
    pub mcp: Vec<McpServer>,
    /// How long the MCP servers live, where there are any.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub mcp_lifetime: Applied<McpLifetime>,
    /// Which part the model is sent of an MCP result that has two, where
    /// there are servers.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub mcp_result: Applied<McpResult>,
    /// The only tools offered; `None` offers every tool, which no list
    /// does, so it isn't a list of nothing.
    pub allow: Option<BTreeSet<String>>,
    /// Tools that are never offered.
    pub deny: BTreeSet<String>,
    /// How many calls of one turn may run at once.
    pub max_concurrent_calls: NonZeroU32,
    /// The size in bytes above which a tool result is cut.
    pub max_output_bytes: Option<u64>,
    /// What's kept of a result that's cut, where there's a cap to cut it.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub output_cut: Applied<OutputCut>,
    /// How many bytes a preview keeps, where the cut is a preview.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub output_preview_bytes: Applied<u64>,
    /// The length in characters above which a tool description or a
    /// server's instructions are cut.
    pub max_description_chars: Option<u32>,
}

impl From<&Tools> for ResolvedTools {
    fn from(tools: &Tools) -> Self {
        let Tools {
            builtin,
            mcp,
            mcp_lifetime,
            mcp_result,
            allow,
            deny,
            max_concurrent_calls,
            max_output_bytes,
            output_cut,
            output_preview_bytes,
            max_description_chars,
        } = tools;
        let set = |names: &Vec<String>| names.iter().cloned().collect();
        let cuts = max_output_bytes.is_some();
        let serves_mcp = !mcp.is_empty();
        Self {
            builtin: builtin.into(),
            mcp: mcp.clone(),
            mcp_lifetime: when(serves_mcp, *mcp_lifetime),
            mcp_result: when(serves_mcp, *mcp_result),
            allow: allow.as_ref().map(set),
            deny: set(deny),
            max_concurrent_calls: *max_concurrent_calls,
            max_output_bytes: *max_output_bytes,
            output_cut: when(cuts, *output_cut),
            output_preview_bytes: match output_cut {
                OutputCut::Preview => when(cuts, *output_preview_bytes),
                OutputCut::Head | OutputCut::HeadTail => Applied::No,
            },
            max_description_chars: *max_description_chars,
        }
    }
}

/// The `tools.builtin` section with every default filled in. Its settings
/// are left out where no built-in tool is enabled, since nothing applies
/// them then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedBuiltin {
    /// The tools to serve.
    pub enabled: BTreeSet<BuiltinTool>,
    /// The directory `bash` starts in and the file tools stay under.
    #[serde(
        skip_serializing_if = "Applied::is_no",
        serialize_with = "applied_path"
    )]
    pub root: Applied<Option<PathBuf>>,
    /// The longest a call may take.
    #[serde(
        skip_serializing_if = "Applied::is_no",
        serialize_with = "applied_duration"
    )]
    pub timeout: Applied<Duration>,
    /// Variables a command starts with on top of the environment it
    /// inherits.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub env: Applied<BTreeMap<String, String>>,
    /// Credentials given to `bash` under their names, each written as
    /// `[secret]`, or as the file wrote it.
    #[serde(skip_serializing_if = "Applied::is_no")]
    pub credentials: Applied<BTreeMap<String, Secret>>,
}

impl From<&Builtin> for ResolvedBuiltin {
    fn from(builtin: &Builtin) -> Self {
        let Builtin {
            root,
            enabled,
            timeout,
            env,
            credentials,
        } = builtin;
        let serves = !enabled.is_empty();
        Self {
            enabled: enabled.clone(),
            root: when(serves, root.clone()),
            timeout: when(serves, *timeout),
            env: when(serves, env.clone()),
            credentials: when(serves, credentials.clone()),
        }
    }
}

impl ResolvedConfig {
    /// SHA-256 of the resolved config as canonical JSON, in lower-case hex.
    ///
    /// It covers the settings that say what a run does, each as it ran, so
    /// a limit or a model's name a variable gave is in it as the variable
    /// held it. It takes as the file wrote them, `${VAR}` and all, the
    /// settings that say where something is: a path, a variable a command
    /// starts with, and what starts a server, so one file has one digest on
    /// every machine. It leaves out the settings that say where a run's
    /// output goes, `run.transcript_path`, `run.transcript_format` and the
    /// `telemetry` section, and the credentials, which a run has but doesn't
    /// do: `model.api_key`, every value of `tools.builtin.credentials` and of
    /// an HTTP server's `headers`, whose keys stay, and the user information
    /// of `model.base_url` and a server's `url`, whose host stays. So two
    /// runs that differ only in where they write, or only in a rotated
    /// token, share a digest.
    ///
    /// The strip is made here and nowhere else, so the resolved config
    /// `check --resolved` prints holds every setting as written and reads
    /// back to the same digest once its variables are filled again.
    ///
    /// Canonical JSON is compact, with the keys of every object in order.
    #[must_use]
    pub fn digest(&self) -> ConfigDigest {
        // Nothing of a resolved config fails to serialise: every key is
        // text, a path is written as it's shown, and a number JSON can't
        // hold is written as `null`.
        let mut tree = serde_json::to_value(Masked::new(
            &self.sections(),
            &self.filled,
            WrittenBack::AsWritten,
        ))
        .unwrap_or_default();
        if let Value::Object(sections) = &mut tree {
            sections.remove("telemetry");
            if let Some(Value::Object(run)) = sections.get_mut("run") {
                run.remove("transcript_path");
                run.remove("transcript_format");
            }
            if let Some(Value::Object(model)) = sections.get_mut("model") {
                model.remove("api_key");
                if let Some(Value::String(url)) = model.get_mut("base_url") {
                    *url = without_user_information(url);
                }
            }
            if let Some(Value::Object(tools)) = sections.get_mut("tools") {
                if let Some(Value::Object(builtin)) = tools.get_mut("builtin")
                    && let Some(Value::Object(credentials)) = builtin.get_mut("credentials")
                {
                    for value in credentials.values_mut() {
                        *value = Value::Null;
                    }
                }
                if let Some(Value::Array(servers)) = tools.get_mut("mcp") {
                    for server in servers.iter_mut().filter_map(Value::as_object_mut) {
                        if let Some(Value::String(url)) = server.get_mut("url") {
                            *url = without_user_information(url);
                        }
                        if let Some(Value::Object(headers)) = server.get_mut("headers") {
                            for value in headers.values_mut() {
                                *value = Value::Null;
                            }
                        }
                    }
                }
            }
        }
        let mut canonical = String::new();
        write_canonical(&tree, &mut canonical);
        ConfigDigest::from_sha256(Sha256::digest(&canonical).into())
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
