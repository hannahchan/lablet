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
//! a network destination, and `TRACEPARENT`, `TRACESTATE` or `BAGGAGE`
//! would give every run a parent, so the test refuses to run with one set;
//! `cargo xtask test` strips them. `LABLET_UPDATE_GOLDEN=1 cargo test -p
//! lablet --test it golden` writes the fixtures instead of comparing. CI
//! never sets it, and the test refuses it there.
//!
//! A fourth run is the two-turns run again in a child process whose
//! environment sets every variable of the specification's SDK and exporter
//! pages, and the context variables, each to a value that would change
//! what's exported if it were read: those lablet honours to values whose
//! effect its fixture holds, those the config overrides to the opposite of
//! what it states, and those nothing reads to values that would show. It's
//! held to a fixture of its own, so the OpenTelemetry crates coming to read
//! a variable lablet doesn't decide shows as a difference, which the runs
//! above, under an environment with none of them, can't show.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::key;
use lablet::config::Provider;
use lablet::{CancelHandle, Config, FinishedRun, Format, RunId, RunRequest, StopReason};
use lablet_conformance::otlp::{Attributes, Exported, LogRecord, Span, SpanKind, Status};
use lablet_conformance::receiver::Receiver;
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

/// The two-turns run under [`hostile`], in a child process.
const HOSTILE: Golden = Golden {
    name: "hostile",
    ..TWO_TURNS
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
    /// exporter off, its telemetry written to `telemetry`, and whether it
    /// captures content stated.
    ///
    /// `model.script` and `tools.builtin.root` are the only relative paths a
    /// golden config may hold: the others a config takes are checked to be
    /// unset, since one that was set would start at the test's directory
    /// with nothing to say so.
    fn config(&self, telemetry: &Path) -> Config {
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
        // destination and the config's word on content the only one,
        // whatever the environment says.
        config.telemetry.otlp.enabled = Some(false);
        config.telemetry.file.path = Some(telemetry.to_path_buf());
        config.telemetry.capture_content = Some(config.telemetry.capture_content.unwrap_or(false));
        config
    }

    fn fixture(&self) -> PathBuf {
        fixtures().join(self.name).join("expected.json")
    }

    /// Runs once, under its own id, and reads back what the run exported.
    async fn run(&self) -> (FinishedRun, Exported) {
        let scratch = Scratch::new(&format!("golden-{}", self.name));
        let telemetry = scratch.at("telemetry.otlp.jsonl");
        let finished = self.run_to(&telemetry).await;
        (finished, Exported::read(&telemetry).unwrap())
    }

    /// Runs once, under its own id, exporting to `telemetry`.
    async fn run_to(&self, telemetry: &Path) -> FinishedRun {
        let mut lablet = lablet::build(self.config(telemetry)).await.unwrap();
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
        finished
    }

    /// Runs, holds what the run must come to, and compares the normalised
    /// export with the fixture, or writes the fixture when [`UPDATE`] is
    /// set.
    async fn check(&self) -> (FinishedRun, Exported) {
        refuse_the_environment();
        let (finished, exported) = self.run().await;
        assert_eq!(finished.summary.outcome.stop_reason(), self.stop);
        self.compare(&exported);
        (finished, exported)
    }

    /// Holds what the run exported to the counts it must come to, and
    /// compares its normalised form with the fixture, or writes the fixture
    /// when [`UPDATE`] is set.
    fn compare(&self, exported: &Exported) {
        assert_eq!(exported.spans.len(), self.spans, "{}: spans", self.name);
        assert_eq!(
            exported.records.len(),
            self.records,
            "{}: records",
            self.name
        );
        assert_eq!(exported.records_of("lablet.run").len(), 1, "{}", self.name);

        let normalised = normalised(exported);
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
    }
}

/// The variables that give every run a parent, read by the propagators.
const CONTEXT: [&str; 3] = ["TRACEPARENT", "TRACESTATE", "BAGGAGE"];

/// Refuses an environment the comparison can't be trusted in: an `OTEL_*`
/// variable, which lablet, the SDK and the exporter read, one of
/// [`CONTEXT`], and [`UPDATE`] in CI, where a regeneration would pass
/// whatever the run emitted. Names are read, never a value.
fn refuse_the_environment() {
    let set = refused(std::env::vars_os().map(|(name, _)| name.to_string_lossy().into_owned()));
    assert!(
        set.is_empty(),
        "{} is set, and an `OTEL_*` variable changes the resource or the exporter of every run, \
         and a context variable its parent; unset it, or run `cargo xtask test`, which strips \
         them",
        set.into_iter().collect::<Vec<_>>().join(", ")
    );
    if std::env::var_os(UPDATE).is_some() {
        assert!(
            std::env::var_os("CI").is_none(),
            "{UPDATE} is set in CI, and the fixtures are regenerated locally, never there"
        );
    }
}

