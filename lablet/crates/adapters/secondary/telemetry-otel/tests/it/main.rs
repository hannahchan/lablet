//! Integration tests of `lablet-telemetry-otel`, through its public surface
//! only: the loop runs a script, the observer exports the run to a file,
//! and the file is read back.

#[cfg(test)]
mod content;
#[cfg(test)]
mod file;
#[cfg(test)]
mod harness;
#[cfg(test)]
mod spans;
