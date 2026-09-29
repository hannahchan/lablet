use serde::de::IntoDeserializer as _;
use serde::de::value::{BytesDeserializer, Error as ValueError, F64Deserializer};
use serde_json::json;

use super::*;

fn from_json(text: &str) -> Result<Value, String> {
    serde_json::from_str::<Tree>(text)
        .map(|Tree(value)| value)
        .map_err(|error| error.to_string())
}

fn from_yaml(text: &str) -> Result<Value, String> {
    serde_saphyr::from_str::<Tree>(text)
        .map(|Tree(value)| value)
        .map_err(|error| error.to_string())
}

#[test]
fn every_kind_of_value_is_read_from_json_as_json_holds_it() {
    let text = r#"{
        "nothing": null,
        "yes": true,
        "count": 12,
        "below": -3,
        "share": 0.25,
        "text": "hello",
        "list": [1, "two", [3], {"four": 4}],
        "mapping": {"inner": {"deep": []}}
    }"#;

    assert_eq!(
        from_json(text).unwrap(),
        serde_json::from_str::<Value>(text).unwrap()
    );
}

#[test]
fn every_kind_of_value_is_read_from_yaml_as_json_would_hold_it() {
    let text = "
nothing: null
yes: true
count: 12
below: -3
share: 0.25
text: hello
quoted: '12'
list:
  - 1
  - two
  - [3]
  - four: 4
mapping:
  inner:
    deep: []
";

    assert_eq!(
        from_yaml(text).unwrap(),
        json!({
            "nothing": null,
            "yes": true,
            "count": 12,
            "below": -3,
            "share": 0.25,
            "text": "hello",
            "quoted": "12",
            "list": [1, "two", [3], { "four": 4 }],
            "mapping": { "inner": { "deep": [] } },
        })
    );
}

#[test]
fn a_key_written_twice_in_json_is_refused_by_name_and_place() {
    let refused = from_json(r#"{"finish": "end_turn", "finish": "tool_use"}"#).unwrap_err();

    assert_eq!(
        refused,
        r#"the key "finish" is written twice in one mapping at line 1 column 31"#
    );
}

#[test]
fn a_key_written_twice_is_refused_however_deep_it_is() {
    let refused = from_json(r#"[{"a": [{"b": 1, "c": 2, "b": 3}]}]"#).unwrap_err();

    assert!(
        refused.starts_with(r#"the key "b" is written twice in one mapping"#),
        "{refused}"
    );
}

#[test]
fn a_key_written_twice_in_yaml_is_refused_by_name_and_place() {
    let refused = from_yaml("finish: end_turn\nfinish: tool_use\n").unwrap_err();

    assert!(
        refused.contains("duplicate mapping key: finish"),
        "{refused}"
    );
    assert!(refused.contains("line 2"), "{refused}");
}

#[test]
fn two_mappings_may_each_hold_the_same_key() {
    let text = r#"[{"text": "a"}, {"text": "b"}, {"text": {"text": "c"}}]"#;

    assert_eq!(
        from_json(text).unwrap(),
        json!([{ "text": "a" }, { "text": "b" }, { "text": { "text": "c" } }])
    );
}

#[test]
fn a_number_json_cannot_hold_is_refused_and_never_read_as_null() {
    for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let deserializer: F64Deserializer<ValueError> = number.into_deserializer();

        let refused = Tree::deserialize(deserializer).map(|Tree(value)| value);

        assert_eq!(
            refused.unwrap_err().to_string(),
            format!("{number} isn't a number JSON can hold")
        );
    }
}

#[test]
fn a_value_json_has_no_form_for_is_refused() {
    let deserializer: BytesDeserializer<'_, ValueError> = BytesDeserializer::new(b"bytes");

    let refused = Tree::deserialize(deserializer).map(|Tree(value)| value);

    assert_eq!(
        refused.unwrap_err().to_string(),
        "invalid type: byte array, expected a value JSON can hold"
    );
}

#[test]
fn text_that_is_not_the_format_is_refused_with_the_parsers_place() {
    assert_eq!(
        from_json(r#"[{"text": "a"}"#).unwrap_err(),
        "EOF while parsing a list at line 1 column 14"
    );
    assert!(
        from_yaml("a: [1, 2\nb: 3\n")
            .unwrap_err()
            .contains("line 2")
    );
}
