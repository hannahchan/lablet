use serde::Deserialize;
use serde::de::Error as _;
use serde_json::json;

use super::*;

fn read<T: for<'de> Deserialize<'de>>(value: &Value) -> Result<T, Fault> {
    T::deserialize(At {
        value,
        path: KeyPath::default(),
    })
}

#[test]
fn accepted_values_are_listed_in_backticks() {
    assert_eq!(listed(&[]), "none");
    assert_eq!(listed(&["bash"]), "`bash`");
    assert_eq!(listed(&["head", "head_tail"]), "`head`, `head_tail`");
}

#[test]
fn a_refusal_keeps_what_was_expected_and_never_what_was_found() {
    for fault in [
        Fault::invalid_type(Unexpected::Str("sk-ant-secret"), &"a number"),
        Fault::invalid_value(Unexpected::Str("sk-ant-secret"), &"a number"),
        Fault::invalid_length(3, &"a number"),
    ] {
        assert_eq!(fault.to_string(), "expected a number");
    }
    assert_eq!(
        Fault::unknown_variant("sk-ant-secret", &["low", "high"]).to_string(),
        "the accepted values are `low`, `high`"
    );
    assert_eq!(
        Fault::duplicate_field("timeout").to_string(),
        "the key is written twice"
    );
    assert_eq!(Fault::custom("a rule").to_string(), "a rule");
}

#[test]
fn a_refusal_is_placed_where_it_was_made_and_not_moved_on_the_way_out() {
    let run = KeyPath::of("run");

    let unknown = Fault::unknown_field("max_turn", &["max_turns"]).at(&run);
    assert_eq!(unknown.key, Some(KeyPath::of("run.max_turn")));
    assert_eq!(unknown.to_string(), "unknown key max_turn");

    let missing = Fault::missing_field("command").at(&KeyPath::of("tools.mcp").index(0));
    assert_eq!(
        missing.key,
        Some(KeyPath::of("tools.mcp").index(0).key("command"))
    );
    assert_eq!(missing.to_string(), "command isn't set");

    let refused = Fault::custom("a rule").at(&KeyPath::of("run.timeout"));
    assert_eq!(refused.key, Some(KeyPath::of("run.timeout")));
    assert_eq!(refused.clone().at(&run).key, refused.key);
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Shape {
    Unit,
    Newtype(u32),
    Tuple(u32, u32),
    Struct { side: u32 },
}

#[test]
fn every_kind_of_variant_is_read_from_its_name_or_a_mapping_of_one_name() {
    assert_eq!(read::<Shape>(&json!("unit")), Ok(Shape::Unit));
    assert_eq!(read::<Shape>(&json!({ "unit": null })), Ok(Shape::Unit));
    assert_eq!(
        read::<Shape>(&json!({ "newtype": 3 })),
        Ok(Shape::Newtype(3))
    );
    assert_eq!(
        read::<Shape>(&json!({ "tuple": [1, 2] })),
        Ok(Shape::Tuple(1, 2))
    );
    assert_eq!(
        read::<Shape>(&json!({ "struct": { "side": 4 } })),
        Ok(Shape::Struct { side: 4 })
    );
}

#[test]
fn a_variant_written_another_way_is_refused_where_it_was_written() {
    let refused = |value: Value| read::<Shape>(&value).unwrap_err();

    assert_eq!(
        refused(json!({ "unit": 1 })).to_string(),
        "expected the value's name alone"
    );
    for name in ["newtype", "tuple", "struct"] {
        assert_eq!(
            refused(json!(name)).to_string(),
            "expected a mapping of the value's name to what it holds",
            "{name}"
        );
    }
    for value in [json!(3), json!({ "unit": null, "newtype": 3 }), json!({})] {
        assert_eq!(
            refused(value.clone()).to_string(),
            "expected a value's name, or a mapping of one name to what it holds",
            "{value}"
        );
    }
    assert_eq!(
        refused(json!({ "newtype": "three" })).key,
        Some(KeyPath::of("newtype"))
    );
    assert_eq!(
        refused(json!({ "struct": { "side": 4, "sides": 5 } })).key,
        Some(KeyPath::of("struct.sides"))
    );
    assert_eq!(
        refused(json!({ "tuple": [1, "two"] })).key,
        Some(KeyPath::of("tuple").index(1))
    );
}

#[test]
fn numbers_options_and_newtypes_are_read_as_the_tree_holds_them() {
    #[derive(Debug, PartialEq, Deserialize)]
    struct Wrapped(i64);

    assert_eq!(read::<u64>(&json!(u64::MAX)), Ok(u64::MAX));
    assert_eq!(read::<i64>(&json!(-3)), Ok(-3));
    assert_eq!(read::<f64>(&json!(0.25)), Ok(0.25));
    assert_eq!(read::<f64>(&json!(2)), Ok(2.0));
    assert_eq!(read::<Option<u32>>(&json!(null)), Ok(None));
    assert_eq!(read::<Option<u32>>(&json!(7)), Ok(Some(7)));
    assert_eq!(read::<Wrapped>(&json!(-9)), Ok(Wrapped(-9)));
    assert_eq!(read::<bool>(&json!(true)), Ok(true));
    assert_eq!(read::<()>(&json!(null)), Ok(()));
    assert_eq!(
        read::<Vec<String>>(&json!(["a", "b"])),
        Ok(vec!["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(
        read::<u32>(&json!(-1)).unwrap_err().to_string(),
        "expected u32"
    );
}
