//! What every built-in tool is to the executor, and the results they share.

use std::time::Duration;

use lablet_model::{KeptOutput, OutputKeep, ToolConcurrency};
use lablet_run::{ToolError, ToolErrorKind, ToolOutput};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Tool;

/// What a tool tells the model of itself.
pub(crate) struct Offer {
    /// What the tool does, and what the model has to know to call it.
    pub(crate) description: &'static str,
    /// The JSON Schema of its arguments.
    pub(crate) input_schema: Value,
    /// Whether a call may run beside other calls.
    pub(crate) concurrency: ToolConcurrency,
}

/// What one call is run under.
#[derive(Clone, Copy)]
pub(crate) struct Terms {
    /// What's kept of the tool's text.
    pub(crate) keep: Option<OutputKeep>,
    /// The longest the call may take, which is more than no time.
    pub(crate) limit: Duration,
}

/// One built-in tool.
#[async_trait::async_trait]
pub(crate) trait BuiltIn: Send + Sync {
    /// Which tool it is.
    fn tool(&self) -> Tool;

    /// What it tells the model of itself.
    fn offer(&self) -> Offer;

    /// Runs one call, whose arguments are `input`. A failure the tool
    /// reports of its own is `Ok`, as an error result.
    async fn run(&self, input: Value, terms: Terms) -> Result<ToolOutput, ToolError>;
}

impl Terms {
    /// `input` read as the arguments `T`, or the error result that tells the
    /// model what's wrong with them: it can call again with arguments that
    /// fit, which makes this the tool's own report and no failure to run it.
    pub(crate) fn arguments<T: DeserializeOwned>(
        self,
        tool: Tool,
        input: Value,
    ) -> Result<T, Box<ToolOutput>> {
        serde_json::from_value(input).map_err(|error| {
            Box::new(self.error(&format!("the arguments of {tool} don't fit: {error}")))
        })
    }

    /// A result that says `text`.
    pub(crate) fn says(self, text: &str) -> ToolOutput {
        self.result(text, false)
    }

    /// An error result that says `text`.
    pub(crate) fn error(self, text: &str) -> ToolOutput {
        self.result(text, true)
    }

    fn result(self, text: &str, is_error: bool) -> ToolOutput {
        let mut output = KeptOutput::new(self.keep);
        output.push(text);
        ToolOutput {
            output,
            is_error,
            mcp: None,
        }
    }

    /// A call to `tool` ran for as long as it may, and what it started has
    /// stopped.
    pub(crate) fn ran_out(self, tool: Tool) -> ToolError {
        ToolError::new(
            ToolErrorKind::Timeout,
            format!(
                "{tool} was stopped after {:?}, the longest the call could take",
                self.limit
            ),
        )
    }
}

/// A tool couldn't be run, or couldn't be run to its end, as `says`.
pub(crate) fn failed(says: String) -> ToolError {
    ToolError::new(ToolErrorKind::Failed, says)
}
