//! The resource every export describes itself with: the service, the SDK,
//! and what the composer and the environment add.

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::resource::{EnvResourceDetector, TelemetryResourceDetector};

use crate::otel_env;

/// The name of the service.
const LABLET: &str = "lablet";

/// The resource keys lablet sets itself, as the resource conventions name
/// them. They're the service's, not a signal's: the registry's attributes
/// are named by the crates that emit them.
const SERVICE_NAME: &str = "service.name";
const SERVICE_VERSION: &str = "service.version";

/// The resource of lablet `version`: the composer's own `attributes`
/// beside `service.name`, `service.version` and what the SDK says of
/// itself, over those `OTEL_RESOURCE_ATTRIBUTES` holds when it's set. A key
/// the environment names too takes the composer's value, and a key of
/// lablet's own keeps lablet's value whichever names it.
pub(crate) fn resource(
    version: &str,
    attributes: Vec<(String, String)>,
    _context: &otel_env::Context,
) -> Resource {
    // A later source wins a key an earlier one set. So the environment's
    // attributes are defaults beneath the composer's, as every `OTEL_*`
    // variable lablet inherits is, and what the SDK says of itself and
    // lablet's own two keys come last, so neither source can rename the
    // service or misstate the SDK. `OTEL_SERVICE_NAME` goes unread: the
    // detector that reads it isn't one of these.
    Resource::builder_empty()
        .with_detector(Box::new(EnvResourceDetector::new()))
        .with_attributes(
            attributes
                .into_iter()
                .map(|(key, value)| KeyValue::new(key, value)),
        )
        .with_detector(Box::new(TelemetryResourceDetector))
        .with_attributes([
            KeyValue::new(SERVICE_NAME, LABLET),
            KeyValue::new(SERVICE_VERSION, version.to_owned()),
        ])
        .build()
}
