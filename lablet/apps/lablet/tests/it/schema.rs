//! The config's JSON Schema, which `lablet schema` prints and
//! `lablet/schema.json` holds.

use lablet::{Config, Format};
use serde_json::{Map, Value, json};

// Compiled in, so the test reads the same file whatever directory it runs from.
const CHECKED_IN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema.json"));

fn schema() -> Value {
    lablet::schema()
}

/// The file is the schema byte for byte, indented and ended by a newline,
/// so a change to a config type is a change to the file, which the
/// changelog gate watches.
#[test]
fn the_schema_is_the_checked_in_file_byte_for_byte() {
    let mut written = serde_json::to_string_pretty(&schema()).unwrap();
    written.push('\n');

    assert_eq!(written, CHECKED_IN);
}

#[test]
fn the_schema_is_a_json_schema_of_a_mapping_of_the_five_sections_and_nothing_else() {
    let schema = schema();

    assert_eq!(
        schema["$schema"],
        json!("https://json-schema.org/draft/2020-12/schema")
    );
    assert_eq!(schema["type"], json!("object"));
    assert_eq!(schema["additionalProperties"], json!(false));
    let sections: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
    assert_eq!(sections, ["model", "prompt", "run", "telemetry", "tools"]);
}

/// The defaults the schema states are the reader's: a config made of them
/// is the config of nothing.
#[test]
fn the_defaults_the_schema_states_are_the_defaults_a_config_is_read_with() {
    let schema = schema();
    let defaults: Map<String, Value> = schema["properties"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(section, property)| (section.clone(), property["default"].clone()))
        .collect();

    let config = Config::from_str(&Value::Object(defaults).to_string(), Format::Json).unwrap();

    assert_eq!(config, Config::default());
}

/// A value a setting takes is one the schema lists, as the reader does.
#[test]
fn the_schema_lists_the_values_a_setting_takes() {
    let schema = schema();
    let values = |definition: &str| -> Vec<Value> {
        schema["$defs"][definition]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|variant| variant.get("const").cloned())
            .collect()
    };

    assert_eq!(
        values("Provider"),
        [json!("anthropic"), json!("openai"), json!("fake")]
    );
    assert_eq!(
        values("BuiltinTool"),
        [json!("bash"), json!("read_file"), json!("write_file")]
    );
    assert_eq!(
        schema["$defs"]["Run"]["properties"]["timeout"]["default"],
        json!("10m")
    );
}
