//! The resource, from the composer's attributes and an environment the
//! tests state, read through the seam as a build reads it.

use std::collections::BTreeMap;
use std::ffi::OsString;

use opentelemetry::{Key, Value};

use super::*;
use crate::otel_env::OtelEnv;

const VERSION: &str = "0.1.0-test";

/// The resource of an environment that holds `held` and nothing else, with
/// the composer's `attributes`.
fn resource_of(held: &[(&str, &str)], attributes: &[(&str, &str)]) -> BTreeMap<String, String> {
    let env = |name: &str| {
        held.iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| OsString::from(value))
    };
    let context = OtelEnv::read(&env).context;
    let attributes = attributes
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    resource(VERSION, attributes, &context)
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn get<'a>(resource: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    resource.get(key).map(String::as_str)
}

#[test]
fn otel_service_name_names_the_service_over_the_resource_attributes() {
    let resource = resource_of(
        &[
            ("OTEL_SERVICE_NAME", "checkout"),
            ("OTEL_RESOURCE_ATTRIBUTES", "service.name=from-the-pairs"),
        ],
        &[],
    );

    assert_eq!(get(&resource, SERVICE_NAME), Some("checkout"));
}

#[test]
fn the_configs_service_name_wins_and_lablet_is_the_name_only_when_none_gives_one() {
    let everything = [
        ("OTEL_SERVICE_NAME", "checkout"),
        ("OTEL_RESOURCE_ATTRIBUTES", "service.name=from-the-pairs"),
    ];
    let composers = [(SERVICE_NAME, "composer")];

    assert_eq!(
        get(&resource_of(&everything, &composers), SERVICE_NAME),
        Some("composer")
    );
    assert_eq!(
        get(&resource_of(&everything[1..], &[]), SERVICE_NAME),
        Some("from-the-pairs")
    );
    assert_eq!(get(&resource_of(&[], &[]), SERVICE_NAME), Some(LABLET));
    assert_eq!(
        get(
            &resource_of(&[("OTEL_SERVICE_NAME", "")], &[]),
            SERVICE_NAME
        ),
        Some(LABLET),
        "an empty variable names nothing"
    );
}

#[test]
fn the_composers_attributes_win_a_key_the_environment_names_too() {
    let resource = resource_of(
        &[(
            "OTEL_RESOURCE_ATTRIBUTES",
            "team=a,deployment.environment=test",
        )],
        &[("team", "b")],
    );

    assert_eq!(get(&resource, "team"), Some("b"));
    assert_eq!(get(&resource, "deployment.environment"), Some("test"));
}

#[test]
fn resource_attribute_values_are_percent_decoded() {
    let resource = resource_of(
        &[(
            "OTEL_RESOURCE_ATTRIBUTES",
            "team=a%2Cb,owner=x%3Dy,note=%2520",
        )],
        &[],
    );

    assert_eq!(get(&resource, "team"), Some("a,b"));
    assert_eq!(get(&resource, "owner"), Some("x=y"));
    assert_eq!(get(&resource, "note"), Some("%20"), "decoded once");
}

#[test]
fn a_resource_attributes_value_that_does_not_decode_adds_nothing_to_the_resource() {
    let resource = resource_of(
        &[(
            "OTEL_RESOURCE_ATTRIBUTES",
            "team=evals,service.name=named,broken=%zz",
        )],
        &[],
    );

    assert_eq!(get(&resource, "team"), None);
    assert_eq!(get(&resource, SERVICE_NAME), Some(LABLET));
}

/// A Rust application sets these in code, after the environment, so no
/// variable and no config key restates them.
#[test]
fn service_version_and_the_sdks_keys_stay_lablets() {
    let claims = [
        (SERVICE_VERSION, "9.9.9"),
        ("telemetry.sdk.name", "claimed"),
        ("telemetry.sdk.language", "claimed"),
        ("telemetry.sdk.version", "claimed"),
    ];
    let pairs = claims
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    let sdk = Resource::builder_empty()
        .with_detector(Box::new(TelemetryResourceDetector))
        .build();

    for resource in [
        resource_of(&[("OTEL_RESOURCE_ATTRIBUTES", &pairs)], &[]),
        resource_of(&[], &claims),
    ] {
        assert_eq!(get(&resource, SERVICE_VERSION), Some(VERSION));
        for (key, value) in &sdk {
            assert_eq!(
                resource.get(key.as_str()),
                Some(&value.to_string()),
                "{key}"
            );
        }
    }
    assert_eq!(
        sdk.get(&Key::new("telemetry.sdk.language")),
        Some(Value::from("rust"))
    );
}
