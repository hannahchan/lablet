//! The `lablet` command's composition root, beside its binary: the
//! command's own object graph, which reads the config and the process's
//! environment, configures the SDK from them, and runs the use case in
//! `lablet-run`. It never depends on the library root, `lablet`: both
//! compose through the kernels under `apps/shared/`, so what a run does
//! doesn't depend on which root started it.

mod clock;
pub mod compose;
