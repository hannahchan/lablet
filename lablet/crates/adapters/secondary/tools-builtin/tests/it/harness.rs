//! A root of a test's own, an executor under it, and calls to it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lablet_model::{
    Answer, KeptOutput, OutputCap, OutputKeep, ToolCallEnd, ToolCallId, ToolCallStatus, ToolName,
    ToolResultContent, ToolSource,
};
use lablet_run::{ToolCall, ToolError, ToolExecutor, ToolOutput};
use lablet_test_support::Scratch;
use lablet_tools_builtin::{BuiltinTools, Settings, Tool, Withheld};
use nix::errno::Errno;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use serde_json::Value;

/// The longest a call of these tests may take when it's given no deadline
/// of its own. No call that's meant to end takes a tenth of it.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// How long a test waits for something that another process does.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// A root of one test's own, in a scratch directory with room beside the
/// root for what's outside it. The directory is the one the system
/// resolves it to, so that a path a command prints is the path the test
/// expects.
pub struct Root(Scratch);

impl Root {
    pub fn new(test: &str) -> Self {
        let scratch = Scratch::new(test);
        scratch.create_dir("root");
        Self(scratch)
    }

    /// The root.
    pub fn root(&self) -> PathBuf {
        self.0.at("root")
    }

    /// A path beside the root, and so outside it.
    pub fn outside(&self, name: &str) -> PathBuf {
        self.0.at(name)
    }

    /// Writes `text` to `path` under the root, with the directories on the
    /// way to it.
    pub fn holds(&self, path: &str, text: impl AsRef<[u8]>) -> PathBuf {
        self.0.write(&format!("root/{path}"), text)
    }

    /// What the file at `path` under the root holds.
    pub fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root().join(path)).unwrap()
    }

    /// The settings of an executor that serves every tool under the root.
    pub fn settings(&self) -> Settings {
        Settings {
            root: self.root(),
            enabled: Tool::ALL.into(),
            timeout: TIMEOUT,
            env: BTreeMap::new(),
            withheld: Withheld::default(),
        }
    }

    /// An executor that serves every tool under the root.
    pub fn tools(&self) -> BuiltinTools {
        BuiltinTools::new(self.settings()).unwrap()
    }
}

/// A link at `link` that leads to `target`.
pub fn link(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

pub fn name(tool: &str) -> ToolName {
    ToolName::new(tool).unwrap()
}

/// A call to `tool` that has the executor's own limit to run in, and of
/// whose text everything is kept.
pub fn call(tool: &str, input: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("call_1").unwrap(),
        name: name(tool),
        input,
        deadline: TIMEOUT,
        keep: None,
        trace_context: None,
    }
}

pub fn within(deadline: Duration, mut call: ToolCall) -> ToolCall {
    call.deadline = deadline;
    call
}

pub fn keeping(keep: OutputKeep, mut call: ToolCall) -> ToolCall {
    call.keep = Some(keep);
    call
}

/// What a call to `tool` with `input` returns.
pub async fn ask(
    tools: &BuiltinTools,
    tool: &str,
    input: Value,
) -> Result<ToolOutput, Box<ToolError>> {
    tools.execute(call(tool, input)).await.map_err(Box::new)
}

/// The text of a call that returned a result and no error result.
pub async fn said(tools: &BuiltinTools, tool: &str, input: Value) -> String {
    let output = ask(tools, tool, input).await.unwrap();
    let text = text(output.output);
    assert!(!output.is_error, "{tool} returned an error result: {text}");
    text
}

/// The text of a call that returned an error result.
pub async fn refused(tools: &BuiltinTools, tool: &str, input: Value) -> String {
    let output = ask(tools, tool, input).await.unwrap();
    let text = text(output.output);
    assert!(output.is_error, "{tool} returned a result: {text}");
    text
}

/// The items the model is sent of `output` under `cap`.
pub fn sent(output: KeptOutput, cap: Option<OutputCap>) -> Vec<String> {
    let status = ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok);
    Answer::measured(status, output, cap, Duration::ZERO, Duration::ZERO)
        .content()
        .iter()
        .map(|ToolResultContent::Text(text)| text.clone())
        .collect()
}

/// The text of `output`, of which everything was kept.
pub fn text(output: KeptOutput) -> String {
    sent(output, None).concat()
}

/// Whether the process table holds a process `id`, one that has exited and
/// that nobody has waited for among them.
pub fn is_there(id: i32) -> bool {
    kill(Pid::from_raw(id), None) != Err(Errno::ESRCH)
}

/// The process id the file at `path` holds; `None` until it's written.
pub fn id_in(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Waits until `holds` does, for as long as [`PATIENCE`] lasts, and says
/// whether it came to hold.
pub async fn came_to_hold(holds: impl Fn() -> bool) -> bool {
    let began = Instant::now();
    while !holds() {
        if began.elapsed() > PATIENCE {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    true
}
