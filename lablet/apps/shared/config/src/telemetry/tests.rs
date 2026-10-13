use serde_json::{Value, json};

use super::*;
use crate::{Config, ConfigError, Format, RawConfig};

fn yaml(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

fn refused(text: &str) -> ConfigError {
    Config::from_str(text, Format::Yaml).unwrap_err()
}

/// The telemetry section of the config `text` holds, as it serialises.
fn written(text: &str) -> Value {
    serde_json::to_value(&yaml(text).telemetry).unwrap()
}

#[test]
fn propagators_are_read_as_a_list_or_as_the_comma_text_in_any_case_once_each() {
    let list = yaml("telemetry: { propagators: [b3, tracecontext, b3] }");
    let text = yaml("telemetry: { propagators: ' B3 ,TraceContext,b3, ' }");

    let names = [Propagator::B3, Propagator::TraceContext];
    assert_eq!(list.telemetry.propagators.as_ref().unwrap().names(), names);
    assert_eq!(text.telemetry.propagators.as_ref().unwrap().names(), names);
    assert_eq!(
        written("telemetry: { propagators: 'b3multi,baggage' }")["propagators"],
        json!(["b3multi", "baggage"]),
        "written as a list whichever form was read"
    );
    assert_eq!(yaml("").telemetry.propagators, None);
    assert_eq!(
        Propagators::specified().names(),
        [Propagator::TraceContext, Propagator::Baggage]
    );
}

#[test]
fn propagators_none_alone_names_no_propagator_and_beside_another_is_refused() {
    for text in ["none", " NONE ", "[none]"] {
        let config = yaml(&format!("telemetry: {{ propagators: {text} }}"));
        assert_eq!(
            config.telemetry.propagators.as_ref().unwrap().names(),
            Propagators::none().names(),
            "{text}"
        );
    }
    assert_eq!(
        written("telemetry: { propagators: none }")["propagators"],
        json!([])
    );

    assert_eq!(
        refused("telemetry: { propagators: 'none,b3' }").to_string(),
        "telemetry.propagators (line 1): \"none,b3\" is refused: `none` stands alone, and names no \
         propagator"
    );
    assert_eq!(
        refused("telemetry: { propagators: [tracecontext, xray] }").to_string(),
        "telemetry.propagators (line 1): [\"tracecontext\",\"xray\"] is refused: the accepted \
         values are `tracecontext`, `baggage`, `b3`, `b3multi` and `none`"
    );
    assert_eq!(
        refused("telemetry: { propagators: 7 }").to_string(),
        "telemetry.propagators (line 1): 7 is refused: expected a list of propagators, or a comma \
         text, of `tracecontext`, `baggage`, `b3`, `b3multi` and `none`"
    );
}

#[test]
fn headers_are_read_as_a_map_or_as_the_k_v_text_with_each_value_decoded() {
    let map = yaml("telemetry: { otlp: { headers: { Authorization: 'Bearer t', X-Env: '1' } } }");
    let text = yaml(
        "telemetry: { otlp: { headers: 'authorization=Bearer%20t, x-env=1,,=dropped,empty=' } }",
    );

    let from_map = map.telemetry.otlp.headers.unwrap();
    assert_eq!(from_map.len(), 2);
    assert_eq!(
        from_map.get("Authorization"),
        Some(&Secret::new("Bearer t"))
    );
    assert_eq!(from_map.text(), None);
    let from_text = text.telemetry.otlp.headers.unwrap();
    assert_eq!(
        from_text
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["authorization", "x-env"],
        "a pair with no name or an empty value is left out"
    );
    assert_eq!(
        from_text.get("authorization"),
        Some(&Secret::new("Bearer t")),
        "percent-decoded"
    );
    assert_eq!(
        from_text.text(),
        Some(&Secret::new(
            "authorization=Bearer%20t, x-env=1,,=dropped,empty="
        ))
    );
    assert_eq!(
        written("telemetry: { otlp: { headers: 'a=b' } }")["otlp"]["headers"],
        json!({ "a": "[secret]" }),
        "written as a map of markers whichever form was read"
    );
    assert_eq!(
        yaml("telemetry: { otlp: { headers: {} } }")
            .telemetry
            .otlp
            .headers
            .map(|headers| headers.is_empty()),
        Some(true)
    );
    assert_eq!(
        refused("telemetry: { otlp: { headers: [a] } }").to_string(),
        "telemetry.otlp.headers (line 1): its value is refused: expected a mapping of header names \
         to values, or `k=v` text",
        "a credential's value is never shown, whatever shape it has"
    );
}

#[test]
fn the_resource_is_read_as_a_map_or_as_the_k_v_text_and_the_first_key_wins() {
    let map = yaml("telemetry: { resource: { team: evals, 'service.name': svc } }");
    let text = yaml(
        "telemetry: { resource: 'service.name=first,team=a%20b,service.name=second,,team=c' }",
    );

    assert_eq!(map.telemetry.resource.get("service.name"), Some("svc"));
    assert_eq!(
        text.telemetry.resource.pairs(),
        [
            ("service.name".to_owned(), "first".to_owned()),
            ("team".to_owned(), "a b".to_owned()),
        ]
    );
    assert_eq!(
        written("telemetry: { resource: 'b=2,a=1' }")["resource"],
        json!({ "b": "2", "a": "1" }),
        "written as a map in the order written"
    );
    assert!(yaml("").telemetry.resource.is_empty());
    assert!(
        yaml("telemetry: { resource: null }")
            .telemetry
            .resource
            .is_empty()
    );
    let unset = RawConfig::from_str("telemetry: { resource: '${R:-}' }", Format::Yaml)
        .unwrap()
        .substituted(|_| None)
        .unwrap()
        .config()
        .unwrap();
    assert!(unset.telemetry.resource.is_empty());
    assert_eq!(
        refused("telemetry: { resource: [a] }").to_string(),
        "telemetry.resource (line 1): [\"a\"] is refused: expected a mapping of attribute keys to \
         values, or `k=v` text"
    );
}

#[test]
fn a_null_entry_of_a_map_form_is_left_out() {
    let config = RawConfig::from_str(
        "telemetry: { otlp: { headers: { A: '${A:-}', B: 'b' } }, resource: { X: '${X:-}', Y: y } }",
        Format::Yaml,
    )
    .unwrap()
    .substituted(|_| None)
    .unwrap()
    .config()
    .unwrap();

    let headers = config.telemetry.otlp.headers.unwrap();
    assert_eq!(
        headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["B"]
    );
    assert_eq!(
        config.telemetry.resource.pairs(),
        [("Y".to_owned(), "y".to_owned())]
    );
}

#[test]
fn enabled_is_a_key_no_setting_has_and_the_tls_section_takes_three_paths() {
    let refusal = refused("telemetry: { otlp: { enabled: false } }").to_string();
    assert_eq!(
        refusal,
        "telemetry.otlp.enabled (line 1) is refused: no setting has the key; the keys beside it \
         are `endpoint`, `protocol`, `headers`, `tls`"
    );

    let tls = yaml(
        "telemetry: { otlp: { tls: { ca_file: ca.pem, client_cert: c.pem, client_key: c.key } } }",
    )
    .telemetry
    .otlp
    .tls;
    assert_eq!(tls.ca_file, Some("ca.pem".into()));
    assert_eq!(tls.client_cert, Some("c.pem".into()));
    assert_eq!(tls.client_key, Some("c.key".into()));
    assert_eq!(
        refused("telemetry: { otlp: { tls: { certificate: ca.pem } } }").to_string(),
        "telemetry.otlp.tls.certificate (line 1) is refused: no setting has the key; the keys \
         beside it are `ca_file`, `client_cert`, `client_key`"
    );
}

/// `node` with a `$ref` to one of the schema's definitions followed.
fn defined<'a>(schema: &'a Value, node: &'a Value) -> &'a Value {
    match node["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix("#/$defs/"))
    {
        Some(name) => &schema["$defs"][name],
        None => node,
    }
}

/// Every form a setting's schema offers, flattening each `anyOf`,
/// however schemars nests an `Option`'s null among them.
fn forms(setting: &Value) -> Vec<Value> {
    match setting.get("anyOf").and_then(Value::as_array) {
        Some(alternatives) => alternatives.iter().flat_map(forms).collect(),
        None => vec![setting.clone()],
    }
}

/// The schema says each of the two-form settings takes text or its
/// structured form, and `enabled` is gone.
#[test]
fn the_schema_gives_the_two_forms_of_the_propagators_the_headers_and_the_resource() {
    let schema = crate::schema();
    let telemetry = defined(&schema, &schema["properties"]["telemetry"]);
    let otlp = &defined(&schema, &telemetry["properties"]["otlp"])["properties"];

    assert!(otlp.is_object(), "{telemetry}");
    assert!(otlp.get("enabled").is_none(), "{otlp}");
    let tls = defined(&schema, &otlp["tls"]);
    assert!(tls["properties"]["client_key"].is_object(), "{tls}");
    let types = |setting: &Value| -> Vec<Value> {
        forms(setting)
            .iter()
            .map(|form| form["type"].clone())
            .collect()
    };
    assert_eq!(
        types(&otlp["headers"]),
        [json!("string"), json!("object"), json!("null")]
    );
    assert_eq!(
        types(&telemetry["properties"]["resource"]),
        [json!("string"), json!("object"), json!("null")]
    );
    let propagators = forms(&telemetry["properties"]["propagators"]);
    assert_eq!(propagators[0]["type"], json!("string"));
    assert_eq!(propagators[1]["type"], json!("array"));
    let items = &propagators[1]["items"];
    let names: Vec<Value> = defined(&schema, items)["oneOf"]
        .as_array()
        .unwrap_or_else(|| panic!("{items}"))
        .iter()
        .map(|member| member["const"].clone())
        .collect();
    assert_eq!(
        names,
        [
            json!("tracecontext"),
            json!("baggage"),
            json!("b3"),
            json!("b3multi")
        ]
    );
    assert_eq!(propagators[2]["type"], json!("null"));
}
