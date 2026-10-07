//! `cargo xtask weaver live-check`: what lablet emits, checked against the
//! registry by `weaver registry live-check` over the wire. Weaver listens on
//! two free ports, lablet runs the `init` starter and the two-turn example
//! against it with content captured, weaver is stopped through its admin
//! endpoint, and its report is read back. A violation fails the step, and so
//! does a report that saw less than the runs emit: an idle listener reports
//! nothing wrong.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::{self, Write as _};
use std::io::{PipeWriter, Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::error::{Error, Verb};
use crate::gates::{CheckResult, Failure, LOCKED, weaver_diagnostic_args};
use crate::process::{self, Invocation};
use crate::report::Note;
use crate::workspace::{read_json, repo_root};

/// Paths are relative to the repository root, where weaver runs.
const REGISTRY: &str = "lablet/telemetry/registry";

/// Handed to weaver by path rather than found by it, so the same filters
/// apply wherever the step is started from.
const CONFIG: &str = "lablet/telemetry/.weaver.toml";

/// Where weaver writes its report, which git ignores under `**/target/`.
const OUTPUT: &str = "lablet/target/weaver-live-check";

/// The name weaver gives the report in its output directory.
const REPORT: &str = "live_check.json";

/// The example whose script calls a tool, so the tool span and its content
/// record are checked beside what the starter emits.
const TWO_TURNS: &str = "lablet/examples/two-turns";

/// Where `lablet init` writes the starter, under the run directory.
const STARTER: &str = "starter";

/// The wide event, one per run; the report counts the events it saw by name.
const WIDE_EVENT: &str = "lablet.run";

/// Weaver stops on its own after this long without a sample: a backstop for
/// a listener that outlives this process. The runs take seconds, and lablet
/// is built before weaver starts, so a cold build never eats into it.
const INACTIVITY_SECONDS: &str = "120";

const READY_WITHIN: Duration = Duration::from_secs(60);
const STOP_WITHIN: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(250);

/// For one request to the admin endpoint, which answers at once.
const ADMIN_TIMEOUT: Duration = Duration::from_secs(5);

/// What the check runs outside this process, so the tests run neither
/// weaver nor cargo.
trait Programs {
    /// Starts weaver with `args` in the repository root, both streams to
    /// `output`.
    fn spawn_weaver(&mut self, args: &[&str], output: PipeWriter) -> Result<Child, Error>;

    /// Runs cargo with `args` in `directory` to its end: a build, or lablet
    /// through `cargo run`.
    fn cargo(&mut self, directory: &Path, args: &[&str]) -> Result<Output, Error>;

    /// How long weaver may take to serve `/health`.
    fn ready_within(&self) -> Duration {
        READY_WITHIN
    }

    /// How long weaver may take to exit once asked to stop.
    fn stop_within(&self) -> Duration {
        STOP_WITHIN
    }
}

struct Real;

impl Programs for Real {
    fn spawn_weaver(&mut self, args: &[&str], output: PipeWriter) -> Result<Child, Error> {
        let invocation = process::invocation("weaver", args);
        let not_started = invocation.not_started();
        let mut command = process::command("weaver", args)?;
        command
            .stdin(Stdio::null())
            .stdout(output.try_clone().map_err(&not_started)?)
            .stderr(output);
        command.spawn().map_err(not_started)
    }

    fn cargo(&mut self, directory: &Path, args: &[&str]) -> Result<Output, Error> {
        let mut command = process::command_in(directory, "cargo", args)?;
        without_the_opentelemetry_environment(
            &mut command,
            std::env::vars_os().map(|(name, _)| name),
        );
        command
            .output()
            .map_err(Invocation::new(directory, "cargo", args).not_started())
    }
}

/// Runs the check against the working tree; see the module doc.
pub fn check() -> CheckResult {
    check_with(&repo_root(), &mut Real)
}

/// [`check`] for the repository at `root`, running programs through
/// `programs`.
fn check_with(root: &Path, programs: &mut dyn Programs) -> CheckResult {
    let output = root.join(OUTPUT);
    let report = output.join(REPORT);
    // A report from an earlier run must not stand in for one weaver never
    // wrote.
    if let Err(error) = std::fs::remove_file(&report)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(Error::file(Verb::Remove, &report)(error).into());
    }
    let scratch = Scratch::new(&output)?;
    build_lablet(root, programs)?;
    let mut listener = Listener::start(root, programs)?;
    let runs = run_configs(root, programs, &scratch, listener.ports.grpc)?;
    listener.stop(programs.stop_within())?;
    if !report.is_file() {
        return Err(Error::Missing {
            what: format!(
                "the report weaver was to write at {} is not there; weaver said:\n{}",
                report.display(),
                listener.log()
            ),
            remedy: "Run the step again, and read weaver's output above".to_owned(),
        }
        .into());
    }
    let parsed: Report = read_json(&report)?;
    let found = findings(&parsed).map_err(Error::parse(&report))?;
    verdict(&parsed, &found, runs, &report)
}

/// `cargo build` of the binary first, so each run starts at once and
/// weaver's inactivity timeout stays a backstop rather than a build budget.
fn build_lablet(root: &Path, programs: &mut dyn Programs) -> Result<(), Error> {
    let args = ["build", LOCKED, "--bin", "lablet"];
    let workspace = root.join("lablet");
    let output = programs.cargo(&workspace, &args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Failed {
            command: Invocation::new(&workspace, "cargo", &args),
            stderr: String::from_utf8_lossy(&output.stderr)
                .trim_end()
                .to_owned(),
        })
    }
}

