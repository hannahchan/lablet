//! The capture replayed through lablet, by hand, at the capture (P8, the
//! first step of the comparison): each recorded response of one reference
//! conversation served in order from wiremock, lablet run against it with
//! the three built-ins on an empty scratch root, and what lablet sent
//! written out beside the capture for `skeleton.py` to reduce.
//!
//! Ignored, since it needs a capture, which stays out of the repository.
//! Run it with the capture's directory in `LABLET_REPLAY_CAPTURE`, the
//! directory that holds one conversation's `*.response.json` files and its
//! `index.jsonl`:
//!
//! ```bash
//! LABLET_REPLAY_CAPTURE=/private/tmp/lab-capture/bodies/R2/claude-opus-5-5 \
//!   cargo test --locked -p lablet-provider-anthropic --test it -- --ignored replay
//! ```
//!
//! It reads the model from the first recorded request, so the replayed
//! config names what the reference named, takes the system prompt and the
//! task from `product/research/parity/capture/{prompt,task}.md`, as the
//! reference did, builds the `lablet` binary with cargo and runs it on a
//! config of lablet's defaults, and writes each request wiremock received
//! as `<capture>/lablet-replay/<n>.request.json`. The run's outcome is
//! printed; whether it completed is the capture's business, not this
//! test's, since a recorded conversation may end as lablet's wouldn't.
//!
//! The reference reaches the stand-in's tools under the SDK's prefixed names,
//! `mcp__lab__bash`, where lablet offers the bare `bash`; each response is
//! served with that prefix taken out of every name, and out of any text that
//! happens to carry it, so the recorded `tool_use` blocks run lablet's tools
//! rather than being refused as unknown.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use lablet_test_support::Scratch;
use serde_json::Value;
use wiremock::{MockServer, ResponseTemplate};

use crate::harness;

/// The variable that names the capture's directory.
const CAPTURE: &str = "LABLET_REPLAY_CAPTURE";

/// The variable the replayed config reads its key from, set here to a value
/// of no key's shape, since wiremock checks no signature.
const KEY_VARIABLE: &str = "LABLET_REPLAY_KEY";

/// The variables stripped from the binary's environment, so nothing of the
/// shell's redirects or silences the run, or gives it a parent; and the key's
/// variable, since the owner runs the replay in the shell that holds her key,
/// and the replayed config reads `KEY_VARIABLE` instead.
const STRIPPED: &[&str] = &[
    "TRACEPARENT",
    "TRACESTATE",
    "BAGGAGE",
    "B3",
    "ANTHROPIC_API_KEY",
];
const STRIPPED_PREFIXES: &[&str] = &["OTEL_", "X_B3_"];

/// The prefix the reference's SDK puts on the stand-in server's tool names.
const REFERENCE_PREFIX: &str = "mcp__lab__";

/// The capture's files, read from `index.jsonl` when it names them, else
/// by modification time.
struct Recorded {
    first_request: Value,
    responses: Vec<Vec<u8>>,
}

fn capture_directory() -> PathBuf {
    #[expect(
        clippy::disallowed_methods,
        reason = "test code reads its environment; the variable names the capture, which stays out of the tree"
    )]
    let path = std::env::var_os(CAPTURE)
        .unwrap_or_else(|| panic!("set {CAPTURE} to the capture's directory to run the replay"));
    PathBuf::from(path)
}

/// The files under `directory` whose name ends in `suffix`, oldest first.
fn by_age(directory: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("reading {}: {error}", directory.display()))
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(suffix))
        .map(|entry| (entry.metadata().unwrap().modified().unwrap(), entry.path()))
        .collect();
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

/// The response files `index.jsonl` names, in its order: a line's value that
/// ends in `.response.json`, or its `request_id` with that suffix added.
fn by_index(directory: &Path) -> Vec<PathBuf> {
    let Ok(index) = fs::read_to_string(directory.join("index.jsonl")) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for line in index.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(Value::Object(entry)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let named = entry
            .values()
            .filter_map(Value::as_str)
            .find(|value| value.ends_with(".response.json"))
            .map(|value| directory.join(Path::new(value).file_name().unwrap()))
            .or_else(|| {
                entry
                    .get("request_id")
                    .and_then(Value::as_str)
                    .map(|id| directory.join(format!("{id}.response.json")))
            });
        if let Some(path) = named
            && path.is_file()
            && !files.contains(&path)
        {
            files.push(path);
        }
    }
    files
}

