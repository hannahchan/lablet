//! The `lablet` command's composition root, beside its binary: the
//! command's own object graph, which reads the config and the process's
//! environment, configures the OpenTelemetry SDK from them, extracts the
//! inbound context, and runs the use case in `lablet-run` with its SDK's
//! tracer and logger, as a host of the library hands its own in. It holds
//! the SDK, its export module and the seam that reads its environment, and
//! never depends on the library root, `lablet`: both compose through the
//! kernels under `apps/shared/`, so what a run does doesn't depend on which
//! root started it.

pub mod compose;
mod export;
mod exports;
mod otel_env;
mod otlp;
mod propagation;