/// The two fake-provider runs against the listener on `grpc`, both with
/// content captured: the starter `init` writes, and the two-turn example,
/// copied so that its run writes nothing into the tree. Returns how many ran.
fn run_configs(
    root: &Path,
    programs: &mut dyn Programs,
    scratch: &Scratch,
    grpc: u16,
) -> Result<usize, Failure> {
    let endpoint = format!("http://127.0.0.1:{grpc}");
    let manifest = root.join("lablet/Cargo.toml");
    let starter = scratch.dir.join(STARTER);
    let two_turns = scratch.dir.join("two-turns");
    copy_dir(&root.join(TWO_TURNS), &two_turns)?;
    let runs = [
        (
            scratch.dir.clone(),
            ["init", "--provider", "fake", STARTER]
                .map(str::to_owned)
                .to_vec(),
        ),
        (starter, run_args("Say hello.", &endpoint)),
        (
            two_turns,
            run_args("Read notes.md and say what it holds.", &endpoint),
        ),
    ];
    for (directory, args) in &runs {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        lablet(programs, &manifest, directory, &args)?;
    }
    Ok(runs.len() - 1)
}

/// The arguments of one run against `endpoint`, over gRPC, which is what
/// weaver listens for, with no file written since `telemetry.file.path`
/// stays null.
fn run_args(prompt: &str, endpoint: &str) -> Vec<String> {
    [
        "run",
        "--config",
        "lablet.yaml",
        "--prompt",
        prompt,
        "--quiet",
        "--set",
        &format!("telemetry.otlp.endpoint={endpoint}"),
        "--set",
        "telemetry.otlp.protocol=grpc",
        "--set",
        "telemetry.capture_content=true",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// One `lablet` through `cargo run`, in `directory`, which must exit 0: a
/// run that stopped short or a config that was refused is the step's
/// verdict, told with what lablet printed.
fn lablet(
    programs: &mut dyn Programs,
    manifest: &Path,
    directory: &Path,
    args: &[&str],
) -> Result<(), Failure> {
    let manifest = manifest.display().to_string();
    let cargo = [
        &[
            "run",
            LOCKED,
            "--manifest-path",
            &manifest,
            "--bin",
            "lablet",
            "--",
        ],
        args,
    ]
    .concat();
    let output = programs.cargo(directory, &cargo)?;
    if output.status.success() {
        return Ok(());
    }
    let mut verdict = String::new();
    for stream in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(stream);
        let text = text.trim_end();
        if !text.is_empty() {
            let _ = writeln!(verdict, "{text}");
        }
    }
    let _ = write!(
        verdict,
        "error: {} ({})",
        Invocation::new(directory, "cargo", &cargo).failed(),
        output.status
    );
    Err(Failure::Verdict(verdict))
}

/// Strips from `command`'s environment, `names` being the environment's
/// names, what's kept from the tests: an endpoint or an
/// `OTEL_TRACES_EXPORTER` there could redirect or silence the export, and
/// the step would judge a run the listener never saw, and a `TRACEPARENT`
/// that isn't sampled would leave it no span to judge.
fn without_the_opentelemetry_environment(
    command: &mut Command,
    names: impl IntoIterator<Item = OsString>,
) {
    process::stripped(command, names, process::KEPT_FROM_TESTS);
}

/// Copies the files under `from` to `to`, which is made.
fn copy_dir(from: &Path, to: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(to).map_err(Error::file(Verb::Create, to))?;
    for entry in std::fs::read_dir(from).map_err(Error::file(Verb::Read, from))? {
        let entry = entry.map_err(Error::file(Verb::Read, from))?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if source.is_dir() {
            copy_dir(&source, &target)?;
        } else {
            std::fs::copy(&source, &target).map_err(Error::file(Verb::Write, &target))?;
        }
    }
    Ok(())
}

/// The run directory under weaver's output directory, holding the starter
/// and the copy of the example. Removed on drop; the report beside it stays.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(output: &Path) -> Result<Self, Error> {
        let dir = output.join(format!("run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(Error::file(Verb::Create, &dir))?;
        Ok(Self { dir })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The two ports weaver listens on: OTLP over gRPC, and the admin endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ports {
    grpc: u16,
    admin: u16,
}

impl Ports {
    /// Two ports the system had free, each bound at 0 and read back, both
    /// held until both are known so that they differ. Weaver binds them
    /// next; a port taken in between fails its start, which is tried once
    /// more.
    fn free() -> Result<Self, Error> {
        let taken = |error: std::io::Error| Error::Missing {
            what: format!(
                "no free port could be bound on 127.0.0.1 ({error}), and weaver listens on two"
            ),
            remedy: "Check the loopback interface and the limit on open files".to_owned(),
        };
        let bind = || TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(&taken);
        let (grpc, admin) = (bind()?, bind()?);
        let port = |listener: &TcpListener| listener.local_addr().map(|address| address.port());
        Ok(Self {
            grpc: port(&grpc).map_err(&taken)?,
            admin: port(&admin).map_err(&taken)?,
        })
    }
}

/// Weaver listening on its ports, its output collected as it runs. Killed
/// when dropped, so no path leaves it behind.
struct Listener {
    child: Child,
    ports: Ports,
    command: Invocation,
    log: Arc<Mutex<Vec<u8>>>,
    collector: Option<JoinHandle<()>>,
}

impl Listener {
    /// Weaver started and serving. A start that exits before it serves, as
    /// one does when a port was taken between the pick and its bind, is tried
    /// once more on fresh ports.
    fn start(root: &Path, programs: &mut dyn Programs) -> Result<Self, Error> {
        match Self::started(root, programs) {
            Err(Error::Failed { .. }) => Self::started(root, programs),
            started => started,
        }
    }

    fn started(root: &Path, programs: &mut dyn Programs) -> Result<Self, Error> {
        let mut listener = Self::spawn(root, programs, Ports::free()?)?;
        listener.ready(programs.ready_within())?;
        Ok(listener)
    }

    fn spawn(root: &Path, programs: &mut dyn Programs, ports: Ports) -> Result<Self, Error> {
        let args = weaver_args(ports);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let command = Invocation::new(root, "weaver", &args);
        let (mut reader, writer) = std::io::pipe().map_err(command.not_started())?;
        let child = programs.spawn_weaver(&args, writer)?;
        let log = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&log);
        // Read as it comes: a weaver that wrote more than a pipe holds would
        // block on the write and never serve.
        let collector = std::thread::spawn(move || {
            let mut chunk = [0; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => collected
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend_from_slice(&chunk[..read]),
                }
            }
        });
        Ok(Self {
            child,
            ports,
            command,
            log,
            collector: Some(collector),
        })
    }

    /// Waits until weaver answers `/health` as ready. Weaver that exited
    /// first is told by its own output, and one that never answered as
    /// stalled, once `within` has passed and it has been stopped.
    fn ready(&mut self, within: Duration) -> Result<(), Error> {
        let started = Instant::now();
        loop {
            if let Ok(Some(_)) = self.child.try_wait() {
                return Err(self.exited());
            }
            if health(self.ports.admin) {
                return Ok(());
            }
            if started.elapsed() >= within {
                return Err(self.stalled("serve GET /health", within));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Stops weaver through its admin endpoint and waits for it to exit,
    /// which is when the report is whole. Its exit status says nothing here:
    /// weaver exits 1 on a violation, and the report is the verdict.
    fn stop(&mut self, within: Duration) -> Result<(), Error> {
        if let Err(error) = admin(self.ports.admin, "POST", "/stop") {
            return Err(match self.child.try_wait() {
                Ok(Some(_)) => self.exited(),
                _ => self.stalled(&format!("answer POST /stop ({error})"), ADMIN_TIMEOUT),
            });
        }
        if self.wait_until(within).is_none() {
            return Err(self.stalled("exit after POST /stop", within));
        }
        Ok(())
    }

    fn wait_until(&mut self, within: Duration) -> Option<ExitStatus> {
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            if started.elapsed() >= within {
                return None;
            }
            std::thread::sleep(POLL);
        }
    }

    /// Everything weaver wrote so far, both streams in one.
    fn log(&self) -> String {
        let log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&log).trim_end().to_owned()
    }

    /// Kills weaver and collects the rest of its output.
    fn stop_now(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(collector) = self.collector.take() {
            let _ = collector.join();
        }
    }

    fn exited(&mut self) -> Error {
        self.stop_now();
        Error::Failed {
            command: self.command.clone(),
            stderr: self.log(),
        }
    }

    fn stalled(&mut self, expected: &str, waited: Duration) -> Error {
        self.stop_now();
        Error::Stalled {
            command: self.command.clone(),
            expected: format!("{expected} within {}s", waited.as_secs()),
            output: self.log().into(),
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop_now();
    }
}

/// Never `--v2`: in that mode the index leaves out every attribute a
/// dependency defines, and every `gen_ai.*` sample reads as undeclared.
fn weaver_args(ports: Ports) -> Vec<String> {
    let mut args: Vec<String> = [
        "registry",
        "live-check",
        "--quiet",
        "--registry",
        REGISTRY,
        "--config",
        CONFIG,
        "--format",
        "json",
        "--output",
        OUTPUT,
        "--inactivity-timeout",
        INACTIVITY_SECONDS,
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend([
        "--otlp-grpc-port".to_owned(),
        ports.grpc.to_string(),
        "--admin-port".to_owned(),
        ports.admin.to_string(),
    ]);
    args.extend(weaver_diagnostic_args().map(str::to_owned));
    args
}

/// One request to weaver's admin endpoint on `port`, the connection closed
/// after it: the status code, and the body.
fn admin(port: u16, method: &str, path: &str) -> std::io::Result<(u16, String)> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, ADMIN_TIMEOUT)?;
    stream.set_read_timeout(Some(ADMIN_TIMEOUT))?;
    stream.set_write_timeout(Some(ADMIN_TIMEOUT))?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\
         Content-Length: 0\r\n\r\n"
    )?;
    let mut response = Vec::new();
    // Read to the close. A server that held the connection open past the
    // timeout has still answered.
    if let Err(error) = stream.read_to_end(&mut response)
        && response.is_empty()
    {
        return Err(error);
    }
    let response = String::from_utf8_lossy(&response);
    let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
    let code = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("no HTTP status line in: {head}"),
            )
        })?;
    Ok((code, body.to_owned()))
}