/// Those of `names` the golden runs refuse.
fn refused(names: impl Iterator<Item = String>) -> BTreeSet<String> {
    names
        .filter(|name| name.starts_with("OTEL_") || CONTEXT.contains(&name.as_str()))
        .collect()
}

/// Set in the environment of the child process the refusal's test starts.
const REFUSAL_CHILD: &str = "LABLET_TEST_GOLDEN_REFUSAL_CHILD";

/// The child's side, which does nothing unless the test below started it.
#[test]
fn a_child_process_checks_its_environment_as_a_golden_run_does() {
    if std::env::var_os(REFUSAL_CHILD).is_some() {
        refuse_the_environment();
    }
}

/// The refusal is held in a child process, since what it reads is the
/// process's own environment.
#[test]
fn the_golden_runs_refuse_every_otel_variable_and_the_context_variables() {
    refuse_the_environment();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "golden::a_child_process_checks_its_environment_as_a_golden_run_does",
            "--test-threads=1",
        ])
        .env(REFUSAL_CHILD, "1")
        .envs(
            [
                "OTEL_SERVICE_NAME",
                "TRACEPARENT",
                "TRACESTATE",
                "BAGGAGE",
                "traceparent",
                "TRACEPARENTS",
                "NOT_OTEL_X",
            ]
            .map(|name| (name, "1")),
        )
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );

    assert!(!child.status.success(), "the child ran:\n{output}");
    assert!(
        output.contains("\nBAGGAGE, OTEL_SERVICE_NAME, TRACEPARENT, TRACESTATE is set,"),
        "{output}"
    );
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

/// Set in the environment of the child process the hostile run starts, to
/// the file the child exports its run to.
const HOSTILE_CHILD: &str = "LABLET_TEST_HOSTILE_GOLDEN_CHILD";

/// The trace and the span of the sampled parent the hostile environment's
/// `TRACEPARENT` names.
const INBOUND_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const INBOUND_SPAN: &str = "00f067aa0ba902b7";

