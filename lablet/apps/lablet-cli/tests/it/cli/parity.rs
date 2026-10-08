//! The command line and the library make the same run of one config: the
//! config and the script of the library's doctest, whose outcome the
//! doctest asserts.

use lablet::{Config, Otel, OutcomeDocument, RunRequest};
use serde_json::json;

use super::harness::{Lab, shared};

/// The script the doctest plays: a call of `bash`, and a response that ends
/// the run.
const SCRIPT: &str = "
- response:
    content:
      - text: I'll look at what's there.
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo parser.rs } } }
    finish: tool_use
    usage: { input_tokens: 120, output_tokens: 30 }
- response:
    content:
      - text: There's one file, parser.rs.
    finish: end_turn
    usage: { input_tokens: 180, output_tokens: 12 }
";

const PROMPT: &str = "What's in the directory?";

#[tokio::test]
async fn the_command_line_and_the_library_make_the_doctest_s_outcome_of_its_config() {
    let lab = Lab::new("parity");
    let directory = lab.path().display();
    lab.write("script.yaml", SCRIPT);
    let path = lab.write(
        "lablet.yaml",
        &format!(
            "
model:
  provider: fake
  script: {directory}/script.yaml
  name: scripted-1
prompt:
  system: You answer tersely.
tools:
  builtin:
    root: {directory}/work
    enabled: [bash]
telemetry:
  file:
    path: {directory}/telemetry.otlp.jsonl
  otlp:
    enabled: false
"
        ),
    );

    let command_line = lab.run(&["run", "--config", "lablet.yaml", "--prompt", PROMPT]);
    assert_eq!(command_line.code, Some(0), "{command_line:?}");

    let mut lablet = lablet::build(Config::from_path(&path).unwrap(), Otel::noop())
        .await
        .unwrap();
    let library = lablet.run(RunRequest::new(PROMPT).unwrap()).await;
    lablet.shutdown().await;
    let library = serde_json::to_value(OutcomeDocument::from(library.summary.outcome)).unwrap();

    let outcome = command_line.outcome();
    assert_eq!(shared(outcome.clone()), shared(library));
    // What the doctest asserts of each of its runs.
    assert_eq!(outcome["stop_reason"], json!("completed"));
    assert_eq!(
        outcome["result"]["text"],
        json!("There's one file, parser.rs.")
    );
    assert_eq!(
        (&outcome["turns"], &outcome["tool_calls"]),
        (&json!(2), &json!(1))
    );
    assert_eq!(
        (
            &outcome["usage"]["input_tokens"],
            &outcome["usage"]["output_tokens"]
        ),
        (&json!(300), &json!(42))
    );
}
