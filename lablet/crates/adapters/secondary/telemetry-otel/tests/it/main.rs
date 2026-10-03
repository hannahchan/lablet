//! Integration tests of `lablet-telemetry-otel`, through its public surface
//! only: the loop runs a script, the observer exports the run to a file or
//! to the in-process receiver, and what it exported is read back.

#[cfg(test)]
mod cancelled;
#[cfg(test)]
mod conformance;
#[cfg(test)]
mod content;
#[cfg(test)]
mod file;
#[cfg(test)]
mod harness;
#[cfg(test)]
mod network;
#[cfg(test)]
mod resource;
#[cfg(test)]
mod spans;
#[cfg(test)]
mod wide;
