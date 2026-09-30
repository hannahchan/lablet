//! The binary, run in a lab's directory, and what it printed and how it
//! exited.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

pub use crate::harness::{ENDS, Lab, PROMPT};

/// The name of the config [`Lab::write_config`] writes.
pub const CONFIG: &str = "lablet.json";

impl Lab {
    /// Writes [`CONFIG`], the lab's config of a fake model that plays the
    /// YAML script `script`, with `more` stated over it.
    pub fn write_config(&self, script: &str, more: Value) {
        self.write(CONFIG, &self.tree(script, more).to_string());
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