/// Whether weaver's admin endpoint on `port` says it is ready.
fn health(port: u16) -> bool {
    admin(port, "GET", "/health")
        .is_ok_and(|(code, body)| code == 200 && body.contains("\"ready\""))
}

/// Weaver's report, the parts the verdict reads.
#[derive(Debug, Deserialize)]
struct Report {
    samples: Vec<serde_json::Value>,
    statistics: Statistics,
}

#[derive(Debug, Deserialize)]
struct Statistics {
    /// Entities seen, by weaver's names for them: `span`, `log`, `resource`,
    /// `instrumentation_scope`, `attribute`.
    total_entities_by_type: BTreeMap<String, u64>,
    /// Findings by level: `violation`, `improvement`, `information`.
    advice_level_counts: BTreeMap<String, u64>,
    /// The share of the registry's attributes that were seen, 0 to 1.
    registry_coverage: f64,
    /// Log records matched to registry events, by event name.
    seen_registry_events: BTreeMap<String, u64>,
}

/// One piece of advice weaver gave on a sample or on an attribute of one.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Finding {
    id: String,
    level: String,
    message: String,
    signal_type: String,
    #[serde(default)]
    signal_name: Option<String>,
}

impl Finding {
    /// Violations first, then what weaver ranks below them.
    fn rank(&self) -> u8 {
        match self.level.as_str() {
            "violation" => 0,
            "improvement" => 1,
            "information" => 2,
            _ => 3,
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}", self.signal_type)?;
        if let Some(name) = &self.signal_name {
            write!(f, " {name}")?;
        }
        write!(f, "] {}/{}: {}", self.level, self.id, self.message)
    }
}