fn recorded(directory: &Path) -> Recorded {
    let mut responses = by_index(directory);
    if responses.is_empty() {
        responses = by_age(directory, ".response.json");
    }
    assert!(
        !responses.is_empty(),
        "no *.response.json under {}",
        directory.display()
    );
    let requests = by_age(directory, ".request.json");
    let first = requests
        .first()
        .unwrap_or_else(|| panic!("no *.request.json under {}", directory.display()));
    Recorded {
        first_request: serde_json::from_slice(&fs::read(first).unwrap()).unwrap(),
        responses: responses
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect(),
    }
}

/// The workspace manifest, from this crate's directory.
fn workspace_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../Cargo.toml")
        .canonicalize()
        .unwrap()
}

/// `product/research/parity/capture/`, from this crate's directory.
fn capture_tooling() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../product/research/parity/capture")
        .canonicalize()
        .unwrap()
}

/// The names in the test's environment that the binary doesn't get.
fn stripped_names() -> Vec<OsString> {
    #[expect(
        clippy::disallowed_methods,
        reason = "test code reads its environment, to strip the variables the binary mustn't inherit"
    )]
    let names = std::env::vars_os().map(|(name, _)| name);
    names
        .filter(|name| {
            let name = name.to_string_lossy();
            STRIPPED.contains(&name.as_ref())
                || STRIPPED_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs a capture in LABLET_REPLAY_CAPTURE; see the module doc"]
#[expect(
    clippy::print_stdout,
    reason = "a replay run by hand says how lablet's run ended and where its requests were written"
)]
async fn the_recorded_conversation_replayed_through_lablet_is_written_beside_the_capture() {
    let directory = capture_directory();
    let Recorded {
        first_request,
        responses,
    } = recorded(&directory);
    let model = first_request["model"]
        .as_str()
        .expect("the first recorded request names its model")
        .to_owned();

    let server = MockServer::start().await;
    harness::answer_in_turn(
        &server,
        responses
            .into_iter()
            .map(|body| {
                String::from_utf8(body)
                    .expect("a recorded response is UTF-8")
                    .replace(REFERENCE_PREFIX, "")
                    .into_bytes()
            })
            // Raw, so a `tool_use` input's keys arrive in the model's order.
            .map(|body| ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .collect(),
    )
    .await;

    let tooling = capture_tooling();
    let scratch = Scratch::new("replay");
    let root = scratch.path().join("work");
    fs::create_dir_all(&root).unwrap();
    let config = scratch.path().join("replay.yaml");
    fs::write(
        &config,
        format!(
            "model:\n  provider: anthropic\n  name: {model}\n  api_key: ${{{KEY_VARIABLE}}}\n  \
             base_url: {uri}\nprompt:\n  system_file: {prompt}\ntools:\n  builtin:\n    root: \
             work\n    enabled: [bash, read_file, write_file]\n",
            uri = server.uri(),
            prompt = tooling.join("prompt.md").display(),
        ),
    )
    .unwrap();

    let mut command = Command::new("cargo");
    command
        .args(["run", "--locked", "--quiet", "--manifest-path"])
        .arg(workspace_manifest())
        .args(["--bin", "lablet", "--", "run", "--quiet", "--config"])
        .arg(&config)
        .arg("--prompt-file")
        .arg(tooling.join("task.md"))
        .env(KEY_VARIABLE, "replay-key-of-no-key-s-shape-0123456789");
    for name in stripped_names() {
        command.env_remove(name);
    }
    // Blocking, off the runtime's thread, so wiremock keeps answering.
    let output = tokio::task::spawn_blocking(move || command.output())
        .await
        .unwrap()
        .expect("cargo runs");
    println!(
        "lablet exited with {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let out = directory.join("lablet-replay");
    fs::create_dir_all(&out).unwrap();
    let received = harness::received(&server).await;
    for (index, request) in received.iter().enumerate() {
        fs::write(out.join(format!("{index:03}.request.json")), &request.body).unwrap();
    }
    println!(
        "wrote {} request(s) lablet sent under {}",
        received.len(),
        out.display()
    );
    assert!(
        !received.is_empty(),
        "lablet sent nothing to the replayed API; read its standard error above"
    );
}
