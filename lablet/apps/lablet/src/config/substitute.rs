//! `${VAR}`: the value of an environment variable, in a setting that holds
//! text.
//!
//! A config holds its settings as they're written, and the config a run is
//! built from is that config with each `${NAME}` replaced by what the
//! variable holds. So the digest and every message see the config as it's
//! written, and nothing a variable holds reaches either.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use serde_json::Value;

use super::key::KeyPath;
use super::{
    Builtin, Config, McpServer, Model, Otlp, Prompt, Refusal, Run, Telemetry, TelemetryFile, Tools,
};

/// Where lablet runs, as a function from a variable's name to what it
/// holds, so a test can say what it holds.
pub(crate) type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// `text` with each `${NAME}` in it replaced by what the variable `NAME`
/// holds, and each `$${` by `${`, so a text can hold `${` itself. Any other
/// `$` is text.
///
/// # Errors
///
/// Returns why the text was refused: a `${` that begins no reference to a
/// variable, a variable that isn't set, or one that holds what isn't UTF-8.
/// No reason shows what a variable holds.
pub(crate) fn substituted(text: &str, env: Env<'_>) -> Result<String, String> {
    let mut written = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        written.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(beyond) = after.strip_prefix("${") {
            written.push_str("${");
            rest = beyond;
        } else if let Some(reference) = after.strip_prefix('{') {
            let (name, beyond) = reference
                .split_once('}')
                .filter(|(name, _)| is_name(name))
                .ok_or_else(|| {
                    "`${` begins a variable, written `${NAME}` with a name of ASCII letters, \
                     digits and `_`; write `$${` for the text `${`"
                        .to_owned()
                })?;
            let value = env(name).ok_or_else(|| format!("the variable `{name}` isn't set"))?;
            let value = value
                .into_string()
                .map_err(|_| format!("the variable `{name}` holds what isn't UTF-8"))?;
            written.push_str(&value);
            rest = beyond;
        } else {
            written.push('$');
            rest = after;
        }
    }
    written.push_str(rest);
    Ok(written)
}

/// Whether `name` is the name of an environment variable: ASCII letters,
/// digits and `_`, beginning with no digit.
fn is_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Substitutes into one setting's text, and refuses it by its key.
struct Substitute<'a> {
    env: Env<'a>,
}

impl Substitute<'_> {
    fn text(&self, text: &mut String, key: &KeyPath) -> Result<(), Refusal> {
        if text.contains('$') {
            *text = substituted(text, self.env).map_err(|reason| Refusal::Invalid {
                key: key.clone(),
                reason,
            })?;
        }
        Ok(())
    }

    fn optional(&self, text: &mut Option<String>, key: &str) -> Result<(), Refusal> {
        text.as_mut()
            .map_or(Ok(()), |text| self.text(text, &KeyPath::of(key)))
    }

    /// A path that isn't UTF-8 was made in code, and holds no reference to
    /// a variable a config could write.
    fn path(&self, path: &mut PathBuf, key: &KeyPath) -> Result<(), Refusal> {
        if let Some(text) = path.to_str() {
            let mut text = text.to_owned();
            self.text(&mut text, key)?;
            *path = text.into();
        }
        Ok(())
    }

    fn optional_path(&self, path: &mut Option<PathBuf>, key: &str) -> Result<(), Refusal> {
        path.as_mut()
            .map_or(Ok(()), |path| self.path(path, &KeyPath::of(key)))
    }

    fn texts(&self, texts: &mut [String], key: &KeyPath) -> Result<(), Refusal> {
        texts
            .iter_mut()
            .enumerate()
            .try_for_each(|(index, text)| self.text(text, &key.index(index)))
    }

    /// The values of a mapping; its keys are names, and stay as written.
    fn values(&self, values: &mut BTreeMap<String, String>, key: &KeyPath) -> Result<(), Refusal> {
        values
            .iter_mut()
            .try_for_each(|(name, value)| self.text(value, &key.key(name)))
    }

    /// Every text a JSON value holds, at any depth.
    fn json(&self, value: &mut Value, key: &KeyPath) -> Result<(), Refusal> {
        match value {
            Value::String(text) => self.text(text, key),
            Value::Array(items) => items
                .iter_mut()
                .enumerate()
                .try_for_each(|(index, item)| self.json(item, &key.index(index))),
            Value::Object(entries) => entries
                .iter_mut()
                .try_for_each(|(name, item)| self.json(item, &key.key(name))),
            Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
        }
    }

    fn server(&self, server: &mut McpServer, key: &KeyPath) -> Result<(), Refusal> {
        match server {
            McpServer::Stdio {
                name,
                command,
                args,
                env,
                startup_timeout: _,
                call_timeout: _,
                names: _,
                instructions: _,
            } => {
                self.text(name, &key.key("name"))?;
                self.text(command, &key.key("command"))?;
                self.texts(args, &key.key("args"))?;
                self.values(env, &key.key("env"))
            }
            McpServer::Http {
                name,
                url,
                headers,
                startup_timeout: _,
                call_timeout: _,
                names: _,
                instructions: _,
            } => {
                self.text(name, &key.key("name"))?;
                self.text(url, &key.key("url"))?;
                self.values(headers, &key.key("headers"))
            }
        }
    }
}

