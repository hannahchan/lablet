//! What a config comes to for the loop and its adapters: the policies, the
//! limits and the request parameters, each built through the constructor
//! that checks it.
//!
//! Nothing here reads a file, a variable or a clock, so whether a config is
//! refused depends on the config alone.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use lablet_model::{
    CompletionMode, OutputCap, OutputCapError, OutputCut, Rates, RequestParams, ToolName,
};
use lablet_policy::{Pricing, RetryPolicy, RetryPolicyError, RetrySettings, StopPolicy};
use lablet_run::{CallLimits, ToolFilter};
use lablet_tools_builtin::Tool;

use lablet_config::{
    self as config, BuiltinTool, Completion, Config, KeyPath, Provider, Refusal, ResolvedModel,
    Setting,
};

/// What a backoff grows by after each attempt.
const BACKOFF_FACTOR: f64 = 2.0;

/// Where a run's system prompt comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum System {
    /// The config states it.
    Text(String),
    /// The config names the file that holds it.
    File(PathBuf),
}

/// The provider a config selects, with what only that provider takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selected {
    /// The Anthropic Messages API.
    Anthropic,
    /// The OpenAI Responses API, or an OpenAI-compatible one.
    Openai,
    /// The fake provider, which plays a script.
    Fake {
        /// The file of the script it plays.
        script: PathBuf,
    },
}

/// What the loop and its adapters are built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The provider, with what only it takes.
    pub provider: Selected,
    /// When a run stops.
    pub stop: StopPolicy,
    /// How a failed provider call is retried.
    pub retry: RetryPolicy,
    /// What each provider request asks for.
    pub request: RequestParams,
    /// What a run's usage costs, when the config prices it.
    pub pricing: Option<Pricing>,
    /// The limits on each call.
    pub calls: CallLimits,
    /// How a run ends.
    pub completion: CompletionMode,
    /// `tools.allow` and `tools.deny`.
    pub filter: ToolFilter,
    /// Where the system prompt comes from.
    pub system: System,
    /// `None` when no built-in tool is enabled.
    pub builtin: Option<lablet_tools_builtin::Settings>,
}

fn shown(duration: Duration) -> String {
    humantime::format_duration(duration).to_string()
}

impl Settings {
    /// What `config` comes to.
    ///
    /// # Errors
    ///
    /// Returns the [`Refusal`] of the first setting that's refused, in the
    /// order of the config's sections.
    pub fn of(config: &Config) -> Result<Self, Refusal> {
        let resolved = config.resolved();
        let run = &config.run;
        Ok(Self {
            stop: StopPolicy {
                max_turns: run.max_turns,
                timeout: run.timeout,
                max_total_tokens: run.max_total_tokens,
                max_consecutive_invalid_turns: run.max_consecutive_invalid_turns,
            },
            retry: retry(run)?,
            provider: {
                key_variable(&config.model)?;
                selected(&config.model)?
            },
            request: request(&config.model, &resolved.model)?,
            pricing: config.model.pricing.map(pricing).transpose()?,
            system: system(&config.prompt)?,
            calls: CallLimits {
                provider_timeout: run.provider_timeout,
                output_cap: output_cap(&config.tools)?,
                max_concurrent_tool_calls: config.tools.max_concurrent_calls,
            },
            completion: match run.completion {
                Completion::Natural => CompletionMode::Natural,
                Completion::Explicit => CompletionMode::Explicit,
            },
            filter: filter(&config.tools)?,
            builtin: {
                mcp_names(&config.tools.mcp)?;
                builtin(&config.tools.builtin)?
            },
        })
    }
}

fn retry(run: &config::Run) -> Result<RetryPolicy, Refusal> {
    RetryPolicy::new(RetrySettings {
        max_retries: run.max_retries,
        base: run.retry_backoff_base,
        max: run.retry_backoff_max,
        factor: BACKOFF_FACTOR,
        hint_max: run.retry_hint_max,
        jitter: run.retry_jitter,
    })
    .map_err(|error| match error {
        // The factor is lablet's own, so a refusal of it has no key of its
        // own to name and is reported with the backoff it's the factor of.
        RetryPolicyError::BaseAboveMax { .. } | RetryPolicyError::Factor(_) => Refusal::invalid(
            "run.retry_backoff_base",
            format!(
                "a backoff starts no longer than `run.retry_backoff_max`, which is {}",
                shown(run.retry_backoff_max)
            ),
        ),
        RetryPolicyError::Jitter(_) => Refusal::invalid(
            "run.retry_jitter",
            "the jitter is a share of a wait, from 0 to 1",
        ),
    })
}

