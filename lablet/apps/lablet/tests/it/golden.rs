//! The golden comparison: three runs through the library on a paused clock,
//! each normalised and held equal to the fixture checked in under
//! `lablet/tests/fixtures/golden/`, so a change in what lablet emits is a
//! change to a file the changelog gate watches.
//!
//! The runs are the `init --provider fake` starter, `lablet/examples/two-turns`
//! and a run that retries, captures content, runs two `read_file` calls as
//! one group and is cancelled during a `bash` call. Each config is read from
//! the file it's kept in, with its relative paths rebased on that file's
//! directory, so a golden run can't drift from what a user runs, and the
//! paths stay relative, so the config digest is the same on every machine.
//! That digest is of the rebased paths: it isn't the one a user's run of the
//! same file carries.
//!
//! Tokio's clock is paused, so every time is decided by the script: a
//! latency or a backoff passes at once and is measured as exactly what was
//! asked, a `read_file` call holds the clock still while its blocking reads
//! run, and a `bash` command parks the runtime, which moves the clock to the
//! next timer, the one that cancels the run. The run id is fixed, so the
//! backoff's jitter, which is salted with it, is the same every time.
//!
//! What's normalised: trace and span ids become their order of first
//! appearance in a canonical order of the signals, times become their offset
//! in nanoseconds from the root span's start, so a fixture shows a time as
//! finely as lablet exports it, the attributes of every
//! signal are sorted by key, and the SDK's own version in the resource
//! becomes the word `normalised`, so a bump of the SDK stays a commit of its
//! own. Nothing else is: lablet's version is in the fixture as it is, and a
//! version bump regenerates it.
//!
//! An `OTEL_*` variable in the environment would change the resource or add
//! a network destination, so the test refuses to run with one set; `cargo
//! xtask test` strips them. `LABLET_UPDATE_GOLDEN=1 cargo test -p lablet
//! --test it golden` writes the fixtures instead of comparing. CI never sets
//! it, and the test refuses it there.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::key;
use lablet::config::Provider;
use lablet::{CancelHandle, Config, FinishedRun, Format, RunId, RunRequest, StopReason};
use lablet_conformance::otlp::{Attributes, Exported, LogRecord, Span, SpanKind, Status};
use lablet_run::telemetry::generated::GenAiClientOperationException;
use lablet_test_support::Scratch;
use serde_json::{Value, json};

/// Set, the test writes each fixture in place of comparing with it.
const UPDATE: &str = "LABLET_UPDATE_GOLDEN";

/// What the SDK's own version is written as in a fixture.
const NORMALISED: &str = "normalised";

/// Where the fixtures are: `lablet/tests/fixtures/golden`, two directories
/// up from this package. Walked up rather than joined with `..`, so a
/// failure names a fixture as the repository knows it.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("tests/fixtures/golden")
}

/// One golden run: a config file, where it is, and what the run must come
/// to before its export is compared.
struct Golden {
    /// The fixture's directory under [`fixtures`], and the run's id.
    name: &'static str,
    /// The config as its file states it.
    config: &'static str,
    /// The config file's directory, from the directory the test runs in,
    /// which the config's relative paths start at.
    directory: &'static str,
    prompt: &'static str,
    /// When the run is cancelled, in tokio's time from when it starts.
    cancelled_after: Option<Duration>,
    stop: StopReason,
    spans: usize,
    records: usize,
}

const STARTER: Golden = Golden {
    name: "starter",
    config: include_str!("../../src/cli/init/fake.yaml"),
    directory: "src/cli/init",
    prompt: "Say hello.",
    cancelled_after: None,
    stop: StopReason::Completed,
    spans: 2,
    records: 1,
};

const TWO_TURNS: Golden = Golden {
    name: "two-turns",
    config: include_str!("../../../../examples/two-turns/lablet.yaml"),
    directory: "../../examples/two-turns",
    prompt: "Read notes.md and say what it holds.",
    cancelled_after: None,
    stop: StopReason::Completed,
    spans: 4,
    records: 1,
};

/// The cancellation comes while `bash` runs: the earliest timer then, since
/// a call may take 120 s and the run 600 s.
const CANCELLED: Golden = Golden {
    name: "cancelled",
    config: include_str!("../../../../tests/fixtures/golden/cancelled/lablet.yaml"),
    directory: "../../tests/fixtures/golden/cancelled",
    prompt: "Read the notes and the plan, then run the build.",
    cancelled_after: Some(Duration::from_secs(10)),
    stop: StopReason::Cancelled,
    spans: 6,
    records: 8,
};

