//! Secondary adapter: a `RunObserver` that maps run events to OpenTelemetry spans
//! and log records once, with pluggable exporters (OTLP network, OTLP/JSON file).
//!
//! An empty shell from the phase 0 scaffold; build-plan phase 4 fills it with
//! the file exporter and phase 6 adds the network exporter.
