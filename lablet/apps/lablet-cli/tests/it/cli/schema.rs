//! `lablet schema`: the config's JSON Schema, as `lablet/schema.json`
//! holds it (C10).

use serde_json::{Map, Value};

use super::harness::Lab;

// Compiled in, so the test reads the same file whatever directory it runs from.
const CHECKED_IN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema.json"));

/// The keywords of JSON Schema 2020-12 and its vocabularies: core,
/// applicator, unevaluated, validation, meta-data, format and content.
const KEYWORDS: &[&str] = &[
    "$schema",
    "$id",
    "$ref",
    "$defs",
    "$comment",
    "$anchor",
    "$dynamicRef",
    "$dynamicAnchor",
    "$vocabulary",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "if",
    "then",
    "else",
    "dependentSchemas",
    "prefixItems",
    "items",
    "contains",
    "properties",
    "patternProperties",
    "additionalProperties",
    "propertyNames",
    "unevaluatedItems",
    "unevaluatedProperties",
    "type",
    "enum",
    "const",
    "multipleOf",
    "maximum",
    "exclusiveMaximum",
    "minimum",
    "exclusiveMinimum",
    "maxLength",
    "minLength",
    "pattern",
    "maxItems",
    "minItems",
    "uniqueItems",
    "maxContains",
    "minContains",
    "maxProperties",
    "minProperties",
    "required",
    "dependentRequired",
    "title",
    "description",
    "default",
    "deprecated",
    "readOnly",
    "writeOnly",
    "examples",
    "format",
    "contentEncoding",
    "contentMediaType",
    "contentSchema",
];

/// The keywords whose value is a schema.
const ONE_SCHEMA: &[&str] = &[
    "not",
    "if",
    "then",
    "else",
    "items",
    "contains",
    "additionalProperties",
    "propertyNames",
    "unevaluatedItems",
    "unevaluatedProperties",
    "contentSchema",
];

/// The keywords whose value is a list of schemas.
const LIST_OF_SCHEMAS: &[&str] = &["allOf", "anyOf", "oneOf", "prefixItems"];

/// The keywords whose value maps names to schemas.
const SCHEMAS_BY_NAME: &[&str] = &[
    "$defs",
    "properties",
    "patternProperties",
    "dependentSchemas",
];

const TYPES: &[&str] = &[
    "null", "boolean", "object", "array", "number", "string", "integer",
];

/// Every fault that keeps `schema`, found at `at`, from being a JSON Schema
/// whose references all name one of `defs`.
fn faults(schema: &Value, at: &str, defs: &Map<String, Value>, found: &mut Vec<String>) {
    let keywords = match schema {
        Value::Bool(_) => return,
        Value::Object(keywords) => keywords,
        other => {
            found.push(format!("{at} is {other}, which is no schema"));
            return;
        }
    };
    for (keyword, value) in keywords {
        let at = format!("{at}/{keyword}");
        if !KEYWORDS.contains(&keyword.as_str()) {
            found.push(format!("{at} is no keyword of JSON Schema"));
        } else if ONE_SCHEMA.contains(&keyword.as_str()) {
            faults(value, &at, defs, found);
        } else if LIST_OF_SCHEMAS.contains(&keyword.as_str()) {
            match value.as_array() {
                Some(schemas) if !schemas.is_empty() => {
                    for (index, schema) in schemas.iter().enumerate() {
                        faults(schema, &format!("{at}/{index}"), defs, found);
                    }
                }
                _ => found.push(format!("{at} is no list of schemas")),
            }
        } else if SCHEMAS_BY_NAME.contains(&keyword.as_str()) {
            match value.as_object() {
                Some(schemas) => {
                    for (name, schema) in schemas {
                        faults(schema, &format!("{at}/{name}"), defs, found);
                    }
                }
                None => found.push(format!("{at} maps no names to schemas")),
            }
        } else {
            match keyword.as_str() {
                "$ref" => {
                    let named = value
                        .as_str()
                        .and_then(|reference| reference.strip_prefix("#/$defs/"));
                    if !named.is_some_and(|name| defs.contains_key(name)) {
                        found.push(format!("{at} names no definition: {value}"));
                    }
                }
                "type" => {
                    let named: Vec<&Value> = match value {
                        Value::Array(types) => types.iter().collect(),
                        one => vec![one],
                    };
                    for named in named {
                        if !named.as_str().is_some_and(|name| TYPES.contains(&name)) {
                            found.push(format!("{at} names no type: {named}"));
                        }
                    }
                }
                "required" => {
                    let properties = keywords.get("properties").and_then(Value::as_object);
                    let names = value.as_array().map(Vec::as_slice).unwrap_or_default();
                    if value.as_array().is_none() {
                        found.push(format!("{at} is no list"));
                    }
                    for name in names {
                        let known = name
                            .as_str()
                            .zip(properties)
                            .is_some_and(|(name, properties)| properties.contains_key(name));
                        if !known {
                            found.push(format!("{at} names no property: {name}"));
                        }
                    }
                }
                "enum" if !value.is_array() => found.push(format!("{at} is no list")),
                _ => {}
            }
        }
    }
}

#[test]
fn schema_prints_the_checked_in_file_which_is_a_json_schema() {
    let lab = Lab::new("schema");

    let run = lab.run(&["schema"]);

    assert_eq!(run.code, Some(0), "{run:?}");
    assert_eq!(run.stderr, "", "{run:?}");
    assert_eq!(run.stdout, CHECKED_IN);
    let schema: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    let defs = schema["$defs"].as_object().unwrap();
    let mut found = Vec::new();
    faults(&schema, "#", defs, &mut found);
    assert_eq!(found, [""; 0]);
}

#[test]
fn a_reader_that_closed_its_end_is_told_on_standard_error_and_nothing_panics() {
    let lab = Lab::new("schema-closed");
    // The read end is gone before lablet starts, so its first write fails
    // whatever the size of a pipe's buffer.
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut command = lab.lablet(&["schema"]);
    command.stdout(writer).stderr(std::process::Stdio::piped());

    let finished = command.output().unwrap();

    let stderr = String::from_utf8(finished.stderr).unwrap();
    assert_eq!(finished.status.code(), Some(0), "{stderr}");
    assert!(
        stderr.starts_with("lablet: standard output couldn't be written: "),
        "{stderr}"
    );
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn a_schema_that_breaks_a_rule_of_json_schema_has_the_fault_found() {
    let defs: Map<String, Value> = [("Run".to_owned(), Value::Bool(true))]
        .into_iter()
        .collect();
    let broken = serde_json::json!({
        "type": ["object", "map"],
        "properties": { "run": { "$ref": "#/$defs/Model" }, "model": 3 },
        "required": ["run", "tools"],
        "oneOf": [],
        "additional": false,
    });

    let mut found = Vec::new();
    faults(&broken, "#", &defs, &mut found);
    found.sort();

    assert_eq!(
        found,
        [
            "#/additional is no keyword of JSON Schema",
            "#/oneOf is no list of schemas",
            "#/properties/model is 3, which is no schema",
            "#/properties/run/$ref names no definition: \"#/$defs/Model\"",
            "#/required names no property: \"tools\"",
            "#/type names no type: \"map\"",
        ]
    );
}