impl Golden {
    /// The config, with its paths rebased on its directory, its network
    /// exporter off and its telemetry written to `telemetry`.
    ///
    /// `model.script` and `tools.builtin.root` are the only relative paths a
    /// golden config may hold: the others a config takes are checked to be
    /// unset, since one that was set would start at the test's directory
    /// with nothing to say so.
    fn config(&self, telemetry: PathBuf) -> Config {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let current = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert_eq!(
            current, manifest,
            "the golden runs start in the package directory, which the configs' relative paths \
             are rebased from; cargo runs the tests there"
        );
        let mut config = Config::from_str(self.config, Format::Yaml).unwrap();
        assert_eq!(config.model.provider, Provider::Fake, "{}", self.name);
        assert_eq!(config.prompt.system_file, None, "{}", self.name);
        assert!(config.prompt.skills.is_empty(), "{}", self.name);
        assert_eq!(config.run.transcript_path, None, "{}", self.name);
        assert!(config.tools.mcp.is_empty(), "{}", self.name);
        let directory = Path::new(self.directory);
        let rebased = |path: Option<PathBuf>| {
            path.map(|path| {
                assert!(path.is_relative(), "{}: {}", self.name, path.display());
                directory.join(path)
            })
        };
        config.model.script = rebased(config.model.script.take());
        config.tools.builtin.root = rebased(config.tools.builtin.root.take());
        // The `telemetry` section is left out of the config digest, so this
        // changes nothing a fixture holds, and the file is the only
        // destination whatever the environment names.
        config.telemetry.otlp.enabled = Some(false);
        config.telemetry.file.path = Some(telemetry);
        config
    }

    fn fixture(&self) -> PathBuf {
        fixtures().join(self.name).join("expected.json")
    }

    /// Runs once, under its own id, and reads back what the run exported.
    async fn run(&self) -> (FinishedRun, Exported) {
        let scratch = Scratch::new(&format!("golden-{}", self.name));
        let telemetry = scratch.at("telemetry.otlp.jsonl");
        let mut lablet = lablet::build(self.config(telemetry.clone())).await.unwrap();
        let mut request = RunRequest::new(self.prompt)
            .unwrap()
            .run_id(RunId::new(format!("golden-{}", self.name)).unwrap())
            .unwrap();
        if let Some(after) = self.cancelled_after {
            let handle = CancelHandle::new();
            let fired = handle.clone();
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                fired.cancel();
            });
            request = request.cancellation(handle);
        }

        let finished = lablet.run(request).await;
        lablet.shutdown().await;

        (finished, Exported::read(&telemetry).unwrap())
    }

    /// Runs, holds what the run must come to, and compares the normalised
    /// export with the fixture, or writes the fixture when [`UPDATE`] is
    /// set.
    async fn check(&self) -> (FinishedRun, Exported) {
        refuse_the_environment();
        let (finished, exported) = self.run().await;

        assert_eq!(finished.summary.outcome.stop_reason(), self.stop);
        assert_eq!(exported.spans.len(), self.spans, "{}: spans", self.name);
        assert_eq!(
            exported.records.len(),
            self.records,
            "{}: records",
            self.name
        );
        assert_eq!(exported.records_of("lablet.run").len(), 1, "{}", self.name);

        let normalised = normalised(&exported);
        let fixture = self.fixture();
        if std::env::var_os(UPDATE).is_some() {
            let written = serde_json::to_string_pretty(&normalised).unwrap() + "\n";
            std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
            std::fs::write(&fixture, written).unwrap();
        } else {
            let expected: Value = match std::fs::read_to_string(&fixture) {
                Ok(text) => serde_json::from_str(&text).unwrap(),
                Err(error) => panic!(
                    "{} couldn't be read: {error}\n{}",
                    fixture.display(),
                    regenerate()
                ),
            };
            let mut found = Vec::new();
            differences("", &expected, &normalised, &mut found);
            assert!(
                found.is_empty(),
                "the {} run's telemetry differs from {}:\n  {}\n{}",
                self.name,
                fixture.display(),
                found.join("\n  "),
                regenerate()
            );
        }
        (finished, exported)
    }
}

/// Refuses an environment the comparison can't be trusted in: an `OTEL_*`
/// variable, which the SDK and the exporter read, and [`UPDATE`] in CI,
/// where a regeneration would pass whatever the run emitted. Names are
/// read, never a value.
fn refuse_the_environment() {
    let otel: BTreeSet<String> = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .filter(|name| name.starts_with("OTEL_"))
        .collect();
    assert!(
        otel.is_empty(),
        "{} is set, and an `OTEL_*` variable changes the resource or the exporter of every run; \
         unset it, or run `cargo xtask test`, which strips them",
        otel.into_iter().collect::<Vec<_>>().join(", ")
    );
    if std::env::var_os(UPDATE).is_some() {
        assert!(
            std::env::var_os("CI").is_none(),
            "{UPDATE} is set in CI, and the fixtures are regenerated locally, never there"
        );
    }
}

