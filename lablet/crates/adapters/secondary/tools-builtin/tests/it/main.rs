//! Integration tests of `lablet-tools-builtin`, through its public surface
//! only: an executor is built from settings, under a root of the test's
//! own, and called as the loop calls it.
//!
//! The tests leave tokio's clock running, because what `bash` starts takes
//! real time. None waits for a command to end that takes long: a deadline
//! is tens of milliseconds, and the command it cuts short would have slept
//! for a minute.

#[cfg(test)]
mod bash;
#[cfg(test)]
mod conformance;
#[cfg(test)]
mod context;
#[cfg(test)]
mod executor;
#[cfg(test)]
mod files;
#[cfg(test)]
mod harness;
#[cfg(test)]
mod run;