impl Substitute<'_> {
    fn run(&self, run: &mut Run) -> Result<(), Refusal> {
        let Run {
            transcript_path,
            completion_schema,
            completion: _,
            max_turns: _,
            timeout: _,
            max_total_tokens: _,
            max_retries: _,
            retry_backoff_base: _,
            retry_backoff_max: _,
            retry_jitter: _,
            retry_hint_max: _,
            max_consecutive_invalid_turns: _,
            provider_timeout: _,
            context: _,
            transcript_format: _,
        } = run;
        self.optional_path(transcript_path, "run.transcript_path")?;
        completion_schema.as_mut().map_or(Ok(()), |schema| {
            self.json(schema, &KeyPath::of("run.completion_schema"))
        })
    }

    fn model(&self, model: &mut Model) -> Result<(), Refusal> {
        let Model {
            script,
            name,
            api_key_env,
            base_url,
            provider: _,
            api: _,
            max_tokens: _,
            temperature: _,
            thinking: _,
            effort: _,
            seed: _,
            reasoning_replay: _,
            cache: _,
            cache_scope: _,
            pricing: _,
        } = model;
        self.optional_path(script, "model.script")?;
        self.text(name, &KeyPath::of("model.name"))?;
        self.optional(api_key_env, "model.api_key_env")?;
        self.optional(base_url, "model.base_url")
    }

    fn prompt(&self, prompt: &mut Prompt) -> Result<(), Refusal> {
        let Prompt {
            system,
            system_file,
            skills,
            skills_mode: _,
        } = prompt;
        self.optional(system, "prompt.system")?;
        self.optional_path(system_file, "prompt.system_file")?;
        let key = KeyPath::of("prompt.skills");
        skills
            .iter_mut()
            .enumerate()
            .try_for_each(|(index, skill)| self.path(skill, &key.index(index)))
    }

    fn tools(&self, tools: &mut Tools) -> Result<(), Refusal> {
        let Tools {
            builtin:
                Builtin {
                    root,
                    env,
                    enabled: _,
                    timeout: _,
                },
            mcp,
            allow,
            deny,
            mcp_lifetime: _,
            mcp_result: _,
            max_concurrent_calls: _,
            max_output_bytes: _,
            output_cut: _,
            output_preview_bytes: _,
            max_description_chars: _,
        } = tools;
        self.optional_path(root, "tools.builtin.root")?;
        self.values(env, &KeyPath::of("tools.builtin.env"))?;
        let key = KeyPath::of("tools.mcp");
        for (index, server) in mcp.iter_mut().enumerate() {
            self.server(server, &key.index(index))?;
        }
        if let Some(allow) = allow {
            self.texts(allow, &KeyPath::of("tools.allow"))?;
        }
        self.texts(deny, &KeyPath::of("tools.deny"))
    }

    fn telemetry(&self, telemetry: &mut Telemetry) -> Result<(), Refusal> {
        let Telemetry {
            otlp:
                Otlp {
                    endpoint,
                    headers,
                    protocol: _,
                },
            file: TelemetryFile { path },
            resource,
            capture_content: _,
        } = telemetry;
        self.optional(endpoint, "telemetry.otlp.endpoint")?;
        self.values(headers, &KeyPath::of("telemetry.otlp.headers"))?;
        self.optional_path(path, "telemetry.file.path")?;
        self.values(resource, &KeyPath::of("telemetry.resource"))
    }
}

impl Config {
    /// The config a run is built from: this one, with each `${NAME}` in a
    /// setting that holds text replaced by what the variable holds.
    ///
    /// Every setting is named on the way, so a setting the config gains
    /// doesn't compile until it says whether it holds text.
    pub(crate) fn substituted(&self, env: Env<'_>) -> Result<Self, Refusal> {
        let at = Substitute { env };
        let mut config = self.clone();
        let Self {
            run,
            model,
            prompt,
            tools,
            telemetry,
            source: _,
            places: _,
            prompt_file: _,
        } = &mut config;
        at.run(run)?;
        at.model(model)?;
        at.prompt(prompt)?;
        at.tools(tools)?;
        at.telemetry(telemetry)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests;
