use super::*;

#[test]
fn an_identifier_keeps_the_string_it_was_built_from() {
    let run = RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap();
    let call = ToolCallId::new("toolu_01A").unwrap();
    let tool = ToolName::new("read_file").unwrap();

    assert_eq!(run.as_str(), "01K5F3Z8Q4X9T2M7B6W1R0VNEC");
    assert_eq!(call.as_str(), "toolu_01A");
    assert_eq!(tool.as_str(), "read_file");
    assert_eq!(String::from(tool), "read_file");
}

#[test]
fn an_identifier_displays_as_its_string() {
    assert_eq!(RunId::new("run-1").unwrap().to_string(), "run-1");
    assert_eq!(ToolCallId::new("call_1").unwrap().to_string(), "call_1");
    assert_eq!(ToolName::new("bash").unwrap().to_string(), "bash");
}

#[test]
fn an_empty_identifier_is_refused_and_the_error_names_its_kind() {
    assert_eq!(RunId::new(""), Err(IdError::Empty { kind: "run id" }));
    assert_eq!(
        ToolCallId::new(""),
        Err(IdError::Empty {
            kind: "tool call id"
        })
    );
    assert_eq!(ToolName::new(""), Err(IdError::Empty { kind: "tool name" }));
    assert_eq!(RunId::new("").unwrap_err().to_string(), "run id is empty");
}

#[test]
fn surrounding_whitespace_is_refused_on_either_side() {
    for value in [" id", "id ", "\tid", "id\n", " "] {
        assert_eq!(
            RunId::new(value),
            Err(IdError::SurroundingWhitespace {
                kind: "run id",
                value: value.to_owned(),
            }),
            "{value:?}"
        );
        assert_eq!(
            ToolCallId::new(value),
            Err(IdError::SurroundingWhitespace {
                kind: "tool call id",
                value: value.to_owned(),
            }),
            "{value:?}"
        );
    }
}

#[test]
fn a_tool_name_with_surrounding_whitespace_is_reported_as_that_not_as_a_bad_character() {
    assert_eq!(
        ToolName::new("bash "),
        Err(IdError::SurroundingWhitespace {
            kind: "tool name",
            value: "bash ".to_owned(),
        })
    );
}

#[test]
fn whitespace_inside_a_run_id_or_call_id_is_the_providers_business() {
    assert!(RunId::new("run 1").is_ok());
    assert!(ToolCallId::new("call 1").is_ok());
}

#[test]
fn a_tool_name_takes_ascii_letters_digits_underscore_and_hyphen() {
    for name in ["bash", "read_file", "docs__search-v2", "A9", "_", "-"] {
        assert!(ToolName::new(name).is_ok(), "{name:?}");
    }
}

#[test]
fn a_tool_name_with_any_other_character_is_refused() {
    for name in ["read file", "docs.search", "a/b", "caf\u{e9}", "tool!"] {
        assert_eq!(
            ToolName::new(name),
            Err(IdError::ToolNameCharacter {
                value: name.to_owned()
            }),
            "{name:?}"
        );
    }
}

#[test]
fn a_tool_name_may_be_as_long_as_the_limit_and_no_longer() {
    let longest = "a".repeat(ToolName::MAX_LEN);
    let too_long = "a".repeat(ToolName::MAX_LEN + 1);

    assert_eq!(ToolName::MAX_LEN, 64);
    assert!(ToolName::new(longest).is_ok());
    assert_eq!(
        ToolName::new(too_long.clone()),
        Err(IdError::ToolNameTooLong { value: too_long })
    );
}

#[test]
fn the_length_error_states_the_limit() {
    let message = ToolName::new("a".repeat(65)).unwrap_err().to_string();

    assert!(
        message.ends_with("is longer than 64 characters"),
        "{message}"
    );
}

/// A name that breaks both rules is refused for its characters, because the
/// length message counts characters and only an ASCII name has as many
/// characters as bytes. Checking the length first would report these 40
/// characters as longer than 64, which is 80 bytes but a false sentence.
#[test]
fn a_name_that_is_both_too_long_in_bytes_and_not_ascii_is_refused_for_its_characters() {
    let accented = "é".repeat(40);

    assert_eq!(accented.chars().count(), 40);
    assert!(accented.len() > ToolName::MAX_LEN);
    assert_eq!(
        ToolName::new(accented.clone()),
        Err(IdError::ToolNameCharacter { value: accented })
    );
}

/// The two identifiers a request holds are measured with it, so what each
/// adds to a request's size is its own length and two quotes.
#[test]
fn a_call_id_and_a_tool_name_are_measured_as_bare_strings() {
    assert_eq!(
        serde_json::to_string(&ToolCallId::new("call_1").unwrap()).unwrap(),
        r#""call_1""#
    );
    assert_eq!(
        serde_json::to_string(&ToolName::new("bash").unwrap()).unwrap(),
        r#""bash""#
    );
}

/// The loop builds this name without an error path, so the rule this module
/// holds and the constant the completion mode names must agree.
#[test]
fn the_completion_tools_name_is_one_this_module_would_accept() {
    assert_eq!(
        ToolName::task_complete(),
        ToolName::new(crate::CompletionMode::TASK_COMPLETE).unwrap()
    );
}

// The salt of a retry

const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";

fn salt(run: &str, turn: u32, attempt: u32) -> u64 {
    RunId::new(run).unwrap().salt(turn, attempt)
}

/// The values were worked out apart from this crate, from the published
/// FNV-1a parameters, so a toolchain or a refactor that changed the hash
/// changes a replayed run's waits and fails here.
#[test]
fn a_salt_is_fnv_1a_over_the_id_then_the_turn_then_the_attempt() {
    assert_eq!(salt("a", 0, 0), 0xbfe4_d88f_2353_f60c);
    assert_eq!(salt(RUN, 1, 1), 0x205a_d082_d84b_3829);
    assert_eq!(salt(RUN, u32::MAX, u32::MAX), 0x0667_aa2a_d53d_5631);
}

#[test]
fn a_salt_reads_every_byte_of_both_numbers_least_significant_first() {
    assert_eq!(salt(RUN, 0x0102_0304, 0x0506_0708), 0x99a2_4cdb_d309_8449);
    assert_eq!(salt(RUN, 256, 1), 0x7a03_1c01_9166_af6f);
    assert_eq!(salt(RUN, 1, 256), 0x8909_9f7a_86fd_7a43);
}

#[test]
fn another_run_another_turn_and_another_attempt_each_give_another_salt() {
    let salts = [
        salt(RUN, 1, 1),
        salt("01K5F3Z8Q4X9T2M7B6W1R0VNED", 1, 1),
        salt(RUN, 2, 1),
        salt(RUN, 1, 2),
    ];

    assert_eq!(
        salts,
        [
            0x205a_d082_d84b_3829,
            0xb1c5_5be1_d659_495c,
            0x3d55_cd8d_4cad_9d6a,
            0xc055_7c8b_2e80_f4ba,
        ]
    );
}
