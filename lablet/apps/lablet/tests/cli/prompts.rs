//! Where a run's task prompt comes from: `--prompt`, `--prompt-file`, or
//! standard input with neither (C10), and a blank one, which starts no run.

use serde_json::{Value, json};

use crate::harness::{CONFIG, ENDS, Lab, PROMPT, ran, shared};

#[test]
fn each_prompt_source_gives_the_run_the_same_task() {
    let lab = Lab::new("prompt-sources");
    lab.config(
        ENDS,
        json!({ "run": { "transcript_path": "transcript-{run_id}.json" } }),
    );
    let file = lab.write("task.md", PROMPT);
    let file = file.to_str().unwrap();

    let runs = [
        (
            "given",
            ran(
                lab.lablet(&[
                    "run", "--config", CONFIG, "--prompt", PROMPT, "--run-id", "given",
                ]),
                "",
            ),
        ),
        (
            "file",
            ran(
                lab.lablet(&[
                    "run",
                    "--config",
                    CONFIG,
                    "--prompt-file",
                    file,
                    "--run-id",
                    "file",
                ]),
                "",
            ),
        ),
        (
            "stdin",
            ran(
                lab.lablet(&["run", "--config", CONFIG, "--run-id", "stdin"]),
                PROMPT,
            ),
        ),
    ];

    let mut outcomes = Vec::new();
    for (run_id, run) in runs {
        assert_eq!(run.code, Some(0), "{run_id}: {run:?}");
        let transcript: Value = serde_json::from_str(
            &std::fs::read_to_string(lab.at(&format!("transcript-{run_id}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(
            transcript["turns"][0]["input"],
            json!([{ "text": PROMPT }]),
            "{run_id}"
        );
        outcomes.push(shared(run.outcome()));
    }
    assert_eq!(outcomes[0], outcomes[1]);
    assert_eq!(outcomes[0], outcomes[2]);
}

#[test]
fn a_blank_prompt_from_any_source_is_a_config_error_and_no_run_starts() {
    let lab = Lab::new("prompt-blank");
    lab.config(ENDS, json!({}));
    let file = lab.write("task.md", " \n");
    let file = file.to_str().unwrap();

    for (args, stdin, source) in [
        (
            &["run", "--config", CONFIG, "--prompt", ""][..],
            "",
            "from --prompt".to_owned(),
        ),
        (
            &["run", "--config", CONFIG, "--prompt-file", file],
            "",
            format!("from --prompt-file {file}"),
        ),
        (
            &["run", "--config", CONFIG],
            "\n\t\n",
            "from standard input".to_owned(),
        ),
    ] {
        let run = ran(lab.lablet(args), stdin);
        assert_eq!(run.code, Some(1), "{run:?}");
        assert!(run.stdout.is_empty(), "{run:?}");
        assert_eq!(
            run.stderr,
            format!("config: the task prompt {source} is blank\n")
        );
    }
    assert!(!lab.telemetry().exists(), "a run started");
}

#[test]
fn a_prompt_file_that_isnt_there_is_a_config_error() {
    let lab = Lab::new("prompt-missing");
    lab.config(ENDS, json!({}));

    let run = lab.run(&["run", "--config", CONFIG, "--prompt-file", "task.md"]);

    assert_eq!(run.code, Some(1), "{run:?}");
    assert!(
        run.stderr
            .starts_with("config: the task prompt from --prompt-file task.md can't be read: "),
        "{run:?}"
    );
}
