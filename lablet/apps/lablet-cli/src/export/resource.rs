//! The resource every export describes itself with: the service, the SDK,
//! and what the config's `telemetry.resource` adds.

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::resource::TelemetryResourceDetector;

/// The name of the service when nothing names it.
const LABLET: &str = "lablet";

/// The resource keys lablet sets itself, as the resource conventions name
/// them. They're the service's, not a signal's: the registry's attributes
/// are named by the crates that emit them.
const SERVICE_NAME: &str = "service.name";
const SERVICE_VERSION: &str = "service.version";

/// The resource of lablet `version`: the config's `attributes`, in their
/// order, with `service.name` the first of them that names one and
/// `lablet` when none does, `service.version` lablet's and the
/// `telemetry.sdk.*` keys the SDK's, whichever names them. Nothing of the
/// process environment is read into it: `Resource::builder` would detect
/// `OTEL_SERVICE_NAME` and `OTEL_RESOURCE_ATTRIBUTES`, so the resource is
/// built from nothing.
#[must_use]
pub fn resource(version: &str, attributes: &[(String, String)]) -> Resource {
    // A later source wins a key an earlier one set, and the attributes hold
    // each key once, the first written, so the config's `service.name`
    // comes before lablet's name only by being stated.
    Resource::builder_empty()
        .with_attribute(KeyValue::new(SERVICE_NAME, LABLET))
        .with_attributes(
            attributes
                .iter()
                .map(|(key, value)| KeyValue::new(key.clone(), value.clone())),
        )
        .with_detector(Box::new(TelemetryResourceDetector))
        .with_attribute(KeyValue::new(SERVICE_VERSION, version.to_owned()))
        .build()
}

#[cfg(test)]
mod tests;
