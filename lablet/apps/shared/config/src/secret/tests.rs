use serde_json::json;

use super::*;

const VALUE: &str = "sk-ant-0123456789abcdef0123456789";

#[test]
fn a_secret_is_shown_and_written_as_the_marker_and_holds_its_value() {
    let secret = Secret::new(VALUE);

    assert_eq!(format!("{secret:?}"), "[secret]");
    assert_eq!(secret.to_string(), "[secret]");
    assert_eq!(serde_json::to_value(&secret).unwrap(), json!("[secret]"));
    assert_eq!(secret.expose_secret(), VALUE);
    assert_eq!(secret, Secret::new(VALUE.to_owned()));
    assert_ne!(secret, Secret::new("another"));
}

#[test]
fn a_secret_is_read_from_text_and_the_marker_is_refused() {
    let read: Secret = serde_json::from_value(json!(VALUE)).unwrap();
    assert_eq!(read, Secret::new(VALUE));

    let refused = serde_json::from_value::<Secret>(json!("[secret]")).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "it's how lablet writes a secret set in code; set it with Secret::new"
    );
    serde_json::from_value::<Secret>(json!(7)).unwrap_err();
}

#[test]
fn the_schema_of_a_secret_is_a_string() {
    assert_eq!(
        schemars::schema_for!(Secret).to_value(),
        schemars::schema_for!(String).to_value()
    );
}