/// The hostile environment: every variable of the specification's SDK and
/// OTLP exporter pages, and the context variables, each set to a value that
/// would change what the run exports if it were read. Two are left out,
/// since their honoured values would leave the fixture empty:
/// `OTEL_SDK_DISABLED`, which a canary and the command line's tests hold,
/// and `OTEL_TRACES_SAMPLER`, which the export module's tests hold.
fn hostile() -> Vec<(String, String)> {
    let refused = format!("http://{}", Receiver::closed());
    let missing = "/nonexistent/lablet-hostile-golden";
    let mut env: Vec<(String, String)> = [
        // Honoured, and the fixture, or the assertions after the
        // comparison, hold each one's effect: `BAGGAGE`'s is that nothing
        // emits it, and a batch of one is a line of its own.
        ("OTEL_SERVICE_NAME", "hostile-service"),
        (
            "OTEL_RESOURCE_ATTRIBUTES",
            "service.name=not-this,service.version=9.9.9,telemetry.sdk.name=not-this,\
             deployment.environment.name=hostile%2Cdecoded,team=a%20b",
        ),
        ("OTEL_PROPAGATORS", " TraceContext , baggage "),
        (
            "TRACEPARENT",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        ),
        ("TRACESTATE", "hostile=1"),
        ("BAGGAGE", "hostile=baggage"),
        ("OTEL_BSP_MAX_EXPORT_BATCH_SIZE", "1"),
        ("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE", "1"),
        // Honoured, with no effect this run can show: it has no span
        // events or links, its spans hold fewer than 32 attributes, the
        // three attribute limits leave none to `OTEL_ATTRIBUTE_COUNT_LIMIT`,
        // the flush exports whatever the delay, and a queue of 64 holds the
        // run. A sampler argument decides nothing under the default sampler.
        ("OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT", "32"),
        ("OTEL_SPAN_EVENT_COUNT_LIMIT", "0"),
        ("OTEL_SPAN_LINK_COUNT_LIMIT", "0"),
        ("OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT", "1"),
        ("OTEL_LINK_ATTRIBUTE_COUNT_LIMIT", "1"),
        ("OTEL_ATTRIBUTE_COUNT_LIMIT", "1"),
        ("OTEL_BSP_SCHEDULE_DELAY", "3600000"),
        ("OTEL_BSP_MAX_QUEUE_SIZE", "64"),
        ("OTEL_BLRP_SCHEDULE_DELAY", "3600000"),
        ("OTEL_BLRP_MAX_QUEUE_SIZE", "64"),
        ("OTEL_TRACES_SAMPLER_ARG", "0"),
        // Read, and overridden by the config: it states its file, no
        // network and no content.
        ("OTEL_TRACES_EXPORTER", "none"),
        ("OTEL_LOGS_EXPORTER", "otlp"),
        ("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT", "true"),
        // Read by nothing lablet uses, or inert in the crates.
        ("OTEL_METRICS_EXPORTER", "otlp"),
        ("OTEL_BSP_EXPORT_TIMEOUT", "1"),
        ("OTEL_BLRP_EXPORT_TIMEOUT", "1"),
        ("OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT", "1"),
        ("OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT", "1"),
        ("OTEL_LOGRECORD_ATTRIBUTE_VALUE_LENGTH_LIMIT", "1"),
        ("OTEL_LOGRECORD_ATTRIBUTE_COUNT_LIMIT", "1"),
        ("OTEL_LOG_LEVEL", "debug"),
        ("OTEL_CONFIG_FILE", missing),
        ("OTEL_EXPERIMENTAL_CONFIG_FILE", missing),
        ("OTEL_METRIC_EXPORT_INTERVAL", "1"),
        ("OTEL_METRIC_EXPORT_TIMEOUT", "1"),
        ("OTEL_METRICS_EXEMPLAR_FILTER", "always_on"),
        ("OTEL_EXPORTER_PROMETHEUS_HOST", "127.0.0.1"),
        ("OTEL_EXPORTER_PROMETHEUS_PORT", "4"),
        ("OTEL_EXPORTER_ZIPKIN_ENDPOINT", &refused),
        ("OTEL_EXPORTER_ZIPKIN_PROTOCOL", "http/json"),
        ("OTEL_EXPORTER_ZIPKIN_TIMEOUT", "1"),
        ("OTEL_EXPORTER_OTLP_SPAN_INSECURE", "true"),
        ("OTEL_EXPORTER_OTLP_METRIC_INSECURE", "true"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect();
    // Each OTLP exporter variable, generic and for each signal, metrics
    // included. The config's turning the network off overrides those lablet
    // reads.
    for signal in ["", "TRACES_", "LOGS_", "METRICS_"] {
        for (setting, value) in [
            ("ENDPOINT", refused.as_str()),
            ("PROTOCOL", "grpc"),
            ("HEADERS", "x-hostile=1"),
            ("TIMEOUT", "1"),
            ("COMPRESSION", "gzip"),
            ("INSECURE", "true"),
            ("CERTIFICATE", missing),
            ("CLIENT_KEY", missing),
            ("CLIENT_CERTIFICATE", missing),
        ] {
            env.push((
                format!("OTEL_EXPORTER_OTLP_{signal}{setting}"),
                value.to_owned(),
            ));
        }
    }
    env
}

/// The child's side, which does nothing unless the test below started it:
/// the two-turns run, exported to the file the parent named.
#[tokio::test(start_paused = true)]
async fn a_child_process_runs_the_two_turns_run_under_a_hostile_environment() {
    let Some(telemetry) = std::env::var_os(HOSTILE_CHILD) else {
        return;
    };
    let finished = HOSTILE.run_to(Path::new(&telemetry)).await;
    assert_eq!(finished.summary.outcome.stop_reason(), HOSTILE.stop);
}

#[test]
fn the_two_turns_run_under_a_hostile_environment_emits_what_its_fixture_holds() {
    refuse_the_environment();
    let scratch = Scratch::new("golden-hostile");
    let telemetry = scratch.at("telemetry.otlp.jsonl");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "golden::a_child_process_runs_the_two_turns_run_under_a_hostile_environment",
            "--test-threads=1",
        ])
        .env(HOSTILE_CHILD, &telemetry)
        .envs(hostile())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert!(
        child.status.success() && stdout.contains("1 passed"),
        "the child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&child.stderr)
    );

    let exported = Exported::read(&telemetry).unwrap();
    HOSTILE.compare(&exported);
    assert_eq!(
        exported.lines,
        exported.spans.len() + exported.records.len(),
        "a batch of one signal to a line"
    );
    for span in &exported.spans {
        assert_eq!(span.trace_id, INBOUND_TRACE, "{}", span.name);
    }
    for record in &exported.records {
        assert_eq!(record.trace_id, INBOUND_TRACE, "{}", record.event_name);
    }
    let root = &exported.spans_of(key::INVOKE_AGENT)[0];
    assert_eq!(root.parent_span_id.as_deref(), Some(INBOUND_SPAN));
}

/// The export in its normalised form, as the fixture holds it.
fn normalised(exported: &Exported) -> Value {
    let ungrouped = exported.ungrouped();
    let ids: BTreeSet<&str> = ungrouped.spans.iter().map(|span| &*span.span_id).collect();
    // A root's parent is no span of the run: none, or the inbound one.
    let is_root = |span: &Span| {
        span.parent_span_id
            .as_deref()
            .is_none_or(|id| !ids.contains(id))
    };
    let roots: Vec<&Span> = ungrouped
        .spans
        .iter()
        .filter(|span| is_root(span))
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
            !is_root(span),
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
            let normalised = json!({
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
            });
            with_trace_state(normalised, &span.trace_state)
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

/// `normalised` with `trace_state` beside its other keys, when the span has
/// one: only a run under an inbound parent does, so the runs without one
/// hold no key for it.
fn with_trace_state(mut normalised: Value, trace_state: &str) -> Value {
    if !trace_state.is_empty() {
        normalised["trace_state"] = json!(trace_state);
    }
    normalised
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
