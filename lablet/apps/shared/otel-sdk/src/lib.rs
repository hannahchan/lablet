//! The OpenTelemetry SDK as lablet configures it from the config and the
//! environment: the export module, the seam the SDK's environment is read
//! through, the network exporters' settings, the inbound context and the
//! resource. Both roots build an SDK until phase 6c's L3, when the library
//! root stops configuring one, so it's a kernel for now, and then folds
//! into the CLI root, the one place the SDK is left.

pub mod export;
pub mod exports;
pub mod otel_env;
pub mod otlp;
pub mod propagation;