/// Holds `model.api_key_env` to the name of a variable, whichever
/// provider is selected: anything else there is a mistake, and the mistake
/// to expect is the key itself.
fn key_variable(model: &config::Model) -> Result<(), Refusal> {
    let Some(named) = &model.api_key_env else {
        return Ok(());
    };
    let mut bytes = named.bytes();
    let begins_a_name = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_');
    if begins_a_name && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
        Ok(())
    } else {
        Err(Refusal::KeyVariable {
            reason: "it holds something other than the name of an environment variable, \
                     which is ASCII letters, digits and `_` and begins with no digit. What \
                     it holds isn't shown, since a key may have been written in its place",
        })
    }
}

fn selected(model: &config::Model) -> Result<Selected, Refusal> {
    match (model.provider, &model.script) {
        (Provider::Anthropic, _) => Ok(Selected::Anthropic),
        (Provider::Openai, _) => Ok(Selected::Openai),
        (Provider::Fake, Some(script)) => Ok(Selected::Fake {
            script: script.clone(),
        }),
        (Provider::Fake, None) => Err(Refusal::Missing {
            key: "model.script",
            reason: "the provider `fake` plays the script it names".to_owned(),
        }),
    }
}

/// The provider a model is reached through, with its API when it has more
/// than one.
fn reached(model: &config::Model) -> String {
    match model.openai_api() {
        Some(api) => format!("the provider `{}` over its API `{api}`", model.provider),
        None => format!("the provider `{}`", model.provider),
    }
}

fn request(model: &config::Model, resolved: &ResolvedModel) -> Result<RequestParams, Refusal> {
    for setting in Setting::ALL {
        if model.stated(setting).is_some() && !model.applies(setting) {
            return Err(Refusal::NotApplied {
                setting,
                reached: reached(model),
            });
        }
    }
    base_url(model)?;
    if let Some(temperature) = model.temperature
        && !temperature.is_finite()
    {
        return Err(Refusal::invalid(
            "model.temperature",
            "a temperature is a finite number",
        ));
    }
    if let Some(config::Thinking::Budget(tokens)) = model.thinking
        && tokens.get() >= model.max_tokens
    {
        return Err(Refusal::invalid(
            "model.thinking",
            format!(
                "a budget is fewer tokens than `model.max_tokens`, which is {}",
                model.max_tokens
            ),
        ));
    }
    Ok(RequestParams {
        max_tokens: resolved.max_tokens,
        temperature: resolved.temperature,
        thinking: resolved
            .thinking
            .value()
            .map(Into::into)
            .unwrap_or_default(),
        effort: resolved.effort.value().flatten().map(Into::into),
        seed: resolved.seed.value().flatten(),
        cache_scope: resolved.cache_scope.into(),
    })
}

/// Holds `model.base_url` to what a provider can be reached at. The value
/// may come from a variable, and the refusal is shown from the config as
/// it's written, so it says nothing of what the variable holds.
fn base_url(model: &config::Model) -> Result<(), Refusal> {
    let Some(url) = &model.base_url else {
        return Ok(());
    };
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    let names_a_host = host.is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'));
    if names_a_host && !url.contains(char::is_whitespace) {
        Ok(())
    } else {
        Err(Refusal::invalid(
            "model.base_url",
            "a URL begins `http://` or `https://` and names a host, as \
             `http://localhost:11434/v1` does",
        ))
    }
}

fn pricing(pricing: config::Pricing) -> Result<Pricing, Refusal> {
    let config::Pricing {
        input,
        output,
        cache_read,
        cache_write,
    } = pricing;
    Rates::new(input, output, cache_read, cache_write)
        .map(Pricing::new)
        .map_err(|error| {
            Refusal::invalid(
                &format!("model.pricing.{}", error.name),
                "a rate is a finite number of US dollars of at least 0",
            )
        })
}

