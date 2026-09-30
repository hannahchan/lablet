use std::io;
use std::path::PathBuf;

use lablet::{RunId, RunLabels, RunRequest};
use lablet_test_support::Scratch;

use super::{Source, read, request};
use crate::cli::args::{Names, PromptArgs};
use crate::cli::refusal::Refusal;

const PROMPT: &str = "Fix the failing test.";

fn unnamed() -> Names {
    Names {
        run_id: None,
        task: None,
        experiment: None,
        trial: None,
    }
}

/// Standard input that fails any read, for a source that mustn't touch it.
struct Untouched;

impl io::Read for Untouched {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("standard input was read"))
    }
}

#[test]
fn the_prompt_comes_from_the_flag_given_and_from_standard_input_with_neither() {
    let source = |prompt: Option<&str>, prompt_file: Option<&str>| {
        Source::from(PromptArgs {
            prompt: prompt.map(str::to_owned),
            prompt_file: prompt_file.map(PathBuf::from),
        })
    };
    assert_eq!(source(Some(PROMPT), None), Source::Given(PROMPT.to_owned()));
    assert_eq!(
        source(None, Some("task.md")),
        Source::File("task.md".into())
    );
    assert_eq!(source(None, None), Source::Stdin);
}

#[test]
fn a_given_prompt_and_a_file_leave_standard_input_alone() {
    let scratch = Scratch::new("prompt-file");
    let file = scratch.write("task.md", PROMPT);

    assert_eq!(
        read(&Source::Given(PROMPT.to_owned()), Untouched).unwrap(),
        PROMPT
    );
    assert_eq!(read(&Source::File(file), Untouched).unwrap(), PROMPT);
}

#[test]
fn standard_input_is_read_to_its_end() {
    let text = format!("{PROMPT}\n\nThe test is in tests/parser.rs.\n");
    assert_eq!(read(&Source::Stdin, text.as_bytes()).unwrap(), text);
}

#[test]
fn a_prompt_that_cant_be_read_is_a_config_error_naming_where_it_was_looked_for() {
    let scratch = Scratch::new("prompt-missing");
    let missing = scratch.at("task.md");
    let Err(refusal) = read(&Source::File(missing.clone()), Untouched) else {
        panic!("a missing file was read");
    };
    let message = refusal.to_string();
    let named = format!(
        "config: the task prompt from --prompt-file {} can't be read: ",
        missing.display()
    );
    assert!(message.starts_with(&named), "{message}");

    let Err(refusal) = read(&Source::Stdin, Untouched) else {
        panic!("standard input that failed was read");
    };
    assert_eq!(
        refusal.to_string(),
        "config: the task prompt from standard input can't be read: standard input was read"
    );
}

#[test]
fn a_blank_prompt_is_a_config_error_naming_its_source() {
    for (source, named) in [
        (Source::Given(String::new()), "from --prompt"),
        (Source::File("task.md".into()), "from --prompt-file task.md"),
        (Source::Stdin, "from standard input"),
    ] {
        for blank in ["", " \n\t"] {
            assert_eq!(
                request(blank.to_owned(), &source, unnamed()),
                Err(Refusal::config(format!("the task prompt {named} is blank")))
            );
        }
    }
}

#[test]
fn the_names_fill_the_request() {
    let names = Names {
        run_id: Some("r".to_owned()),
        task: Some("t".to_owned()),
        experiment: Some("e".to_owned()),
        trial: Some("3".to_owned()),
    };
    let expected = RunRequest::new(PROMPT)
        .unwrap()
        .run_id(RunId::new("r").unwrap())
        .unwrap()
        .labels(RunLabels {
            task: Some("t".to_owned()),
            experiment: Some("e".to_owned()),
            trial: Some("3".to_owned()),
        });
    assert_eq!(
        request(PROMPT.to_owned(), &Source::Stdin, names),
        Ok(expected)
    );
}

#[test]
fn a_request_that_names_nothing_has_no_run_id_and_no_labels() {
    assert_eq!(
        request(PROMPT.to_owned(), &Source::Stdin, unnamed()),
        Ok(RunRequest::new(PROMPT).unwrap())
    );
}

#[test]
fn a_run_id_the_library_refuses_is_a_config_error_naming_the_flag() {
    let names = Names {
        run_id: Some(" r".to_owned()),
        ..unnamed()
    };
    let refused = RunId::new(" r").unwrap_err();
    assert_eq!(
        request(PROMPT.to_owned(), &Source::Stdin, names),
        Err(Refusal::config(format!("--run-id: {refused}")))
    );
}
