//! The resource, from the config's attributes alone.

use std::collections::BTreeMap;

use opentelemetry::{Key, Value};

use super::*;

const VERSION: &str = "0.1.0-test";

/// The resource of the config's `attributes`.
fn resource_of(attributes: &[(&str, &str)]) -> BTreeMap<String, String> {
    let attributes: Vec<(String, String)> = attributes
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    resource(VERSION, &attributes)
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn get<'a>(resource: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    resource.get(key).map(String::as_str)
}

#[test]
fn the_configs_service_name_names_the_service_and_lablet_is_the_name_only_when_none_does() {
    assert_eq!(
        get(&resource_of(&[(SERVICE_NAME, "composer")]), SERVICE_NAME),
        Some("composer")
    );
    assert_eq!(get(&resource_of(&[]), SERVICE_NAME), Some(LABLET));
    assert_eq!(
        get(&resource_of(&[("team", "evals")]), SERVICE_NAME),
        Some(LABLET)
    );
}

#[test]
fn the_configs_attributes_are_the_resource_and_nothing_else_is() {
    let resource = resource_of(&[("team", "b"), ("deployment.environment", "test")]);

    assert_eq!(get(&resource, "team"), Some("b"));
    assert_eq!(get(&resource, "deployment.environment"), Some("test"));
    let keys: Vec<&str> = resource.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "deployment.environment",
            "service.name",
            "service.version",
            "team",
            "telemetry.sdk.language",
            "telemetry.sdk.name",
            "telemetry.sdk.version",
        ]
    );
}

/// A Rust application sets these in code, after the environment, so no
/// config key restates them.
#[test]
fn service_version_and_the_sdks_keys_stay_lablets() {
    let claims = [
        (SERVICE_VERSION, "9.9.9"),
        ("telemetry.sdk.name", "claimed"),
        ("telemetry.sdk.language", "claimed"),
        ("telemetry.sdk.version", "claimed"),
    ];
    let sdk = Resource::builder_empty()
        .with_detector(Box::new(TelemetryResourceDetector))
        .build();

    let resource = resource_of(&claims);
    assert_eq!(get(&resource, SERVICE_VERSION), Some(VERSION));
    for (key, value) in &sdk {
        assert_eq!(
            resource.get(key.as_str()),
            Some(&value.to_string()),
            "{key}"
        );
    }
    assert_eq!(
        sdk.get(&Key::new("telemetry.sdk.language")),
        Some(Value::from("rust"))
    );
}