fn system(prompt: &config::Prompt) -> Result<System, Refusal> {
    match (&prompt.system, &prompt.system_file) {
        (Some(text), None) => Ok(System::Text(text.clone())),
        (None, Some(file)) => Ok(System::File(file.clone())),
        (Some(_), Some(_)) => Err(Refusal::invalid(
            "prompt.system_file",
            "`prompt.system` is stated too, and a config states one of the two",
        )),
        (None, None) => Err(Refusal::Missing {
            key: "prompt.system",
            reason: "a config states `prompt.system` or `prompt.system_file`".to_owned(),
        }),
    }
}

fn output_cap(tools: &config::Tools) -> Result<Option<OutputCap>, Refusal> {
    let cut = match tools.output_cut {
        config::OutputCut::Preview => OutputCut::Preview {
            bytes: tools.output_preview_bytes,
        },
        config::OutputCut::Head => OutputCut::Head,
        config::OutputCut::HeadTail => OutputCut::HeadTail,
    };
    tools
        .max_output_bytes
        .map(|max_bytes| OutputCap::new(max_bytes, cut))
        .transpose()
        .map_err(|OutputCapError::PreviewAboveCap { max_bytes, .. }| {
            Refusal::invalid(
                "tools.output_preview_bytes",
                format!(
                    "a preview is no longer than `tools.max_output_bytes`, which is \
                         {max_bytes}"
                ),
            )
        })
}

fn filter(tools: &config::Tools) -> Result<ToolFilter, Refusal> {
    let names = |key: &str, names: &[String]| {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                ToolName::new(name.as_str()).map_err(|_| Refusal::Invalid {
                    key: KeyPath::of(key).index(index),
                    reason: "a tool's name is 1 to 64 ASCII letters, digits, `_` and `-`"
                        .to_owned(),
                })
            })
            .collect::<Result<Vec<_>, _>>()
    };
    // The loop reads an empty allow list as no list at all, which offers
    // every tool: the opposite of what a list that names none says.
    if tools.allow.as_ref().is_some_and(Vec::is_empty) {
        return Err(Refusal::invalid(
            "tools.allow",
            "a list that names no tool would offer none; leave the list out to offer every \
             tool, and enable no tool to offer none",
        ));
    }
    Ok(ToolFilter {
        allow: names("tools.allow", tools.allow.as_deref().unwrap_or_default())?,
        deny: names("tools.deny", &tools.deny)?,
    })
}

/// The built-in tool a config names, as the adapter knows it. A function
/// rather than a `From`, since neither type is this crate's.
fn tool(tool: BuiltinTool) -> Tool {
    match tool {
        BuiltinTool::Bash => Tool::Bash,
        BuiltinTool::ReadFile => Tool::ReadFile,
        BuiltinTool::WriteFile => Tool::WriteFile,
    }
}

fn builtin(builtin: &config::Builtin) -> Result<Option<lablet_tools_builtin::Settings>, Refusal> {
    let Some(&first) = builtin.enabled.first() else {
        return Ok(None);
    };
    let Some(root) = &builtin.root else {
        return Err(Refusal::Missing {
            key: "tools.builtin.root",
            reason: format!(
                "`tools.builtin.enabled` holds `{}`, and a built-in tool works under the root",
                tool(first)
            ),
        });
    };
    Ok(Some(lablet_tools_builtin::Settings {
        root: root.clone(),
        enabled: builtin.enabled.iter().copied().map(tool).collect(),
        timeout: builtin.timeout,
        env: builtin.env.clone(),
        // lablet's secrets are derived from the config and where it runs,
        // which `build` reads and this doesn't.
        withheld: BTreeSet::new(),
    }))
}

/// Holds each server's name to what a tool name allows, since a tool is
/// offered under its server's name, and to one reading: a name that held
/// `__` would make `mcp__<server>__<tool>` read two ways.
fn mcp_names(servers: &[config::McpServer]) -> Result<(), Refusal> {
    for (index, server) in servers.iter().enumerate() {
        let name = server.name();
        let refused = if name.is_empty() {
            "a server has a name"
        } else if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            "a server's name holds ASCII letters, digits, `-` and `_`"
        } else if name.contains("__") {
            "a server's name holds no `__`, which is what parts a tool's name from its server's"
        } else {
            continue;
        };
        return Err(Refusal::Invalid {
            key: KeyPath::of("tools.mcp").index(index).key("name"),
            reason: refused.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
