use lablet_model::{RunId, ToolCallId, ToolName};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// A shape that holds one of each, the way the documents hold them.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Ids {
    #[serde(with = "super::run_id")]
    run: RunId,
    #[serde(with = "super::tool_call_id")]
    call: ToolCallId,
    #[serde(with = "super::tool_name")]
    tool: ToolName,
}

fn ids() -> Ids {
    Ids {
        run: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        call: ToolCallId::new("toolu_01A").unwrap(),
        tool: ToolName::new("read_file").unwrap(),
    }
}

#[test]
fn an_identifier_is_written_as_a_bare_string_and_reads_back_as_itself() {
    let written = serde_json::to_string(&ids()).unwrap();

    assert_eq!(
        written,
        r#"{"run":"01K5F3Z8Q4X9T2M7B6W1R0VNEC","call":"toolu_01A","tool":"read_file"}"#
    );
    assert_eq!(serde_json::from_str::<Ids>(&written).unwrap(), ids());
}

#[test]
fn reading_an_identifier_holds_it_to_the_rule_the_domain_has_for_it() {
    for (run, call, tool, broken) in [
        ("", "toolu_01A", "bash", "run id is empty"),
        (
            "run-1",
            " toolu_01A",
            "bash",
            r#"tool call id " toolu_01A" has leading or trailing whitespace"#,
        ),
        (
            "run-1",
            "toolu_01A",
            "no dots.please",
            r#"tool name "no dots.please" has a character other than"#,
        ),
    ] {
        let refused =
            serde_json::from_value::<Ids>(json!({ "run": run, "call": call, "tool": tool }))
                .unwrap_err();

        assert!(refused.to_string().contains(broken), "{refused}");
    }
}

#[test]
fn an_identifier_that_is_not_a_string_is_refused() {
    let refused =
        serde_json::from_value::<Ids>(json!({ "run": 7, "call": "toolu_01A", "tool": "bash" }))
            .unwrap_err();

    assert!(
        refused.to_string().contains("expected a string"),
        "{refused}"
    );
}
