//! The `tools` section: the built-in tools, the MCP servers, which of their
//! tools a run offers, and what bounds a call.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::written::{duration, path};

/// The `tools` section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tools {
    /// The built-in tools.
    pub builtin: Builtin,
    /// The MCP servers.
    pub mcp: Vec<McpServer>,
    /// How long the MCP servers live.
    pub mcp_lifetime: McpLifetime,
    /// Which part the model is sent of an MCP result that has two.
    pub mcp_result: McpResult,
    /// The only tools offered, by the names the model sees; `None` offers
    /// every tool.
    pub allow: Option<Vec<String>>,
    /// Tools that are never offered, by the names the model sees.
    pub deny: Vec<String>,
    /// How many calls of one turn may run at once; 1 runs every call alone.
    pub max_concurrent_calls: NonZeroU32,
    /// The size in bytes above which a tool result is cut; `None` is no
    /// cap.
    pub max_output_bytes: Option<u64>,
    /// What's kept of a result that's cut.
    pub output_cut: OutputCut,
    /// How many bytes a preview keeps.
    pub output_preview_bytes: u64,
    /// The length in characters above which a tool description or a
    /// server's instructions are cut; `None` is no cap.
    pub max_description_chars: Option<u32>,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            builtin: Builtin::default(),
            mcp: Vec::new(),
            mcp_lifetime: McpLifetime::Run,
            mcp_result: McpResult::Structured,
            allow: None,
            deny: Vec::new(),
            max_concurrent_calls: NonZeroU32::MIN.saturating_add(9),
            max_output_bytes: Some(50_000),
            output_cut: OutputCut::Preview,
            output_preview_bytes: 2_000,
            max_description_chars: Some(2_048),
        }
    }
}

/// The `tools.builtin` section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Builtin {
    /// The directory `bash` starts in and the file tools stay under. It's
    /// needed once a tool is enabled, and it holds none of lablet's own
    /// files.
    #[serde(serialize_with = "path::optional")]
    pub root: Option<PathBuf>,
    /// The tools to serve. `task_complete` isn't one of them: explicit
    /// completion implies it.
    pub enabled: BTreeSet<BuiltinTool>,
    /// The longest a call may take.
    #[serde(with = "duration")]
    pub timeout: Duration,
    /// Variables a command starts with beside the short list `bash` has.
    pub env: BTreeMap<String, String>,
}

impl Default for Builtin {
    fn default() -> Self {
        Self {
            root: None,
            enabled: BTreeSet::new(),
            timeout: Duration::from_secs(120),
            env: BTreeMap::new(),
        }
    }
}

/// One of the built-in tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinTool {
    /// Runs a command.
    Bash,
    /// Reads a file under the root.
    ReadFile,
    /// Writes a file under the root.
    WriteFile,
}

/// One MCP server, by how it's reached.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpServer {
    /// A child process, spoken to over its standard input and output.
    Stdio {
        /// The server's name: letters, digits, `-` and `_`.
        name: String,
        /// The command that starts the server.
        command: String,
        /// The command's arguments.
        #[serde(default)]
        args: Vec<String>,
        /// Variables the server starts with beside the short list it has.
        #[serde(default)]
        env: BTreeMap<String, String>,
        /// How long the server may take to start.
        #[serde(default = "startup_timeout", with = "duration")]
        startup_timeout: Duration,
        /// The longest a call may take.
        #[serde(default = "call_timeout", with = "duration")]
        call_timeout: Duration,
        /// Whether a tool's name carries the server's.
        #[serde(default)]
        names: McpNames,
        /// Whether the server's instructions follow the system prompt.
        #[serde(default = "instructions")]
        instructions: bool,
    },
    /// A server reached over streamable HTTP.
    Http {
        /// The server's name: letters, digits, `-` and `_`.
        name: String,
        /// Where the server listens.
        url: String,
        /// Headers sent with every request.
        #[serde(default)]
        headers: BTreeMap<String, String>,
        /// How long the server may take to answer its first request.
        #[serde(default = "startup_timeout", with = "duration")]
        startup_timeout: Duration,
        /// The longest a call may take.
        #[serde(default = "call_timeout", with = "duration")]
        call_timeout: Duration,
        /// Whether a tool's name carries the server's.
        #[serde(default)]
        names: McpNames,
        /// Whether the server's instructions follow the system prompt.
        #[serde(default = "instructions")]
        instructions: bool,
    },
}

impl McpServer {
    /// The server's name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Stdio { name, .. } | Self::Http { name, .. } => name,
        }
    }
}

const fn startup_timeout() -> Duration {
    Duration::from_secs(30)
}

const fn call_timeout() -> Duration {
    Duration::from_secs(300)
}

const fn instructions() -> bool {
    true
}

/// Whether an MCP tool's name carries its server's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpNames {
    /// A tool is offered as `mcp__<server>__<tool>`.
    #[default]
    Prefixed,
    /// A tool is offered under the name its server gives it.
    Own,
}

/// How long a run's MCP servers live.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpLifetime {
    /// The servers are started again for each run after the first.
    #[default]
    Run,
    /// The servers are started once and serve every run.
    Lablet,
}

/// Which part the model is sent of an MCP result that has two.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpResult {
    /// The structured content.
    #[default]
    Structured,
    /// The content items.
    Content,
}

/// What's kept of a tool result that's cut.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputCut {
    /// A short preview of the start.
    #[default]
    Preview,
    /// The start, up to the cap.
    Head,
    /// Half the cap from each end.
    HeadTail,
}
