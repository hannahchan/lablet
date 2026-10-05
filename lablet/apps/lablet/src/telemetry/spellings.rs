//! The domain's closed sets spelt as the registry spells them: each enum the
//! composition root emits, converted exhaustively to the generated enum of
//! the signal that carries it, so a variant added on either side doesn't
//! compile until the other side has it.

use lablet_model::{CacheScope, CompletionMode, McpLifetime, ProviderApi, StopReason};

use super::generated::{
    LabletInvokeAgentErrorType, LabletMcpLifetime, LabletRequestApi, LabletRequestCacheScope,
    LabletRunCompletionMode, LabletRunErrorType, LabletRunStopReason,
};

impl From<StopReason> for LabletRunStopReason {
    fn from(reason: StopReason) -> Self {
        match reason {
            StopReason::Completed => Self::Completed,
            StopReason::EndedWithoutCompletion => Self::EndedWithoutCompletion,
            StopReason::MaxTurns => Self::MaxTurns,
            StopReason::Timeout => Self::Timeout,
            StopReason::MaxTotalTokens => Self::MaxTotalTokens,
            StopReason::OutputTruncated => Self::OutputTruncated,
            StopReason::ContextExhausted => Self::ContextExhausted,
            StopReason::RetriesExhausted => Self::RetriesExhausted,
            StopReason::InvalidCallsExhausted => Self::InvalidCallsExhausted,
            StopReason::Cancelled => Self::Cancelled,
            StopReason::ProviderError => Self::ProviderError,
            StopReason::Refused => Self::Refused,
        }
    }
}

/// The root span's `error.type` of a run that stopped for `reason`: the
/// reason itself, for every reason but `completed`, which is no error.
pub(crate) const fn invoke_agent_error_type(
    reason: StopReason,
) -> Option<LabletInvokeAgentErrorType> {
    match reason {
        StopReason::Completed => None,
        StopReason::EndedWithoutCompletion => {
            Some(LabletInvokeAgentErrorType::EndedWithoutCompletion)
        }
        StopReason::MaxTurns => Some(LabletInvokeAgentErrorType::MaxTurns),
        StopReason::Timeout => Some(LabletInvokeAgentErrorType::Timeout),
        StopReason::MaxTotalTokens => Some(LabletInvokeAgentErrorType::MaxTotalTokens),
        StopReason::OutputTruncated => Some(LabletInvokeAgentErrorType::OutputTruncated),
        StopReason::ContextExhausted => Some(LabletInvokeAgentErrorType::ContextExhausted),
        StopReason::RetriesExhausted => Some(LabletInvokeAgentErrorType::RetriesExhausted),
        StopReason::InvalidCallsExhausted => {
            Some(LabletInvokeAgentErrorType::InvalidCallsExhausted)
        }
        StopReason::Cancelled => Some(LabletInvokeAgentErrorType::Cancelled),
        StopReason::ProviderError => Some(LabletInvokeAgentErrorType::ProviderError),
        StopReason::Refused => Some(LabletInvokeAgentErrorType::Refused),
    }
}

/// The wide event's `error.type` of a run that stopped for `reason`, which
/// is the root span's, spelt for the event's own enum.
pub(crate) const fn run_error_type(reason: StopReason) -> Option<LabletRunErrorType> {
    match reason {
        StopReason::Completed => None,
        StopReason::EndedWithoutCompletion => Some(LabletRunErrorType::EndedWithoutCompletion),
        StopReason::MaxTurns => Some(LabletRunErrorType::MaxTurns),
        StopReason::Timeout => Some(LabletRunErrorType::Timeout),
        StopReason::MaxTotalTokens => Some(LabletRunErrorType::MaxTotalTokens),
        StopReason::OutputTruncated => Some(LabletRunErrorType::OutputTruncated),
        StopReason::ContextExhausted => Some(LabletRunErrorType::ContextExhausted),
        StopReason::RetriesExhausted => Some(LabletRunErrorType::RetriesExhausted),
        StopReason::InvalidCallsExhausted => Some(LabletRunErrorType::InvalidCallsExhausted),
        StopReason::Cancelled => Some(LabletRunErrorType::Cancelled),
        StopReason::ProviderError => Some(LabletRunErrorType::ProviderError),
        StopReason::Refused => Some(LabletRunErrorType::Refused),
    }
}

impl From<CompletionMode> for LabletRunCompletionMode {
    fn from(mode: CompletionMode) -> Self {
        match mode {
            CompletionMode::Natural => Self::Natural,
            CompletionMode::Explicit => Self::Explicit,
        }
    }
}

impl From<ProviderApi> for LabletRequestApi {
    fn from(api: ProviderApi) -> Self {
        match api {
            ProviderApi::Messages => Self::Messages,
            ProviderApi::Responses => Self::Responses,
            ProviderApi::ChatCompletions => Self::ChatCompletions,
            ProviderApi::Script => Self::Script,
        }
    }
}

impl From<CacheScope> for LabletRequestCacheScope {
    fn from(scope: CacheScope) -> Self {
        match scope {
            CacheScope::Shared => Self::Shared,
            CacheScope::Run => Self::Run,
        }
    }
}

impl From<McpLifetime> for LabletMcpLifetime {
    fn from(lifetime: McpLifetime) -> Self {
        match lifetime {
            McpLifetime::Run => Self::Run,
            McpLifetime::Lablet => Self::Lablet,
        }
    }
}

#[cfg(test)]
mod tests;
