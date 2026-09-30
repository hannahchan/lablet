//! Where a run's prompts come from: the task's from `--prompt`,
//! `--prompt-file`, or standard input with neither, and the system prompt's
//! from the config or the file it names (C10); and a blank task, which
//! starts no run.

use lablet_telemetry_registry::attribute as key;
use serde_json::{Value, json};

use super::harness::{CONFIG, ENDS, Lab, PROMPT, ran, shared};
use crate::harness::{SYSTEM, Traced};

#[test]
fn each_prompt_source_gives_the_run_the_same_task() {
    let lab = Lab::new("prompt-sources");
    lab.write_config(
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
    lab.write_config(ENDS, json!({}));
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
    lab.write_config(ENDS, json!({}));

    let run = lab.run(&["run", "--config", CONFIG, "--prompt-file", "task.md"]);

    assert_eq!(run.code, Some(1), "{run:?}");
    assert!(
        run.stderr
            .starts_with("config: the task prompt from --prompt-file task.md can't be read: "),
        "{run:?}"
    );
}

#[test]
fn a_prompt_file_the_root_of_the_built_in_tools_holds_is_refused_before_the_run() {
    let lab = Lab::new("prompt-in-root");
    lab.write_config(ENDS, json!({ "tools": lab.builtin_tools(&["read_file"]) }));
    let inside = lab.write("work/task.md", PROMPT);

    let run = lab.run(&[
        "run",
        "--config",
        CONFIG,
        "--prompt-file",
        inside.to_str().unwrap(),
    ]);

    assert_eq!(run.code, Some(1), "{run:?}");
    assert!(run.stdout.is_empty(), "{run:?}");
    assert_eq!(
        run.stderr,
        format!(
            "config: tools.builtin.root (line 1): {} is refused: it holds the task prompt's \
             file, {}\n",
            lab.root().display(),
            inside.display()
        )
    );
    assert!(!lab.telemetry().exists(), "a run started");

    let beside = lab.write("task.md", PROMPT);
    let run = lab.run(&[
        "run",
        "--config",
        CONFIG,
        "--prompt-file",
        beside.to_str().unwrap(),
    ]);
    assert_eq!(run.code, Some(0), "{run:?}");
}

#[test]
fn a_system_prompt_from_its_file_gives_the_run_the_system_prompt_written_out() {
    let lab = Lab::new("prompt-system-file");
    let file = lab.write("system.md", SYSTEM);
    let mut outcomes = Vec::new();
    for (run_id, prompt) in [
        ("inline", json!({ "system": SYSTEM })),
        ("file", json!({ "system": null, "system_file": file })),
    ] {
        lab.write_config(
            ENDS,
            json!({
                "prompt": prompt,
                "run": { "transcript_path": "transcript-{run_id}.json" },
            }),
        );
        let run = lab.run_config(&["--run-id", run_id]);
        assert_eq!(run.code, Some(0), "{run_id}: {run:?}");
        outcomes.push(shared(run.outcome()));
        let transcript: Value = serde_json::from_str(
            &std::fs::read_to_string(lab.at(&format!("transcript-{run_id}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(transcript["system"], json!(SYSTEM), "{run_id}");
    }

    assert_eq!(outcomes[0], outcomes[1]);
    let exported = lab.exported();
    let digest = |run_id| {
        Traced::of(&exported, run_id).wide().attributes[key::LABLET_PROMPT_SYSTEM_DIGEST].clone()
    };
    assert_eq!(digest("inline"), digest("file"));
}
