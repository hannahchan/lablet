//! The line a run ends with on standard error (C11), and a standard error
//! that holds nothing but OTLP lines when the telemetry goes there (C15).

use lablet_conformance::otlp::Exported;
use serde_json::json;

use super::harness::{CONFIG, ENDS, Lab, ran};

/// A call of a tool the run offers, and a response that ends the run.
const CALLS_ENDS: &str = "
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo parser.rs } } }
    usage: { input_tokens: 1200, output_tokens: 30 }
    finish: tool_use
- response:
    content:
      - text: There's one file, parser.rs.
    usage: { input_tokens: 1300, output_tokens: 12 }
    finish: end_turn
";

/// What a config states over the lab's to offer `bash`, with `run` as its
/// `run` section.
fn with_bash(lab: &Lab, run: serde_json::Value) -> serde_json::Value {
    lab.write("work/.keep", "");
    let mut more = json!({
        "tools": { "builtin": { "root": lab.at("work"), "enabled": ["bash"] } },
    });
    more["run"] = run;
    more
}

/// Whether `line` ends in a duration as the summary line gives one.
fn ends_in_a_duration(line: &str) -> bool {
    let Some((_, duration)) = line.rsplit_once(", ") else {
        return false;
    };
    let digits = duration
        .strip_suffix("ms")
        .or_else(|| duration.strip_suffix('s'))
        .unwrap_or("x");
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
}

#[test]
fn a_run_ends_with_one_summary_line_on_standard_error() {
    let lab = Lab::new("summary-line");
    lab.write_config(CALLS_ENDS, with_bash(&lab, json!({})));

    let run = lab.run_config(&[]);

    assert_eq!(run.code, Some(0), "{run:?}");
    let lines = run.stderr_lines();
    assert_eq!(lines.len(), 1, "{run:?}");
    assert!(
        lines[0].starts_with("completed: 2 turns, 2,542 tokens, 1 tool call, "),
        "{run:?}"
    );
    assert!(ends_in_a_duration(lines[0]), "{run:?}");
}

#[test]
fn quiet_leaves_the_summary_line_out_and_the_outcome_in() {
    let lab = Lab::new("summary-quiet");
    lab.write_config(ENDS, json!({}));

    for quiet in ["--quiet", "-q"] {
        let run = lab.run_config(&[quiet]);

        assert_eq!(run.code, Some(0), "{run:?}");
        assert_eq!(run.stderr, "", "{quiet}");
        assert_eq!(run.outcome()["stop_reason"], json!("completed"));
    }
}

#[test]
fn a_run_that_reached_its_token_budget_says_how_far_and_what_the_numbers_count() {
    let lab = Lab::new("summary-budget");
    lab.write_config(
        CALLS_ENDS,
        with_bash(&lab, json!({ "max_total_tokens": 1000 })),
    );

    let run = lab.run_config(&[]);

    assert_eq!(run.code, Some(2), "{run:?}");
    assert_eq!(run.outcome()["stop_reason"], json!("max_total_tokens"));
    let lines = run.stderr_lines();
    assert_eq!(lines.len(), 1, "{run:?}");
    assert!(
        lines[0].starts_with(
            "max_total_tokens: 1 turn, token budget reached: 1,230 of 1,000 tokens, 1 tool call, "
        ),
        "{run:?}"
    );
}

/// A config whose telemetry goes to standard error, and whose transcript
/// can't be written, which lablet warns of on the diagnostic log.
fn on_stderr_with_a_warning(lab: &Lab) {
    let blocked = lab.write("blocked", "");
    lab.write_config(
        ENDS,
        json!({
            "run": { "transcript_path": blocked.join("transcript.json") },
            "telemetry": { "file": { "path": "-" } },
        }),
    );
}

#[test]
fn with_the_telemetry_on_standard_error_it_holds_nothing_but_otlp_lines() {
    let lab = Lab::new("summary-stderr");
    on_stderr_with_a_warning(&lab);

    let run = lab.run_config(&[]);

    assert_eq!(run.code, Some(0), "{run:?}");
    let run_id = run.outcome()["run_id"].clone();
    let exported = Exported::parse(&run.stderr).unwrap();
    let wide = exported.records_of("lablet.run");
    assert_eq!(wide.len(), 1, "{run:?}");
    assert_eq!(wide[0].attributes["gen_ai.conversation.id"], run_id);
}

