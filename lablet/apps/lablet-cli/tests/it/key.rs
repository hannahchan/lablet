//! Every attribute key these tests read, from `lablet-run`'s generated
//! module, where every span and record of a run is declared, the root span
//! and the wide event among them.

pub use lablet_run::telemetry::generated::key::*;

/// The name of the wide event and the operation the root span is of: the
/// generated structs hold both, and the run keeps those structs to itself.
pub const WIDE_EVENT: &str = "lablet.run";
pub const INVOKE_AGENT: &str = "invoke_agent";

/// The resource keys lablet sets itself, as the resource conventions name
/// them; no signal's attribute, so no generated module declares them.
pub const SERVICE_NAME: &str = "service.name";
pub const SERVICE_VERSION: &str = "service.version";
/// The SDK's own version on the resource, which the golden comparison
/// normalises out.
pub const TELEMETRY_SDK_VERSION: &str = "telemetry.sdk.version";
