//! The domain's closed sets spelt as the registry spells them: each enum the
//! loop emits, converted exhaustively to the generated enum of the signal
//! that carries it, so a variant added on either side doesn't compile until
//! the other side has it; and the run's [`Join`], from its context.

use lablet_model::{ProviderErrorKind, RunContext, ToolCallEnd, ToolCallStatus, ToolSource};

use super::generated::{
    GenAiClientOperationExceptionExceptionType, Join, LabletChatErrorType,
    LabletExecuteToolErrorType, LabletExecuteToolGenAiToolType, LabletExecuteToolNetworkTransport,
    LabletToolSource, LabletToolStatus,
};
use crate::NetworkTransport;

/// The model nests the registry's values in two levels, so both are matched
/// here and the flattening is what the tool span carries.
impl From<&ToolCallStatus> for LabletToolStatus {
    fn from(status: &ToolCallStatus) -> Self {
        match status {
            ToolCallStatus::Unknown => Self::Unknown,
            ToolCallStatus::MalformedInput => Self::MalformedInput,
            ToolCallStatus::Rejected => Self::Rejected,
            ToolCallStatus::NotRun => Self::NotRun,
            ToolCallStatus::Ran { ended, .. } => match ended {
                ToolCallEnd::Ok => Self::Ok,
                ToolCallEnd::ToolError => Self::ToolError,
                ToolCallEnd::Timeout => Self::Timeout,
                ToolCallEnd::Failed => Self::Failed,
                ToolCallEnd::Cancelled => Self::Cancelled,
            },
        }
    }
}

/// The tool span's `error.type` of a call with `status`: every status but
/// `ok`, which has no error, and `not_run`, which has no span to carry one.
#[must_use]
pub const fn tool_error_type(status: &ToolCallStatus) -> Option<LabletExecuteToolErrorType> {
    match status {
        ToolCallStatus::Unknown => Some(LabletExecuteToolErrorType::Unknown),
        ToolCallStatus::MalformedInput => Some(LabletExecuteToolErrorType::MalformedInput),
        ToolCallStatus::Rejected => Some(LabletExecuteToolErrorType::Rejected),
        ToolCallStatus::NotRun
        | ToolCallStatus::Ran {
            ended: ToolCallEnd::Ok,
            ..
        } => None,
        ToolCallStatus::Ran {
            ended: ToolCallEnd::ToolError,
            ..
        } => Some(LabletExecuteToolErrorType::ToolError),
        ToolCallStatus::Ran {
            ended: ToolCallEnd::Timeout,
            ..
        } => Some(LabletExecuteToolErrorType::Timeout),
        ToolCallStatus::Ran {
            ended: ToolCallEnd::Failed,
            ..
        } => Some(LabletExecuteToolErrorType::Failed),
        ToolCallStatus::Ran {
            ended: ToolCallEnd::Cancelled,
            ..
        } => Some(LabletExecuteToolErrorType::Cancelled),
    }
}

/// An MCP source names its server, which `lablet.tool.source` leaves out.
impl From<&ToolSource> for LabletToolSource {
    fn from(source: &ToolSource) -> Self {
        match source {
            ToolSource::Builtin => Self::Builtin,
            ToolSource::Mcp { .. } => Self::Mcp,
        }
    }
}

/// A built-in tool runs in lablet's own process, and an MCP tool is served
/// from outside it, which is the distinction `gen_ai.tool.type` draws.
impl From<&ToolSource> for LabletExecuteToolGenAiToolType {
    fn from(source: &ToolSource) -> Self {
        match source {
            ToolSource::Builtin => Self::Function,
            ToolSource::Mcp { .. } => Self::Extension,
        }
    }
}

impl From<NetworkTransport> for LabletExecuteToolNetworkTransport {
    fn from(transport: NetworkTransport) -> Self {
        match transport {
            NetworkTransport::Pipe => Self::Pipe,
            NetworkTransport::Tcp => Self::Tcp,
        }
    }
}

/// A failed attempt's class is its span's `error.type`; the span's other
/// class, `cancelled`, is no provider error, so it has no source here.
impl From<ProviderErrorKind> for LabletChatErrorType {
    fn from(kind: ProviderErrorKind) -> Self {
        match kind {
            ProviderErrorKind::Retryable => Self::Retryable,
            ProviderErrorKind::ContextExhausted => Self::ContextExhausted,
            ProviderErrorKind::Auth => Self::Auth,
            ProviderErrorKind::Fatal => Self::Fatal,
            ProviderErrorKind::Malformed => Self::Malformed,
        }
    }
}

/// The same class is the exception record's `exception.type`.
impl From<ProviderErrorKind> for GenAiClientOperationExceptionExceptionType {
    fn from(kind: ProviderErrorKind) -> Self {
        match kind {
            ProviderErrorKind::Retryable => Self::Retryable,
            ProviderErrorKind::ContextExhausted => Self::ContextExhausted,
            ProviderErrorKind::Auth => Self::Auth,
            ProviderErrorKind::Fatal => Self::Fatal,
            ProviderErrorKind::Malformed => Self::Malformed,
        }
    }
}

/// The join keys are the run's: its id, twice, since some backends group by
/// session; the digest of its config; and the labels its request gave it.
impl From<&RunContext> for Join {
    fn from(context: &RunContext) -> Self {
        let run_id = context.run_id.as_str().to_owned();
        Self {
            gen_ai_conversation_id: run_id.clone(),
            lablet_config_digest: context.config_digest.as_str().to_owned(),
            session_id: run_id,
            lablet_experiment_id: context.labels.experiment.clone(),
            lablet_task_id: context.labels.task.clone(),
            lablet_trial: context.labels.trial.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
