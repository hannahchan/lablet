//! A directory for each test, a config whose files are in it, and the
//! binary run there.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lablet_conformance::otlp::Exported;
use lablet_test_support::Scratch;
use serde_json::{Value, json};

pub use lablet_test_support::{PROMPT, SYSTEM};

pub const MODEL: &str = "scripted-1";

/// The name of the config [`Lab::config`] writes.
pub const CONFIG: &str = "lablet.json";

/// A response that ends the run, and nothing before it.
pub const ENDS: &str = "
- response:
    content:
      - text: Nothing to fix.
    usage: { input_tokens: 100, output_tokens: 10 }
    finish: end_turn
";

/// A scratch directory of one test's own, which the binary runs in.
pub struct Lab(Scratch);

impl Lab {
    pub fn new(test: &str) -> Self {
        Self(Scratch::new(test))
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }

    pub fn at(&self, path: &str) -> PathBuf {
        self.0.at(path)
    }

    pub fn write(&self, path: &str, text: &str) -> PathBuf {
        self.0.write(path, text)
    }

    /// The file the telemetry of [`Lab::config`]'s runs is appended to.
    pub fn telemetry(&self) -> PathBuf {
        self.at("telemetry.otlp.jsonl")
    }

    /// What the runs so far exported to [`Lab::telemetry`].
    pub fn exported(&self) -> Exported {
        Exported::read(&self.telemetry()).unwrap()
    }

    /// Writes [`CONFIG`], a config of a fake model that plays the YAML
    /// script `script`, with `more` stated over it.
    pub fn config(&self, script: &str, more: Value) {
        let mut tree = json!({
            "model": {
                "provider": "fake",
                "script": self.write("script.yaml", script),
                "name": MODEL,
            },
            "prompt": { "system": SYSTEM },
            "telemetry": { "file": { "path": self.telemetry() } },
        });
        state(&mut tree, more);
        self.write(CONFIG, &tree.to_string());
    }

    /// The binary with `args`, to be run in the directory, with no
    /// `RUST_LOG` of the test's.
    pub fn lablet(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lablet"));
        command
            .args(args)
            .current_dir(self.path())
            .env_remove("RUST_LOG");
        command
    }

    /// Runs `lablet args` in the directory with nothing on standard input.
    pub fn run(&self, args: &[&str]) -> Ran {
        ran(self.lablet(args), "")
    }

    /// Runs [`CONFIG`] on [`PROMPT`], with `more` arguments.
    pub fn run_config(&self, more: &[&str]) -> Ran {
        let mut args = vec!["run", "--config", CONFIG, "--prompt", PROMPT];
        args.extend(more);
        self.run(&args)
    }
}

/// States `more` over `tree`: a mapping is stated key by key, and anything
/// else in place of what was there.
fn state(tree: &mut Value, more: Value) {
    match (tree, more) {
        (Value::Object(tree), Value::Object(more)) => {
            for (key, value) in more {
                state(tree.entry(key).or_insert(Value::Null), value);
            }
        }
        (tree, more) => *tree = more,
    }
}

/// What a command printed and how it exited.
#[derive(Debug)]
pub struct Ran {
    /// The exit code; `None` when a signal ended the process.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    /// The outcome document, which is standard output's one line.
    pub fn outcome(&self) -> Value {
        let lines: Vec<&str> = self.stdout.lines().collect();
        assert_eq!(lines.len(), 1, "standard output is one line: {self:?}");
        serde_json::from_str(lines[0]).unwrap()
    }

    pub fn stderr_lines(&self) -> Vec<&str> {
        self.stderr.lines().collect()
    }
}

/// Runs `command` with `stdin` on its standard input, to its end.
pub fn ran(mut command: Command, stdin: &str) -> Ran {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A command that reads none of its input closes the pipe, and the write
    // then fails; what it read is what the test asserts on.
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    let output = child.wait_with_output().unwrap();
    Ran {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// `document` without what names a run and what times it, which is what
/// two runs of one config share.
pub fn shared(mut document: Value) -> Value {
    for key in ["run_id", "duration_ms"] {
        document.as_object_mut().unwrap().remove(key);
    }
    document
}
