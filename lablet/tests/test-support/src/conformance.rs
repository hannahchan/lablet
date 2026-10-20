//! The shared `ToolExecutor` conformance cases, pulled in by each adapter that
//! implements the port, the reader of the OTLP/JSON lines lablet exports, a
//! host's providers to hand the library, read back as those lines, and an OTLP
//! receiver in the test's own process, so a network scenario needs no
//! collector. Behind the `conformance` feature, since a test that builds only
//! the loop needs none of the receiver's servers and TLS.

pub mod executor;
pub mod host;
pub mod otlp;
pub mod receiver;
