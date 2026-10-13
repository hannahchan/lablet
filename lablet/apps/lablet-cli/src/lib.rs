//! The `lablet` command's composition root, beside its binary: the
//! command's own object graph, which fills the config from the process's
//! environment in its input module, the one place lablet reads it, takes
//! the environment every process a run starts begins from, configures the
//! OpenTelemetry SDK from the config's `telemetry` section, extracts the
//! inbound context through the propagators the config names, and runs the
//! use case in `lablet-run` with its SDK's tracer and logger, as a host of
//! the library hands its own in. It holds the SDK, its export module and
//! the seam that read the OpenTelemetry variables, which no production
//! code calls, and never depends on the library root, `lablet`: both
//! compose through the kernels under `apps/shared/`, so what a run does
//! doesn't depend on which root started it.

pub mod compose;
mod export;
pub mod exports;
pub mod input;
pub mod otel_env;
mod otlp;
mod propagation;
