//! The `lablet` command's composition root, beside its binary: the
//! command's own object graph, which fills the config from the process's
//! environment in its input module, the one place lablet reads it, takes
//! the environment every process a run starts begins from, configures the
//! OpenTelemetry SDK, extracts the inbound context, and runs the use case
//! in `lablet-run` with its SDK's tracer and logger, as a host of the
//! library hands its own in. It holds the SDK, its export module and the
//! seam that still reads the OpenTelemetry variables, and never depends on
//! the library root, `lablet`: both compose through the kernels under
//! `apps/shared/`, so what a run does doesn't depend on which root started
//! it.

pub mod compose;
mod export;
mod exports;
pub mod input;
mod otel_env;
mod otlp;
mod propagation;
