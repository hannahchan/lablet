use std::os::unix::fs::symlink;

use lablet_test_support::Scratch;

use super::*;

/// A scratch directory of one test's own, holding `work/nested`,
/// `work-notes` and `out`.
fn laid_out(test: &str) -> Scratch {
    let scratch = Scratch::new(test);
    for directory in ["work/nested", "work-notes", "out"] {
        scratch.create_dir(directory);
    }
    scratch
}

#[test]
fn a_file_that_exists_resolves_to_where_its_links_lead() {
    let scratch = laid_out("exists");
    std::fs::write(scratch.at("work/nested/config.yaml"), "").unwrap();
    symlink(scratch.at("work/nested"), scratch.at("out/linked")).unwrap();

    assert_eq!(
        resolved(&scratch.at("work/nested/config.yaml")),
        scratch.at("work/nested/config.yaml")
    );
    assert_eq!(
        resolved(&scratch.at("out/linked/config.yaml")),
        scratch.at("work/nested/config.yaml")
    );
    assert_eq!(
        resolved(&scratch.at("work/nested/../nested/config.yaml")),
        scratch.at("work/nested/config.yaml")
    );
}

#[test]
fn a_file_that_does_not_exist_yet_resolves_by_the_part_of_its_path_that_does() {
    let scratch = laid_out("missing");
    symlink(scratch.at("work"), scratch.at("out/linked")).unwrap();

    assert_eq!(
        resolved(&scratch.at("out/transcript.json")),
        scratch.at("out/transcript.json")
    );
    assert_eq!(
        resolved(&scratch.at("out/{run_id}/deep/transcript.json")),
        scratch.at("out/{run_id}/deep/transcript.json")
    );
    assert_eq!(
        resolved(&scratch.at("out/linked/runs/{run_id}.json")),
        scratch.at("work/runs/{run_id}.json"),
        "the link is followed, and the names after it are kept in their order"
    );
}

#[test]
fn a_path_that_is_not_absolute_starts_at_the_working_directory() {
    let here = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();

    assert_eq!(
        resolved(Path::new("no-such-directory/telemetry.jsonl")),
        here.join("no-such-directory/telemetry.jsonl")
    );
    assert_eq!(resolved(Path::new("Cargo.toml")), here.join("Cargo.toml"));
}

#[test]
fn the_root_holds_what_leads_under_it_and_nothing_beside_it() {
    let scratch = laid_out("held");
    let root = scratch.at("work");
    symlink(scratch.at("work/nested"), scratch.at("out/linked")).unwrap();
    symlink(scratch.at("out"), scratch.at("work/escape")).unwrap();
    let holds =
        |path: &str| held(&root, [(OwnFile::Transcript, scratch.at(path).as_path())]).is_some();

    assert!(holds("work/transcript.json"));
    assert!(holds("work/nested/{run_id}.json"));
    assert!(
        holds("work"),
        "the root itself is no place for a file of lablet's"
    );
    assert!(
        holds("out/linked/transcript.json"),
        "a link that leads under the root leads to what the model can read"
    );

    assert!(!holds("out/transcript.json"));
    assert!(
        !holds("work-notes/transcript.json"),
        "a directory whose name starts as the root's does isn't under the root"
    );
    assert!(
        !holds("work/escape/transcript.json"),
        "a link that leads out of the root leads to what the file tools refuse"
    );
    assert!(!holds(""), "the directory above the root isn't under it");
}

#[test]
fn the_first_file_the_root_holds_is_the_one_reported() {
    let scratch = laid_out("first");
    let root = scratch.at("work");
    let (config, prompt, transcript, telemetry) = (
        scratch.at("lablet.yaml"),
        scratch.at("work/system.md"),
        scratch.at("out/{run_id}.json"),
        scratch.at("work/telemetry.jsonl"),
    );
    let files = [
        (OwnFile::Config, config.as_path()),
        (OwnFile::SystemPrompt, prompt.as_path()),
        (OwnFile::Transcript, transcript.as_path()),
        (OwnFile::Telemetry, telemetry.as_path()),
    ];

    assert_eq!(
        held(&root, files),
        Some((OwnFile::SystemPrompt, prompt.as_path()))
    );
    assert_eq!(
        held(&root, files.into_iter().skip(2)),
        Some((OwnFile::Telemetry, telemetry.as_path()))
    );
    assert_eq!(held(&root, files.into_iter().take(1)), None);
    assert_eq!(held(&root, []), None);
}

#[test]
fn each_file_of_lablets_own_is_named_by_the_key_that_names_it() {
    assert_eq!(OwnFile::Config.to_string(), "the config");
    assert_eq!(
        OwnFile::SystemPrompt.to_string(),
        "the system prompt that `prompt.system_file` names"
    );
    assert_eq!(
        OwnFile::Transcript.to_string(),
        "the transcript that `run.transcript_path` names"
    );
    assert_eq!(
        OwnFile::Telemetry.to_string(),
        "the telemetry file that `telemetry.file.path` names"
    );
}
