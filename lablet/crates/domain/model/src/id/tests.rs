use serde_json::json;

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

#[test]
fn an_identifier_serialises_as_a_bare_string() {
    assert_eq!(
        serde_json::to_value(RunId::new("run-1").unwrap()).unwrap(),
        json!("run-1")
    );
    assert_eq!(
        serde_json::to_value(ToolCallId::new("call_1").unwrap()).unwrap(),
        json!("call_1")
    );
    assert_eq!(
        serde_json::to_value(ToolName::new("bash").unwrap()).unwrap(),
        json!("bash")
    );
}

#[test]
fn deserialising_an_identifier_runs_the_same_validation() {
    let tool: ToolName = serde_json::from_value(json!("bash")).unwrap();
    assert_eq!(tool, ToolName::new("bash").unwrap());

    let error = serde_json::from_value::<ToolName>(json!("no dots.please")).unwrap_err();
    assert!(
        error.to_string().contains("has a character other than"),
        "{error}"
    );

    let error = serde_json::from_value::<RunId>(json!("")).unwrap_err();
    assert!(error.to_string().contains("run id is empty"), "{error}");

    let error = serde_json::from_value::<ToolCallId>(json!(" call_1")).unwrap_err();
    assert!(
        error.to_string().contains("leading or trailing whitespace"),
        "{error}"
    );
}

#[test]
fn a_tool_name_is_a_json_map_key() {
    let mut calls = std::collections::BTreeMap::new();
    calls.insert(ToolName::new("bash").unwrap(), 2_u64);

    let value = serde_json::to_value(&calls).unwrap();
    assert_eq!(value, json!({ "bash": 2 }));
    assert_eq!(
        serde_json::from_value::<std::collections::BTreeMap<ToolName, u64>>(value).unwrap(),
        calls
    );
    assert!(
        serde_json::from_value::<std::collections::BTreeMap<ToolName, u64>>(json!({ "": 1 }))
            .is_err()
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
