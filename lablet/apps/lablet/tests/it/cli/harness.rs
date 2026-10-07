//! The binary, run in a lab's directory, and what it printed and how it
//! exited.

use std::ffi::OsString;
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
    /// `RUST_LOG` of the test's and none of its `OTEL_*` or context
    /// variables, since an endpoint among them would turn the network
    /// exporter on under every test, and a `TRACEPARENT` would give every
    /// run a parent; a test that needs one sets it with `env` after.
    pub fn lablet(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lablet"));
        command
            .args(args)
            .current_dir(self.path())
            .env_remove("RUST_LOG");
        without_otel(&mut command, std::env::vars_os().map(|(name, _)| name));
        command
    }
}

/// Takes every `OTEL_*` variable and every context variable among
/// `names`, the environment's names, off `command`'s environment. The names
/// alone are read, never a value.
fn without_otel(command: &mut Command, names: impl IntoIterator<Item = OsString>) {
    for name in names {
        let text = name.to_string_lossy();
        if text.starts_with("OTEL_") || ["TRACEPARENT", "TRACESTATE", "BAGGAGE"].contains(&&*text) {
            command.env_remove(name);
        }
    }
}

#[test]
fn the_binary_inherits_no_otel_or_context_variable_of_the_test_and_a_test_may_set_one_after() {
    let mut command = Command::new("lablet");
    let names = [
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "OTEL_TRACES_EXPORTER",
        "TRACEPARENT",
        "TRACESTATE",
        "BAGGAGE",
        "NOT_OTEL_X",
        "traceparent",
        "RUST_LOG",
    ];

    without_otel(&mut command, names.map(OsString::from));
    command.env("OTEL_TRACES_EXPORTER", "none");

    let given: Vec<(String, Option<String>)> = command
        .get_envs()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect();
    assert_eq!(
        given,
        [
            ("BAGGAGE".to_owned(), None),
            ("OTEL_EXPORTER_OTLP_ENDPOINT".to_owned(), None),
            ("OTEL_TRACES_EXPORTER".to_owned(), Some("none".to_owned())),
            ("TRACEPARENT".to_owned(), None),
            ("TRACESTATE".to_owned(), None),
        ]
    );
}

impl Lab {
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
