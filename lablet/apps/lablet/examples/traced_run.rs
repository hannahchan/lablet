//! A traced run that needs no model and no key: a script is played in
//! place of the model, `bash` works under a root of its own, and the run
//! leaves its telemetry and its transcript in files.
//!
//! ```bash
//! cargo run -p lablet --example traced_run
//! ```

use std::error::Error;

use lablet::{Config, Format, OutcomeDocument, RunLabels, RunRequest};

const SCRIPT: &str = "
- response:
    content:
      - text: I'll look at what's there.
      - tool_use: { id: call_1, name: bash, input: { json: { command: ls -a } } }
    finish: tool_use
    usage: { input_tokens: 120, output_tokens: 30 }
    latency: 40ms
- response:
    content:
      - text: The directory is empty.
    finish: end_turn
    usage: { input_tokens: 180, output_tokens: 12 }
    latency: 25ms
";

#[tokio::main]
#[expect(
    clippy::print_stdout,
    reason = "an example says what the run came to and where it left its files"
)]
async fn main() -> Result<(), Box<dyn Error>> {
    let directory = std::env::temp_dir().join("lablet-traced-run");
    std::fs::create_dir_all(directory.join("work"))?;
    let script = directory.join("script.yaml");
    std::fs::write(&script, SCRIPT)?;
    let telemetry = directory.join("telemetry.otlp.jsonl");
    // A file of an earlier run would be added to.
    if telemetry.exists() {
        std::fs::remove_file(&telemetry)?;
    }

    let config = Config::from_str(
        &format!(
            "
model:
  provider: fake
  script: {script}
  name: scripted-1
prompt:
  system: You answer tersely.
run:
  transcript_path: {directory}/transcript-{{run_id}}.json
tools:
  builtin:
    root: {directory}/work
    enabled: [bash]
telemetry:
  capture_content: true
  file:
    path: {telemetry}
",
            script = script.display(),
            directory = directory.display(),
            telemetry = telemetry.display(),
        ),
        Format::Yaml,
    )?;

    let mut lablet = lablet::build(config).await?;
    let request = RunRequest::new("What's in the directory?")?.labels(RunLabels {
        task: Some("list-the-directory".to_owned()),
        ..RunLabels::default()
    });
    let finished = lablet.run(request).await;
    lablet.shutdown().await;

    let outcome = finished.summary.outcome;
    let transcript = directory.join(format!("transcript-{}.json", outcome.run_id));
    println!(
        "{}",
        serde_json::to_string_pretty(&OutcomeDocument::from(outcome))?
    );
    println!();
    println!("telemetry:  {}", telemetry.display());
    println!("transcript: {}", transcript.display());
    Ok(())
}
