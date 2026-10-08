//! Every attribute key these tests read, from `lablet::telemetry::key`,
//! which re-exports `lablet-run`'s generated module, where every span and
//! record of a run is declared, the root span and the wide event among
//! them.

pub use lablet::telemetry::key::*;

/// The name of the wide event and the operation the root span is of: the
/// generated structs hold both, and the run keeps those structs to itself.
pub const WIDE_EVENT: &str = "lablet.run";
pub const INVOKE_AGENT: &str = "invoke_agent";

/// The resource key a host's resource names its service with, as the
/// resource conventions name it; no signal's attribute, so no generated
/// module declares it.
pub const SERVICE_NAME: &str = "service.name";