/// Every finding in the report, violations first and otherwise as found: a
/// sample's own, then those of its attributes, in order.
fn findings(report: &Report) -> Result<Vec<Finding>, serde_json::Error> {
    fn walk(value: &serde_json::Value, found: &mut Vec<Finding>) -> Result<(), serde_json::Error> {
        match value {
            serde_json::Value::Object(fields) => {
                if let Some(advice) = fields
                    .get("live_check_result")
                    .and_then(|result| result.get("all_advice"))
                    .and_then(serde_json::Value::as_array)
                {
                    for piece in advice {
                        found.push(Finding::deserialize(piece)?);
                    }
                }
                for (key, nested) in fields {
                    if key != "live_check_result" {
                        walk(nested, found)?;
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, found)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut found = Vec::new();
    for sample in &report.samples {
        walk(sample, &mut found)?;
    }
    found.sort_by_key(Finding::rank);
    Ok(found)
}

/// The step's judgement of a report for `runs` runs: a violation fails it,
/// with every finding listed; so does a report that saw less than the runs
/// emit, which is what an export that went elsewhere or nowhere leaves.
fn verdict(report: &Report, found: &[Finding], runs: usize, path: &Path) -> CheckResult {
    let statistics = &report.statistics;
    let count = |counts: &BTreeMap<String, u64>, key: &str| counts.get(key).copied().unwrap_or(0);
    let summary = summary(statistics);
    let violations = count(&statistics.advice_level_counts, "violation");
    if violations > 0 {
        let mut verdict =
            format!("weaver live-check found {violations} violation(s); every finding:\n");
        for finding in found {
            let _ = writeln!(verdict, "  {finding}");
        }
        let _ = write!(verdict, "{summary}\nthe report is at {}", path.display());
        return Err(Failure::Verdict(verdict));
    }
    let spans = count(&statistics.total_entities_by_type, "span");
    let wide = count(&statistics.seen_registry_events, WIDE_EVENT);
    let runs = u64::try_from(runs).unwrap_or(u64::MAX);
    // A run emits its root span, a chat span and its wide event at the least.
    let least = runs.saturating_mul(2);
    if wide < runs || spans < least {
        return Err(Failure::Verdict(format!(
            "the checker saw {spans} span(s) and {wide} `{WIDE_EVENT}` event(s), where {runs} \
             run(s) emit at least {least} span(s) and {runs} event(s): a run never reached it, or \
             exported less than it emits\n{summary}\nthe report is at {}",
            path.display()
        )));
    }
    Ok(Some(Note::Info(summary)))
}

/// What was seen and found, in one line: entities by type, the registry
/// coverage, and findings by level, with the violations counted even when
/// there are none.
fn summary(statistics: &Statistics) -> String {
    let entities = ordered(
        &statistics.total_entities_by_type,
        &[
            "span",
            "log",
            "resource",
            "instrumentation_scope",
            "attribute",
        ],
    );
    let mut levels = statistics.advice_level_counts.clone();
    levels.entry("violation".to_owned()).or_insert(0);
    let findings = ordered(&levels, &["violation", "improvement", "information"]);
    format!(
        "entities {entities}; registry coverage {:.0}%; findings {findings}",
        statistics.registry_coverage * 100.0
    )
}

/// `key count, ...`: the keys of `first` in that order where present, then
/// the rest as sorted.
fn ordered(counts: &BTreeMap<String, u64>, first: &[&str]) -> String {
    let named = first
        .iter()
        .filter_map(|key| counts.get(*key).map(|count| format!("{key} {count}")));
    let rest = counts
        .iter()
        .filter(|(key, _)| !first.contains(&key.as_str()))
        .map(|(key, count)| format!("{key} {count}"));
    let parts: Vec<String> = named.chain(rest).collect();
    if parts.is_empty() {
        "none".to_owned()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::chain;
    use crate::workspace::fixture::TempDir;
    use std::collections::BTreeSet;
    use std::os::unix::process::ExitStatusExt as _;

    /// What weaver wrote for two conforming runs: no violation, and the
    /// informations a fake-provider run always has.
    const PASSING: &str = include_str!("../tests/fixtures/live-check/passing.json");

    /// What weaver wrote for a span and a wide event with undeclared enum
    /// members and required attributes missing.
    const VIOLATIONS: &str = include_str!("../tests/fixtures/live-check/violations.json");

    const REPORT_PATH: &str = "lablet/target/weaver-live-check/live_check.json";

    fn report(text: &str) -> Report {
        serde_json::from_str(text).unwrap()
    }

    fn judged(text: &str, runs: usize) -> CheckResult {
        let parsed = report(text);
        let found = findings(&parsed).unwrap();
        verdict(&parsed, &found, runs, Path::new(REPORT_PATH))
    }

    #[test]
    fn weaver_listens_on_the_given_ports_without_v2_under_the_live_check_config() {
        let args = weaver_args(Ports {
            grpc: 40001,
            admin: 40002,
        });
        assert_eq!(args[..3], ["registry", "live-check", "--quiet"]);
        assert!(!args.iter().any(|arg| arg.starts_with("--v2")), "{args:?}");
        let after = |flag: &str| {
            args.iter()
                .position(|arg| arg == flag)
                .map(|at| args[at + 1].as_str())
        };
        assert_eq!(after("--registry"), Some(REGISTRY));
        assert_eq!(after("--config"), Some(CONFIG));
        assert_eq!(after("--format"), Some("json"));
        assert_eq!(after("--output"), Some(OUTPUT));
        assert_eq!(after("--otlp-grpc-port"), Some("40001"));
        assert_eq!(after("--admin-port"), Some("40002"));
        assert_eq!(after("--inactivity-timeout"), Some(INACTIVITY_SECONDS));
        assert_eq!(args[args.len() - 4..], weaver_diagnostic_args());

        // Local paths, never git URLs weaver would clone on every run.
        let root = repo_root();
        assert!(root.join(REGISTRY).join("manifest.yaml").is_file());
        assert!(root.join(CONFIG).is_file());
        assert!(root.join(TWO_TURNS).join("lablet.yaml").is_file());

        // The config must not switch weaver to v2 either, and it must fail
        // on violations, which is what the verdict counts.
        let config = std::fs::read_to_string(root.join(CONFIG)).unwrap();
        let config: toml::Value = toml::from_str(&config).unwrap();
        assert!(
            config.get("registry").is_none(),
            "a `[registry]` table in {CONFIG} would apply to live-check"
        );
        assert_eq!(config["live-check"]["fail_on"].as_str(), Some("violation"));
    }

    /// Whether a released port can be bound again is the system's to
    /// answer, and another test may take it in between, so only what the
    /// pair promises is checked: two ports, each one the system gave.
    #[test]
    fn a_free_port_pair_is_two_ports_that_differ() {
        let ports = Ports::free().unwrap();
        assert_ne!(ports.grpc, ports.admin);
        assert!(ports.grpc != 0 && ports.admin != 0);
    }

    /// Reads one request's head from `stream`.
    fn request_head(stream: &mut TcpStream) -> String {
        let mut head = Vec::new();
        let mut byte = [0];
        while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
            head.push(byte[0]);
        }
        String::from_utf8(head).unwrap()
    }

    fn respond(stream: &mut TcpStream, code: u16, body: &str) {
        let reason = if code == 200 { "OK" } else { "Not Ready" };
        write!(
            stream,
            "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    }

    /// A listener on a free port that answers `answers.len()` requests in
    /// order, then returns their heads.
    fn stub(answers: Vec<(u16, &'static str)>) -> (u16, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut heads = Vec::new();
            for (code, body) in answers {
                let (mut stream, _) = listener.accept().unwrap();
                heads.push(request_head(&mut stream));
                respond(&mut stream, code, body);
            }
            heads
        });
        (port, server)
    }

    #[test]
    fn the_admin_client_asks_over_http_and_reads_the_status_and_the_body() {
        // The last answer says ready in its body under a status that is not
        // 200, since the status is read and not only the body.
        let ready = "{\"status\":\"ready\"}";
        let (port, server) = stub(vec![(200, ready), (200, ""), (503, ready)]);
        assert_eq!(
            admin(port, "GET", "/health").unwrap(),
            (200, "{\"status\":\"ready\"}".to_owned())
        );
        assert_eq!(admin(port, "POST", "/stop").unwrap(), (200, String::new()));
        assert!(!health(port), "503 read as ready");
        let heads = server.join().unwrap();
        assert!(
            heads[0].starts_with("GET /health HTTP/1.1\r\n"),
            "{}",
            heads[0]
        );
        assert!(
            heads[1].starts_with("POST /stop HTTP/1.1\r\n"),
            "{}",
            heads[1]
        );
        for head in &heads {
            assert!(head.contains("\r\nConnection: close\r\n"), "{head}");
            assert!(
                head.contains(&format!("\r\nHost: 127.0.0.1:{port}\r\n")),
                "{head}"
            );
        }

        let (port, server) = stub(vec![(200, "{\"status\":\"ready\"}")]);
        assert!(health(port));
        server.join().unwrap();
        // Nothing listens there any more.
        admin(port, "GET", "/health").unwrap_err();
        assert!(!health(port));
    }

    #[test]
    fn a_report_with_violations_fails_listing_every_finding_violations_first() {
        let verdict = judged(VIOLATIONS, 1).unwrap_err().into_verdict();
        assert!(
            verdict.starts_with(
                "weaver live-check found 40 violation(s); every finding:\n  [span chat scripted] \
                 violation/undefined_enum_variant: Enum attribute 'lablet.chat.purpose' has value \
                 'not_a_purpose' which is not documented.\n"
            ),
            "{verdict}"
        );
        let lines: Vec<&str> = verdict
            .lines()
            .filter(|line| line.starts_with("  ["))
            .collect();
        assert_eq!(lines.len(), 74, "{verdict}");
        let last_violation = lines
            .iter()
            .rposition(|line| line.contains("] violation/"))
            .unwrap();
        let first_information = lines
            .iter()
            .position(|line| line.contains("] information/"))
            .unwrap();
        assert!(last_violation < first_information, "{verdict}");
        assert!(
            verdict.contains(
                "\n  [log lablet.run] violation/required_attribute_not_present: Required attribute \
                 'gen_ai.agent.name' is not present.\n"
            ),
            "{verdict}"
        );
        assert!(
            verdict.ends_with(
                "entities span 1, log 1, attribute 7; registry coverage 6%; findings violation 40, \
                 information 34\nthe report is at lablet/target/weaver-live-check/live_check.json"
            ),
            "{verdict}"
        );
    }

    #[test]
    fn a_report_with_no_violation_passes_with_what_it_saw_as_the_note() {
        assert_eq!(
            judged(PASSING, 2).unwrap(),
            Some(Note::Info(
                "entities span 4, log 2, attribute 136; registry coverage 41%; findings violation \
                 0, information 68"
                    .to_owned()
            ))
        );

        // `gen_ai.provider.name: fake` grades as information under the
        // config as written, on a span and on the wide event alike, so no
        // filter is needed for the one provider the check runs.
        let fake: BTreeSet<(String, String)> = findings(&report(PASSING))
            .unwrap()
            .into_iter()
            .filter(|finding| {
                finding
                    .message
                    .contains("'gen_ai.provider.name' has value 'fake'")
            })
            .map(|finding| (finding.level, finding.signal_type))
            .collect();
        let information = |signal: &str| ("information".to_owned(), signal.to_owned());
        assert_eq!(fake, [information("span"), information("log")].into());
    }

    #[test]
    fn a_report_that_saw_less_than_the_runs_emit_fails() {
        let verdict = judged(PASSING, 3).unwrap_err().into_verdict();
        assert!(
            verdict.starts_with(
                "the checker saw 4 span(s) and 2 `lablet.run` event(s), where 3 run(s) emit at \
                 least 6 span(s) and 3 event(s): a run never reached it, or exported less than it \
                 emits\nentities span 4, log 2"
            ),
            "{verdict}"
        );
        assert!(
            verdict.ends_with("\nthe report is at lablet/target/weaver-live-check/live_check.json")
        );

        // What an idle listener writes at its inactivity timeout.
        let idle = r#"{"samples": [], "statistics": {"total_entities": 0, "total_entities_by_type": {},
            "advice_level_counts": {}, "registry_coverage": 0.0, "seen_registry_events": {}}}"#;
        let verdict = judged(idle, 1).unwrap_err().into_verdict();
        assert!(
            verdict.starts_with(
                "the checker saw 0 span(s) and 0 `lablet.run` event(s), where 1 run(s)"
            ),
            "{verdict}"
        );
        assert!(
            verdict.contains("\nentities none; registry coverage 0%; findings violation 0\n"),
            "{verdict}"
        );
    }

    #[test]
    fn only_a_violation_fails_a_report_the_runs_reached() {
        let advised = r#"{"samples": [], "statistics": {"total_entities_by_type": {"span": 2, "log": 1},
            "advice_level_counts": {"improvement": 2, "information": 5}, "registry_coverage": 0.5,
            "seen_registry_events": {"lablet.run": 1}}}"#;
        assert_eq!(
            judged(advised, 1).unwrap(),
            Some(Note::Info(
                "entities span 2, log 1; registry coverage 50%; findings violation 0, improvement \
                 2, information 5"
                    .to_owned()
            ))
        );
    }

    #[test]
    fn lablet_runs_without_the_opentelemetry_environment() {
        let mut command = Command::new("lablet");
        let names = [
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_TRACES_EXPORTER",
            "TRACEPARENT",
            "TRACESTATE",
            "BAGGAGE",
            "OTEL",
            "HOME",
            "NOT_OTEL_X",
        ]
        .map(OsString::from);
        without_the_opentelemetry_environment(&mut command, names);
        let removed: BTreeSet<String> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            removed,
            [
                "BAGGAGE",
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                "OTEL_TRACES_EXPORTER",
                "TRACEPARENT",
                "TRACESTATE"
            ]
            .map(String::from)
            .into()
        );
        assert_eq!(command.get_envs().count(), 5);
    }

    const FAKE_WEAVER: &str = "XTASK_TEST_FAKE_WEAVER";
    const FAKE_WEAVER_PORT: &str = "XTASK_TEST_FAKE_WEAVER_PORT";
    const FAKE_WEAVER_REPORT: &str = "XTASK_TEST_FAKE_WEAVER_REPORT";
    const FAKE_WEAVER_FIXTURE: &str = "XTASK_TEST_FAKE_WEAVER_FIXTURE";
    const FAKE_WEAVER_TEST: &str =
        "live_check::tests::a_fake_weaver_serves_health_and_writes_the_report_on_stop";

    /// Stands in for weaver when a test runs the check: this test binary,
    /// started on this test with `FAKE_WEAVER` naming how to behave. Run as
    /// a test, with nothing set, it does nothing.
    #[test]
    fn a_fake_weaver_serves_health_and_writes_the_report_on_stop() {
        let Some(mode) = std::env::var(FAKE_WEAVER).ok() else {
            return;
        };
        let port: u16 = std::env::var(FAKE_WEAVER_PORT).unwrap().parse().unwrap();
        match mode.as_str() {
            "exits" => {
                eprintln!("could not bind the admin port");
                std::process::exit(3);
            }
            "hangs" => std::thread::sleep(Duration::from_secs(120)),
            serves @ ("serves" | "never-stops") => {
                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
                println!("fake weaver listening on {port}");
                loop {
                    let (mut stream, _) = listener.accept().unwrap();
                    let head = request_head(&mut stream);
                    if head.starts_with("GET /health ") {
                        respond(&mut stream, 200, "{\"status\":\"ready\"}");
                    } else if head.starts_with("POST /stop ") {
                        respond(&mut stream, 200, "");
                        if serves == "serves" {
                            std::fs::copy(
                                std::env::var(FAKE_WEAVER_FIXTURE).unwrap(),
                                std::env::var(FAKE_WEAVER_REPORT).unwrap(),
                            )
                            .unwrap();
                            std::process::exit(0);
                        }
                    } else {
                        respond(&mut stream, 404, "");
                    }
                }
            }
            other => panic!("{other}"),
        }
    }

    /// The programs as a test runs them: weaver is the fake above, and cargo
    /// records where it ran and whether a config was there, writes the
    /// starter for `init`, and exits as told for `run`.
    struct Fake {
        root: PathBuf,
        weaver: &'static str,
        fixture: PathBuf,
        run_exits: i32,
        spawned: Vec<u32>,
        grpc: Option<String>,
        ran: Vec<(PathBuf, Vec<String>, bool)>,
    }

    impl Fake {
        fn new(root: &TempDir, weaver: &'static str, run_exits: i32) -> Self {
            root.write(
                "lablet/examples/two-turns/lablet.yaml",
                "model: { provider: fake }\n",
            );
            root.write("lablet/examples/two-turns/lablet-script.yaml", "[]\n");
            root.write("lablet/examples/two-turns/work/notes.md", "notes\n");
            root.write("passing.json", PASSING);
            Self {
                root: root.path().to_path_buf(),
                weaver,
                fixture: root.path().join("passing.json"),
                run_exits,
                spawned: Vec::new(),
                grpc: None,
                ran: Vec::new(),
            }
        }
    }

    impl Programs for Fake {
        fn spawn_weaver(&mut self, args: &[&str], output: PipeWriter) -> Result<Child, Error> {
            let after = |flag: &str| args[args.iter().position(|arg| *arg == flag).unwrap() + 1];
            self.grpc = Some(after("--otlp-grpc-port").to_owned());
            let report = self.root.join(after("--output")).join(REPORT);
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    FAKE_WEAVER_TEST,
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(FAKE_WEAVER, self.weaver)
                .env(FAKE_WEAVER_PORT, after("--admin-port"))
                .env(FAKE_WEAVER_REPORT, &report)
                .env(FAKE_WEAVER_FIXTURE, &self.fixture)
                .stdin(Stdio::null())
                .stdout(output.try_clone().unwrap())
                .stderr(output);
            let child = command.spawn().unwrap();
            self.spawned.push(child.id());
            Ok(child)
        }

        fn cargo(&mut self, directory: &Path, args: &[&str]) -> Result<Output, Error> {
            let config_there = directory.join("lablet.yaml").is_file();
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            let subcommand = args.get(7).map(String::as_str);
            if subcommand == Some("init") {
                let starter = directory.join(args.last().unwrap());
                std::fs::create_dir_all(&starter).unwrap();
                std::fs::write(starter.join("lablet.yaml"), "model: { provider: fake }\n").unwrap();
            }
            let code = if subcommand == Some("run") {
                self.run_exits
            } else {
                0
            };
            self.ran.push((directory.to_path_buf(), args, config_there));
            Ok(Output {
                status: ExitStatus::from_raw(code << 8),
                stdout: b"{\"stop_reason\":\"max_turns\"}\n".to_vec(),
                stderr: b"lablet: the run stopped\n".to_vec(),
            })
        }

        fn ready_within(&self) -> Duration {
            Duration::from_secs(3)
        }

        fn stop_within(&self) -> Duration {
            Duration::from_secs(2)
        }
    }

    fn alive(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .unwrap()
            .status
            .success()
    }

    #[test]
    fn the_check_runs_the_starter_and_the_example_against_weaver_and_judges_its_report() {
        let root = TempDir::new("live-check");
        let mut fake = Fake::new(&root, "serves", 0);

        let result = check_with(root.path(), &mut fake);

        assert_eq!(
            result.unwrap(),
            Some(Note::Info(
                "entities span 4, log 2, attribute 136; registry coverage 41%; findings violation \
                 0, information 68"
                    .to_owned()
            ))
        );
        // The report is kept where a person finds it; the run directory is
        // gone, and so is weaver.
        let output = root.path().join(OUTPUT);
        let scratch = output.join(format!("run-{}", std::process::id()));
        assert!(output.join(REPORT).is_file());
        assert!(!scratch.exists());
        assert_eq!(fake.spawned.len(), 1);
        assert!(!alive(fake.spawned[0]));

        // What ran, where, and against which port.
        let manifest = root.path().join("lablet/Cargo.toml").display().to_string();
        let cargo_run = [
            "run",
            "--locked",
            "--manifest-path",
            &manifest,
            "--bin",
            "lablet",
            "--",
        ];
        let endpoint = format!(
            "telemetry.otlp.endpoint=http://127.0.0.1:{}",
            fake.grpc.unwrap()
        );
        let run = |prompt: &str| -> Vec<String> {
            let lablet = [
                "run",
                "--config",
                "lablet.yaml",
                "--prompt",
                prompt,
                "--quiet",
                "--set",
                &endpoint,
                "--set",
                "telemetry.otlp.protocol=grpc",
                "--set",
                "telemetry.capture_content=true",
            ];
            [&cargo_run[..], &lablet[..]]
                .concat()
                .into_iter()
                .map(str::to_owned)
                .collect()
        };
        let init = [&cargo_run[..], &["init", "--provider", "fake", "starter"]]
            .concat()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            fake.ran,
            [
                (
                    root.path().join("lablet"),
                    ["build", "--locked", "--bin", "lablet"]
                        .map(str::to_owned)
                        .to_vec(),
                    false
                ),
                (scratch.clone(), init, false),
                (scratch.join("starter"), run("Say hello."), true),
                (
                    scratch.join("two-turns"),
                    run("Read notes.md and say what it holds."),
                    true
                ),
            ]
        );
    }

    #[test]
    fn weaver_that_exits_before_it_serves_is_started_once_more_then_told_by_its_output() {
        let root = TempDir::new("live-check-exits");
        let mut fake = Fake::new(&root, "exits", 0);

        let error = check_with(root.path(), &mut fake).unwrap_err().into_error();

        let Error::Failed { command, stderr } = &error else {
            panic!("{error:?}");
        };
        assert!(
            command
                .to_string()
                .starts_with("weaver registry live-check "),
            "{command}"
        );
        assert!(stderr.contains("could not bind the admin port"), "{stderr}");
        assert_eq!(
            fake.spawned.len(),
            2,
            "a start that exited is tried once more"
        );
        assert_eq!(
            fake.ran.len(),
            1,
            "nothing ran against a listener that never served"
        );
        assert!(
            !root
                .path()
                .join(OUTPUT)
                .join(format!("run-{}", std::process::id()))
                .exists()
        );
    }

    #[test]
    fn weaver_that_never_serves_is_killed_and_told_as_stalled() {
        let root = TempDir::new("live-check-hangs");
        let mut fake = Fake::new(&root, "hangs", 0);

        let error = check_with(root.path(), &mut fake).unwrap_err().into_error();

        let Error::Stalled { expected, .. } = &error else {
            panic!("{error:?}");
        };
        assert_eq!(expected, "serve GET /health within 3s");
        let told = chain(&error);
        assert!(
            told.contains(" did not serve GET /health within 3s"),
            "{told}"
        );
        assert_eq!(fake.spawned.len(), 1);
        assert!(!alive(fake.spawned[0]), "weaver was left running");
    }

    #[test]
    fn weaver_that_does_not_stop_when_asked_is_killed_and_told_as_stalled() {
        let root = TempDir::new("live-check-never-stops");
        let mut fake = Fake::new(&root, "never-stops", 0);

        let error = check_with(root.path(), &mut fake).unwrap_err().into_error();

        let Error::Stalled {
            expected, output, ..
        } = &error
        else {
            panic!("{error:?}");
        };
        assert_eq!(expected, "exit after POST /stop within 2s");
        assert!(output.contains("fake weaver listening on "), "{output}");
        assert_eq!(fake.ran.len(), 4, "every run was made before the stop");
        assert!(!alive(fake.spawned[0]), "weaver was left running");
    }

    #[test]
    fn a_run_that_does_not_complete_is_the_verdict_with_what_lablet_printed() {
        let root = TempDir::new("live-check-run-fails");
        let mut fake = Fake::new(&root, "serves", 2);

        let verdict = check_with(root.path(), &mut fake)
            .unwrap_err()
            .into_verdict();

        let starter = root
            .path()
            .join(OUTPUT)
            .join(format!("run-{}", std::process::id()))
            .join("starter");
        assert!(
            verdict.starts_with("{\"stop_reason\":\"max_turns\"}\nlablet: the run stopped\nerror: command failed (in "),
            "{verdict}"
        );
        assert!(
            verdict.contains(&format!(
                "(in {}): cargo run --locked --manifest-path ",
                starter.display()
            )),
            "{verdict}"
        );
        assert!(verdict.ends_with(" (exit status: 2)"), "{verdict}");
        assert_eq!(
            fake.ran.len(),
            3,
            "the second run never starts after a failed first"
        );
        assert!(!alive(fake.spawned[0]), "weaver was left running");
        assert!(!starter.exists(), "the run directory was left behind");
    }
}