/// What a difference says to do: a change to what lablet emits is a change
/// to the telemetry contract, so the fixture is rewritten on purpose and
/// recorded.
fn regenerate() -> String {
    format!(
        "If what lablet emits was meant to change, regenerate the fixtures with \
         `{UPDATE}=1 cargo test -p lablet --test it golden`, read the diff, and add the \
         CHANGELOG entry the changelog gate asks for."
    )
}

#[tokio::test(start_paused = true)]
async fn the_starter_run_emits_what_its_fixture_holds() {
    let (_, exported) = STARTER.check().await;
    assert!(exported.spans_of("execute_tool").is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_two_turns_run_emits_what_its_fixture_holds() {
    let (_, exported) = TWO_TURNS.check().await;
    assert_eq!(exported.spans_of("chat").len(), 2);
    assert_eq!(exported.spans_of("execute_tool").len(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_cancelled_run_emits_what_its_fixture_holds() {
    let (finished, exported) = CANCELLED.check().await;
    assert_eq!(finished.summary.outcome.tool_calls, 3);
    let statuses: Vec<(&Value, Option<&Value>)> = exported
        .spans_of("execute_tool")
        .iter()
        .map(|tool| {
            (
                &tool.attributes[key::GEN_AI_TOOL_NAME],
                tool.attributes.get(key::ERROR_TYPE),
            )
        })
        .collect();
    assert_eq!(
        statuses,
        [
            (&json!("read_file"), None),
            (&json!("read_file"), None),
            (&json!("bash"), Some(&json!("cancelled"))),
        ]
    );
    assert_eq!(exported.spans_of("chat").len(), 2);
    assert_eq!(
        exported
            .records_of(GenAiClientOperationException::NAME)
            .len(),
        1
    );
}

/// The export in its normalised form, as the fixture holds it.
fn normalised(exported: &Exported) -> Value {
    let ungrouped = exported.ungrouped();
    let roots: Vec<&Span> = ungrouped
        .spans
        .iter()
        .filter(|span| span.parent_span_id.is_none())
        .collect();
    assert_eq!(roots.len(), 1, "one run has one root span");
    let origin = roots[0].start_unix_nano;
    // Nothing of a run is timed before its root span starts, so the
    // difference is checked rather than saturated.
    let offset = |nanos: u64| {
        nanos
            .checked_sub(origin)
            .unwrap_or_else(|| panic!("{nanos} is before the root span's start {origin}"))
    };

    // Concurrent calls and export batches put the signals in no fixed
    // order, so they're sorted by what they are before the ids are named.
    // A root sorts before a child that starts with it, so the root is
    // `span-1`.
    let mut spans: Vec<&Span> = ungrouped.spans.iter().collect();
    spans.sort_by_cached_key(|span| {
        (
            span.start_unix_nano,
            span.parent_span_id.is_some(),
            span.end_unix_nano,
            span.name.clone(),
            text(&span.attributes),
        )
    });
    let mut records: Vec<&LogRecord> = ungrouped.records.iter().collect();
    records.sort_by_cached_key(|record| {
        (
            record.time_unix_nano,
            record.event_name.clone(),
            text(&record.attributes),
        )
    });

    let mut names = Names::default();
    let spans: Vec<Value> = spans
        .into_iter()
        .map(|span| {
            let trace_id = names.trace(&span.trace_id);
            let span_id = names.span(&span.span_id);
            let parent_span_id = span
                .parent_span_id
                .as_deref()
                .map_or(Value::Null, |parent| names.span(parent));
            let events: Vec<Value> = span
                .events
                .iter()
                .map(|event| {
                    json!({
                        "name": event.name,
                        "time_ns": offset(event.time_unix_nano),
                        "attributes": event.attributes,
                    })
                })
                .collect();
            json!({
                "trace_id": trace_id,
                "span_id": span_id,
                "parent_span_id": parent_span_id,
                "name": span.name,
                "kind": kind(span.kind),
                "start_ns": offset(span.start_unix_nano),
                "end_ns": offset(span.end_unix_nano),
                "status": status(&span.status),
                "flags": span.flags,
                "scope": scope(&span.scope),
                "resource": resource(&span.resource),
                "attributes": span.attributes,
                "events": events,
            })
        })
        .collect();
    let records: Vec<Value> = records
        .into_iter()
        .map(|record| {
            let trace_id = names.trace(&record.trace_id);
            let span_id = names.span(&record.span_id);
            json!({
                "trace_id": trace_id,
                "span_id": span_id,
                "event_name": record.event_name,
                "severity_number": record.severity_number,
                "severity_text": record.severity_text,
                "time_ns": offset(record.time_unix_nano),
                "observed_time_ns": offset(record.observed_time_unix_nano),
                "flags": record.flags,
                "scope": scope(&record.scope),
                "resource": resource(&record.resource),
                "attributes": record.attributes,
                "body": record.body,
            })
        })
        .collect();
    json!({ "spans": spans, "records": records })
}

/// The attributes as one text, which orders two signals that share a start,
/// an end and a name, as two calls of one group do.
fn text(attributes: &Attributes) -> String {
    serde_json::to_string(attributes).unwrap()
}

/// The resource with the SDK's own version as [`NORMALISED`]: the SDK
/// describing itself isn't part of what lablet emits, and held as written
/// it would make a bump of the SDK a change to the contract.
fn resource(resource: &Attributes) -> Attributes {
    let mut resource = resource.clone();
    let version = resource
        .get_mut(key::TELEMETRY_SDK_VERSION)
        .unwrap_or_else(|| panic!("the resource has no {}", key::TELEMETRY_SDK_VERSION));
    assert!(
        version.as_str().is_some_and(|version| !version.is_empty()),
        "{} is {version}, not a version",
        key::TELEMETRY_SDK_VERSION
    );
    *version = Value::String(NORMALISED.to_owned());
    resource
}

/// The ids seen so far, each named by its order of first appearance.
#[derive(Default)]
struct Names {
    traces: Vec<String>,
    spans: Vec<String>,
}

impl Names {
    fn trace(&mut self, id: &str) -> Value {
        named(&mut self.traces, "trace", id)
    }

    fn span(&mut self, id: &str) -> Value {
        named(&mut self.spans, "span", id)
    }
}

/// `kind-N`, where `N` counts from 1 in the order ids were first seen; a
/// record that names no span has an empty id, which is `null`.
fn named(seen: &mut Vec<String>, kind: &str, id: &str) -> Value {
    if id.is_empty() {
        return Value::Null;
    }
    let index = seen
        .iter()
        .position(|known| known == id)
        .unwrap_or_else(|| {
            seen.push(id.to_owned());
            seen.len() - 1
        });
    Value::String(format!("{kind}-{}", index + 1))
}

fn kind(kind: SpanKind) -> &'static str {
    match kind {
        SpanKind::Unspecified => "unspecified",
        SpanKind::Internal => "internal",
        SpanKind::Server => "server",
        SpanKind::Client => "client",
        SpanKind::Producer => "producer",
        SpanKind::Consumer => "consumer",
    }
}

fn status(status: &Status) -> Value {
    match status {
        Status::Unset => json!({ "code": "unset" }),
        Status::Ok => json!({ "code": "ok" }),
        Status::Error(description) => json!({ "code": "error", "description": description }),
    }
}

fn scope(scope: &lablet_conformance::otlp::Scope) -> Value {
    json!({
        "name": scope.name,
        "version": scope.version,
        "schema_url": scope.schema_url,
    })
}

/// Every place `actual` differs from `expected`, each as the path to it and
/// the two values, so a failure says which signal and which field moved.
fn differences(at: &str, expected: &Value, actual: &Value, found: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let keys: BTreeSet<&String> = expected.keys().chain(actual.keys()).collect();
            for key in keys {
                let at = format!("{at}/{key}");
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => differences(&at, expected, actual, found),
                    (Some(expected), None) => found.push(format!(
                        "{at}: the fixture has {}, the run has nothing",
                        shown(expected)
                    )),
                    (None, Some(actual)) => found.push(format!(
                        "{at}: the run has {}, the fixture has nothing",
                        shown(actual)
                    )),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(expected), Value::Array(actual)) => {
            if expected.len() != actual.len() {
                found.push(format!(
                    "{at}: the fixture has {} items, the run has {}",
                    expected.len(),
                    actual.len()
                ));
            }
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                differences(&format!("{at}/{index}"), expected, actual, found);
            }
        }
        (expected, actual) => {
            if expected != actual {
                found.push(format!(
                    "{at}: the fixture has {}, the run has {}",
                    shown(expected),
                    shown(actual)
                ));
            }
        }
    }
}

/// A value cut to what a line of a failure can show.
fn shown(value: &Value) -> String {
    const MOST: usize = 200;
    let text = value.to_string();
    match text.char_indices().nth(MOST) {
        Some((cut, _)) => format!("{}... ({} bytes)", &text[..cut], text.len()),
        None => text,
    }
}
