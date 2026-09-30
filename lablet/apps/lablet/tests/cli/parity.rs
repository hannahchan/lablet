//! The command line and the library make the same run of one config.

use lablet::{Config, OutcomeDocument, RunRequest};
use serde_json::json;

use crate::harness::{CONFIG, Lab, PROMPT, shared};

/// A call of a tool the run offers, and a response that ends the run.
const CALLS_ENDS: &str = "
- response:
    content:
      - text: I'll look at what's there.
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo parser.rs } } }
    usage: { input_tokens: 120, output_tokens: 30 }
    finish: tool_use
- response:
    content:
      - text: There's one file, parser.rs.
    usage: { input_tokens: 180, output_tokens: 12 }
    finish: end_turn
";

#[tokio::test]
async fn the_command_line_and_the_library_make_one_outcome_of_one_config() {
    let lab = Lab::new("parity");
    lab.write("work/.keep", "");
    lab.config(
        CALLS_ENDS,
        json!({ "tools": { "builtin": { "root": lab.at("work"), "enabled": ["bash"] } } }),
    );

    let command_line = lab.run_config(&[]);
    assert_eq!(command_line.code, Some(0), "{command_line:?}");

    let mut lablet = lablet::build(Config::from_path(lab.at(CONFIG)).unwrap())
        .await
        .unwrap();
    let library = lablet.run(RunRequest::new(PROMPT).unwrap()).await;
    lablet.shutdown().await;
    let library = serde_json::to_value(OutcomeDocument::from(library.summary.outcome)).unwrap();

    assert_eq!(shared(command_line.outcome()), shared(library));
    assert_eq!(command_line.outcome()["tool_calls"], json!(1));
}
