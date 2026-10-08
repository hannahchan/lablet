//! Every attribute key these tests read, from the generated modules of the
//! two crates that emit them: the composition root's own, for the root span
//! and the wide event, and `lablet-run`'s, for the join keys and the loop's
//! spans and records. The two modules spell a key they share alike, so the
//! root's are taken whole and `lablet-run`'s are named where the root has
//! none, as far as a test reads them.

pub use lablet::telemetry::key::*;
pub use lablet_run::telemetry::generated::key::{
    EXCEPTION_TYPE, GEN_AI_CONVERSATION_ID, GEN_AI_INPUT_MESSAGES, GEN_AI_OUTPUT_MESSAGES,
    GEN_AI_RESPONSE_MODEL, GEN_AI_SYSTEM_INSTRUCTIONS, GEN_AI_TOOL_CALL_ARGUMENTS,
    GEN_AI_TOOL_CALL_ID, GEN_AI_TOOL_CALL_RESULT, GEN_AI_TOOL_DEFINITIONS, GEN_AI_TOOL_NAME,
    GEN_AI_TOOL_TYPE, LABLET_ATTEMPT, LABLET_CHAT_KEYS, LABLET_CHAT_REQUIRED, LABLET_CONFIG_DIGEST,
    LABLET_EXECUTE_TOOL_KEYS, LABLET_EXECUTE_TOOL_REQUIRED, LABLET_EXPERIMENT_ID,
    LABLET_RETRY_WILL_RETRY, LABLET_TASK_ID, LABLET_TOOL_INPUT_BYTES, LABLET_TOOL_IS_ERROR,
    LABLET_TOOL_OUTPUT_BYTES, LABLET_TOOL_OUTPUT_TRUNCATED, LABLET_TOOL_SOURCE, LABLET_TOOL_STATUS,
    LABLET_TRIAL, LABLET_TURN, SESSION_ID,
};

/// The name of the wide event and the operation the root span is of: the
/// composition root's generated structs hold both, and its library keeps
/// those structs to itself.
pub const WIDE_EVENT: &str = "lablet.run";
pub const INVOKE_AGENT: &str = "invoke_agent";

/// The resource keys lablet sets itself, as the resource conventions name
/// them; no signal's attribute, so no generated module declares them.
pub const SERVICE_NAME: &str = "service.name";
pub const SERVICE_VERSION: &str = "service.version";
