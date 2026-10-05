//! Every attribute key these tests read, from the generated modules of the
//! two crates that emit them: the composition root's own, for the root span
//! and the wide event, and `lablet-run`'s, for the join keys and the loop's
//! spans and records. The two modules spell a key they share alike, so the
//! root's are taken whole and `lablet-run`'s are named where the root has
//! none, as far as a test reads them.

pub use lablet::telemetry::generated::key::*;
pub use lablet_run::telemetry::generated::key::{
    EXCEPTION_TYPE, GEN_AI_CONVERSATION_ID, GEN_AI_INPUT_MESSAGES, GEN_AI_OUTPUT_MESSAGES,
    GEN_AI_RESPONSE_ID, GEN_AI_RESPONSE_MODEL, GEN_AI_SYSTEM_INSTRUCTIONS,
    GEN_AI_TOOL_CALL_ARGUMENTS, GEN_AI_TOOL_CALL_ID, GEN_AI_TOOL_CALL_RESULT,
    GEN_AI_TOOL_DEFINITIONS, GEN_AI_TOOL_DESCRIPTION, GEN_AI_TOOL_NAME, GEN_AI_TOOL_TYPE,
    JSONRPC_REQUEST_ID, LABLET_ATTEMPT, LABLET_CHAT_PURPOSE, LABLET_CONFIG_DIGEST,
    LABLET_EXPERIMENT_ID, LABLET_REQUEST_BYTES, LABLET_RETRY_WILL_RETRY, LABLET_TASK_ID,
    LABLET_TOOL_INPUT_BYTES, LABLET_TOOL_IS_ERROR, LABLET_TOOL_OUTPUT_BYTES,
    LABLET_TOOL_OUTPUT_ORIGINAL_BYTES, LABLET_TOOL_OUTPUT_TRUNCATED, LABLET_TOOL_SOURCE,
    LABLET_TOOL_STATUS, LABLET_TRIAL, LABLET_TURN, MCP_METHOD_NAME, MCP_PROTOCOL_VERSION,
    MCP_SESSION_ID, NETWORK_TRANSPORT, RPC_RESPONSE_STATUS_CODE, SESSION_ID,
};

/// The resource keys lablet sets itself, as the resource conventions name
/// them; no signal's attribute, so no generated module declares them.
pub const SERVICE_NAME: &str = "service.name";
pub const SERVICE_VERSION: &str = "service.version";
/// The SDK's own version on the resource, which the golden comparison
/// normalises out.
pub const TELEMETRY_SDK_VERSION: &str = "telemetry.sdk.version";
