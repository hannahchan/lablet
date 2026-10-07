//! The resource every export describes itself with: the service, the SDK,
//! and what the composer and the environment add.

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::resource::TelemetryResourceDetector;

use crate::otel_env;

/// The name of the service when nothing names it.
const LABLET: &str = "lablet";

/// The resource keys lablet sets itself, as the resource conventions name
/// them. They're the service's, not a signal's: the registry's attributes
/// are named by the crates that emit them.
const SERVICE_NAME: &str = "service.name";
const SERVICE_VERSION: &str = "service.version";

/// The resource of lablet `version`: the composer's own `attributes` over
/// what the environment's `context` holds, `OTEL_SERVICE_NAME` over the
/// pairs of `OTEL_RESOURCE_ATTRIBUTES`. `service.name` is the first of the
/// composer's, `OTEL_SERVICE_NAME` and the pairs' that names one, and
/// `lablet` when none does. `service.version` is lablet's and the
/// `telemetry.sdk.*` keys are the SDK's, whichever names them.
pub(crate) fn resource(
    version: &str,
    attributes: Vec<(String, String)>,
    context: &otel_env::Context,
) -> Resource {
    // A later source wins a key an earlier one set. The environment's pairs
    // are read by the seam rather than by the SDK's `EnvResourceDetector`,
    // which doesn't decode them, and whose output decoded again would
    // decode twice once the SDK decodes too. `Resource::builder` would put
    // `OTEL_SERVICE_NAME` beneath the pairs, the reverse of the
    // specification's order.
    let environment = context
        .resource_attributes
        .iter()
        .map(|(key, value)| KeyValue::new(key.clone(), value.clone()));
    let service_name = context
        .service_name
        .iter()
        .map(|name| KeyValue::new(SERVICE_NAME, name.clone()));
    Resource::builder_empty()
        .with_attribute(KeyValue::new(SERVICE_NAME, LABLET))
        .with_attributes(environment)
        .with_attributes(service_name)
        .with_attributes(
            attributes
                .into_iter()
                .map(|(key, value)| KeyValue::new(key, value)),
        )
        .with_detector(Box::new(TelemetryResourceDetector))
        .with_attribute(KeyValue::new(SERVICE_VERSION, version.to_owned()))
        .build()
}

#[cfg(test)]
mod tests;