#[test]
fn with_the_telemetry_on_standard_error_a_closed_standard_output_leaves_it_nothing_but_otlp() {
    let lab = Lab::new("summary-stderr-closed");
    lab.write_config(ENDS, json!({ "telemetry": { "file": { "path": "-" } } }));
    // The read end is gone before lablet starts, so writing the outcome fails.
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut command = lab.lablet(&["run", "--config", CONFIG, "--prompt", "Say hello."]);
    command.stdout(writer).stderr(std::process::Stdio::piped());

    let finished = command.output().unwrap();

    let stderr = String::from_utf8(finished.stderr).unwrap();
    assert_eq!(finished.status.code(), Some(0), "{stderr}");
    let exported = Exported::parse(&stderr).unwrap();
    assert_eq!(exported.records_of("lablet.run").len(), 1, "{stderr}");
    assert!(
        stderr.lines().all(|line| line.starts_with('{')),
        "a line that isn't OTLP: {stderr}"
    );
}

#[test]
fn rust_log_brings_the_diagnostic_log_back_beside_the_otlp_lines() {
    let lab = Lab::new("summary-stderr-log");
    on_stderr_with_a_warning(&lab);

    let mut logged = lab.lablet(&["run", "--config", CONFIG, "--prompt", "Say hello."]);
    logged.env("RUST_LOG", "warn");
    let run = ran(logged, "");

    assert_eq!(run.code, Some(0), "{run:?}");
    let (diagnostic, otlp): (Vec<&str>, Vec<&str>) = run
        .stderr_lines()
        .into_iter()
        .partition(|line| !line.starts_with('{'));
    assert_eq!(diagnostic.len(), 1, "{run:?}");
    assert!(diagnostic[0].contains("transcript"), "{run:?}");
    let otlp = format!("{}\n", otlp.join("\n"));
    assert_eq!(
        Exported::parse(&otlp)
            .unwrap()
            .records_of("lablet.run")
            .len(),
        1
    );
}

/// The variable a config's telemetry path is read from.
const TELEMETRY: &str = "LABLET_TEST_TELEMETRY";

/// Runs the lab's config with its telemetry path written as `path`, and
/// [`TELEMETRY`] holding `held`.
fn with_the_telemetry_path(lab: &Lab, path: &str, held: &str) -> super::harness::Ran {
    let blocked = lab.write("blocked", "");
    lab.write_config(
        ENDS,
        json!({
            "run": { "transcript_path": blocked.join("transcript.json") },
            "telemetry": { "file": { "path": path } },
        }),
    );
    let mut command = lab.lablet(&["run", "--config", CONFIG, "--prompt", "Say hello."]);
    command.env(TELEMETRY, held);
    ran(command, "")
}

#[test]
fn a_dash_a_variable_gives_the_telemetry_path_leaves_standard_error_nothing_but_otlp_lines() {
    let lab = Lab::new("summary-stderr-variable");

    let run = with_the_telemetry_path(&lab, &format!("${{{TELEMETRY}}}"), "-");

    assert_eq!(run.code, Some(0), "{run:?}");
    let exported = Exported::parse(&run.stderr).unwrap();
    assert_eq!(exported.records_of("lablet.run").len(), 1, "{run:?}");
}

#[test]
fn a_telemetry_path_that_is_a_dash_and_more_names_a_file_and_leaves_the_log_and_the_summary() {
    let from_the_variable = format!("${{{TELEMETRY}}}/");
    for (path, held) in [("-/", ""), (from_the_variable.as_str(), "-")] {
        let lab = Lab::new("summary-stderr-slash");

        let run = with_the_telemetry_path(&lab, path, held);

        assert_eq!(run.code, Some(0), "{path}: {run:?}");
        let lines = run.stderr_lines();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("the telemetry file couldn't be written")),
            "{path}: {run:?}"
        );
        assert!(
            lines
                .last()
                .is_some_and(|line| line.starts_with("completed: ")),
            "{path}: {run:?}"
        );
        assert!(!run.stderr.contains("resourceSpans"), "{path}: {run:?}");
    }
}

#[test]
fn the_warning_those_runs_are_given_reaches_standard_error_when_the_telemetry_is_elsewhere() {
    let lab = Lab::new("summary-warning");
    let blocked = lab.write("blocked", "");
    lab.write_config(
        ENDS,
        json!({ "run": { "transcript_path": blocked.join("transcript.json") } }),
    );

    let run = lab.run_config(&[]);

    assert_eq!(run.code, Some(0), "{run:?}");
    let lines = run.stderr_lines();
    assert_eq!(lines.len(), 2, "{run:?}");
    assert!(lines[0].contains("transcript"), "{run:?}");
    assert!(lines[1].starts_with("completed: "), "{run:?}");
}
