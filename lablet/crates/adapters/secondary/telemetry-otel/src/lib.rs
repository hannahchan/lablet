//! Secondary adapter: a `RunObserver` that maps run events to OpenTelemetry spans
//! and log records once, with pluggable exporters (OTLP network, OTLP/JSON file).
