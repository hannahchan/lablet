//! A directory for each test, a config whose files are in it, and what a
//! run left there, read back.

use std::path::{Path, PathBuf};

use lablet_conformance::otlp::{Exported, LogRecord, Span};
use lablet_test_support::Scratch;
use serde_json::{Value, json};

pub use lablet_test_support::{PROMPT, SYSTEM};

pub const MODEL: &str = "scripted-1";

/// A response that ends the run, and nothing before it.
pub const ENDS: &str = "
- response:
    content:
      - text: Nothing to fix.
    usage: { input_tokens: 100, output_tokens: 10 }
    finish: end_turn
";

/// A scratch directory of one test's own, laid out for lablet: the root of
/// the built-in tools is `work` in it, and lablet's own files are beside
/// the root.
pub struct Lab(Scratch);

impl Lab {
    pub fn new(test: &str) -> Self {
        let scratch = Scratch::new(test);
        scratch.create_dir("work");
        Self(scratch)
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }

    pub fn at(&self, path: &str) -> PathBuf {
        self.0.at(path)
    }

    pub fn root(&self) -> PathBuf {
        self.at("work")
    }

    /// The file every run's telemetry is appended to.
    pub fn telemetry(&self) -> PathBuf {
        self.at("telemetry.otlp.jsonl")
    }

    pub fn write(&self, path: &str, text: &str) -> PathBuf {
        self.0.write(path, text)
    }

    /// The config, as a tree, of a fake model that plays the YAML script
    /// `script`, with `more` stated over it. Its telemetry goes to the
    /// lab's file and nowhere else: a test that exports to a receiver
    /// states `telemetry.otlp.enabled: true` beside the receiver's
    /// endpoint, so no test sends to a collector on the developer's
    /// `localhost:4318`.
    pub fn tree(&self, script: &str, more: Value) -> Value {
        let mut tree = json!({
            "model": {
                "provider": "fake",
                "script": self.write("script.yaml", script),
                "name": MODEL,
            },
            "prompt": { "system": SYSTEM },
            "telemetry": {
                "file": { "path": self.telemetry() },
                "otlp": { "enabled": false },
                "resource": { "team": "evals" },
            },
        });
        state(&mut tree, more);
        tree
    }

    /// What `tools.builtin` states to enable `tools` under the root.
    pub fn builtin(&self, tools: &[&str]) -> Value {
        json!({ "root": self.root(), "enabled": tools })
    }

    /// What `tools` states to enable `tools` under the root.
    pub fn builtin_tools(&self, tools: &[&str]) -> Value {
        json!({ "builtin": self.builtin(tools) })
    }

    /// What the runs so far exported.
    pub fn exported(&self) -> Exported {
        Exported::read(&self.telemetry()).unwrap()
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

/// What one run exported, out of everything a file holds.
pub struct Traced<'a> {
    pub spans: Vec<&'a Span>,
    pub records: Vec<&'a LogRecord>,
}

impl<'a> Traced<'a> {
    /// The spans and records of the run `run_id`, which each names as its
    /// conversation.
    pub fn of(exported: &'a Exported, run_id: &str) -> Self {
        let of_run = |attributes: &lablet_conformance::otlp::Attributes| {
            attributes.get(crate::key::GEN_AI_CONVERSATION_ID) == Some(&json!(run_id))
        };
        Self {
            spans: exported
                .spans
                .iter()
                .filter(|span| of_run(&span.attributes))
                .collect(),
            records: exported
                .records
                .iter()
                .filter(|record| of_run(&record.attributes))
                .collect(),
        }
    }

    fn spans_of(&self, operation: &str) -> Vec<&'a Span> {
        self.spans
            .iter()
            .copied()
            .filter(|span| span.name.split(' ').next() == Some(operation))
            .collect()
    }

    pub fn chats(&self) -> Vec<&'a Span> {
        self.spans_of("chat")
    }

    pub fn wide(&self) -> &'a LogRecord {
        let wide: Vec<_> = self
            .records
            .iter()
            .copied()
            .filter(|record| record.event_name == crate::key::WIDE_EVENT)
            .collect();
        assert_eq!(wide.len(), 1, "a run has one wide event");
        wide[0]
    }
}

/// What a JSON file holds.
pub fn json_of(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}
