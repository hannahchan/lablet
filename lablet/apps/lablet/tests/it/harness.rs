//! A directory for each test, a config whose files are in it, the host
//! whose providers a `Lablet` is built with, and what the host was handed,
//! read back.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lablet::{BuildError, Config, Format, Lablet, Otel, RunRequest, Unsupported};
use lablet_conformance::host::Host;
use lablet_conformance::otlp::{Exported, LogRecord, Span};
use lablet_test_support::Scratch;
use opentelemetry::trace::noop::NoopTextMapPropagator;
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
/// the root. Its host is the one every `Lablet` it builds is handed.
pub struct Lab(Scratch, Host);

impl Lab {
    pub fn new(test: &str) -> Self {
        let scratch = Scratch::new(test);
        scratch.create_dir("work");
        Self(scratch, Host::new())
    }

    /// The host every `Lablet` the lab builds is handed.
    pub fn host(&self) -> &Host {
        &self.1
    }

    /// A `Lablet` of `config`, built with the lab's host's [`Lab::otel`].
    pub async fn build(&self, config: Config) -> Result<Lablet, BuildError> {
        Lablet::builder(config, self.otel()).build().await
    }

    /// The lab's host's tracer and logger providers, and the API's no-op
    /// propagator, since no test of this binary reads a command's context.
    pub fn otel(&self) -> Otel {
        Otel::new(
            self.1.tracer_provider(),
            self.1.logger_provider(),
            NoopTextMapPropagator::new(),
        )
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

    pub fn write(&self, path: &str, text: &str) -> PathBuf {
        self.0.write(path, text)
    }

    /// The config, as a tree, of a fake model that plays the YAML script
    /// `script`, with `more` stated over it. It states no telemetry: what a
    /// run emits goes to the lab's host.
    pub fn tree(&self, script: &str, more: Value) -> Value {
        let mut tree = json!({
            "model": {
                "provider": "fake",
                "script": self.write("script.yaml", script),
                "name": MODEL,
            },
            "prompt": { "system": SYSTEM },
        });
        state(&mut tree, more);
        tree
    }

    /// That config, read.
    pub fn config(&self, script: &str, more: Value) -> Config {
        read(&self.tree(script, more))
    }

    /// What `tools.builtin` states to enable `tools` under the root.
    pub fn builtin(&self, tools: &[&str]) -> Value {
        json!({ "root": self.root(), "enabled": tools })
    }

    /// What `tools` states to enable `tools` under the root.
    pub fn builtin_tools(&self, tools: &[&str]) -> Value {
        json!({ "builtin": self.builtin(tools) })
    }

    /// What the runs so far handed the lab's host.
    pub fn exported(&self) -> Exported {
        self.1.exported()
    }

    /// The same, as the file exporter's lines would hold it.
    pub fn exported_text(&self) -> String {
        self.1.lines()
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

pub fn read(tree: &Value) -> Config {
    Config::from_str(&tree.to_string(), Format::Json).unwrap()
}

pub fn request() -> RunRequest {
    RunRequest::new(PROMPT).unwrap()
}

/// A `Lablet` of `config` whose records go nowhere, for a test that reads
/// none of what a run emits.
pub async fn quiet(config: Config) -> Result<Lablet, BuildError> {
    lablet::build(config, Otel::noop()).await
}

/// What `build` refuses `config` with, which `check` refuses it with too,
/// but for a provider this lablet has no adapter for yet, which the check
/// passes since it stops before the provider is selected.
pub async fn refusal(config: Config) -> BuildError {
    let checked = lablet::check(&config).await.map(drop);
    let built = lablet::build(config, Otel::noop())
        .await
        .map(drop)
        .unwrap_err();
    match &built {
        BuildError::Unsupported {
            kind: Unsupported::Anthropic | Unsupported::Openai,
            ..
        } => assert_eq!(checked, Ok(()), "the check passes a provider: {built:?}"),
        _ => assert_eq!(checked, Err(built.clone()), "check and build refuse alike"),
    }
    built
}

/// What one run emitted, out of everything a host was handed.
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

    pub fn root(&self) -> &'a Span {
        let roots = self.spans_of("invoke_agent");
        assert_eq!(roots.len(), 1, "a run has one root span");
        roots[0]
    }

    pub fn chats(&self) -> Vec<&'a Span> {
        self.spans_of("chat")
    }

    pub fn tools(&self) -> Vec<&'a Span> {
        self.spans_of("execute_tool")
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

/// The diagnostic log of the thread that captures it, for as long as this
/// lives.
pub struct Diagnostics {
    written: Arc<Mutex<Vec<u8>>>,
    _captured: tracing::subscriber::DefaultGuard,
}

struct Written(Arc<Mutex<Vec<u8>>>);

impl io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Diagnostics {
    pub fn capture() -> Self {
        let written = Arc::new(Mutex::new(Vec::new()));
        let to = Arc::clone(&written);
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || Written(Arc::clone(&to)))
            .finish();
        Self {
            written,
            _captured: tracing::subscriber::set_default(subscriber),
        }
    }

    /// The lines written so far.
    pub fn lines(&self) -> Vec<String> {
        String::from_utf8(self.written.lock().unwrap().clone())
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}
