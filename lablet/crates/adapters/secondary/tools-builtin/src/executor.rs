//! The executor: the tools it was built with, and the limit every call is
//! held to.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use lablet_model::{ToolName, ToolSource, ToolSpec};
use lablet_run::{ToolCall, ToolError, ToolErrorKind, ToolExecutor, ToolOutput};
use opentelemetry::propagation::TextMapPropagator;

use crate::bash::Bash;
use crate::read_file::ReadFile;
use crate::root::Root;
use crate::settings::environment;
use crate::tool::{BuiltIn, Offer, Terms, failed};
use crate::write_file::WriteFile;
use crate::{Settings, SettingsError, Tool};

/// Serves the built-in tools it was built with, and no other.
///
/// The default serves none, which is what a run that measures an MCP server
/// wants of it.
#[derive(Default)]
pub struct BuiltinTools {
    tools: Vec<Box<dyn BuiltIn>>,
    timeout: Duration,
}

impl core::fmt::Debug for BuiltinTools {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let tools: Vec<Tool> = self.tools.iter().map(|held| held.tool()).collect();
        f.debug_struct("BuiltinTools")
            .field("tools", &tools)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl BuiltinTools {
    /// An executor that serves the tools `settings` enables, under its root,
    /// and injects the context a command starts in through `propagator`.
    ///
    /// lablet's environment is read here, once, so every command of every
    /// run starts with the same variables, but for the context's: those are
    /// the command's own, injected as it starts.
    ///
    /// # Errors
    ///
    /// Returns a [`SettingsError`] when the root isn't a directory that
    /// exists, and when a variable is one no process can be started with.
    pub fn new(
        settings: Settings,
        propagator: Arc<dyn TextMapPropagator + Send + Sync>,
    ) -> Result<Self, SettingsError> {
        let Settings {
            root,
            enabled,
            timeout,
            env,
            withheld,
        } = settings;
        let root = Root::open(&root)?;
        // A withheld variable is a secret, and a context the command starts in
        // may carry its value: a trace state or a baggage passes on verbatim.
        let kept: BTreeSet<String> = env.keys().chain(&withheld).cloned().collect();
        let environment = environment(std::env::vars_os(), &withheld, env)?;
        let tools = enabled
            .into_iter()
            .map(move |tool| -> Box<dyn BuiltIn> {
                let root = root.clone();
                match tool {
                    Tool::Bash => Box::new(Bash {
                        root,
                        environment: environment.clone(),
                        kept: kept.clone(),
                        propagator: Arc::clone(&propagator),
                    }),
                    Tool::ReadFile => Box::new(ReadFile { root }),
                    Tool::WriteFile => Box::new(WriteFile { root }),
                }
            })
            .collect();
        Ok(Self { tools, timeout })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for BuiltinTools {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError> {
        let mut specs = Vec::with_capacity(self.tools.len());
        for held in &self.tools {
            let Offer {
                description,
                input_schema,
                concurrency,
            } = held.offer();
            specs.push(ToolSpec {
                name: ToolName::new(held.tool().name())
                    .map_err(|error| failed(error.to_string()))?,
                description: format!(
                    "{description} A call that takes longer than {:?} is stopped.",
                    self.timeout
                ),
                input_schema,
                source: ToolSource::Builtin,
                concurrency,
            });
        }
        Ok(specs)
    }

    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError> {
        let ToolCall {
            name,
            input,
            deadline,
            keep,
            secrets,
            id: _,
        } = call;
        let name = name.as_str();
        let Some(held) = self.tools.iter().find(|held| held.tool().name() == name) else {
            return Err(ToolError::new(
                ToolErrorKind::Unknown,
                format!("no built-in tool named {name} is enabled"),
            ));
        };
        let limit = deadline.min(self.timeout);
        if limit.is_zero() {
            return Err(ToolError::new(
                ToolErrorKind::Timeout,
                format!("{name} wasn't started: the call had no time left"),
            ));
        }
        let terms = Terms {
            keep,
            limit,
            secrets: &secrets,
        };
        held.run(input, terms).await
    }
}
